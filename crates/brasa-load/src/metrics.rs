//! Live metrics for a load test: the [`Aggregator`] every sample is fed into, and the
//! [`LoadSnapshot`] read out of it.
//!
//! # Percentile method
//!
//! Percentiles use the nearest rank method: sort the latencies, then take the value at rank
//! `ceil(p / 100 * n)` (clamped to `1..=n`). Nearest rank never interpolates between two
//! samples, so every percentile value shown actually occurred, which is the usual convention for
//! load testing tools. See [`percentile`] for the implementation and its cost.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::samples::{SampleLog, SampleRecord};
use crate::status::StatusKey;

/// One recorded sample: the outcome of a single request within a load test iteration, ready to
/// feed into [`Aggregator::record`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sample {
    /// The index of the target this sample belongs to, into the same
    /// [`crate::LoadConfig::targets`] list the run was started with.
    pub target_index: usize,
    /// The virtual user that sent it, 0-based.
    pub vu: u32,
    /// How many virtual users were active when it was recorded, for [`SecondPoint::vus`].
    pub active_vus: u32,
    /// How far into the run this sample was recorded, used to bucket it into
    /// [`LoadSnapshot::series`].
    pub elapsed: Duration,
    /// The latency of this single request, in microseconds.
    pub latency_us: u32,
    /// The response body size in bytes, `0` when there was no response.
    pub size: u32,
    /// How the sample is classified for the status breakdown.
    pub status: StatusKey,
    /// Whether this sample counts as an error. Kept alongside `status` instead of recomputed from
    /// it, since a caller may have a status of [`StatusKey::Code`] with a value below 400 alongside
    /// a script failure that is still not counted as an error, see [`StatusKey::is_error`].
    pub is_error: bool,
}

/// One point of the throughput and latency time series
/// (the dashboard's polylines): one entry per
/// second of the run that has fully elapsed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SecondPoint {
    /// Requests completed during this second.
    pub rps: f64,
    /// The 95th percentile latency of the samples completed during this second, in
    /// microseconds. `0` if no sample fell in this second.
    pub p95: u32,
    /// The median latency of this second, in microseconds. `0` if no sample fell in it, and in a
    /// run file saved before this field existed.
    #[serde(default)]
    pub p50: u32,
    /// The 90th percentile latency of this second, in microseconds, `0` as for `p50`.
    #[serde(default)]
    pub p90: u32,
    /// The 99th percentile latency of this second, in microseconds, `0` as for `p50`.
    #[serde(default)]
    pub p99: u32,
    /// The most virtual users active at any moment a sample of this second was recorded. `0` if
    /// no sample fell in this second, and in a run file saved before this field existed.
    #[serde(default)]
    pub vus: u32,
}

/// The first error of a [`StatusKey`] seen during a run, kept as an example of what that kind of
/// failure looks like.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorExample {
    /// The status the example is for.
    pub status: StatusKey,
    /// For an HTTP status, `"HTTP <code>"` followed, when the body is not empty, by `" \u{b7} "`
    /// and the first 160 characters of the body (whitespace runs collapsed to one space). For a
    /// timeout or a failure, the text of the error.
    pub text: String,
}

/// Metrics for a single [`crate::LoadTarget`], one row of the per-request table.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TargetStats {
    /// Samples recorded for this target.
    pub count: u64,
    /// Median latency, in microseconds.
    pub p50: u32,
    /// 95th percentile latency, in microseconds.
    pub p95: u32,
    /// 99th percentile latency, in microseconds.
    pub p99: u32,
    /// The fraction of this target's samples that were errors (`0.0` to `1.0`).
    pub error_rate: f64,
}

/// A point-in-time read of a load test's metrics: live while it runs, final once it stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSnapshot {
    /// How long the run has been going for. On the final snapshot of a finished (not stopped
    /// early) run, this can exceed [`crate::LoadConfig::duration`] by a little: `stop` does not
    /// abort a request already in flight (`crate::run`'s module docs), so the run only actually
    /// ends once every virtual user's current request finishes, bounded by the slowest one still
    /// in flight at that moment rather than the configured duration itself. Against a fast local
    /// server this is a handful of milliseconds; against a slower or remote target it tracks that
    /// target's own latency.
    #[serde(with = "duration_millis")]
    pub elapsed: Duration,
    /// How many virtual users are currently running.
    pub active_vus: u32,
    /// Every sample recorded so far, across every target.
    pub total: u64,
    /// While a run is live ([`Aggregator::snapshot`]): requests completed during the last fully
    /// elapsed second, `0.0` before the first second has elapsed. Once a run has finished
    /// ([`Aggregator::final_snapshot`]): the average requests per second over the whole run
    /// instead, since the last elapsed second is often idle by then (every virtual user has
    /// already stopped) and would otherwise report a misleading `0.0` for a run that clearly
    /// wasn't idle (see that method's doc comment).
    pub rps: f64,
    /// The median latency across every sample, in microseconds.
    pub p50: u32,
    /// The 95th percentile latency across every sample, in microseconds.
    pub p95: u32,
    /// The 99th percentile latency across every sample, in microseconds.
    pub p99: u32,
    /// The fraction of every sample that was an error (`0.0` to `1.0`).
    pub error_rate: f64,
    /// The throughput and latency time series, one point per second that has fully elapsed, in
    /// order.
    pub series: Vec<SecondPoint>,
    /// The latency distribution: 26 buckets of 10 ms each, the last one (index 25) holding every
    /// latency of 250 ms and above.
    pub histogram: [u64; 26],
    /// How many samples landed in each [`StatusKey`], in the order first seen.
    pub status_counts: Vec<(StatusKey, u64)>,
    /// Metrics per target, in the same order as the [`crate::LoadConfig::targets`] the run was
    /// started with (there is no target id here: zip this with that list to label a row).
    pub per_target: Vec<TargetStats>,
    /// The first error of each [`StatusKey`] that had one, in the order first seen.
    /// `#[serde(default)]` (empty) for a run file saved before this field existed.
    #[serde(default)]
    pub error_examples: Vec<ErrorExample>,
}

/// The mutable metrics state of a running load test, behind a `Mutex` shared by every virtual
/// user thread and the thread reading snapshots. See the crate docs for the concurrency model.
#[derive(Debug)]
pub(crate) struct Aggregator {
    total: u64,
    errors: u64,
    global_latencies_us: Vec<u32>,
    histogram: [u64; 26],
    status_counts: Vec<(StatusKey, u64)>,
    per_second: Vec<SecondBucket>,
    per_target: Vec<TargetBucket>,
    log: SampleLog,
    error_examples: Vec<ErrorExample>,
}

/// One second's worth of samples, kept while building [`LoadSnapshot::series`].
#[derive(Debug, Default)]
struct SecondBucket {
    count: u64,
    latencies_us: Vec<u32>,
    /// The most active virtual users seen with a sample of this second.
    max_vus: u32,
    /// The percentiles of `latencies_us`, computed the first time the second is read after it
    /// completed and dropped by [`Aggregator::record`] if a late sample lands in it, so a
    /// completed second is sorted once and not on every snapshot.
    percentiles: Option<SecondPercentiles>,
}

/// The cached percentiles of one second, in microseconds.
#[derive(Debug, Clone, Copy)]
struct SecondPercentiles {
    p50: u32,
    p90: u32,
    p95: u32,
    p99: u32,
}

/// One target's samples, kept while building [`LoadSnapshot::per_target`].
#[derive(Debug, Default)]
struct TargetBucket {
    count: u64,
    errors: u64,
    latencies_us: Vec<u32>,
}

impl Aggregator {
    /// A fresh aggregator for a run with `target_count` targets, no samples recorded yet.
    pub(crate) fn new(target_count: usize) -> Self {
        Self {
            total: 0,
            errors: 0,
            global_latencies_us: Vec::new(),
            histogram: [0; 26],
            status_counts: Vec::new(),
            per_second: Vec::new(),
            per_target: (0..target_count).map(|_| TargetBucket::default()).collect(),
            log: SampleLog::new(),
            error_examples: Vec::new(),
        }
    }

    /// The log of every sample recorded so far.
    pub(crate) fn log(&self) -> &SampleLog {
        &self.log
    }

    /// Records `sample`, updating every metric it feeds into. Returns the total error and
    /// sample counts after recording it, so a caller can check
    /// [`crate::LoadConfig::stop_on_error_rate`] without locking the aggregator a second time.
    ///
    /// `url` is the URL the request was sent to, for the [`SampleLog`]. `example` builds the
    /// [`ErrorExample`] text and is only called for the first error of a status, so its cost
    /// (reading a response body) is paid a handful of times per run, not per sample.
    pub(crate) fn record(
        &mut self,
        sample: Sample,
        url: &str,
        example: impl FnOnce() -> String,
    ) -> (u64, u64) {
        let url = self.log.intern(url);
        self.log.push(SampleRecord {
            elapsed_ms: u32::try_from(sample.elapsed.as_millis()).unwrap_or(u32::MAX),
            vu: sample.vu,
            target: u32::try_from(sample.target_index).unwrap_or(u32::MAX),
            url,
            status: sample.status,
            latency_us: sample.latency_us,
            size: sample.size,
            is_error: sample.is_error,
        });
        if sample.is_error
            && !self
                .error_examples
                .iter()
                .any(|known| known.status == sample.status)
        {
            self.error_examples.push(ErrorExample {
                status: sample.status,
                text: example(),
            });
        }

        self.total += 1;
        if sample.is_error {
            self.errors += 1;
        }
        self.histogram[histogram_bucket(sample.latency_us)] += 1;
        self.global_latencies_us.push(sample.latency_us);

        match self
            .status_counts
            .iter_mut()
            .find(|(key, _)| *key == sample.status)
        {
            Some((_, count)) => *count += 1,
            None => self.status_counts.push((sample.status, 1)),
        }

        let second = sample.elapsed.as_secs() as usize;
        if self.per_second.len() <= second {
            self.per_second
                .resize_with(second + 1, SecondBucket::default);
        }
        let bucket = &mut self.per_second[second];
        bucket.count += 1;
        bucket.latencies_us.push(sample.latency_us);
        bucket.max_vus = bucket.max_vus.max(sample.active_vus);
        bucket.percentiles = None;

        if let Some(bucket) = self.per_target.get_mut(sample.target_index) {
            bucket.count += 1;
            if sample.is_error {
                bucket.errors += 1;
            }
            bucket.latencies_us.push(sample.latency_us);
        }

        (self.errors, self.total)
    }

    /// Builds a [`LoadSnapshot`] from the current state. `elapsed` and `active_vus` come from
    /// outside the aggregator (a monotonic clock and an atomic counter, respectively), since
    /// this type only tracks what samples were recorded, not when "now" is.
    ///
    /// Sorts a fresh copy of the latencies of every percentile it computes, so this is
    /// `O(n log n)` in the number of samples recorded so far (see the module docs). A caller
    /// holding the metrics lock calls [`Self::pending_snapshot`] instead and sorts after
    /// releasing it.
    pub(crate) fn snapshot(&mut self, elapsed: Duration, active_vus: u32) -> LoadSnapshot {
        self.pending_snapshot(elapsed, active_vus).finish()
    }

    /// The part of [`Self::snapshot`] that reads the aggregator: the counters, the per second
    /// series and unsorted copies of the latencies, an `O(n)` copy. The `O(n log n)` sorts are
    /// left to [`PendingSnapshot::finish`], so [`crate::LoadRun::snapshot`] releases the metrics
    /// lock before them and the virtual users are not blocked while it sorts. The exception is
    /// the per second series: each completed second is sorted once and its percentiles are kept,
    /// which is why this takes `&mut self`.
    pub(crate) fn pending_snapshot(
        &mut self,
        elapsed: Duration,
        active_vus: u32,
    ) -> PendingSnapshot {
        let error_rate = ratio(self.errors, self.total);

        let completed_seconds = elapsed.as_secs() as usize;
        let series: Vec<SecondPoint> = (0..completed_seconds)
            .map(|second| match self.per_second.get_mut(second) {
                Some(bucket) => {
                    let percentiles = *bucket.percentiles.get_or_insert_with(|| {
                        let mut sorted = bucket.latencies_us.clone();
                        sorted.sort_unstable();
                        SecondPercentiles {
                            p50: percentile(&sorted, 50.0),
                            p90: percentile(&sorted, 90.0),
                            p95: percentile(&sorted, 95.0),
                            p99: percentile(&sorted, 99.0),
                        }
                    });
                    SecondPoint {
                        rps: bucket.count as f64,
                        p95: percentiles.p95,
                        p50: percentiles.p50,
                        p90: percentiles.p90,
                        p99: percentiles.p99,
                        vus: bucket.max_vus,
                    }
                }
                None => SecondPoint {
                    rps: 0.0,
                    p95: 0,
                    p50: 0,
                    p90: 0,
                    p99: 0,
                    vus: 0,
                },
            })
            .collect();
        let rps = series.last().map_or(0.0, |point| point.rps);

        PendingSnapshot {
            snapshot: LoadSnapshot {
                elapsed,
                active_vus,
                total: self.total,
                rps,
                p50: 0,
                p95: 0,
                p99: 0,
                error_rate,
                series,
                histogram: self.histogram,
                status_counts: self.status_counts.clone(),
                per_target: Vec::new(),
                error_examples: self.error_examples.clone(),
            },
            global_latencies_us: self.global_latencies_us.clone(),
            per_target: self
                .per_target
                .iter()
                .map(|bucket| PendingTarget {
                    count: bucket.count,
                    errors: bucket.errors,
                    latencies_us: bucket.latencies_us.clone(),
                })
                .collect(),
        }
    }

    /// Builds the final [`LoadSnapshot`] of a run that has just ended (`crate::run`'s
    /// `run_supervisor`, called once every virtual user thread has been joined).
    ///
    /// Identical to [`Self::snapshot`] except for `rps`: [`Self::snapshot`]'s "last fully elapsed
    /// second" is meant to read as a live, current rate while a run is in progress, but by the
    /// time every virtual user has stopped and been joined, `elapsed` has usually ticked a little
    /// past the last second any of them actually completed a request in (`brasa_load`'s own
    /// crate docs: `stop` does not abort a request already in flight, so joining waits for it).
    /// That trailing, now-idle second would otherwise report `0.0`, misrepresenting a run that
    /// had real, nonzero throughput throughout. The average over the whole run is a truthful
    /// summary instead.
    #[must_use]
    pub(crate) fn final_snapshot(&mut self, elapsed: Duration) -> LoadSnapshot {
        let mut snapshot = self.snapshot(elapsed, 0);
        snapshot.rps = average_rps(self.total, elapsed);
        snapshot
    }
}

/// A [`LoadSnapshot`] whose percentiles are not computed yet, from
/// [`Aggregator::pending_snapshot`]: it owns unsorted copies of the latencies, so it can be
/// finished without the aggregator or its lock.
pub(crate) struct PendingSnapshot {
    /// Everything but `p50`, `p95`, `p99` and `per_target`, which [`Self::finish`] fills in.
    snapshot: LoadSnapshot,
    global_latencies_us: Vec<u32>,
    per_target: Vec<PendingTarget>,
}

/// One target's counters and unsorted latencies, copied for [`PendingSnapshot::finish`].
struct PendingTarget {
    count: u64,
    errors: u64,
    latencies_us: Vec<u32>,
}

impl PendingSnapshot {
    /// Sorts the latencies and returns the complete [`LoadSnapshot`], `O(n log n)`.
    pub(crate) fn finish(self) -> LoadSnapshot {
        let Self {
            mut snapshot,
            global_latencies_us: mut sorted_global,
            per_target,
        } = self;
        sorted_global.sort_unstable();
        snapshot.p50 = percentile(&sorted_global, 50.0);
        snapshot.p95 = percentile(&sorted_global, 95.0);
        snapshot.p99 = percentile(&sorted_global, 99.0);
        snapshot.per_target = per_target
            .into_iter()
            .map(|target| {
                let mut sorted = target.latencies_us;
                sorted.sort_unstable();
                TargetStats {
                    count: target.count,
                    p50: percentile(&sorted, 50.0),
                    p95: percentile(&sorted, 95.0),
                    p99: percentile(&sorted, 99.0),
                    error_rate: ratio(target.errors, target.count),
                }
            })
            .collect();
        snapshot
    }
}

/// The average requests per second over `elapsed`, or `0.0` when `elapsed` is (near) zero,
/// instead of dividing by it.
fn average_rps(total: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds < f64::EPSILON {
        0.0
    } else {
        total as f64 / seconds
    }
}

/// `numerator / denominator`, or `0.0` when `denominator` is `0` instead of dividing by zero.
fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// The percentile of `sorted` at `p` (`0.0` to `100.0`), nearest rank method: the value at rank
/// `ceil(p / 100 * n)`, clamped to `1..=n`. `sorted` must already be sorted ascending. Returns
/// `0` for an empty slice.
///
/// This function itself is `O(1)` once `sorted` exists; producing it from raw samples by sorting
/// is `O(n log n)`, paid again on every [`Aggregator::snapshot`] call (see the module docs).
pub(crate) fn percentile(sorted: &[u32], p: f64) -> u32 {
    let Some(n) = u32::try_from(sorted.len()).ok().filter(|&n| n > 0) else {
        return 0;
    };
    let rank = ((p / 100.0) * f64::from(n)).ceil();
    // `rank` is `ceil` of a value in `[0.0, n]`, so it is already in `[0.0, n]` here; the clamp
    // only guards the boundary itself (`p == 0.0` would otherwise ask for rank `0`).
    let rank = (rank as u32).clamp(1, n);
    sorted[rank as usize - 1]
}

/// The histogram bucket for `latency_us`: 26 buckets of 10 ms, the last one (index 25) holding
/// every latency of 250 ms and above.
fn histogram_bucket(latency_us: u32) -> usize {
    let bucket = latency_us / 10_000;
    (bucket as usize).min(25)
}

/// Serializes a [`Duration`] as whole milliseconds, since `serde` has no built-in support for
/// `std::time::Duration` and this crate has no reason to add a dependency just for one field.
mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(
        duration: &Duration,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        millis.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Duration, D::Error> {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Records `sample` with a fixed URL and example text.
    fn record(aggregator: &mut Aggregator, sample: Sample) -> (u64, u64) {
        aggregator.record(sample, "http://t/x", || "boom".to_string())
    }

    fn sample(
        target_index: usize,
        elapsed_secs: u64,
        latency_us: u32,
        status: StatusKey,
    ) -> Sample {
        Sample {
            target_index,
            vu: 0,
            active_vus: 1,
            elapsed: Duration::from_secs(elapsed_secs),
            latency_us,
            size: 10,
            status,
            is_error: status.is_error(),
        }
    }

    #[test]
    fn percentile_of_an_empty_slice_is_zero() {
        assert_eq!(percentile(&[], 50.0), 0);
        assert_eq!(percentile(&[], 99.0), 0);
    }

    #[test]
    fn percentile_nearest_rank_on_ten_values() {
        // 1..=10, so p50 is rank 5 (ceil(0.5*10)=5) and p95 is rank 10 (ceil(0.95*10)=10).
        let sorted: Vec<u32> = (1..=10).collect();
        assert_eq!(percentile(&sorted, 50.0), 5);
        assert_eq!(percentile(&sorted, 95.0), 10);
        assert_eq!(percentile(&sorted, 99.0), 10);
        assert_eq!(percentile(&sorted, 100.0), 10);
    }

    #[test]
    fn percentile_of_a_single_value() {
        assert_eq!(percentile(&[42], 1.0), 42);
        assert_eq!(percentile(&[42], 99.0), 42);
    }

    #[test]
    fn histogram_bucket_boundaries() {
        assert_eq!(histogram_bucket(0), 0);
        assert_eq!(histogram_bucket(9_999), 0);
        assert_eq!(histogram_bucket(10_000), 1);
        assert_eq!(histogram_bucket(249_999), 24);
        assert_eq!(histogram_bucket(250_000), 25);
        assert_eq!(histogram_bucket(10_000_000), 25);
    }

    #[test]
    fn record_updates_totals_histogram_and_status_counts() {
        let mut aggregator = Aggregator::new(1);
        record(&mut aggregator, sample(0, 0, 5_000, StatusKey::Code(200)));
        let (errors, total) = record(&mut aggregator, sample(0, 0, 15_000, StatusKey::Code(500)));

        assert_eq!(total, 2);
        assert_eq!(errors, 1);

        let snapshot = aggregator.snapshot(Duration::from_secs(1), 3);
        assert_eq!(snapshot.total, 2);
        assert_eq!(snapshot.active_vus, 3);
        assert_eq!(snapshot.histogram[0], 1);
        assert_eq!(snapshot.histogram[1], 1);
        assert_eq!(
            snapshot.status_counts,
            vec![(StatusKey::Code(200), 1), (StatusKey::Code(500), 1)]
        );
        assert!((snapshot.error_rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn per_target_is_kept_in_target_index_order() {
        let mut aggregator = Aggregator::new(2);
        record(&mut aggregator, sample(1, 0, 10_000, StatusKey::Code(200)));
        record(&mut aggregator, sample(0, 0, 20_000, StatusKey::Code(200)));

        let snapshot = aggregator.snapshot(Duration::from_secs(1), 0);
        assert_eq!(snapshot.per_target.len(), 2);
        assert_eq!(snapshot.per_target[0].count, 1);
        assert_eq!(snapshot.per_target[0].p50, 20_000);
        assert_eq!(snapshot.per_target[1].count, 1);
        assert_eq!(snapshot.per_target[1].p50, 10_000);
    }

    #[test]
    fn series_has_one_point_per_completed_second_and_rps_is_the_last_one() {
        let mut aggregator = Aggregator::new(1);
        record(&mut aggregator, sample(0, 0, 10_000, StatusKey::Code(200)));
        record(&mut aggregator, sample(0, 0, 10_000, StatusKey::Code(200)));
        record(&mut aggregator, sample(0, 1, 10_000, StatusKey::Code(200)));

        // 2.5 seconds elapsed: second 0 and second 1 are fully elapsed, second 2 is not.
        let snapshot = aggregator.snapshot(Duration::from_millis(2_500), 0);
        assert_eq!(snapshot.series.len(), 2);
        assert!((snapshot.series[0].rps - 2.0).abs() < f64::EPSILON);
        assert!((snapshot.series[1].rps - 1.0).abs() < f64::EPSILON);
        assert!((snapshot.rps - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_second_with_no_samples_is_zero_in_the_series() {
        let mut aggregator = Aggregator::new(1);
        record(&mut aggregator, sample(0, 0, 10_000, StatusKey::Code(200)));
        // Nothing recorded for second 1: the run was idle.
        record(&mut aggregator, sample(0, 2, 10_000, StatusKey::Code(200)));

        let snapshot = aggregator.snapshot(Duration::from_secs(3), 0);
        assert_eq!(snapshot.series.len(), 3);
        assert!((snapshot.series[1].rps - 0.0).abs() < f64::EPSILON);
        assert_eq!(snapshot.series[1].p95, 0);
    }

    /// Reproduces a bug where every sample landed in second 0 (a run that
    /// clearly had throughput throughout), but by the time every virtual user thread is joined,
    /// `elapsed` has ticked into a new, idle second (`crate::run::run_supervisor`'s own join wait,
    /// since `stop` does not abort a request already in flight). [`Aggregator::snapshot`]'s "last
    /// fully elapsed second" rule reports that idle second's `0.0`, not the run's real throughput.
    #[test]
    fn snapshot_reports_zero_rps_when_elapsed_ticks_past_the_runs_last_active_second() {
        let mut aggregator = Aggregator::new(1);
        for _ in 0..10 {
            record(&mut aggregator, sample(0, 0, 10_000, StatusKey::Code(200)));
        }

        // Every sample landed in second 0, but `elapsed` (measured after joining every virtual
        // user) has already ticked into second 1, which has no samples at all.
        let snapshot = aggregator.snapshot(Duration::from_millis(2_050), 0);
        assert_eq!(snapshot.total, 10);
        assert!(
            (snapshot.rps - 0.0).abs() < f64::EPSILON,
            "expected the documented (if misleading) live behavior, got {}",
            snapshot.rps
        );
    }

    #[test]
    fn final_snapshot_reports_the_whole_runs_average_rps_instead_of_a_trailing_idle_second() {
        let mut aggregator = Aggregator::new(1);
        for _ in 0..10 {
            record(&mut aggregator, sample(0, 0, 10_000, StatusKey::Code(200)));
        }

        // Same trailing-idle-second shape as the live case above: `final_snapshot` must not
        // report `0.0` here, since 10 requests clearly did complete over this run.
        let snapshot = aggregator.final_snapshot(Duration::from_millis(2_050));
        assert!(
            (snapshot.rps - 10.0 / 2.05).abs() < 1e-9,
            "expected the whole-run average, got {}",
            snapshot.rps
        );
        // Every other field is unaffected: still built by the ordinary `snapshot`.
        assert_eq!(snapshot.total, 10);
        assert_eq!(snapshot.series.len(), 2);
    }

    #[test]
    fn final_snapshot_of_an_empty_run_has_zero_rps_not_a_division_by_zero() {
        let mut aggregator = Aggregator::new(1);
        let snapshot = aggregator.final_snapshot(Duration::ZERO);
        assert!((snapshot.rps - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_of_an_empty_aggregator_has_no_samples_and_no_error_rate() {
        let mut aggregator = Aggregator::new(1);
        let snapshot = aggregator.snapshot(Duration::from_secs(0), 0);
        assert_eq!(snapshot.total, 0);
        assert_eq!(snapshot.p50, 0);
        assert!((snapshot.error_rate - 0.0).abs() < f64::EPSILON);
        assert!(snapshot.series.is_empty());
    }

    #[test]
    fn record_appends_every_sample_to_the_log_with_interned_urls() {
        let mut aggregator = Aggregator::new(2);
        let first = Sample {
            vu: 3,
            size: 77,
            elapsed: Duration::from_millis(1_234),
            ..sample(1, 0, 5_000, StatusKey::Code(404))
        };
        aggregator.record(first, "http://t/a", || "HTTP 404".to_string());
        aggregator.record(
            sample(0, 0, 6_000, StatusKey::Code(200)),
            "http://t/b",
            String::new,
        );
        aggregator.record(
            sample(0, 0, 7_000, StatusKey::Code(200)),
            "http://t/a",
            String::new,
        );

        let log = aggregator.log();
        assert_eq!(log.urls(), ["http://t/a", "http://t/b"]);
        let samples = log.samples();
        assert_eq!(samples.len(), 3);
        assert_eq!(
            samples[0],
            SampleRecord {
                elapsed_ms: 1_234,
                vu: 3,
                target: 1,
                url: 0,
                status: StatusKey::Code(404),
                latency_us: 5_000,
                size: 77,
                is_error: true,
            }
        );
        assert_eq!((samples[1].url, samples[2].url), (1, 0));
    }

    #[test]
    fn only_the_first_error_of_each_status_keeps_an_example() {
        let mut aggregator = Aggregator::new(1);
        aggregator.record(sample(0, 0, 1, StatusKey::Code(200)), "u", || {
            "never asked for".to_string()
        });
        aggregator.record(sample(0, 0, 1, StatusKey::Code(500)), "u", || {
            "first".to_string()
        });
        aggregator.record(sample(0, 0, 1, StatusKey::Code(500)), "u", || {
            "second".to_string()
        });
        aggregator.record(sample(0, 0, 1, StatusKey::Timeout), "u", || {
            "slow".to_string()
        });

        let snapshot = aggregator.snapshot(Duration::from_secs(1), 0);
        assert_eq!(
            snapshot.error_examples,
            vec![
                ErrorExample {
                    status: StatusKey::Code(500),
                    text: "first".to_string()
                },
                ErrorExample {
                    status: StatusKey::Timeout,
                    text: "slow".to_string()
                },
            ]
        );
    }

    #[test]
    fn series_points_carry_the_percentiles_and_the_most_active_vus_of_their_second() {
        let mut aggregator = Aggregator::new(1);
        for (latency, active_vus) in [(10, 2), (20, 5), (30, 4), (40, 3)] {
            let sample = Sample {
                active_vus,
                ..sample(0, 0, latency * 1_000, StatusKey::Code(200))
            };
            record(&mut aggregator, sample);
        }

        let snapshot = aggregator.snapshot(Duration::from_secs(1), 0);
        let point = snapshot.series[0];
        assert_eq!(point.vus, 5);
        // Nearest rank on [10, 20, 30, 40] ms.
        assert_eq!(point.p50, 20_000);
        assert_eq!(point.p90, 40_000);
        assert_eq!(point.p95, 40_000);
        assert_eq!(point.p99, 40_000);
    }

    #[test]
    fn a_completed_seconds_percentiles_are_cached_until_a_late_sample_lands_in_it() {
        let mut aggregator = Aggregator::new(1);
        record(&mut aggregator, sample(0, 0, 10_000, StatusKey::Code(200)));
        aggregator.snapshot(Duration::from_secs(1), 0);
        assert!(aggregator.per_second[0].percentiles.is_some());

        // A thread that measured its time before the second ended records after it.
        record(&mut aggregator, sample(0, 0, 90_000, StatusKey::Code(200)));
        assert!(aggregator.per_second[0].percentiles.is_none());
        let snapshot = aggregator.snapshot(Duration::from_secs(1), 0);
        assert_eq!(snapshot.series[0].p99, 90_000);
    }

    #[test]
    fn an_old_snapshot_without_the_newer_fields_still_loads() {
        let mut aggregator = Aggregator::new(1);
        record(&mut aggregator, sample(0, 0, 12_345, StatusKey::Code(500)));
        let snapshot = aggregator.snapshot(Duration::from_secs(1), 2);
        let mut value = serde_json::to_value(&snapshot).expect("snapshot should serialize");
        let object = value.as_object_mut().expect("an object");
        object.remove("error_examples");
        let point = object["series"][0].as_object_mut().expect("a point");
        for key in ["p50", "p90", "p99", "vus"] {
            point.remove(key);
        }

        let old: LoadSnapshot = serde_json::from_value(value).expect("an old snapshot loads");
        assert!(old.error_examples.is_empty());
        assert_eq!((old.series[0].p50, old.series[0].vus), (0, 0));
        assert_eq!(old.series[0].p95, 12_345);
    }

    #[test]
    fn a_snapshot_round_trips_through_json() {
        let mut aggregator = Aggregator::new(1);
        record(&mut aggregator, sample(0, 0, 12_345, StatusKey::Code(200)));
        record(&mut aggregator, sample(0, 0, 999_999, StatusKey::Timeout));
        let snapshot = aggregator.snapshot(Duration::from_millis(1_500), 2);

        let json = serde_json::to_string(&snapshot).expect("snapshot should serialize");
        let round_tripped: LoadSnapshot =
            serde_json::from_str(&json).expect("snapshot should deserialize");
        assert_eq!(round_tripped, snapshot);
    }
}
