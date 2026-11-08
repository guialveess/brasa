//! [`LoadRun`], a load test in progress: one OS thread per virtual user, a shared metrics
//! aggregator, and the controls the UI needs. See the crate docs for the concurrency model.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use brasa_core::Environment;
use brasa_http::HttpError;
use brasa_runner::{FailedStage, RunResult, Runner, SessionEnv};

use crate::config::LoadConfig;
use crate::metrics::{Aggregator, LoadSnapshot, Sample};
use crate::samples::SampleLog;
use crate::status::StatusKey;

/// How often a waiting thread (the supervisor between virtual users, or a virtual user during
/// `think_time`) checks whether it should stop. Short enough that [`LoadRun::stop`] and the
/// `stop_on_error_rate` threshold both take effect well within a second, without busy-looping.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The final result of a load test run, read once with [`LoadRun::join`].
#[derive(Debug, Clone, PartialEq)]
pub struct LoadSummary {
    /// The metrics at the moment the run ended.
    pub snapshot: LoadSnapshot,
    /// Every request the run completed, see [`SampleLog`].
    pub log: SampleLog,
    /// Whether the run ended before [`LoadConfig::duration`] fully elapsed: because
    /// [`LoadRun::stop`] was called, or because [`LoadConfig::stop_on_error_rate`] was exceeded.
    /// `false` means the run completed its full configured duration.
    pub stopped_early: bool,
}

/// State shared between every virtual user thread, the supervisor thread and the [`LoadRun`]
/// handle held by the caller.
struct Shared {
    /// Set to stop every virtual user thread and the supervisor as soon as possible: by
    /// [`LoadRun::stop`], by the supervisor once [`LoadConfig::duration`] elapses, or by a
    /// virtual user once [`LoadConfig::stop_on_error_rate`] is exceeded.
    stop: AtomicBool,
    /// How many virtual user threads are currently running.
    active_vus: AtomicU32,
    /// The metrics every virtual user feeds into. See the crate docs for why this is a `Mutex`
    /// rather than something lock-free.
    metrics: Mutex<Aggregator>,
    /// The moment the run started, the origin every [`Sample::elapsed`] is measured from.
    start: Instant,
}

/// Reads the live metrics of a [`LoadRun`] from any thread, see [`LoadRun::snapshot_reader`].
#[derive(Clone)]
pub struct SnapshotReader {
    shared: Arc<Shared>,
}

impl SnapshotReader {
    /// The same as [`LoadRun::snapshot`].
    #[must_use]
    pub fn snapshot(&self) -> LoadSnapshot {
        read_snapshot(&self.shared)
    }
}

/// Copies the metrics under the lock and computes the percentiles after releasing it.
fn read_snapshot(shared: &Shared) -> LoadSnapshot {
    let elapsed = shared.start.elapsed();
    let active_vus = shared.active_vus.load(Ordering::SeqCst);
    let pending = lock_metrics(&shared.metrics).pending_snapshot(elapsed, active_vus);
    pending.finish()
}

/// A load test in progress, or one that has already finished. Created with [`LoadRun::start`].
pub struct LoadRun {
    shared: Arc<Shared>,
    /// The thread that starts every virtual user and waits for the run to end. `None` only
    /// after [`LoadRun::join`] has consumed it.
    supervisor: Option<JoinHandle<LoadSummary>>,
}

impl LoadRun {
    /// Starts a load test in the background and returns immediately.
    ///
    /// One OS thread per virtual user (`ureq`, used by [`brasa_http::send`], is blocking), each
    /// started once its turn in the linear ramp-up over `config.ramp_up` comes up. Each virtual
    /// user clones `session_env` once and loops over `config.targets` in order for as long as the
    /// run lasts, so an environment change a post script makes (for example a login token) is
    /// visible to the next target of the same virtual user, sleeping `config.think_time` between
    /// iterations.
    #[must_use]
    pub fn start(
        config: LoadConfig,
        environment: Environment,
        session_env: SessionEnv,
        runner: Arc<Runner>,
    ) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            active_vus: AtomicU32::new(0),
            metrics: Mutex::new(Aggregator::new(config.targets.len())),
            start: Instant::now(),
        });

        let supervisor_shared = Arc::clone(&shared);
        let supervisor = thread::spawn(move || {
            run_supervisor(
                &supervisor_shared,
                &config,
                &environment,
                session_env,
                &runner,
            )
        });

        Self {
            shared,
            supervisor: Some(supervisor),
        }
    }

    /// Asks the run to stop as soon as possible.
    ///
    /// Every virtual user thread checks this at least once every `POLL_INTERVAL` (between
    /// targets, and while sleeping through `think_time`), so a call to `stop` finishes the run
    /// promptly, well under a second, even with a long `think_time` or `duration` left to go.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
    }

    /// Whether the run has ended (reached `duration`, was stopped, or hit the error rate
    /// threshold) and every virtual user thread has already been joined by the supervisor.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        match &self.supervisor {
            Some(handle) => handle.is_finished(),
            None => true,
        }
    }

    /// A live read of the current metrics. It sorts every latency recorded so far (see the
    /// crate docs for the cost of computing percentiles), after releasing the metrics lock, so a
    /// UI calls it off its own thread through [`Self::snapshot_reader`].
    #[must_use]
    pub fn snapshot(&self) -> LoadSnapshot {
        read_snapshot(&self.shared)
    }

    /// A handle that reads [`Self::snapshot`] from another thread, for as long as it is kept,
    /// even after [`Self::join`].
    #[must_use]
    pub fn snapshot_reader(&self) -> SnapshotReader {
        SnapshotReader {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Appends to `log` the samples and URLs recorded since the last call, so a caller that keeps
    /// one log and polls (for example every 250 ms) pays only for what is new: `log` must be
    /// empty the first time and only ever filled by this method for this run, since what it
    /// already holds is compared by length. Holds the metrics lock while copying.
    pub fn read_samples(&self, log: &mut SampleLog) {
        log.append_missing_from(lock_metrics(&self.shared.metrics).log());
    }

    /// Blocks until the run has finished and returns its final [`LoadSummary`].
    ///
    /// # Panics
    ///
    /// Panics if the supervisor thread itself panicked, which is not expected to happen: it
    /// contains no `unwrap`/`expect` of its own, and every lock it takes recovers from poisoning
    /// instead of propagating a panic (see the crate docs).
    #[allow(clippy::expect_used)]
    pub fn join(mut self) -> LoadSummary {
        let supervisor = self
            .supervisor
            .take()
            .expect("join takes self by value, so it can only ever be called once");
        supervisor
            .join()
            .expect("the supervisor thread never panics, see the crate docs")
    }
}

/// Runs on its own thread for the lifetime of a [`LoadRun`]: starts every virtual user thread
/// over the ramp-up window, waits for the run to end, joins every virtual user, and returns the
/// final [`LoadSummary`].
fn run_supervisor(
    shared: &Arc<Shared>,
    config: &LoadConfig,
    environment: &Environment,
    session_env: SessionEnv,
    runner: &Arc<Runner>,
) -> LoadSummary {
    let mut handles = Vec::with_capacity(config.vus as usize);

    for vu_index in 0..config.vus {
        // Linear ramp-up: the Nth virtual user (0-based) starts this far into `ramp_up`.
        let start_offset = config
            .ramp_up
            .mul_f64(f64::from(vu_index) / f64::from(config.vus));
        wait_or_stop(shared, shared.start + start_offset);
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }

        shared.active_vus.fetch_add(1, Ordering::SeqCst);
        let vu_shared = Arc::clone(shared);
        let vu_config = config.clone();
        let vu_environment = environment.clone();
        let vu_runner = Arc::clone(runner);
        let mut vu_session_env = session_env.clone();

        handles.push(thread::spawn(move || {
            run_virtual_user(
                vu_index,
                &vu_shared,
                &vu_config,
                &vu_environment,
                &mut vu_session_env,
                &vu_runner,
            );
            vu_shared.active_vus.fetch_sub(1, Ordering::SeqCst);
        }));
    }

    // Let the run continue until `duration` has elapsed since the whole run started (not since
    // the last virtual user started), or until something else asked it to stop first.
    wait_or_stop(shared, shared.start + config.duration);
    let stopped_early = shared.start.elapsed() < config.duration;
    shared.stop.store(true, Ordering::SeqCst);

    for handle in handles {
        // `run_virtual_user` never panics (see its own docs), so there is nothing actionable to
        // do with an `Err` here beyond letting the thread finish.
        let _ = handle.join();
    }

    let (snapshot, log) = {
        let mut metrics = lock_metrics(&shared.metrics);
        let snapshot = metrics.final_snapshot(shared.start.elapsed());
        (snapshot, metrics.log().clone())
    };
    LoadSummary {
        snapshot,
        log,
        stopped_early,
    }
}

/// Runs on its own thread for the lifetime of one virtual user: loops over `config.targets` in
/// order, sleeping `config.think_time` between iterations, until `shared.stop` is set. With
/// `config.reuse_connections`, every request goes through one client of its own, which keeps
/// its connections open between them.
///
/// Never panics: the only fallible step is locking `shared.metrics`, and that recovers from a
/// poisoned lock instead of unwrapping (see [`lock_metrics`]), so a virtual user thread can
/// always be joined by the supervisor.
fn run_virtual_user(
    vu_index: u32,
    shared: &Shared,
    config: &LoadConfig,
    environment: &Environment,
    session_env: &mut SessionEnv,
    runner: &Runner,
) {
    let client = config.reuse_connections.then(|| runner.client());
    'iterations: while !shared.stop.load(Ordering::SeqCst) {
        for (target_index, target) in config.targets.iter().enumerate() {
            if shared.stop.load(Ordering::SeqCst) {
                break 'iterations;
            }

            let started = Instant::now();
            let result = match &client {
                Some(client) => runner.run_with(&target.request, environment, session_env, client),
                None => runner.run(&target.request, environment, session_env),
            };
            let latency = response_latency(&result, started.elapsed());
            let elapsed = shared.start.elapsed();

            let active_vus = shared.active_vus.load(Ordering::SeqCst);
            let sample = sample_from(
                target_index,
                (vu_index, active_vus),
                elapsed,
                latency,
                &result,
            );
            let url = result
                .resolved_request
                .as_ref()
                .map_or(target.request.url.as_str(), |resolved| {
                    resolved.url.as_str()
                });
            let (errors, total) = lock_metrics(&shared.metrics)
                .record(sample, url, || error_example(sample.status, &result));

            let exceeded_threshold = config
                .stop_on_error_rate
                .is_some_and(|threshold| total >= 50 && ratio(errors, total) > threshold);
            if exceeded_threshold {
                shared.stop.store(true, Ordering::SeqCst);
                break 'iterations;
            }
        }

        if !shared.stop.load(Ordering::SeqCst) && !config.think_time.is_zero() {
            wait_or_stop(shared, Instant::now() + config.think_time);
        }
    }
}

/// `numerator / denominator` as an `f64`, used here for a live error rate check against
/// [`LoadConfig::stop_on_error_rate`]. `denominator` is always positive when this is called (it
/// is only reached once `total >= 50`).
fn ratio(numerator: u64, denominator: u64) -> f64 {
    numerator as f64 / denominator as f64
}

/// The latency to record for one [`Runner::run`] call: the response's own measured time
/// ([`brasa_core::Response::time`]) when a response was received, since that excludes the time
/// spent in pre/post scripts; `wall_time` (measured around the whole call) otherwise, since there
/// is no response to measure from.
fn response_latency(result: &RunResult, wall_time: Duration) -> Duration {
    result
        .response
        .as_ref()
        .map_or(wall_time, |response| response.time)
}

/// Turns the outcome of one [`Runner::run`] call into a [`Sample`], applying the error rule: no response at all (a send failure, a timeout, or a
/// pre-request script failure) or a status of 400 or above.
///
/// A post-response script failure is deliberately not treated as an error here: the response was
/// already received, so it still speaks for itself. This matches
/// [`brasa_runner::RunResult::failed_stage`]'s own contract: `Post` failures still carry a
/// `response`.
///
/// `(vu, active_vus)` is the 0-based index of the virtual user that sent the request and how many
/// virtual users were running when it completed.
fn sample_from(
    target_index: usize,
    (vu, active_vus): (u32, u32),
    elapsed: Duration,
    latency: Duration,
    result: &RunResult,
) -> Sample {
    let status = match (&result.failed_stage, &result.response) {
        (Some(FailedStage::Send(HttpError::Timeout)), _) => StatusKey::Timeout,
        (Some(FailedStage::Pre(_) | FailedStage::Send(_)), _) | (_, None) => StatusKey::Failed,
        (_, Some(response)) => StatusKey::Code(response.status),
    };

    Sample {
        target_index,
        vu,
        active_vus,
        elapsed,
        latency_us: micros_clamped(latency),
        size: result.response.as_ref().map_or(0, |response| {
            u32::try_from(response.body.len()).unwrap_or(u32::MAX)
        }),
        status,
        is_error: status.is_error(),
    }
}

/// How many characters of a response body an [`crate::ErrorExample`] keeps.
const EXAMPLE_BODY_CHARS: usize = 160;

/// The [`crate::ErrorExample`] text for a sample of `status`, from the `result` that produced it:
/// the HTTP status and the start of the body, or the error that stopped the request.
fn error_example(status: StatusKey, result: &RunResult) -> String {
    match status {
        StatusKey::Code(code) => {
            let body = result
                .response
                .as_ref()
                .map_or(&[][..], |response| response.body.as_slice());
            http_error_example(code, body)
        }
        StatusKey::Timeout | StatusKey::Failed => result
            .failed_stage
            .as_ref()
            .map_or_else(|| "Request failed".to_string(), ToString::to_string),
    }
}

/// `"HTTP <code>"`, then `" \u{b7} "` and the first [`EXAMPLE_BODY_CHARS`] characters of `body`
/// (lossy UTF-8, every run of whitespace collapsed to one space) when there is any text in it.
fn http_error_example(code: u16, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut example = format!("HTTP {code}");
    if !collapsed.is_empty() {
        example.push_str(" \u{b7} ");
        example.extend(collapsed.chars().take(EXAMPLE_BODY_CHARS));
    }
    example
}

/// `duration` as whole microseconds, saturating at `u32::MAX` (about 71.5 minutes) instead of
/// overflowing. A single load test request is never expected to take anywhere near that long;
/// this is just a safe fallback instead of a panic.
fn micros_clamped(duration: Duration) -> u32 {
    u32::try_from(duration.as_micros()).unwrap_or(u32::MAX)
}

/// Sleeps in short slices until `deadline` or until `shared.stop` becomes `true`, whichever comes
/// first. Sleeping in slices of at most [`POLL_INTERVAL`], instead of one long `thread::sleep`,
/// is what makes [`LoadRun::stop`] take effect promptly instead of only once the full remaining
/// wait is over.
fn wait_or_stop(shared: &Shared, deadline: Instant) {
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        thread::sleep(POLL_INTERVAL.min(deadline - now));
    }
}

/// Locks `metrics`, recovering from a poisoned lock instead of panicking.
///
/// A `Mutex` is poisoned when a thread panics while holding it. Nothing in this crate panics
/// while holding `metrics` (every step between locking and unlocking is plain arithmetic and
/// vector operations, see [`Aggregator::record`] and [`Aggregator::snapshot`]), so poisoning here
/// would mean a bug elsewhere already corrupted the process. Recovering the guard is still the
/// right choice for a metrics aggregator read live by a UI: showing slightly stale numbers is far
/// better than making every virtual user thread panic too.
fn lock_metrics(metrics: &Mutex<Aggregator>) -> MutexGuard<'_, Aggregator> {
    metrics.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn an_http_example_without_a_body_is_just_the_status() {
        assert_eq!(http_error_example(502, b""), "HTTP 502");
        assert_eq!(http_error_example(502, b" \n\t "), "HTTP 502");
    }

    #[test]
    fn an_http_example_collapses_whitespace_and_keeps_the_start_of_the_body() {
        let body = b"{\n  \"error\":   \"rate_limited\"\n}";
        assert_eq!(
            http_error_example(429, body),
            "HTTP 429 \u{b7} { \"error\": \"rate_limited\" }"
        );
    }

    #[test]
    fn an_http_example_keeps_at_most_160_characters_of_the_body() {
        let body = "\u{e9}".repeat(500);
        let example = http_error_example(500, body.as_bytes());
        let kept = example.strip_prefix("HTTP 500 \u{b7} ").unwrap_or_default();
        assert_eq!(kept.chars().count(), EXAMPLE_BODY_CHARS);
    }

    #[test]
    fn invalid_utf8_in_a_body_is_replaced_not_dropped() {
        let example = http_error_example(500, &[0x6f, 0x6b, 0xff]);
        assert_eq!(example, "HTTP 500 \u{b7} ok\u{fffd}");
    }
}
