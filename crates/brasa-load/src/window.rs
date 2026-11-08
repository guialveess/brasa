//! [`window_stats`], the statistics of a [`SampleLog`] restricted to a time window.
//!
//! The results view lets the user select a stretch of the run on the chart and shows the KPIs,
//! the latency histogram, the status breakdown, the per request table and the error groups for
//! just that stretch. All of that is computed here, from the log, so it works the same for a run
//! that just ended and for one loaded from the history.
//!
//! # Cost
//!
//! One pass over the log filters the window and gathers everything that needs no ordering
//! (counts, errors, statuses, error groups); the window's latencies are then sorted once for the
//! percentiles, plus once per target for the per request table. That is `O(n log n)` in the
//! samples of the window, with no repeated scans.
//!
//! # Windows
//!
//! A window is a range of whole seconds `[start, end)` of `elapsed_ms / 1000`. Because the log is
//! only approximately time ordered (see [`crate::samples`]), the window filters by the sample's
//! own time and never assumes a sorted log.

use std::ops::Range;

use crate::metrics::{TargetStats, percentile};
use crate::progress::{Progress, Ticker};
use crate::samples::SampleLog;
use crate::status::StatusKey;
use crate::threshold::ThresholdInputs;

/// How many buckets a [`LatencyHistogram`] has. The last one is open ended.
const HISTOGRAM_BUCKETS: usize = 26;

/// The bucket widths a [`LatencyHistogram`] picks from, in microseconds: 100, 200, 400 and 500
/// microseconds, then 1, 2, 4, 5, 10, 20, 25, 40, 50, 100, 200, 250, 400 and 500 milliseconds,
/// then whole seconds up to 500. Round numbers, so the axis labels read cleanly.
const NICE_WIDTHS_US: [u32; 31] = [
    100,
    200,
    400,
    500,
    1_000,
    2_000,
    4_000,
    5_000,
    10_000,
    20_000,
    25_000,
    40_000,
    50_000,
    100_000,
    200_000,
    250_000,
    400_000,
    500_000,
    1_000_000,
    2_000_000,
    4_000_000,
    5_000_000,
    10_000_000,
    20_000_000,
    25_000_000,
    40_000_000,
    50_000_000,
    100_000_000,
    200_000_000,
    250_000_000,
    500_000_000,
];

/// The latency distribution of a window: 26 buckets of `bucket_us` each, the
/// last one open ended.
///
/// The width adapts to the window: it is the smallest round width such that 25 buckets reach
/// past the window's p99, so a fast endpoint is not squeezed into the first bar and a slow tail
/// does not push the bulk to the left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatencyHistogram {
    /// The width of every bucket but the last, in microseconds.
    pub bucket_us: u32,
    /// The samples per bucket: bucket `i` holds latencies in `[i * bucket_us, (i + 1) *
    /// bucket_us)`, and the last one (index 25) holds everything from `25 * bucket_us` up.
    pub counts: Vec<u64>,
}

/// The errors of one [`StatusKey`] inside a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorGroup {
    /// The status shared by every error of the group.
    pub status: StatusKey,
    /// How many errors of this status fall in the window.
    pub count: u64,
    /// When the first of them completed, in milliseconds since the run started.
    pub first_ms: u32,
    /// When the last of them completed, in milliseconds since the run started.
    pub last_ms: u32,
    /// Errors of this status per second of the whole run, not only of the window, one entry per
    /// second of `run_secs`, for the sparkline that shows where the window sits.
    pub per_second: Vec<u64>,
    /// Errors of this status inside the window per target, indexed like
    /// [`crate::LoadConfig::targets`].
    pub per_target: Vec<u64>,
}

/// The statistics of a [`SampleLog`] over a time window, see [`window_stats`].
#[derive(Debug, Clone, PartialEq)]
pub struct WindowStats {
    /// Samples inside the window.
    pub total: u64,
    /// Of those, the ones that are errors.
    pub errors: u64,
    /// Requests per second: over the whole run, `total / elapsed_secs`; over a range,
    /// `total / range length in seconds`.
    pub rps: f64,
    /// The median latency, in microseconds (nearest rank, like [`crate::LoadSnapshot::p50`]).
    pub p50: u32,
    /// The 95th percentile latency, in microseconds.
    pub p95: u32,
    /// The 99th percentile latency, in microseconds.
    pub p99: u32,
    /// `errors / total` (`0.0` to `1.0`), `0.0` for an empty window.
    pub error_rate: f64,
    /// The latency distribution.
    pub histogram: LatencyHistogram,
    /// How many samples have each status: HTTP codes ascending, then [`StatusKey::Timeout`], then
    /// [`StatusKey::Failed`].
    pub statuses: Vec<(StatusKey, u64)>,
    /// Metrics per target, indexed like [`crate::LoadConfig::targets`].
    pub per_target: Vec<TargetStats>,
    /// How many samples of each target have each status, indexed like [`Self::per_target`] and
    /// ordered like [`Self::statuses`].
    pub target_statuses: Vec<Vec<(StatusKey, u64)>>,
    /// The errors grouped by status, the most frequent first, built only from samples inside the
    /// window.
    pub error_groups: Vec<ErrorGroup>,
}

impl WindowStats {
    /// The numbers the thresholds of a run are checked against, with the latencies in
    /// milliseconds.
    #[must_use]
    pub fn threshold_inputs(&self) -> ThresholdInputs {
        ThresholdInputs {
            rps: self.rps,
            p50_ms: f64::from(self.p50) / 1_000.0,
            p95_ms: f64::from(self.p95) / 1_000.0,
            p99_ms: f64::from(self.p99) / 1_000.0,
            error_rate: self.error_rate,
        }
    }
}

/// An [`ErrorGroup`] being filled, which also counts errors outside the window for its
/// sparkline.
struct GroupBuilder {
    status: StatusKey,
    in_window: u64,
    first_ms: u32,
    last_ms: u32,
    per_second: Vec<u64>,
    per_target: Vec<u64>,
}

/// The statistics of the samples of `log` inside `range`.
///
/// - `target_count` is the number of targets of the run, the length of
///   [`WindowStats::per_target`] and of [`ErrorGroup::per_target`].
/// - `run_secs` is the length of the run in whole seconds, the length of
///   [`ErrorGroup::per_second`].
/// - `range` is the window in whole seconds, `[start, end)` over `elapsed_ms / 1000`; `None` is
///   the whole run.
/// - `elapsed_secs` is the real duration of the run, the divisor of the requests per second when
///   `range` is `None`.
#[must_use]
pub fn window_stats(
    log: &SampleLog,
    target_count: usize,
    run_secs: u32,
    range: Option<Range<u32>>,
    elapsed_secs: f64,
) -> WindowStats {
    compute(
        log,
        target_count,
        run_secs,
        range,
        elapsed_secs,
        &Progress::default(),
    )
}

/// [`window_stats`] reporting each sample to `progress`, `None` when it is cancelled.
#[must_use]
pub fn window_stats_with(
    log: &SampleLog,
    target_count: usize,
    run_secs: u32,
    range: Option<Range<u32>>,
    elapsed_secs: f64,
    progress: &Progress,
) -> Option<WindowStats> {
    let stats = compute(log, target_count, run_secs, range, elapsed_secs, progress);
    (!progress.is_cancelled()).then_some(stats)
}

/// [`window_stats_with`], with whatever was counted when cancelled.
fn compute(
    log: &SampleLog,
    target_count: usize,
    run_secs: u32,
    range: Option<Range<u32>>,
    elapsed_secs: f64,
    progress: &Progress,
) -> WindowStats {
    let mut ticker = Ticker::new(progress);
    let mut latencies: Vec<u32> = Vec::new();
    let mut target_latencies: Vec<Vec<u32>> = vec![Vec::new(); target_count];
    let mut target_errors = vec![0u64; target_count];
    let mut errors = 0u64;
    let mut statuses: Vec<(StatusKey, u64)> = Vec::new();
    let mut target_statuses: Vec<Vec<(StatusKey, u64)>> = vec![Vec::new(); target_count];
    let mut groups: Vec<GroupBuilder> = Vec::new();

    for sample in log.samples() {
        if !ticker.tick() {
            break;
        }
        let second = sample.elapsed_ms / 1_000;
        let inside = range.as_ref().is_none_or(|range| range.contains(&second));
        let target = sample.target as usize;

        if sample.is_error {
            let index = match groups
                .iter()
                .position(|group| group.status == sample.status)
            {
                Some(index) => index,
                None => {
                    groups.push(GroupBuilder {
                        status: sample.status,
                        in_window: 0,
                        first_ms: u32::MAX,
                        last_ms: 0,
                        per_second: vec![0; run_secs as usize],
                        per_target: vec![0; target_count],
                    });
                    groups.len() - 1
                }
            };
            let group = &mut groups[index];
            // The run can end a few milliseconds past its last whole second.
            let last_second = (run_secs as usize).saturating_sub(1);
            if let Some(count) = group.per_second.get_mut((second as usize).min(last_second)) {
                *count += 1;
            }
            if inside {
                group.in_window += 1;
                group.first_ms = group.first_ms.min(sample.elapsed_ms);
                group.last_ms = group.last_ms.max(sample.elapsed_ms);
                if let Some(count) = group.per_target.get_mut(target) {
                    *count += 1;
                }
            }
        }

        if !inside {
            continue;
        }
        latencies.push(sample.latency_us);
        if sample.is_error {
            errors += 1;
        }
        count_status(&mut statuses, sample.status);
        if let Some(counts) = target_statuses.get_mut(target) {
            count_status(counts, sample.status);
        }
        if let Some(values) = target_latencies.get_mut(target) {
            values.push(sample.latency_us);
            if sample.is_error {
                target_errors[target] += 1;
            }
        }
    }

    latencies.sort_unstable();
    let total = latencies.len() as u64;
    let p99 = percentile(&latencies, 99.0);

    let per_target = target_latencies
        .into_iter()
        .zip(target_errors)
        .map(|(mut values, errors)| {
            values.sort_unstable();
            let count = values.len() as u64;
            TargetStats {
                count,
                p50: percentile(&values, 50.0),
                p95: percentile(&values, 95.0),
                p99: percentile(&values, 99.0),
                error_rate: ratio(errors, count),
            }
        })
        .collect();

    statuses.sort_by_key(|(status, _)| status_order(*status));
    for counts in &mut target_statuses {
        counts.sort_by_key(|(status, _)| status_order(*status));
    }

    let mut error_groups: Vec<ErrorGroup> = groups
        .into_iter()
        .filter(|group| group.in_window > 0)
        .map(|group| ErrorGroup {
            status: group.status,
            count: group.in_window,
            first_ms: group.first_ms,
            last_ms: group.last_ms,
            per_second: group.per_second,
            per_target: group.per_target,
        })
        .collect();
    error_groups.sort_by_key(|group| (std::cmp::Reverse(group.count), status_order(group.status)));

    let window_secs = match &range {
        Some(range) => f64::from(range.end.saturating_sub(range.start)),
        None => elapsed_secs,
    };

    WindowStats {
        total,
        errors,
        rps: if window_secs > f64::EPSILON {
            total as f64 / window_secs
        } else {
            0.0
        },
        p50: percentile(&latencies, 50.0),
        p95: percentile(&latencies, 95.0),
        p99,
        error_rate: ratio(errors, total),
        histogram: histogram(&latencies, p99),
        statuses,
        per_target,
        target_statuses,
        error_groups,
    }
}

/// Adds one sample of `status` to `counts`.
fn count_status(counts: &mut Vec<(StatusKey, u64)>, status: StatusKey) {
    match counts.iter_mut().find(|(key, _)| *key == status) {
        Some((_, count)) => *count += 1,
        None => counts.push((status, 1)),
    }
}

/// The order statuses are listed in: HTTP codes ascending, then timeouts, then other failures.
fn status_order(status: StatusKey) -> (u8, u16) {
    match status {
        StatusKey::Code(code) => (0, code),
        StatusKey::Timeout => (1, 0),
        StatusKey::Failed => (2, 0),
    }
}

/// `numerator / denominator`, or `0.0` when `denominator` is `0`.
fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

/// The smallest of [`NICE_WIDTHS_US`] for which 25 buckets reach past `p99_us`, or the widest one
/// when none does.
fn bucket_width(p99_us: u32) -> u32 {
    let reach = (HISTOGRAM_BUCKETS - 1) as u64;
    NICE_WIDTHS_US
        .iter()
        .copied()
        .find(|&width| u64::from(width) * reach > u64::from(p99_us))
        .unwrap_or(NICE_WIDTHS_US[NICE_WIDTHS_US.len() - 1])
}

/// Buckets `sorted` (the window's latencies) with a width picked from `p99_us`.
fn histogram(sorted: &[u32], p99_us: u32) -> LatencyHistogram {
    let bucket_us = bucket_width(p99_us);
    let mut counts = vec![0u64; HISTOGRAM_BUCKETS];
    for &latency in sorted {
        let bucket = ((latency / bucket_us) as usize).min(HISTOGRAM_BUCKETS - 1);
        counts[bucket] += 1;
    }
    LatencyHistogram { bucket_us, counts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples::SampleRecord;
    use pretty_assertions::assert_eq;

    /// One hand-made sample: `(elapsed_ms, target, status, latency_us)`.
    type Row = (u32, u32, StatusKey, u32);

    fn log_of(rows: &[Row]) -> SampleLog {
        let mut log = SampleLog::new();
        let url = log.intern("http://t/x");
        for &(elapsed_ms, target, status, latency_us) in rows {
            log.push(SampleRecord {
                elapsed_ms,
                vu: 0,
                target,
                url,
                status,
                latency_us,
                size: 0,
                is_error: status.is_error(),
            });
        }
        log
    }

    const OK: StatusKey = StatusKey::Code(200);

    #[test]
    fn the_whole_run_counts_everything_and_divides_by_elapsed() {
        let log = log_of(&[
            (100, 0, OK, 1_000),
            (1_100, 0, OK, 2_000),
            (2_100, 1, StatusKey::Code(500), 3_000),
            (3_100, 1, StatusKey::Timeout, 4_000),
        ]);
        let stats = window_stats(&log, 2, 4, None, 4.0);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.errors, 2);
        assert!((stats.rps - 1.0).abs() < 1e-9);
        assert!((stats.error_rate - 0.5).abs() < 1e-9);
        // Nearest rank on [1000, 2000, 3000, 4000]: p50 is rank 2, p95 and p99 rank 4.
        assert_eq!((stats.p50, stats.p95, stats.p99), (2_000, 4_000, 4_000));
    }

    #[test]
    fn a_range_keeps_whole_seconds_start_inclusive_end_exclusive() {
        let log = log_of(&[
            (999, 0, OK, 1_000),
            (1_000, 0, OK, 2_000),
            (1_999, 0, OK, 3_000),
            (2_000, 0, OK, 4_000),
        ]);
        let stats = window_stats(&log, 1, 3, Some(1..2), 3.0);
        assert_eq!(stats.total, 2);
        assert_eq!(stats.p50, 2_000);
        // total / range length (1 s), not / elapsed.
        assert!((stats.rps - 2.0).abs() < 1e-9);
        let stats = window_stats(&log, 1, 3, Some(0..2), 3.0);
        assert_eq!(stats.total, 3);
        assert!((stats.rps - 1.5).abs() < 1e-9);
    }

    #[test]
    fn an_empty_window_is_all_zeros_without_dividing_by_zero() {
        let log = log_of(&[(100, 0, OK, 1_000)]);
        let stats = window_stats(&log, 1, 1, Some(5..5), 1.0);
        assert_eq!(stats.total, 0);
        assert!(stats.rps.abs() < f64::EPSILON);
        assert!(stats.error_rate.abs() < f64::EPSILON);
        assert_eq!(stats.p99, 0);
        assert_eq!(stats.histogram.counts, vec![0; 26]);
        assert!(stats.statuses.is_empty());
        assert!(stats.error_groups.is_empty());
        assert_eq!(stats.per_target.len(), 1);
        assert_eq!(stats.per_target[0].count, 0);

        let none = window_stats(&SampleLog::new(), 0, 0, None, 0.0);
        assert_eq!(none.total, 0);
        assert!(none.rps.abs() < f64::EPSILON);
    }

    #[test]
    fn statuses_are_codes_ascending_then_timeout_then_failed() {
        let log = log_of(&[
            (0, 0, StatusKey::Failed, 1),
            (0, 0, StatusKey::Code(500), 1),
            (0, 0, StatusKey::Timeout, 1),
            (0, 0, StatusKey::Code(200), 1),
            (0, 0, StatusKey::Code(200), 1),
            (0, 0, StatusKey::Code(404), 1),
        ]);
        let stats = window_stats(&log, 1, 1, None, 1.0);
        assert_eq!(
            stats.statuses,
            vec![
                (StatusKey::Code(200), 2),
                (StatusKey::Code(404), 1),
                (StatusKey::Code(500), 1),
                (StatusKey::Timeout, 1),
                (StatusKey::Failed, 1),
            ]
        );
    }

    #[test]
    fn per_target_stats_are_indexed_by_target_and_ignore_unknown_targets() {
        let log = log_of(&[
            (0, 1, OK, 10_000),
            (0, 0, OK, 20_000),
            (0, 0, StatusKey::Code(500), 40_000),
            (0, 7, OK, 99_000),
        ]);
        let stats = window_stats(&log, 2, 1, None, 1.0);
        assert_eq!(stats.total, 4);
        assert_eq!(stats.per_target[1].count, 1);
        assert_eq!(stats.per_target[1].p50, 10_000);
        assert_eq!(stats.per_target[0].count, 2);
        assert_eq!(stats.per_target[0].p50, 20_000);
        assert_eq!(stats.per_target[0].p99, 40_000);
        assert!((stats.per_target[0].error_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn each_target_counts_its_own_statuses_in_the_window() {
        let log = log_of(&[
            (0, 0, StatusKey::Code(500), 1),
            (0, 0, OK, 1),
            (0, 1, OK, 1),
            (0, 0, OK, 1),
            (1_000, 1, StatusKey::Timeout, 1),
        ]);
        let stats = window_stats(&log, 2, 2, Some(0..1), 2.0);
        assert_eq!(
            stats.target_statuses,
            vec![vec![(OK, 2), (StatusKey::Code(500), 1)], vec![(OK, 1)],]
        );
    }

    #[test]
    fn error_groups_count_the_window_but_the_sparkline_spans_the_run() {
        let log = log_of(&[
            // Outside the window (second 0): only feeds the sparkline.
            (500, 0, StatusKey::Code(502), 1),
            (1_200, 0, StatusKey::Code(429), 1),
            (1_900, 1, StatusKey::Code(429), 1),
            (2_500, 1, StatusKey::Code(429), 1),
            (2_600, 0, StatusKey::Code(502), 1),
            (2_700, 0, OK, 1),
            // Only outside the window: no group at all.
            (3_500, 0, StatusKey::Timeout, 1),
            // A few ms past the last whole second: clamped into the last sparkline slot.
            (4_010, 0, StatusKey::Code(502), 1),
        ]);
        let stats = window_stats(&log, 2, 4, Some(1..3), 4.0);
        assert_eq!(stats.error_groups.len(), 2);

        let first = &stats.error_groups[0];
        assert_eq!(first.status, StatusKey::Code(429));
        assert_eq!(first.count, 3);
        assert_eq!((first.first_ms, first.last_ms), (1_200, 2_500));
        assert_eq!(first.per_second, vec![0, 2, 1, 0]);
        assert_eq!(first.per_target, vec![1, 2]);

        let second = &stats.error_groups[1];
        assert_eq!(second.status, StatusKey::Code(502));
        assert_eq!(second.count, 1);
        assert_eq!((second.first_ms, second.last_ms), (2_600, 2_600));
        // Second 0, second 2, and the sample at 4_010 ms clamped into second 3.
        assert_eq!(second.per_second, vec![1, 0, 1, 1]);
        assert_eq!(second.per_target, vec![1, 0]);
    }

    #[test]
    fn error_groups_are_sorted_by_count_then_status() {
        let log = log_of(&[
            (0, 0, StatusKey::Code(503), 1),
            (0, 0, StatusKey::Timeout, 1),
            (0, 0, StatusKey::Timeout, 1),
            (0, 0, StatusKey::Code(500), 1),
        ]);
        let stats = window_stats(&log, 1, 1, None, 1.0);
        let order: Vec<StatusKey> = stats
            .error_groups
            .iter()
            .map(|group| group.status)
            .collect();
        assert_eq!(
            order,
            vec![
                StatusKey::Timeout,
                StatusKey::Code(500),
                StatusKey::Code(503)
            ]
        );
    }

    #[test]
    fn the_bucket_width_is_the_smallest_nice_width_reaching_past_p99() {
        assert_eq!(bucket_width(0), 100);
        // 25 * 100 us = 2.5 ms.
        assert_eq!(bucket_width(2_499), 100);
        assert_eq!(bucket_width(2_500), 200);
        // The design's example: p99 of 90 ms needs 4 ms buckets (25 * 4 ms = 100 ms).
        assert_eq!(bucket_width(90_000), 4_000);
        assert_eq!(bucket_width(100_000), 5_000);
        assert_eq!(bucket_width(1_249_999), 50_000);
        assert_eq!(bucket_width(u32::MAX), 200_000_000);
    }

    #[test]
    fn the_histogram_buckets_by_width_and_the_last_bucket_is_open_ended() {
        // The 9 s outlier is the p99, and 25 buckets of 400 ms are the first to reach past it.
        let log = log_of(&[
            (0, 0, OK, 0),
            (0, 0, OK, 4_999),
            (0, 0, OK, 399_999),
            (0, 0, OK, 400_000),
            (0, 0, OK, 9_000_000),
        ]);
        let stats = window_stats(&log, 1, 1, None, 1.0);
        assert_eq!(stats.histogram.bucket_us, 400_000);
        assert_eq!(stats.histogram.counts[0], 3);
        assert_eq!(stats.histogram.counts[1], 1);
        assert_eq!(stats.histogram.counts[22], 1);
        assert_eq!(stats.histogram.counts.len(), 26);

        let small = log_of(&[
            (0, 0, OK, 0),
            (0, 0, OK, 4_999),
            (0, 0, OK, 5_000),
            (0, 0, OK, 90_000),
        ]);
        let stats = window_stats(&small, 1, 1, None, 1.0);
        assert_eq!(stats.histogram.bucket_us, 4_000);
        assert_eq!(stats.histogram.counts[0], 1);
        assert_eq!(stats.histogram.counts[1], 2);
        assert_eq!(stats.histogram.counts[22], 1);
        assert_eq!(stats.histogram.counts.iter().sum::<u64>(), 4);

        let open = histogram(&[1_000_000], 100);
        assert_eq!(open.bucket_us, 100);
        assert_eq!(open.counts[25], 1);
    }

    #[test]
    fn threshold_inputs_convert_latencies_to_milliseconds() {
        let log = log_of(&[(0, 0, OK, 10_000), (0, 0, StatusKey::Code(500), 30_000)]);
        let inputs = window_stats(&log, 1, 1, None, 1.0).threshold_inputs();
        assert!((inputs.rps - 2.0).abs() < 1e-9);
        assert!((inputs.p50_ms - 10.0).abs() < 1e-9);
        assert!((inputs.p99_ms - 30.0).abs() < 1e-9);
        assert!((inputs.error_rate - 0.5).abs() < 1e-9);
    }

    /// Not a timing test: it only makes sure a million samples go through, which would not
    /// finish in a test run if the function scanned the log once per sample or per second.
    #[test]
    fn a_million_samples_are_not_quadratic() {
        let mut log = SampleLog::new();
        let urls = [log.intern("http://t/a"), log.intern("http://t/b")];
        let mut state = 12_345u64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };
        for index in 0..1_000_000u32 {
            let random = next();
            let status = match random % 100 {
                0 => StatusKey::Code(500),
                1 => StatusKey::Timeout,
                2 => StatusKey::Code(429),
                _ => StatusKey::Code(200),
            };
            log.push(SampleRecord {
                elapsed_ms: index / 17,
                vu: random % 50,
                target: random % 3,
                url: urls[(random % 2) as usize],
                status,
                latency_us: 1_000 + next() % 90_000,
                size: 100,
                is_error: status.is_error(),
            });
        }

        let all = window_stats(&log, 3, 60, None, 60.0);
        assert_eq!(all.total, 1_000_000);
        assert_eq!(all.histogram.counts.iter().sum::<u64>(), 1_000_000);
        assert_eq!(
            all.per_target
                .iter()
                .map(|target| target.count)
                .sum::<u64>(),
            1_000_000
        );
        assert!(all.error_groups.len() >= 3);

        let part = window_stats(&log, 3, 60, Some(10..20), 60.0);
        assert!(part.total > 0 && part.total < all.total);
    }
}
