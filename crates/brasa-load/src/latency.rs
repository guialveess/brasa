//! [`latency_view`], the latency of a [`SampleLog`] per request: the histogram, the percentiles
//! and the latency by percentile curve of the results view's Latency tab and of the exports.
//!
//! The window works like [`crate::window_stats`]'s: a range of whole seconds `[start, end)` of
//! `elapsed_ms / 1000`, `None` for the whole run. Errors are left out unless asked for, since a
//! fast `429` or a slow timeout would bend a request's profile.
//!
//! # Histogram
//!
//! The bins span the selection (one request, or all of them) up to its p99.9, the last bin also
//! holding the slower tail, so a single outlier does not squeeze the bulk into a few bars. Each
//! bin counts every request separately, so all requests read as stacked profiles.
//!
//! - Linear: round bin widths from [`NICE_STEPS_US`], at most [`MAX_LINEAR_BINS`] of them. All
//!   requests start at 0; one request zooms to its own range.
//! - Log: [`LOG_BINS`] bins of the same width on a log axis, from just below the fastest
//!   response to just above the p99.9, so requests of very different speeds get the same room.
//!
//! # Cost
//!
//! One pass over the log, then one sort per request, so `O(n log n)` in the samples of the
//! window. Meant for a background thread, never for a render path.

use std::ops::Range;

use crate::metrics::percentile;
use crate::progress::{Progress, Ticker};
use crate::samples::SampleLog;

/// The most bins of a linear histogram.
pub const MAX_LINEAR_BINS: usize = 60;
/// The bins of a log histogram.
pub const LOG_BINS: usize = 72;
/// The points of a [`LatencyCurve`], from the fastest response to p99.9.
pub const CURVE_POINTS: usize = 61;
/// How many decades of `1 - p` the curve spans: 3 reaches p99.9.
pub const CURVE_DECADES: f64 = 3.0;

/// The bin widths of a linear histogram, in microseconds: round numbers from 0.5 ms to 10 s.
const NICE_STEPS_US: [u32; 18] = [
    500, 1_000, 2_000, 5_000, 10_000, 20_000, 25_000, 50_000, 100_000, 200_000, 250_000, 500_000,
    1_000_000, 2_000_000, 2_500_000, 5_000_000, 10_000_000, 20_000_000,
];
/// The share of the fastest response the log axis starts at, and of the p99.9 it ends at, so no
/// bar sits on an edge.
const LOG_MARGIN: f64 = 0.08;

/// How the histogram's latency axis is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LatencyScale {
    /// Evenly spaced latencies.
    #[default]
    Linear,
    /// Evenly spaced powers.
    Log,
}

/// What [`latency_view`] reads from the log.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LatencyOptions {
    /// The window in whole seconds, `None` for the whole run.
    pub range: Option<Range<u32>>,
    /// Whether errors count too.
    pub include_errors: bool,
    /// The request the histogram shows, by target index; `None` for all of them.
    pub target: Option<usize>,
    /// The histogram's latency axis.
    pub scale: LatencyScale,
}

/// The percentiles of a set of responses, in microseconds (nearest rank, like
/// [`crate::LoadSnapshot::p50`]). All zero for no responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LatencyPercentiles {
    /// The responses.
    pub count: u64,
    /// The median.
    pub p50: u32,
    /// The 90th percentile.
    pub p90: u32,
    /// The 95th percentile.
    pub p95: u32,
    /// The 99th percentile.
    pub p99: u32,
    /// The slowest response.
    pub max: u32,
}

/// The latency of a set of responses at each of [`CURVE_POINTS`] percentiles: point `i` is the
/// percentile `100 * (1 - 10^(-x))` with `x = i * CURVE_DECADES / (CURVE_POINTS - 1)`, so point 0
/// is the fastest response and the last one is p99.9, evenly spaced on a log axis of `1 - p`.
/// Empty for no responses.
pub type LatencyCurve = Vec<u32>;

/// A histogram whose bins count each request separately.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StackedHistogram {
    /// The bin edges in microseconds, one more than the bins. The last bin also holds every
    /// response above its upper edge.
    pub edges_us: Vec<f64>,
    /// The responses of each bin, by target index.
    pub counts: Vec<Vec<u64>>,
    /// The scale the edges follow.
    pub scale: LatencyScale,
    /// The width of a linear bin, in microseconds; `0` for a log histogram.
    pub step_us: u32,
}

impl StackedHistogram {
    /// The responses of bin `bin`, every request together.
    #[must_use]
    pub fn total(&self, bin: usize) -> u64 {
        self.counts.get(bin).map_or(0, |counts| counts.iter().sum())
    }

    /// The fraction (0 through 1) of the axis where `latency_us` sits, clamped to the axis.
    #[must_use]
    pub fn fraction(&self, latency_us: f64) -> f64 {
        let (Some(&low), Some(&high)) = (self.edges_us.first(), self.edges_us.last()) else {
            return 0.0;
        };
        let fraction = match self.scale {
            LatencyScale::Linear => (latency_us - low) / (high - low),
            LatencyScale::Log => (latency_us.max(low).ln() - low.ln()) / (high.ln() - low.ln()),
        };
        if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

/// Everything the Latency tab shows for one [`LatencyOptions`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LatencyView {
    /// The percentiles of all requests together.
    pub all: LatencyPercentiles,
    /// The percentiles of each request, by target index.
    pub per_target: Vec<LatencyPercentiles>,
    /// The latency by percentile curve of each request, by target index.
    pub curves: Vec<LatencyCurve>,
    /// The histogram of the selected request, or of all of them.
    pub histogram: StackedHistogram,
    /// The responses the histogram counts.
    pub shown: u64,
    /// The errors of the selected request (or of all of them) left out because
    /// [`LatencyOptions::include_errors`] is off.
    pub excluded: u64,
}

/// The latency of the samples of `log` inside `options.range`, for a run of `target_count`
/// requests. See the [module docs](self).
#[must_use]
pub fn latency_view(log: &SampleLog, target_count: usize, options: &LatencyOptions) -> LatencyView {
    build_view(log, target_count, options, None, &Progress::default())
}

/// [`latency_view`] reporting each sample to `progress`, `None` when it is cancelled.
#[must_use]
pub fn latency_view_with(
    log: &SampleLog,
    target_count: usize,
    options: &LatencyOptions,
    progress: &Progress,
) -> Option<LatencyView> {
    let view = build_view(log, target_count, options, None, progress);
    (!progress.is_cancelled()).then_some(view)
}

/// [`latency_view`] with the histogram on the bins of `axis` (another run's), for comparing two
/// runs on the same scale. Responses outside the axis are left out of the histogram, but not of
/// the percentiles and curves.
#[must_use]
pub fn latency_view_on(
    log: &SampleLog,
    target_count: usize,
    options: &LatencyOptions,
    axis: &StackedHistogram,
) -> LatencyView {
    build_view(log, target_count, options, Some(axis), &Progress::default())
}

/// [`latency_view`], on the bins of `axis` when given, reporting each sample to `progress`. When
/// cancelled, the view of the samples read so far.
fn build_view(
    log: &SampleLog,
    target_count: usize,
    options: &LatencyOptions,
    axis: Option<&StackedHistogram>,
    progress: &Progress,
) -> LatencyView {
    let mut latencies: Vec<Vec<u32>> = vec![Vec::new(); target_count];
    let mut errors = vec![0u64; target_count];
    let mut ticker = Ticker::new(progress);
    for sample in log.samples() {
        if !ticker.tick() {
            break;
        }
        let second = sample.elapsed_ms / 1_000;
        if !options
            .range
            .as_ref()
            .is_none_or(|range| range.contains(&second))
        {
            continue;
        }
        let target = sample.target as usize;
        if target >= target_count {
            continue;
        }
        if sample.is_error {
            errors[target] += 1;
            if !options.include_errors {
                continue;
            }
        }
        latencies[target].push(sample.latency_us);
    }
    for values in &mut latencies {
        values.sort_unstable();
    }
    let mut all: Vec<u32> = latencies.iter().flatten().copied().collect();
    all.sort_unstable();

    let selected = options.target.filter(|target| *target < target_count);
    let shown_values = selected.map_or(all.as_slice(), |target| latencies[target].as_slice());
    let histogram = match axis {
        Some(axis) => {
            let mut histogram = StackedHistogram {
                counts: vec![vec![0; target_count]; axis.counts.len()],
                ..axis.clone()
            };
            fill(&mut histogram, &latencies, selected, false);
            histogram
        }
        None => histogram(&latencies, shown_values, selected, options.scale),
    };
    let excluded = if options.include_errors {
        0
    } else {
        selected.map_or_else(|| errors.iter().sum(), |target| errors[target])
    };

    LatencyView {
        all: percentiles(&all),
        per_target: latencies.iter().map(|values| percentiles(values)).collect(),
        curves: latencies.iter().map(|values| curve(values)).collect(),
        histogram,
        shown: shown_values.len() as u64,
        excluded,
    }
}

/// The percentiles of `sorted`.
#[must_use]
pub fn percentiles(sorted: &[u32]) -> LatencyPercentiles {
    LatencyPercentiles {
        count: sorted.len() as u64,
        p50: percentile(sorted, 50.0),
        p90: percentile(sorted, 90.0),
        p95: percentile(sorted, 95.0),
        p99: percentile(sorted, 99.0),
        max: sorted.last().copied().unwrap_or(0),
    }
}

/// The percentile of curve point `index`, from 0 (the fastest response) to 99.9.
#[must_use]
pub fn curve_percentile(index: usize) -> f64 {
    let x = index as f64 * CURVE_DECADES / (CURVE_POINTS - 1) as f64;
    100.0 * (1.0 - 10f64.powf(-x))
}

/// The [`LatencyCurve`] of `sorted`.
#[must_use]
pub fn curve(sorted: &[u32]) -> LatencyCurve {
    if sorted.is_empty() {
        return Vec::new();
    }
    (0..CURVE_POINTS)
        .map(|index| percentile(sorted, curve_percentile(index)))
        .collect()
}

/// The histogram of `shown` (sorted), counting each request of `per_target` separately: only
/// `selected` when one is.
fn histogram(
    per_target: &[Vec<u32>],
    shown: &[u32],
    selected: Option<usize>,
    scale: LatencyScale,
) -> StackedHistogram {
    let Some(&fastest) = shown.first() else {
        return StackedHistogram {
            scale,
            ..StackedHistogram::default()
        };
    };
    let top = f64::from(percentile(shown, 99.9).max(fastest));
    let (edges_us, step_us) = match scale {
        LatencyScale::Linear => {
            let low = if selected.is_some() {
                f64::from(fastest)
            } else {
                0.0
            };
            let step = NICE_STEPS_US
                .iter()
                .copied()
                .find(|step| (top - low) / f64::from(*step) < (MAX_LINEAR_BINS - 1) as f64)
                .unwrap_or(NICE_STEPS_US[NICE_STEPS_US.len() - 1]);
            let start = (low / f64::from(step)).floor() * f64::from(step);
            let bins = (((top - start) / f64::from(step)).floor() as usize + 1).max(1);
            let edges: Vec<f64> = (0..=bins)
                .map(|edge| start + edge as f64 * f64::from(step))
                .collect();
            (edges, step)
        }
        LatencyScale::Log => {
            let low = (f64::from(fastest) * (1.0 - LOG_MARGIN)).max(1.0);
            let high = (top * (1.0 + LOG_MARGIN)).max(low * 2.0);
            let edges: Vec<f64> = (0..=LOG_BINS)
                .map(|edge| {
                    (low.ln() + edge as f64 / LOG_BINS as f64 * (high.ln() - low.ln())).exp()
                })
                .collect();
            (edges, 0)
        }
    };
    let mut histogram = StackedHistogram {
        counts: vec![vec![0; per_target.len()]; edges_us.len() - 1],
        edges_us,
        scale,
        step_us,
    };
    fill(&mut histogram, per_target, selected, true);
    histogram
}

/// Counts the responses of `per_target` (only `selected` when one is) into the bins of
/// `histogram`. With `clamp`, a response outside the axis goes to the first or last bin;
/// without it, it is left out.
fn fill(
    histogram: &mut StackedHistogram,
    per_target: &[Vec<u32>],
    selected: Option<usize>,
    clamp: bool,
) {
    let bins = histogram.counts.len();
    let (Some(&low), Some(&high)) = (histogram.edges_us.first(), histogram.edges_us.last()) else {
        return;
    };
    for (target, values) in per_target.iter().enumerate() {
        if selected.is_some_and(|selected| selected != target) {
            continue;
        }
        for &latency in values {
            let latency = f64::from(latency);
            if !clamp && (latency < low || latency > high) {
                continue;
            }
            let bin = ((histogram.fraction(latency) * bins as f64) as usize).min(bins - 1);
            histogram.counts[bin][target] += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples::SampleRecord;
    use crate::status::StatusKey;
    use pretty_assertions::assert_eq;

    /// A log of `(elapsed_ms, target, status, latency_ms)` rows.
    fn log_of(rows: &[(u32, u32, u16, u32)]) -> SampleLog {
        let mut log = SampleLog::new();
        let url = log.intern("http://t/x");
        for &(elapsed_ms, target, status, latency_ms) in rows {
            let status = StatusKey::Code(status);
            log.push(SampleRecord {
                elapsed_ms,
                vu: 0,
                target,
                url,
                status,
                latency_us: latency_ms * 1_000,
                size: 0,
                is_error: status.is_error(),
            });
        }
        log
    }

    fn options(target: Option<usize>, scale: LatencyScale) -> LatencyOptions {
        LatencyOptions {
            range: None,
            include_errors: false,
            target,
            scale,
        }
    }

    #[test]
    fn errors_are_left_out_unless_included() {
        let log = log_of(&[(0, 0, 200, 10), (0, 0, 429, 1), (0, 1, 200, 30)]);
        let view = latency_view(&log, 2, &options(None, LatencyScale::Linear));
        assert_eq!(view.all.count, 2);
        assert_eq!(view.per_target[0].p50, 10_000);
        assert_eq!(view.excluded, 1);
        assert_eq!(view.shown, 2);
        let with_errors = LatencyOptions {
            include_errors: true,
            ..options(None, LatencyScale::Linear)
        };
        let view = latency_view(&log, 2, &with_errors);
        assert_eq!(view.all.count, 3);
        assert_eq!(view.per_target[0].p50, 1_000);
        assert_eq!(view.excluded, 0);
    }

    #[test]
    fn the_window_keeps_whole_seconds() {
        let log = log_of(&[(999, 0, 200, 10), (1_000, 0, 200, 20), (2_500, 0, 200, 30)]);
        let view = latency_view(
            &log,
            1,
            &LatencyOptions {
                range: Some(1..2),
                ..options(None, LatencyScale::Linear)
            },
        );
        assert_eq!(view.all.count, 1);
        assert_eq!(view.all.max, 20_000);
    }

    #[test]
    fn percentiles_are_nearest_rank() {
        let sorted: Vec<u32> = (1..=100).collect();
        assert_eq!(
            percentiles(&sorted),
            LatencyPercentiles {
                count: 100,
                p50: 50,
                p90: 90,
                p95: 95,
                p99: 99,
                max: 100,
            }
        );
        assert_eq!(percentiles(&[]), LatencyPercentiles::default());
    }

    #[test]
    fn the_curve_goes_from_the_fastest_response_to_p99_9() {
        assert_eq!(curve_percentile(0), 0.0);
        assert!((curve_percentile(20) - 90.0).abs() < 1e-9);
        assert!((curve_percentile(40) - 99.0).abs() < 1e-9);
        assert!((curve_percentile(60) - 99.9).abs() < 1e-9);
        let sorted: Vec<u32> = (1..=1_000).collect();
        let points = curve(&sorted);
        assert_eq!(points.len(), CURVE_POINTS);
        assert_eq!(points[0], 1);
        assert_eq!(points[20], 900);
        // p99.9 of 1,000 responses is the 999th or, by float rounding, the last one.
        assert!((999..=1_000).contains(&points[60]));
        assert!(curve(&[]).is_empty());
    }

    #[test]
    fn all_requests_stack_on_a_linear_axis_from_zero() {
        // Three profiles: about 300 ms, 450 ms and 2 s.
        let mut rows = Vec::new();
        for i in 0..100 {
            rows.push((0, 0, 200, 300 + i % 5));
            rows.push((0, 1, 200, 450 + i % 7));
            rows.push((0, 2, 200, 2_000 + i % 3));
        }
        let view = latency_view(&log_of(&rows), 3, &options(None, LatencyScale::Linear));
        let histogram = &view.histogram;
        assert_eq!(histogram.step_us, 50_000);
        assert_eq!(histogram.edges_us[0], 0.0);
        assert!(histogram.counts.len() <= MAX_LINEAR_BINS);
        // 300 ms falls in the bin from 300 to 350, 2 s in the one from 2,000 to 2,050.
        assert_eq!(histogram.counts[6], vec![100, 0, 0]);
        assert_eq!(histogram.counts[40][2], 100);
        assert_eq!(
            (0..histogram.counts.len())
                .map(|bin| histogram.total(bin))
                .sum::<u64>(),
            300
        );
    }

    #[test]
    fn one_request_zooms_to_its_own_range_with_finer_bins() {
        let mut rows = Vec::new();
        for i in 0..100 {
            rows.push((0, 0, 200, 300 + i % 10));
            rows.push((0, 1, 200, 2_000));
        }
        let view = latency_view(&log_of(&rows), 2, &options(Some(0), LatencyScale::Linear));
        let histogram = &view.histogram;
        assert_eq!(histogram.step_us, 500);
        assert_eq!(histogram.edges_us[0], 300_000.0);
        assert_eq!(view.shown, 100);
        // Only the selected request is counted.
        assert!(histogram.counts.iter().all(|counts| counts[1] == 0));
    }

    #[test]
    fn a_log_axis_has_fixed_bins_around_the_responses() {
        let rows = [(0, 0, 200, 10), (0, 0, 200, 1_000)];
        let view = latency_view(&log_of(&rows), 1, &options(None, LatencyScale::Log));
        let histogram = &view.histogram;
        assert_eq!(histogram.counts.len(), LOG_BINS);
        assert!(histogram.edges_us[0] < 10_000.0);
        assert!(*histogram.edges_us.last().unwrap_or(&0.0) > 1_000_000.0);
        // Both responses sit just inside the margins at each end.
        assert_eq!(
            histogram.counts[1][0] + histogram.counts[LOG_BINS - 2][0],
            2
        );
        // 100 ms sits halfway between 10 ms and 1 s on a log axis.
        assert!((histogram.fraction(100_000.0) - 0.5).abs() < 0.02);
    }

    #[test]
    fn a_compared_run_is_binned_on_the_same_axis() {
        let this = log_of(&[(0, 0, 200, 300), (0, 0, 200, 400)]);
        let other = log_of(&[(0, 0, 200, 305), (0, 0, 200, 5_000)]);
        let options = options(None, LatencyScale::Linear);
        let view = latency_view(&this, 1, &options);
        let compared = latency_view_on(&other, 1, &options, &view.histogram);
        assert_eq!(compared.histogram.edges_us, view.histogram.edges_us);
        // 305 ms lands in the 300 ms bin (10 ms wide); 5 s is past the axis and left out.
        let bin = |histogram: &StackedHistogram, us: f64| {
            ((histogram.fraction(us) * histogram.counts.len() as f64) as usize)
                .min(histogram.counts.len() - 1)
        };
        assert_eq!(
            compared.histogram.counts[bin(&view.histogram, 300_000.0)][0],
            1
        );
        let binned: u64 = (0..compared.histogram.counts.len())
            .map(|index| compared.histogram.total(index))
            .sum();
        assert_eq!(binned, 1);
        // The percentiles still count every response.
        assert_eq!(compared.all.count, 2);
        assert_eq!(compared.all.max, 5_000_000);
    }

    #[test]
    fn an_empty_selection_has_no_bins() {
        let view = latency_view(&SampleLog::new(), 2, &options(None, LatencyScale::Linear));
        assert!(view.histogram.counts.is_empty());
        assert_eq!(view.histogram.fraction(5.0), 0.0);
        assert!(view.curves.iter().all(Vec::is_empty));
    }
}
