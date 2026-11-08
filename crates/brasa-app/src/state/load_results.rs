//! Plain state and helpers behind the load test results (`views/load_test/dashboard.rs` and its
//! siblings): which tab is shown, the Requests tab's filters and sort, the derived data shared
//! with background tasks, the chart's axis scales and labels, and time labels. No `gpui`, no IO.

use std::ops::Range;
use std::sync::Arc;

use brasa_load::{
    LoadSnapshot, SampleLog, SampleRecord, StatusKey, TargetStats, ThresholdInputs, WindowStats,
};

use super::number::{format_decimal, format_integer};

/// How many intervals a chart y axis is divided into: five labels from 0 to the top.
pub const AXIS_DIVISIONS: usize = 4;

/// How many intervals the chart's x axis is divided into, at most: up to seven labels from 0 to
/// the duration.
pub const X_DIVISIONS: usize = 6;

/// One y axis of the results chart: its top value and its labels from the bottom (0) up.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisScale {
    /// The top of the axis, a "nice" number at or above every value plotted on it.
    pub max: f64,
    /// One label per grid line, bottom to top, [`AXIS_DIVISIONS`] + 1 of them.
    pub labels: Vec<String>,
}

/// The smallest "nice" step (1, 2, 2.5 or 5 times a power of ten) such that
/// [`AXIS_DIVISIONS`] steps reach `value`. `1.0` for zero, negative or non-finite values.
#[must_use]
pub fn nice_step(value: f64) -> f64 {
    if !value.is_finite() || value <= 0.0 {
        return 1.0;
    }
    let raw = value / AXIS_DIVISIONS as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .map(|factor| factor * magnitude)
        .find(|step| *step >= raw * (1.0 - 1e-9))
        .unwrap_or(10.0 * magnitude)
}

/// The requests per second axis: plain numbers, like the design's `0, 600, 1.200, 1.800,
/// 2.400`. The unit goes in the axis title.
#[must_use]
pub fn rps_axis(max_value: f64) -> AxisScale {
    plain_axis(max_value)
}

/// The milliseconds axis: plain numbers, like the design's `0, 25, 50, 75, 100`. The unit goes
/// in the axis title.
#[must_use]
pub fn ms_axis(max_value: f64) -> AxisScale {
    plain_axis(max_value)
}

/// An axis from 0 to a nice number at or above `max_value`, its labels plain numbers, such as
/// the Latency tab's response counts.
#[must_use]
pub fn plain_axis(max_value: f64) -> AxisScale {
    let step = nice_step(max_value);
    let labels = (0..=AXIS_DIVISIONS)
        .map(|index| format_axis_number(step * index as f64))
        .collect();
    AxisScale {
        max: step * AXIS_DIVISIONS as f64,
        labels,
    }
}

impl AxisScale {
    /// The same axis with `suffix` after its top label (`"/s"`, `" ms"`), for a chart without
    /// axis titles.
    #[must_use]
    pub fn with_top_suffix(mut self, suffix: &str) -> Self {
        if let Some(top) = self.labels.last_mut() {
            top.push_str(suffix);
        }
        self
    }
}

/// `value` with as many decimals as it needs (none for `5`, one for `7.5`, two for `0.25`), in
/// the current language.
fn format_axis_number(value: f64) -> String {
    let decimals = (0..3)
        .find(|decimals| {
            let scaled = value * 10f64.powi(*decimals);
            (scaled - scaled.round()).abs() < 1e-6
        })
        .unwrap_or(3) as usize;
    format_decimal(value, decimals)
}

/// The x axis labels of a run of `duration_secs` seconds, evenly spaced from `0s` to the
/// duration: the most intervals, up to [`X_DIVISIONS`], whose step is a round number of seconds
/// (a 20 s run gets `0s, 5s, ... 20s`), else the most that land on whole seconds, else
/// [`X_DIVISIONS`] rounded labels.
#[must_use]
pub fn x_labels(duration_secs: u64) -> Vec<String> {
    x_labels_from(0, duration_secs)
}

/// Like [`x_labels`] for a stretch of `duration_secs` seconds starting at second `start_secs`,
/// the chart of a window.
#[must_use]
pub fn x_labels_from(start_secs: u64, duration_secs: u64) -> Vec<String> {
    let dividing = |divisions: &u64| duration_secs.is_multiple_of(*divisions);
    let nice_step = |divisions: &u64| {
        let step = duration_secs / divisions;
        [1, 2, 5, 10, 15, 20, 30].contains(&step) || step.is_multiple_of(60)
    };
    let divisions = (2..=X_DIVISIONS as u64)
        .rev()
        .find(|divisions| dividing(divisions) && nice_step(divisions))
        .or_else(|| (2..=X_DIVISIONS as u64).rev().find(dividing))
        .unwrap_or(X_DIVISIONS as u64);
    (0..=divisions)
        .map(|index| {
            let secs = start_secs + (duration_secs * index + divisions / 2) / divisions;
            format!("{}s", format_integer(secs))
        })
        .collect()
}

/// `"mm:ss"` for `secs` seconds; minutes keep counting past an hour (`"75:00"`).
#[must_use]
pub fn mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// `"mm:ss.mmm"` for `millis` milliseconds, the time column of the Requests tab.
#[must_use]
pub fn mmss_millis(millis: u64) -> String {
    format!("{}.{:03}", mmss(millis / 1000), millis % 1000)
}

/// The chart tooltip's time line for the point at `second`: `"00:34 \u{2013} 00:35"`.
#[must_use]
pub fn second_range_label(second: usize) -> String {
    let second = second as u64;
    format!("{} \u{2013} {}", mmss(second), mmss(second + 1))
}

/// A selected window of seconds `first..=last` as `"34\u{2013}46 s"`: from the start of its
/// first second to the end of its last.
#[must_use]
pub fn window_label(first: usize, last: usize) -> String {
    format!(
        "{}\u{2013}{} s",
        format_integer(first as u64),
        format_integer(last as u64 + 1)
    )
}

/// A latency in milliseconds for a table cell or a tooltip: one decimal below 10 ms, whole
/// milliseconds up to a second, seconds with one decimal above (`"4.2 ms"`, `"38 ms"`,
/// `"5.0 s"`).
#[must_use]
pub fn format_latency_ms(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{} s", format_decimal(ms / 1000.0, 1))
    } else if ms < 10.0 {
        format!("{} ms", format_decimal(ms, 1))
    } else {
        format!("{} ms", format_decimal(ms, 0))
    }
}

/// A response size for the Requests tab: bytes below 1,000, kilobytes with one decimal up to a
/// megabyte, megabytes above (`"312 B"`, `"4.2 KB"`, `"1.5 MB"`).
#[must_use]
pub fn format_size(bytes: u64) -> String {
    if bytes < 1000 {
        format!("{} B", format_integer(bytes))
    } else if bytes < 1_000_000 {
        format!("{} KB", format_decimal(bytes as f64 / 1024.0, 1))
    } else {
        format!("{} MB", format_decimal(bytes as f64 / (1024.0 * 1024.0), 1))
    }
}

/// The part of `url` the Requests table shows: its path and query, without the scheme and host
/// every request of a run usually shares (`/users?page=2`). A URL without a scheme is shown
/// whole.
#[must_use]
pub fn url_path(url: &str) -> &str {
    let Some(scheme_end) = url.find("://") else {
        return url;
    };
    let after = &url[scheme_end + 3..];
    match after.find(['/', '?']) {
        Some(index) => &after[index..],
        None => "/",
    }
}

/// One series of the results chart, the key its legend toggle hides it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartSeriesKey {
    /// Active virtual users, an area with a scale of its own that peaks at half the chart's
    /// height.
    Vus,
    /// Requests per second, on the left axis.
    Rps,
    /// Median latency, on the right axis.
    P50,
    /// 95th percentile latency, on the right axis.
    P95,
    /// 99th percentile latency, on the right axis.
    P99,
    /// The compared run's 95th percentile, dashed, on the right axis.
    Baseline,
}

impl ChartSeriesKey {
    /// The series in drawing order, back to front.
    pub const ALL: [ChartSeriesKey; 6] = [
        ChartSeriesKey::Vus,
        ChartSeriesKey::Baseline,
        ChartSeriesKey::P99,
        ChartSeriesKey::P95,
        ChartSeriesKey::P50,
        ChartSeriesKey::Rps,
    ];

    /// The series in legend order.
    pub const LEGEND: [ChartSeriesKey; 6] = [
        ChartSeriesKey::Vus,
        ChartSeriesKey::Rps,
        ChartSeriesKey::P50,
        ChartSeriesKey::P95,
        ChartSeriesKey::P99,
        ChartSeriesKey::Baseline,
    ];
}

/// Which results tab is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResultsTab {
    /// KPIs, the chart and the per request table.
    #[default]
    Summary,
    /// The latency distribution, percentiles and curve of each request.
    Latency,
    /// Every request, filtered and sorted.
    Requests,
    /// One group per error kind.
    Errors,
}

/// An `Arc` compared by pointer, so a tab holding large derived data stays cheap to clone and
/// to compare: two values are equal only when they share the same allocation.
#[derive(Debug)]
pub struct Shared<T>(pub Arc<T>);

impl<T> Shared<T> {
    /// Wraps `value`.
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
}

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> std::ops::Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

/// The Requests tab's status filter: every request, one class of HTTP codes, or the requests
/// without a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    /// Every request.
    #[default]
    All,
    /// One class of HTTP codes by its first digit: `2` for 2xx, ...
    Class(u16),
    /// Timed out requests.
    Timeout,
    /// Requests that failed without a response.
    Failed,
}

impl StatusFilter {
    /// Whether a request classified as `status` passes the filter.
    #[must_use]
    pub fn matches(self, status: StatusKey) -> bool {
        match (self, status) {
            (StatusFilter::All, _) => true,
            (StatusFilter::Class(class), StatusKey::Code(code)) => code / 100 == class,
            (StatusFilter::Timeout, StatusKey::Timeout) => true,
            (StatusFilter::Failed, StatusKey::Failed) => true,
            _ => false,
        }
    }

    /// The filter that selects `status`'s kind: its class for an HTTP code.
    #[must_use]
    pub fn for_status(status: StatusKey) -> Self {
        match status {
            StatusKey::Code(code) => StatusFilter::Class(code / 100),
            StatusKey::Timeout => StatusFilter::Timeout,
            StatusKey::Failed => StatusFilter::Failed,
        }
    }

    /// The segment label: `"2xx"`, ...; `None` for [`StatusFilter::All`], [`StatusFilter::Timeout`]
    /// and [`StatusFilter::Failed`], whose labels are translated.
    #[must_use]
    pub fn class_label(self) -> Option<String> {
        match self {
            StatusFilter::Class(class) => Some(format!("{class}xx")),
            _ => None,
        }
    }
}

/// The status filters worth offering for a run: [`StatusFilter::All`], then one per kind that
/// occurs in `statuses` (HTTP classes ascending, then timeouts, then failures).
#[must_use]
pub fn status_filters(statuses: &[(StatusKey, u64)]) -> Vec<StatusFilter> {
    let mut filters: Vec<StatusFilter> = statuses
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(status, _)| StatusFilter::for_status(*status))
        .collect();
    filters.sort_by_key(|filter| match filter {
        StatusFilter::All => 0,
        StatusFilter::Class(class) => *class,
        StatusFilter::Timeout => 1_000,
        StatusFilter::Failed => 1_001,
    });
    filters.dedup();
    filters.insert(0, StatusFilter::All);
    filters
}

/// A column of the Requests table, also what it is sorted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestColumn {
    /// The 1-based position in the log.
    Id,
    /// When the request completed.
    Time,
    /// The virtual user.
    Vu,
    /// The URL.
    Request,
    /// The status.
    Status,
    /// The latency.
    Latency,
    /// The response size.
    Size,
}

impl RequestColumn {
    /// Every column, left to right.
    pub const ALL: [RequestColumn; 7] = [
        RequestColumn::Id,
        RequestColumn::Time,
        RequestColumn::Vu,
        RequestColumn::Request,
        RequestColumn::Status,
        RequestColumn::Latency,
        RequestColumn::Size,
    ];

    /// Whether a column starts sorted from the largest value when picked: latency and size.
    #[must_use]
    pub fn starts_descending(self) -> bool {
        matches!(self, RequestColumn::Latency | RequestColumn::Size)
    }
}

/// How the Requests table is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSort {
    /// The column.
    pub column: RequestColumn,
    /// Largest first.
    pub descending: bool,
}

impl Default for RequestSort {
    /// Slowest first, like the design.
    fn default() -> Self {
        Self {
            column: RequestColumn::Latency,
            descending: true,
        }
    }
}

impl RequestSort {
    /// The sort after clicking `column`'s header: the same column flips its direction, another
    /// one starts in its natural direction ([`RequestColumn::starts_descending`]).
    #[must_use]
    pub fn toggled(self, column: RequestColumn) -> Self {
        if self.column == column {
            Self {
                column,
                descending: !self.descending,
            }
        } else {
            Self {
                column,
                descending: column.starts_descending(),
            }
        }
    }
}

/// The Requests tab's filters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RequestFilter {
    /// Text searched in the URL and the request name, case insensitive.
    pub query: String,
    /// The status filter.
    pub status: StatusFilter,
    /// Only requests at least this slow, in milliseconds.
    pub min_latency_ms: Option<u32>,
    /// Only this virtual user, 1-based.
    pub vu: Option<u32>,
    /// Only errors.
    pub only_errors: bool,
}

/// The positions in `log` of the requests inside `window` (whole seconds `[start, end)`) that
/// pass `filter`, sorted by `sort` (ties keep the log order). `target_names` are the request
/// names, by target index, searched along with the URL.
///
/// One pass over the log plus a sort of the matches; the query is checked once per URL and per
/// name, not once per request, since URLs repeat.
#[must_use]
pub fn filter_requests(
    log: &SampleLog,
    window: Option<Range<u32>>,
    filter: &RequestFilter,
    sort: RequestSort,
    target_names: &[String],
) -> Vec<u32> {
    let query = filter.query.trim().to_lowercase();
    let url_matches: Vec<bool> = log
        .urls()
        .iter()
        .map(|url| query.is_empty() || url.to_lowercase().contains(&query))
        .collect();
    let name_matches: Vec<bool> = target_names
        .iter()
        .map(|name| !query.is_empty() && name.to_lowercase().contains(&query))
        .collect();
    let min_latency_us = filter.min_latency_ms.map(|ms| ms.saturating_mul(1_000));

    let samples = log.samples();
    let mut matches: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| {
            let second = sample.elapsed_ms / 1_000;
            window
                .as_ref()
                .is_none_or(|window| window.contains(&second))
                && filter.status.matches(sample.status)
                && (!filter.only_errors || sample.is_error)
                && filter.vu.is_none_or(|vu| sample.vu + 1 == vu)
                && min_latency_us.is_none_or(|min| sample.latency_us >= min)
                && (url_matches
                    .get(sample.url as usize)
                    .copied()
                    .unwrap_or(false)
                    || name_matches
                        .get(sample.target as usize)
                        .copied()
                        .unwrap_or(false))
        })
        .map(|(index, _)| index as u32)
        .collect();

    let url_rank = url_ranks(log);
    let key = |index: &u32| -> u64 {
        let sample: &SampleRecord = &samples[*index as usize];
        match sort.column {
            RequestColumn::Id => u64::from(*index),
            RequestColumn::Time => u64::from(sample.elapsed_ms),
            RequestColumn::Vu => u64::from(sample.vu),
            RequestColumn::Request => u64::from(url_rank[sample.url as usize]),
            RequestColumn::Status => status_rank(sample.status),
            RequestColumn::Latency => u64::from(sample.latency_us),
            RequestColumn::Size => u64::from(sample.size),
        }
    };
    if sort.descending {
        matches.sort_by_key(|index| std::cmp::Reverse(key(index)));
    } else {
        matches.sort_by_key(key);
    }
    matches
}

/// Each URL's position in the alphabetical order of the log's URL table.
fn url_ranks(log: &SampleLog) -> Vec<u32> {
    let mut order: Vec<usize> = (0..log.urls().len()).collect();
    order.sort_by(|a, b| log.urls()[*a].cmp(&log.urls()[*b]));
    let mut ranks = vec![0; order.len()];
    for (rank, index) in order.into_iter().enumerate() {
        ranks[index] = rank as u32;
    }
    ranks
}

/// The sort order of statuses: HTTP codes ascending, then timeouts, then failures.
fn status_rank(status: StatusKey) -> u64 {
    match status {
        StatusKey::Code(code) => u64::from(code),
        StatusKey::Timeout => 1_000,
        StatusKey::Failed => 1_001,
    }
}

/// The latency filter's options in milliseconds: the run's p50, p95 and p99 rounded down to a
/// "nice" value (1, 2, 2.5 or 5 times a power of ten, at least 1 ms), without duplicates.
#[must_use]
pub fn latency_filter_options(p50_us: u32, p95_us: u32, p99_us: u32) -> Vec<u32> {
    let mut options: Vec<u32> = [p50_us, p95_us, p99_us]
        .into_iter()
        .filter_map(|us| nice_floor(f64::from(us) / 1_000.0))
        .collect();
    options.dedup();
    options
}

/// The largest "nice" whole number of milliseconds (1, 2, 2.5 or 5 times a power of ten) at or
/// below `ms`, or `None` below 1 ms.
fn nice_floor(ms: f64) -> Option<u32> {
    if !ms.is_finite() || ms < 1.0 {
        return None;
    }
    let magnitude = 10f64.powf(ms.log10().floor());
    [5.0, 2.5, 2.0, 1.0]
        .into_iter()
        .map(|factor| factor * magnitude)
        .find(|value| *value <= ms && (value.fract() == 0.0))
        .map(|value| value as u32)
}

/// The chart's series, built once whenever the run's per second data or the compared run
/// changes, not on every render: one value per second, shared with the chart as `Arc`s.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartData {
    /// Active virtual users.
    pub vus: Arc<[f64]>,
    /// Requests per second.
    pub rps: Arc<[f64]>,
    /// Median latency, in milliseconds.
    pub p50: Arc<[f64]>,
    /// 95th percentile latency, in milliseconds.
    pub p95: Arc<[f64]>,
    /// 99th percentile latency, in milliseconds.
    pub p99: Arc<[f64]>,
    /// The compared run's 95th percentile, in milliseconds, when it has one per second.
    pub baseline: Option<Arc<[f64]>>,
    /// The largest requests per second.
    pub max_rps: f64,
    /// The largest latency of any series, in milliseconds.
    pub max_ms: f64,
    /// The largest virtual user count.
    pub max_vus: f64,
}

impl ChartData {
    /// The chart data of `snapshot`'s per second series, with `compare`'s p95 as the baseline.
    /// A second with no request is a gap (`NAN`) in the latency series. Runs saved before p50
    /// and p99 were kept per second have them at zero everywhere: those series are gaps too.
    #[must_use]
    pub fn new(snapshot: &LoadSnapshot, compare: Option<&LoadSnapshot>) -> Self {
        let series = &snapshot.series;
        let latency = |pick: fn(&brasa_load::SecondPoint) -> u32| -> Arc<[f64]> {
            let any = series.iter().any(|point| pick(point) > 0);
            series
                .iter()
                .map(|point| {
                    if !any || point.rps <= 0.0 {
                        f64::NAN
                    } else {
                        f64::from(pick(point)) / 1_000.0
                    }
                })
                .collect()
        };
        let p50 = latency(|point| point.p50);
        let p95 = latency(|point| point.p95);
        let p99 = latency(|point| point.p99);
        let baseline = compare.and_then(|compare| {
            compare.series.iter().any(|point| point.p95 > 0).then(|| {
                compare
                    .series
                    .iter()
                    .map(|point| {
                        if point.rps <= 0.0 {
                            f64::NAN
                        } else {
                            f64::from(point.p95) / 1_000.0
                        }
                    })
                    .collect::<Arc<[f64]>>()
            })
        });
        let finite_max = |values: &[f64]| {
            values
                .iter()
                .copied()
                .filter(|value| value.is_finite())
                .fold(0.0, f64::max)
        };
        let max_ms = [&p50, &p95, &p99]
            .into_iter()
            .map(|values| finite_max(values))
            .chain(baseline.as_deref().map(finite_max))
            .fold(0.0, f64::max);
        let rps: Arc<[f64]> = series.iter().map(|point| point.rps).collect();
        let vus: Arc<[f64]> = series.iter().map(|point| f64::from(point.vus)).collect();
        Self {
            max_rps: finite_max(&rps),
            max_vus: finite_max(&vus),
            vus,
            rps,
            p50,
            p95,
            p99,
            baseline,
            max_ms,
        }
    }
}

/// The shortest run of seconds with the same virtual users that counts as a plateau.
const VU_PLATEAU_MIN_SECS: usize = 3;
/// A plateau lower than this share of the peak is too thin to hold its label inside its area.
const VU_THIN_SHARE: f64 = 0.2;

/// Where a virtual users label of the chart sits, see [`vu_labels`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VuLabelKind {
    /// At the end of a plateau, inside its area.
    Plateau,
    /// At the end of a plateau too thin to hold it, above the area.
    ThinPlateau,
    /// At the live edge of a run still ramping up, with the count it ramps up to.
    LiveEdge {
        /// The configured virtual users.
        target: u32,
    },
}

/// One label of the chart's virtual users area: the second it marks and its count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VuLabel {
    /// The second, an index of the chart.
    pub index: usize,
    /// The virtual users at that second.
    pub vus: u32,
    /// Where it sits.
    pub kind: VuLabelKind,
}

/// The labels of the virtual users area, since it has no axis of its own: one at the end of each
/// plateau (at least [`VU_PLATEAU_MIN_SECS`] seconds with the same count), or, while a run still
/// ramps up to `live_target`, a single one at the live edge.
#[must_use]
pub fn vu_labels(vus: &[f64], live_target: Option<u32>) -> Vec<VuLabel> {
    let counts: Vec<u32> = vus
        .iter()
        .map(|value| {
            if value.is_finite() {
                value.round().max(0.0) as u32
            } else {
                0
            }
        })
        .collect();
    let Some(&last) = counts.last() else {
        return Vec::new();
    };
    if let Some(target) = live_target
        && last < target
    {
        return vec![VuLabel {
            index: counts.len() - 1,
            vus: last,
            kind: VuLabelKind::LiveEdge { target },
        }];
    }
    let peak = counts.iter().copied().max().unwrap_or(0);
    let mut labels = Vec::new();
    let mut start = 0;
    for end in 1..=counts.len() {
        if end < counts.len() && counts[end] == counts[start] {
            continue;
        }
        let value = counts[start];
        if end - start >= VU_PLATEAU_MIN_SECS && value > 0 {
            let thin = f64::from(value) < f64::from(peak) * VU_THIN_SHARE;
            labels.push(VuLabel {
                index: end - 1,
                vus: value,
                kind: if thin {
                    VuLabelKind::ThinPlateau
                } else {
                    VuLabelKind::Plateau
                },
            });
        }
        start = end;
    }
    labels
}

/// The configured virtual users profile from second `from` to the end of a chart of `count`
/// seconds, `NAN` before it: a linear ramp from 0 to `vus` over `ramp_up_secs`, then flat. The
/// chart draws it dashed ahead of a live run, so the plateau to come is already visible.
#[must_use]
pub fn planned_vus(count: usize, from: usize, vus: u32, ramp_up_secs: u64) -> Arc<[f64]> {
    (0..count)
        .map(|second| {
            if second < from {
                f64::NAN
            } else if ramp_up_secs == 0 {
                f64::from(vus)
            } else {
                let started = f64::from(vus) * (second + 1) as f64 / ramp_up_secs as f64;
                started.round().min(f64::from(vus))
            }
        })
        .collect()
}

/// What the Summary tab shows, from a window's statistics or, without them, a snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryData {
    /// Requests per second.
    pub rps: f64,
    /// Median latency, in microseconds.
    pub p50: u32,
    /// 95th percentile latency, in microseconds.
    pub p95: u32,
    /// 99th percentile latency, in microseconds.
    pub p99: u32,
    /// Error rate, `0.0` to `1.0`.
    pub error_rate: f64,
    /// Requests.
    pub total: u64,
    /// Requests per status.
    pub statuses: Vec<(StatusKey, u64)>,
    /// Per target statistics, by target index.
    pub per_target: Vec<TargetStats>,
    /// Requests per status of each target, by target index. Empty when built from a snapshot,
    /// which does not keep them.
    pub target_statuses: Vec<Vec<(StatusKey, u64)>>,
}

impl SummaryData {
    /// The Summary of a window's statistics.
    #[must_use]
    pub fn from_stats(stats: &WindowStats) -> Self {
        Self {
            rps: stats.rps,
            p50: stats.p50,
            p95: stats.p95,
            p99: stats.p99,
            error_rate: stats.error_rate,
            total: stats.total,
            statuses: stats.statuses.clone(),
            per_target: stats.per_target.clone(),
            target_statuses: stats.target_statuses.clone(),
        }
    }

    /// The Summary of a snapshot: a live run, or a past run without its saved requests.
    #[must_use]
    pub fn from_snapshot(snapshot: &LoadSnapshot) -> Self {
        Self {
            rps: snapshot.rps,
            p50: snapshot.p50,
            p95: snapshot.p95,
            p99: snapshot.p99,
            error_rate: snapshot.error_rate,
            total: snapshot.total,
            statuses: snapshot.status_counts.clone(),
            per_target: snapshot.per_target.clone(),
            target_statuses: Vec::new(),
        }
    }

    /// The numbers thresholds are checked against.
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn nice_steps_cover_the_value_in_four_intervals() {
        assert!((nice_step(2_250.0) - 600.0).abs() > 0.0);
        assert!((nice_step(2_250.0) - 1_000.0).abs() < 1e-9);
        assert!((nice_step(2_000.0) - 500.0).abs() < 1e-9);
        assert!((nice_step(95.0) - 25.0).abs() < 1e-9);
        assert!((nice_step(100.0) - 25.0).abs() < 1e-9);
        assert!((nice_step(3.2) - 1.0).abs() < 1e-9);
        assert!((nice_step(0.0) - 1.0).abs() < 1e-9);
        assert!((nice_step(f64::NAN) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn axes_label_every_grid_line_with_the_unit_on_top() {
        let rps = rps_axis(1_900.0);
        assert!((rps.max - 2_000.0).abs() < 1e-9);
        assert_eq!(rps.labels, vec!["0", "500", "1,000", "1,500", "2,000"]);
        let ms = ms_axis(9.0);
        assert!((ms.max - 10.0).abs() < 1e-9);
        assert_eq!(ms.labels, vec!["0", "2.5", "5", "7.5", "10"]);
        assert_eq!(ms.with_top_suffix(" ms").labels[4], "10 ms");
    }

    #[test]
    fn plateaus_of_virtual_users_are_labeled_at_their_end() {
        // A 4 s ramp to 40, a plateau, a step down to 4 and the last second stopping.
        let vus = [10.0, 20.0, 30.0, 40.0, 40.0, 40.0, 40.0, 4.0, 4.0, 4.0, 1.0];
        assert_eq!(
            vu_labels(&vus, None),
            vec![
                VuLabel {
                    index: 6,
                    vus: 40,
                    kind: VuLabelKind::Plateau
                },
                VuLabel {
                    index: 9,
                    vus: 4,
                    kind: VuLabelKind::ThinPlateau
                },
            ]
        );
        assert!(vu_labels(&[], None).is_empty());
    }

    #[test]
    fn a_live_run_still_ramping_up_labels_its_edge() {
        let vus = [5.0, 10.0, 15.0];
        assert_eq!(
            vu_labels(&vus, Some(50)),
            vec![VuLabel {
                index: 2,
                vus: 15,
                kind: VuLabelKind::LiveEdge { target: 50 }
            }]
        );
        // Once it reaches the target, its plateau is labeled like a finished run's.
        let vus = [25.0, 50.0, 50.0, 50.0];
        assert_eq!(vu_labels(&vus, Some(50))[0].kind, VuLabelKind::Plateau);
    }

    #[test]
    fn the_planned_profile_ramps_up_then_stays_flat() {
        let planned = planned_vus(6, 2, 50, 4);
        assert!(planned[0].is_nan() && planned[1].is_nan());
        assert_eq!(&planned[2..], &[38.0, 50.0, 50.0, 50.0]);
        assert_eq!(&planned_vus(2, 0, 7, 0)[..], &[7.0, 7.0]);
    }

    #[test]
    fn x_labels_land_on_whole_seconds() {
        assert_eq!(
            x_labels(60),
            vec!["0s", "10s", "20s", "30s", "40s", "50s", "60s"]
        );
        assert_eq!(x_labels(20), vec!["0s", "5s", "10s", "15s", "20s"]);
        assert_eq!(x_labels(7).len(), 7);
        assert_eq!(
            x_labels_from(5, 5),
            vec!["5s", "6s", "7s", "8s", "9s", "10s"]
        );
    }

    #[test]
    fn time_labels() {
        assert_eq!(mmss(75), "01:15");
        assert_eq!(mmss_millis(34_512), "00:34.512");
        assert_eq!(second_range_label(34), "00:34 \u{2013} 00:35");
        assert_eq!(window_label(34, 45), "34\u{2013}46 s");
    }

    fn log_of(samples: &[(u32, u32, &str, StatusKey, u32)]) -> SampleLog {
        let mut log = SampleLog::new();
        for (elapsed_ms, vu, url, status, latency_us) in samples {
            let url = log.intern(url);
            log.push(SampleRecord {
                elapsed_ms: *elapsed_ms,
                vu: *vu,
                target: 0,
                url,
                status: *status,
                latency_us: *latency_us,
                size: 10,
                is_error: status.is_error(),
            });
        }
        log
    }

    #[test]
    fn filter_requests_applies_every_filter_and_sorts() {
        let log = log_of(&[
            (100, 0, "http://a/users/1", StatusKey::Code(200), 5_000),
            (1_200, 1, "http://a/users/2", StatusKey::Code(429), 1_000),
            (2_500, 0, "http://a/orders", StatusKey::Timeout, 900_000),
            (2_900, 1, "http://a/users/1", StatusKey::Code(201), 60_000),
        ]);
        let names = vec!["create".to_string()];
        let all = filter_requests(
            &log,
            None,
            &RequestFilter::default(),
            RequestSort::default(),
            &names,
        );
        assert_eq!(all, vec![2, 3, 0, 1]);

        let users = RequestFilter {
            query: "USERS".to_string(),
            ..RequestFilter::default()
        };
        let sort_by_time = RequestSort {
            column: RequestColumn::Time,
            descending: false,
        };
        assert_eq!(
            filter_requests(&log, None, &users, sort_by_time, &names),
            vec![0, 1, 3]
        );
        let by_name = RequestFilter {
            query: "creat".to_string(),
            ..RequestFilter::default()
        };
        assert_eq!(
            filter_requests(&log, None, &by_name, sort_by_time, &names).len(),
            4
        );

        let errors_of_vu_2 = RequestFilter {
            only_errors: true,
            vu: Some(2),
            ..RequestFilter::default()
        };
        assert_eq!(
            filter_requests(&log, None, &errors_of_vu_2, sort_by_time, &names),
            vec![1]
        );
        let slow_2xx = RequestFilter {
            status: StatusFilter::Class(2),
            min_latency_ms: Some(50),
            ..RequestFilter::default()
        };
        assert_eq!(
            filter_requests(&log, None, &slow_2xx, sort_by_time, &names),
            vec![3]
        );
        assert_eq!(
            filter_requests(
                &log,
                Some(1..2),
                &RequestFilter::default(),
                sort_by_time,
                &names
            ),
            vec![1]
        );
        let by_url = RequestSort {
            column: RequestColumn::Request,
            descending: false,
        };
        assert_eq!(
            filter_requests(&log, None, &RequestFilter::default(), by_url, &names),
            vec![2, 0, 3, 1]
        );
    }

    #[test]
    fn sort_toggles_the_same_column_and_starts_others_in_their_direction() {
        let sort = RequestSort::default().toggled(RequestColumn::Latency);
        assert!(!sort.descending);
        let sort = sort.toggled(RequestColumn::Time);
        assert_eq!(sort.column, RequestColumn::Time);
        assert!(!sort.descending);
        assert!(sort.toggled(RequestColumn::Size).descending);
    }

    #[test]
    fn status_filters_list_the_kinds_present() {
        let statuses = [
            (StatusKey::Timeout, 2),
            (StatusKey::Code(201), 5),
            (StatusKey::Code(200), 9),
            (StatusKey::Code(502), 1),
        ];
        assert_eq!(
            status_filters(&statuses),
            vec![
                StatusFilter::All,
                StatusFilter::Class(2),
                StatusFilter::Class(5),
                StatusFilter::Timeout
            ]
        );
    }

    #[test]
    fn latency_options_are_nice_values_below_the_percentiles() {
        assert_eq!(
            latency_filter_options(9_000, 31_000, 62_000),
            vec![5, 25, 50]
        );
        assert_eq!(latency_filter_options(400, 1_200, 1_900), vec![1]);
    }

    #[test]
    fn chart_data_has_gaps_for_idle_seconds_and_runs_without_percentiles() {
        let point = |rps: f64, p99: u32| brasa_load::SecondPoint {
            rps,
            p95: p99,
            p50: p99 / 2,
            p90: p99,
            p99,
            vus: 2,
        };
        let snapshot = LoadSnapshot {
            elapsed: std::time::Duration::from_secs(3),
            active_vus: 0,
            total: 3,
            rps: 1.0,
            p50: 0,
            p95: 0,
            p99: 0,
            error_rate: 0.0,
            series: vec![point(10.0, 40_000), point(0.0, 0), point(12.0, 60_000)],
            histogram: [0; 26],
            status_counts: Vec::new(),
            per_target: Vec::new(),
            error_examples: Vec::new(),
        };
        let data = ChartData::new(&snapshot, None);
        assert!((data.p99[0] - 40.0).abs() < 1e-9);
        assert!(data.p99[1].is_nan());
        assert!((data.max_ms - 60.0).abs() < 1e-9);
        assert!((data.max_rps - 12.0).abs() < 1e-9);
        assert!(data.baseline.is_none());

        let mut old = snapshot.clone();
        for point in &mut old.series {
            point.p50 = 0;
            point.p90 = 0;
            point.p99 = 0;
        }
        // p95 was always kept per second, so even an old run can be the baseline.
        let with_old_baseline = ChartData::new(&snapshot, Some(&old));
        assert!(with_old_baseline.baseline.is_some());
        assert!(
            ChartData::new(&old, None)
                .p99
                .iter()
                .all(|value| value.is_nan())
        );
    }

    #[test]
    fn url_paths_drop_the_scheme_and_host() {
        assert_eq!(
            url_path("http://127.0.0.1:8080/users?page=2"),
            "/users?page=2"
        );
        assert_eq!(url_path("https://api.example.com"), "/");
        assert_eq!(url_path("https://api.example.com?q=1"), "?q=1");
        assert_eq!(url_path("not a url"), "not a url");
    }

    #[test]
    fn latencies_and_sizes() {
        assert_eq!(format_latency_ms(4.24), "4.2 ms");
        assert_eq!(format_latency_ms(38.4), "38 ms");
        assert_eq!(format_latency_ms(5_000.0), "5.0 s");
        assert_eq!(format_size(312), "312 B");
        assert_eq!(format_size(4_300), "4.2 KB");
    }
}
