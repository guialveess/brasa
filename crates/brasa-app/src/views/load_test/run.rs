//! Starting, stopping and finishing a load test run, and the periodic 250 ms refresh while it is in
//! progress. Mirrors `views/send.rs`'s split between the actual pipeline (here,
//! `brasa_load::LoadRun`) and driving it off the UI thread, so a 50 VU run never blocks
//! rendering.
//!
//! Known engine caveat (`brasa-load`'s own crate docs): `LoadRun::stop` does not abort
//! in-flight requests, so `LoadRun::join` can block until every virtual user's current request
//! (and think time) finishes. Every call to `join` below runs inside `cx.background_spawn`,
//! never on the UI thread.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::*;
use brasa_load::history::{LoadConfigSummary, RunRecord};
use brasa_load::{LoadConfig, LoadRun, LoadSnapshot, LoadSummary, LoadTarget};
use brasa_runner::Runner;

use crate::state::load_results::Shared;
use crate::state::load_test::{self, LoadTestFailure, LoadTestStatus, LoadTestTarget};
use crate::views::root::AppView;

/// How often the dashboard refreshes while a run is in progress.
const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// How many ticks pass between two background recomputations of the results while a run is in
/// progress (once a second): the distribution, the statuses and the Requests and Errors tabs
/// lag at most that much behind the KPIs.
const REFRESH_EVERY_TICKS: u32 = 4;

/// A load test run in progress: the engine handle, the data needed to save its
/// [`RunRecord`] once it ends, and the periodic refresh task keeping both alive. Removed from
/// [`AppView::load_runs`] the moment the run ends (see [`AppView::start_load_test`]'s tick loop),
/// so its presence in that map is exactly "this tab has a run in progress".
pub(crate) struct LoadRunHandle {
    /// The running engine handle.
    pub(crate) run: LoadRun,
    /// When the run started, for the saved [`RunRecord`].
    started_at_unix: u64,
    /// The configuration the run started with, for the saved [`RunRecord`].
    config_summary: LoadConfigSummary,
    /// Ticks since the results were last recomputed in the background, see
    /// [`AppView::tick_load_test`].
    ticks_since_refresh: u32,
    /// Ticks the tab's log has gone without the newest requests because a background computation
    /// was still reading it, see [`AppView::tick_load_test`].
    ticks_behind: u32,
    /// Kept alive only so it is not dropped (and cancelled) while the run is in progress; see
    /// `views/send.rs`'s `SendingTask::_task` doc comment for the same pattern, including why
    /// dropping this from inside its own body (as the tick loop below does once a run ends) is
    /// safe.
    _tick: Task<()>,
}

impl AppView {
    /// Resolves `tab_id`'s target against the open workspace into `brasa_load::LoadTarget`s,
    /// one per request in tree order (a single request is one target; a collection is every
    /// request under the chosen folder). `Err` with the reason when there is no
    /// workspace, no target picked yet, the target resolves to no requests, or a request fails
    /// to load.
    fn build_load_targets(&self, tab_id: &str) -> Result<Vec<LoadTarget>, LoadTestFailure> {
        let workspace = self
            .state
            .workspace
            .as_ref()
            .ok_or(LoadTestFailure::NoWorkspace)?;
        let load_test = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.load_test())
            .ok_or(LoadTestFailure::TabClosed)?;
        let target = load_test.target.as_ref().ok_or(LoadTestFailure::NoTarget)?;

        let ids = match target {
            LoadTestTarget::Request(id) => vec![id.clone()],
            LoadTestTarget::Collection(folder_id) => {
                load_test::request_ids_under(workspace.tree(), folder_id)
            }
        };
        if ids.is_empty() {
            return Err(LoadTestFailure::EmptyFolder);
        }

        ids.iter()
            .map(|id| {
                let request = workspace
                    .load_request(id)
                    .map_err(|error| LoadTestFailure::Load(error.to_string()))?;
                let label = crate::state::format::tab_label(id).to_string();
                Ok(LoadTarget::new(label, request))
            })
            .collect()
    }

    /// Starts a load test run for `tab_id`. A no-op if a run is already in progress for this tab.
    /// On a setup failure (no target, an empty collection, a request that fails to load), records
    /// [`LoadTestStatus::Failed`] instead of starting anything.
    pub(crate) fn start_load_test(&mut self, tab_id: String, cx: &mut Context<Self>) {
        if self.load_runs.contains_key(&tab_id) {
            return;
        }
        let targets = match self.build_load_targets(&tab_id) {
            Ok(targets) => targets,
            Err(failure) => {
                log::warn!("load test not started: {failure:?}");
                self.edit_load_test(&tab_id, cx, |load_test| {
                    load_test.status = LoadTestStatus::Failed(failure);
                });
                return;
            }
        };
        let Some(resolved) = self
            .state
            .tabs
            .index_of(&tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.load_test())
            .map(|load_test| load_test.config.resolved())
        else {
            return;
        };

        let target_rows: Vec<(brasa_core::Method, String)> = targets
            .iter()
            .map(|target| (target.request.method.clone(), target.id.clone()))
            .collect();
        let config = LoadConfig {
            targets,
            vus: resolved.vus,
            duration: Duration::from_secs(resolved.duration_secs),
            ramp_up: Duration::from_secs(resolved.ramp_up_secs),
            think_time: Duration::from_millis(resolved.think_time_ms),
            stop_on_error_rate: resolved.stop_on_error_rate,
            reuse_connections: resolved.reuse_connections,
        };
        let config_summary = LoadConfigSummary::from(&config);
        log::info!(
            "load test started: {} targets, {} virtual users for {} s (ramp up {} s, think time {} ms, reuse connections {})",
            config.targets.len(),
            config.vus,
            config.duration.as_secs(),
            config.ramp_up.as_secs(),
            config.think_time.as_millis(),
            config.reuse_connections,
        );
        let started_at_unix = unix_now();

        self.state.reload_environment();
        let environment = (*self.active_environment()).clone();
        let session_env = self.state.session_env.clone();
        let engine = self.script_engine.clone();
        let options = self.send_options.clone();
        let base_dir = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf());
        let runner = Arc::new(Runner::new(engine, options).with_base_dir(base_dir));
        let run_number = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| brasa_load::history::next_number(workspace.root()))
            .unwrap_or(1);

        self.edit_load_test(&tab_id, cx, |load_test| {
            load_test.status = LoadTestStatus::Running;
            load_test.run_number = Some(run_number);
            load_test.snapshot = None;
            // Cleared, not just `compare_snapshot`: leaving a previous run's `compare_with`
            // around would show its label in the "Compare with" trigger with no rows under it
            // (since `compare_snapshot` is `None` until this new run finishes and picks its own
            // default), a stale, confusing display while a run is in progress.
            load_test.compare_with = None;
            load_test.compare_snapshot = None;
            load_test.target_rows = target_rows;
            load_test.run_reuse_connections = Some(resolved.reuse_connections);
            load_test.run_duration_secs = Some(resolved.duration_secs);
            load_test.run_thresholds = resolved.thresholds.clone();
            load_test.run_vus = Some(resolved.vus);
            load_test.run_ramp_up_secs = Some(resolved.ramp_up_secs);
            load_test.run_started_at = Some(started_at_unix);
            load_test.replace_log(Some(brasa_load::SampleLog::new()));
        });

        let run = LoadRun::start(config, environment, session_env, runner);

        let tick_tab_id = tab_id.clone();
        let reader = run.snapshot_reader();
        let tick = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor().timer(TICK_INTERVAL).await;
                // Sorting every latency of a long run takes long enough to drop frames.
                let reader = reader.clone();
                let snapshot = cx.background_spawn(async move { reader.snapshot() }).await;
                let Ok(still_running) = weak.update(cx, |view, cx| {
                    view.tick_load_test(&tick_tab_id, snapshot, cx)
                }) else {
                    return;
                };
                if still_running {
                    continue;
                }

                let Ok(Some(handle)) =
                    weak.update(cx, |view, _cx| view.load_runs.remove(&tick_tab_id))
                else {
                    return;
                };
                let LoadRunHandle {
                    run,
                    started_at_unix,
                    config_summary,
                    ..
                } = handle;
                let summary = cx.background_spawn(async move { run.join() }).await;
                let _ = weak.update(cx, |view, cx| {
                    view.finish_load_test(
                        &tick_tab_id,
                        summary,
                        started_at_unix,
                        config_summary,
                        cx,
                    );
                });
                return;
            }
        });

        self.load_runs.insert(
            tab_id,
            LoadRunHandle {
                run,
                started_at_unix,
                config_summary,
                ticks_since_refresh: 0,
                ticks_behind: 0,
                _tick: tick,
            },
        );
        cx.notify();
    }

    /// Stores the run's current metrics, read in the background by the tick loop above, on the
    /// tab. Returns whether the run is still going, so the tick loop knows when to stop polling
    /// and finalize it.
    ///
    /// Also appends the requests completed since the last tick to the tab's log, and every
    /// [`REFRESH_EVERY_TICKS`] ticks recomputes the results from it in the background.
    fn tick_load_test(
        &mut self,
        tab_id: &str,
        snapshot: LoadSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(handle) = self.load_runs.get_mut(tab_id) else {
            return false;
        };
        let still_running = !handle.run.is_finished();
        handle.ticks_since_refresh += 1;
        let refresh = handle.ticks_since_refresh >= REFRESH_EVERY_TICKS;
        if refresh {
            handle.ticks_since_refresh = 0;
        }
        let Some(load_test) = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get_mut(index))
            .and_then(|tab| tab.load_test_mut())
        else {
            return still_running;
        };
        load_test.snapshot = Some(snapshot);
        load_test.refresh_chart();
        if let Some(log) = load_test.log.as_mut() {
            // While a background computation still reads the log, copying it to append would cost
            // the whole log on this thread: the new requests wait for a later tick instead, since
            // `read_samples` appends whatever is missing. Only a log a second behind is copied.
            if let Some(log) = Arc::get_mut(&mut log.0) {
                handle.run.read_samples(log);
                handle.ticks_behind = 0;
            } else if handle.ticks_behind >= REFRESH_EVERY_TICKS {
                handle.run.read_samples(Arc::make_mut(&mut log.0));
                handle.ticks_behind = 0;
            } else {
                handle.ticks_behind += 1;
            }
        }
        if refresh {
            self.refresh_load_results(tab_id, cx);
        }
        cx.notify();
        still_running
    }

    /// Records the final status and snapshot, saves the run to `.brasa/runs/`, and refreshes the
    /// tab's history list so the "Compare with" panel picks it up.
    fn finish_load_test(
        &mut self,
        tab_id: &str,
        summary: LoadSummary,
        started_at_unix: u64,
        config_summary: LoadConfigSummary,
        cx: &mut Context<Self>,
    ) {
        let snapshot = &summary.snapshot;
        log::info!(
            "load test {}: {} requests in {} ms, {:.1} req/s, p50 {} us, p95 {} us, p99 {} us, \
             {:.1}% errors",
            if summary.stopped_early {
                "stopped early"
            } else {
                "finished"
            },
            snapshot.total,
            snapshot.elapsed.as_millis(),
            snapshot.rps,
            snapshot.p50,
            snapshot.p95,
            snapshot.p99,
            snapshot.error_rate * 100.0,
        );
        let root = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf());
        let mut target_label = String::new();
        let mut run_number = None;
        let mut target_labels = Vec::new();
        let mut thresholds = Vec::new();
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.status = if summary.stopped_early {
                LoadTestStatus::Stopped
            } else {
                LoadTestStatus::Finished
            };
            load_test.snapshot = Some(summary.snapshot.clone());
            load_test.refresh_chart();
            // The same run: keep the generation, so the results computed so far stay shown
            // until the final ones replace them.
            load_test.log = Some(Shared::new(summary.log.clone()));
            target_label = load_test.target_label.clone();
            thresholds = load_test.run_thresholds.clone();
            run_number = load_test.run_number;
            target_labels = load_test
                .target_rows
                .iter()
                .map(|(method, name)| (method.to_string(), name.clone()))
                .collect();
        });

        if let (Some(root), Some(number)) = (root.as_ref(), run_number) {
            let samples_root = root.clone();
            let log = summary.log;
            cx.background_spawn(async move {
                if let Err(error) = brasa_load::history::save_samples(&samples_root, number, &log)
                {
                    log::warn!("could not save the requests of load test run {number}: {error}");
                }
            })
            .detach();
            let record = RunRecord {
                number,
                started_at_unix,
                target_label: target_label.clone(),
                config: config_summary,
                snapshot: summary.snapshot,
                stopped_early: summary.stopped_early,
                target_labels,
                thresholds,
            };
            if let Err(error) = brasa_load::history::save(root, &record) {
                log::warn!("could not save load test run {number}: {error}");
            }
        }
        self.refresh_load_test_history(tab_id, cx);

        // Defaults "Compare with" to the latest *other* run of the same target, not simply
        // `run_number - 1`: run numbers are shared across every target in the workspace
        // (`brasa_load::history::next_number`), so the previous sequential number might belong to
        // a different target entirely.
        let previous = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.load_test())
            .and_then(|load_test| {
                load_test
                    .history
                    .iter()
                    .find(|header| Some(header.number) != run_number)
                    .map(|header| header.number)
            });
        self.edit_load_test(tab_id, cx, |load_test| load_test.compare_with = previous);
        self.refresh_load_test_compare_snapshot(tab_id, cx);
        self.refresh_load_results(tab_id, cx);
    }

    /// Refreshes `tab_id`'s cached history list (the
    /// history list and the "Compare with" dropdown), filtered to runs of the same target label.
    pub(crate) fn refresh_load_test_history(&mut self, tab_id: &str, cx: &mut Context<Self>) {
        let Some(root) = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf())
        else {
            return;
        };
        let all = brasa_load::history::list(&root);
        self.edit_load_test(tab_id, cx, |load_test| {
            let label = load_test.target_label.clone();
            load_test.history = all
                .into_iter()
                .filter(|header| header.target_label == label)
                .collect();
        });
    }

    /// Asks the run in progress for `tab_id` to stop as soon as possible
    /// (the "Stop run" button, and closing a tab with
    /// a running test). A no-op if no run is in progress. The tick loop started by
    /// [`Self::start_load_test`] notices within one [`TICK_INTERVAL`] and finalizes the run; this
    /// method itself never blocks.
    pub(crate) fn stop_load_test(&mut self, tab_id: &str) {
        if let Some(handle) = self.load_runs.get(tab_id) {
            handle.run.stop();
        }
    }
}

/// The current time as Unix seconds, saturating at `0` on a clock before the epoch instead of
/// panicking (never expected on a real machine, but cheaper than an `unwrap`). `pub(super)`: also
/// used by `views/load_test/dashboard.rs` for the "Compare with" dropdown's relative day labels.
pub(super) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
