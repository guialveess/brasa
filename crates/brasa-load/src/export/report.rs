//! [`Report`], the already formatted content of a load test report, rendered as Markdown by
//! [`super::markdown`] and as PDF by [`super::pdf`].
//!
//! The app builds a `Report` in the interface language, with numbers already formatted for it,
//! so this crate never translates or formats anything for the user: the renderers only lay out
//! the strings they are given. The chart and histograms also carry raw numbers, since they are
//! drawn, not printed.

/// A color for the PDF, as 8 bit RGB. The app passes its light theme colors, since a PDF is
/// printed on white.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

/// Whether a value reads as good, bad or neither, for coloring a delta, a goal or a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportTone {
    /// Plain text color.
    #[default]
    Neutral,
    /// Success color.
    Good,
    /// Danger color.
    Bad,
    /// Warning color.
    Warn,
}

/// The overall result of the run's thresholds: `"Thresholds passed"` or `"Not passed · 1 of 3"`.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportVerdict {
    /// Whether every threshold passed.
    pub passed: bool,
    /// The verdict text.
    pub text: String,
}

/// One checked threshold: `"p99 < 50 ms"`, the run's value, and whether it passed.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportThreshold {
    /// The threshold as text, for example `"p99 < 50 ms"`.
    pub label: String,
    /// The run's value, for example `"62 ms"`.
    pub value: String,
    /// Whether it passed.
    pub passed: bool,
}

/// One KPI cell: Requests/s, p50, p95, p99, Errors or Total.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportKpi {
    /// The metric name.
    pub label: String,
    /// The value without its unit, for example `"1.850"`.
    pub value: String,
    /// The unit, for example `"ms"` or `"%"`, or empty.
    pub unit: String,
    /// The change against the compared run, for example `"+4,2 %"`, when comparing.
    pub delta: Option<String>,
    /// How the delta reads.
    pub delta_tone: ReportTone,
    /// The goal under the value, for example `"goal < 50 ms"`, and whether it was met.
    pub goal: Option<(String, bool)>,
}

/// Which y axis a chart series is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportAxis {
    /// The left axis (requests per second).
    Left,
    /// The right axis (milliseconds).
    Right,
}

/// How a chart series is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReportSeriesStyle {
    /// A solid line of the given width in points.
    Line(f32),
    /// A dashed line of the given width in points.
    Dashed(f32),
    /// A filled area under the values, with a thin outline.
    Area,
}

/// One series of the chart: one value per second of the run (`f64::NAN` for a gap).
#[derive(Debug, Clone, PartialEq)]
pub struct ReportSeries {
    /// The legend label.
    pub label: String,
    /// The line or area color.
    pub color: Rgb,
    /// The axis the values are measured against.
    pub axis: ReportAxis,
    /// How it is drawn.
    pub style: ReportSeriesStyle,
    /// One value per second, in the axis' unit.
    pub values: Vec<f64>,
}

/// A labeled horizontal line on the chart, for example a threshold goal.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportReferenceLine {
    /// The axis `value` is measured against.
    pub axis: ReportAxis,
    /// Where the line is drawn.
    pub value: f64,
    /// The label drawn above the line's left end.
    pub label: String,
    /// The line and label color.
    pub color: Rgb,
}

/// The "Performance over time" chart: shared x axis in seconds, two y axes.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportChart {
    /// The chart title.
    pub title: String,
    /// The series, drawn in order (the first one at the back).
    pub series: Vec<ReportSeries>,
    /// The top of the left axis; the bottom is always 0.
    pub left_max: f64,
    /// The top of the right axis; the bottom is always 0.
    pub right_max: f64,
    /// The left axis tick labels, bottom to top, evenly spaced from 0 to `left_max`.
    pub left_ticks: Vec<String>,
    /// The right axis tick labels, bottom to top, evenly spaced from 0 to `right_max`.
    pub right_ticks: Vec<String>,
    /// The x axis tick labels, left to right, evenly spaced over the whole run.
    pub x_ticks: Vec<String>,
    /// Horizontal reference lines.
    pub reference_lines: Vec<ReportReferenceLine>,
    /// The selected window as a range of second indices, shaded, when the report covers the
    /// whole run but a window is selected. `None` otherwise.
    pub window: Option<(usize, usize)>,
}

/// One status of a request or of all of them: `"200"`, its share and its color.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportStatus {
    /// The status, for example `"200"` or `"Timeout"`.
    pub label: String,
    /// Its share of the requests, `0.0` to `1.0`, for the bar.
    pub share: f64,
    /// The share as text, for example `"98,2 %"`.
    pub share_text: String,
    /// How many requests, formatted.
    pub count_text: String,
    /// The bar and label color.
    pub color: Rgb,
}

/// The status column of the per request table: the PDF draws a stacked bar per row and a legend
/// of the exact codes below the table, the Markdown report writes the codes and shares.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportStatusColumn {
    /// The column header, for example `"Statuses"`.
    pub header: String,
    /// The statuses of each table row, same length as the table's rows. Empty when the run did
    /// not keep them per request: there is no column then, only the totals.
    pub rows: Vec<Vec<ReportStatus>>,
    /// The statuses of all requests, ascending: the totals row's cell and the legend.
    pub total: Vec<ReportStatus>,
}

/// How a table column is aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportAlign {
    /// Left aligned (names).
    Left,
    /// Right aligned (numbers).
    Right,
}

/// A table of text cells, with an optional totals row and status column.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportTable {
    /// The table title.
    pub title: String,
    /// The column headers.
    pub headers: Vec<String>,
    /// Each column's alignment, same length as `headers`.
    pub align: Vec<ReportAlign>,
    /// The rows, each the same length as `headers`.
    pub rows: Vec<Vec<String>>,
    /// The totals row drawn in bold after the rows, same length as `headers`.
    pub total: Option<Vec<String>>,
    /// The status column, after the last of `headers`.
    pub statuses: Option<ReportStatusColumn>,
}

/// One row of the latency percentile table.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportLatencyRow {
    /// The request, or "All requests".
    pub name: String,
    /// The request's color for the swatch; `None` for "All requests", which is drawn in bold.
    pub color: Option<Rgb>,
    /// The count and the percentiles, formatted, one per header after the name.
    pub cells: Vec<String>,
}

/// The small histogram of one request, on its own latency axis.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportRequestHistogram {
    /// The request.
    pub name: String,
    /// The request's color: the swatch and the bars.
    pub color: Rgb,
    /// For example `"1,200 responses"`.
    pub count_text: String,
    /// The responses of each bin, evenly spaced over the axis.
    pub counts: Vec<u64>,
    /// Vertical dashed markers: where they sit on the axis (`0.0` to `1.0`) and their color.
    pub markers: Vec<(f64, Rgb)>,
    /// The labels under the axis: start, middle and end.
    pub x_labels: Vec<String>,
    /// The colored caption under the axis, for example `("p50 310 ms", color)`.
    pub caption: Vec<(String, Rgb)>,
}

/// One request's line of the latency by percentile chart.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportCurveLine {
    /// The request, written at the line's right end.
    pub name: String,
    /// The line color.
    pub color: Rgb,
    /// The latency in microseconds at each point, evenly spaced over the x axis. Never empty.
    pub values_us: Vec<f64>,
}

/// The latency by percentile chart: log axes.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportCurves {
    /// The chart title, for example `"Latency by percentile"`.
    pub title: String,
    /// The note after the title, for example `"log axes"`.
    pub subtitle: String,
    /// One line per request with responses.
    pub lines: Vec<ReportCurveLine>,
    /// The bottom of the y axis, in microseconds.
    pub y_min_us: f64,
    /// The top of the y axis, in microseconds.
    pub y_max_us: f64,
    /// The y ticks inside the axis: the latency in microseconds and its label.
    pub y_ticks: Vec<(f64, String)>,
    /// The x tick labels, left to right, evenly spaced: min, p90, p99 and p99.9.
    pub x_ticks: Vec<String>,
}

/// The "Latency" section: successful responses per request.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportLatency {
    /// The section title, for example `"Latency"`.
    pub title: String,
    /// The caption over the histograms: they have their own axes.
    pub caption: String,
    /// What was left out, for example `"Successful responses only, 12 errors left out."`.
    pub note: String,
    /// One histogram per request with responses.
    pub histograms: Vec<ReportRequestHistogram>,
    /// The percentile table headers, the request column first.
    pub headers: Vec<String>,
    /// The rows: "All requests" first, then each request.
    pub rows: Vec<ReportLatencyRow>,
    /// The latency by percentile chart, `None` when no request has responses.
    pub curves: Option<ReportCurves>,
}

/// One group of the Errors tab.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportErrorGroup {
    /// The status, for example `"429"` or `"Timeout"`.
    pub code: String,
    /// The title, for example `"429 Too Many Requests"`.
    pub title: String,
    /// How many requests, formatted.
    pub count: String,
    /// The share of all requests, formatted, for example `"1,32 %"`.
    pub share: String,
    /// `"<first label>: 00:34 · 00:45"`, already labeled.
    pub span: String,
    /// `"<affected label>: POST create 412, GET list 18"`, already labeled.
    pub requests: String,
    /// An example response or error message, one line.
    pub sample: String,
}

/// Section headings and fixed texts of a report, in the interface language.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReportLabels {
    /// "Summary".
    pub summary: String,
    /// "Thresholds".
    pub thresholds: String,
    /// "Errors".
    pub errors: String,
    /// "Generated by Brasa 0.4.0 on 2026-10-23 12:04", the footer.
    pub generated_by: String,
    /// "No errors", shown in the Errors section when there are none.
    pub no_errors: String,
}

/// The full, formatted content of a report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The title, for example `"Run #3 · users"`.
    pub title: String,
    /// The line under the title, for example `"Finished · Keep-alive · 23/10/2026 12:04 · 50
    /// VUs · 60 s"`.
    pub subtitle: String,
    /// The selected window the numbers cover, for example `"Window 34–46 s"`, or `None` for the
    /// whole run.
    pub window: Option<String>,
    /// The comparison the deltas are against, for example `"Compared with Run #2"`.
    pub comparison: Option<String>,
    /// The thresholds verdict, `None` when the run had no thresholds.
    pub verdict: Option<ReportVerdict>,
    /// Every checked threshold, against the whole run.
    pub thresholds: Vec<ReportThreshold>,
    /// The KPIs, in order.
    pub kpis: Vec<ReportKpi>,
    /// The chart, `None` when the run has no per second data.
    pub chart: Option<ReportChart>,
    /// The per request table, with its totals row and status column.
    pub per_request: ReportTable,
    /// The latency section, `None` for a run without saved requests.
    pub latency: Option<ReportLatency>,
    /// The error groups, most frequent first.
    pub errors: Vec<ReportErrorGroup>,
    /// Headings and fixed texts.
    pub labels: ReportLabels,
}
