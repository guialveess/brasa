//! Builds the content of a load test report (`brasa_load::export::report::Report`) from a
//! tab's results, in the interface language: the texts come from the locale files, the numbers
//! go through `state::number` and the colors are the light theme's, since a PDF is printed on
//! white. No `gpui` window, no IO: the Export menu (`views/load_test/export.rs`) gathers a
//! [`ReportInput`] on the UI thread and renders the result in the background.
//!
//! The same module formats the local times the dashboard header and the report footer show.

use std::ops::Range;

use chrono::{Local, TimeZone};
use gpui_kit::Rgba;
use brasa_load::export::report::{
    Report, ReportAlign, ReportAxis, ReportChart, ReportCurveLine, ReportCurves, ReportErrorGroup,
    ReportKpi, ReportLabels, ReportLatency, ReportLatencyRow, ReportReferenceLine,
    ReportRequestHistogram, ReportSeries, ReportSeriesStyle, ReportStatus, ReportStatusColumn,
    ReportTable, ReportThreshold, ReportTone, ReportVerdict, Rgb,
};
use brasa_load::{
    ErrorExample, ErrorGroup, LatencyHistogram, LatencyOptions, LatencyPercentiles, LatencyScale,
    LoadSnapshot, Progress, SampleLog, StatusKey, Threshold, ThresholdInputs, ThresholdMetric,
    WindowStats, evaluate_thresholds, latency::latency_view_with,
};
use rust_i18n::t;

use super::load_results::{
    ChartData, ChartSeriesKey, SummaryData, format_latency_ms, mmss, ms_axis, rps_axis,
    window_label, x_labels_from,
};
use super::load_test::{
    DeltaTone, LoadTestStatus, delta_tone, format_delta, format_trimmed, threshold_label,
    threshold_metric_label,
};
use super::locale::plural;
use super::number::{format_decimal, format_integer};
use super::response_render::reason_phrase;
use crate::theme::palette::{Palette, BrasaColors};

/// The share of the left axis' height the virtual users area reaches, like the dashboard chart.
const VUS_AREA_SHARE: f64 = 0.9;

/// The ticks of the percentile chart's latency axis, in milliseconds, before the powers of ten.
const CURVE_TICKS: [f64; 4] = [1.0, 2.0, 3.0, 5.0];

/// How much room the percentile chart's axis leaves around the fastest and slowest latency.
const CURVE_MARGIN: f64 = 0.08;

/// The colors of a report: the light theme's chart series, tones and status colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportColors {
    /// The virtual users area.
    pub vus: Rgb,
    /// The requests per second line, also the histogram bars.
    pub rps: Rgb,
    /// The median latency line.
    pub p50: Rgb,
    /// The 95th percentile latency line, also the slow histogram bars.
    pub p95: Rgb,
    /// The 99th percentile latency line.
    pub p99: Rgb,
    /// The compared run's dashed line.
    pub baseline: Rgb,
    /// The goal lines of the chart.
    pub bad: Rgb,
    /// A timeout or a failure, a darker red than a 5xx.
    pub failure: Rgb,
    /// The color of each request, by target index, cycling.
    pub requests: [Rgb; 6],
    /// A 2xx status.
    pub status_ok: Rgb,
    /// A 3xx status.
    pub status_redirect: Rgb,
    /// A 4xx status.
    pub status_client: Rgb,
    /// A 5xx status.
    pub status_server: Rgb,
    /// Any other status.
    pub neutral: Rgb,
}

impl ReportColors {
    /// The colors of the default light palette.
    #[must_use]
    pub fn light() -> Self {
        let palette = Palette::default_light();
        let status = |code: u16| rgb(palette.status_colors(Some(code)).0);
        Self {
            vus: rgb(palette.series_color(ChartSeriesKey::Vus)),
            rps: rgb(palette.series_color(ChartSeriesKey::Rps)),
            p50: rgb(palette.series_color(ChartSeriesKey::P50)),
            p95: rgb(palette.series_color(ChartSeriesKey::P95)),
            p99: rgb(palette.series_color(ChartSeriesKey::P99)),
            baseline: rgb(palette.series_color(ChartSeriesKey::Baseline)),
            bad: rgb(palette.danger),
            failure: darken(rgb(palette.danger)),
            requests: std::array::from_fn(|index| rgb(palette.request_color(index))),
            status_ok: status(200),
            status_redirect: status(301),
            status_client: status(400),
            status_server: status(500),
            neutral: rgb(palette.fg_muted),
        }
    }

    /// The color of request `index`.
    fn request(&self, index: usize) -> Rgb {
        self.requests[index % self.requests.len()]
    }

    /// The color of `status`'s bar and label.
    fn status(&self, status: StatusKey) -> Rgb {
        match status {
            StatusKey::Code(code) if (200..300).contains(&code) => self.status_ok,
            StatusKey::Code(code) if (300..400).contains(&code) => self.status_redirect,
            StatusKey::Code(code) if (400..500).contains(&code) => self.status_client,
            StatusKey::Code(code) if code >= 500 => self.status_server,
            StatusKey::Code(_) => self.neutral,
            StatusKey::Timeout | StatusKey::Failed => self.failure,
        }
    }
}

/// How a delta reads in the report.
fn report_tone(tone: DeltaTone) -> ReportTone {
    match tone {
        DeltaTone::Better => ReportTone::Good,
        DeltaTone::Worse => ReportTone::Bad,
        DeltaTone::Neutral => ReportTone::Neutral,
    }
}

/// `color` as 8 bit RGB.
fn rgb(color: gpui_kit::Hsla) -> Rgb {
    let rgba = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb(channel(rgba.r), channel(rgba.g), channel(rgba.b))
}

/// `color` 30 % darker.
fn darken(color: Rgb) -> Rgb {
    let channel = |value: u8| (f32::from(value) * 0.7).round() as u8;
    Rgb(channel(color.0), channel(color.1), channel(color.2))
}

/// Everything [`build_report`] needs about one run.
#[derive(Debug, Clone, Copy)]
pub struct ReportInput<'a> {
    /// The run number, for the title.
    pub run_number: u32,
    /// What the run targeted, after the number in the title. May be empty.
    pub target_label: &'a str,
    /// The status badge's text, for example `"Finished"` (see [`status_label`]).
    pub status_label: &'a str,
    /// How the run connected, for example `"Keep-alive"` (see [`connection_label`]).
    pub connection_label: Option<&'a str>,
    /// When the run started, as Unix seconds.
    pub started_at: Option<u64>,
    /// How many virtual users the run was configured with.
    pub vus: Option<u32>,
    /// The configured duration, in seconds.
    pub duration_secs: u64,
    /// The numbers of the covered range: the selected window or the whole run.
    pub summary: &'a SummaryData,
    /// The compared run's number and final snapshot, when a comparison is selected.
    pub compare: Option<(u32, &'a LoadSnapshot)>,
    /// The thresholds of the run.
    pub thresholds: &'a [Threshold],
    /// The whole run's numbers, which the verdict and the threshold list are checked against.
    pub run_inputs: ThresholdInputs,
    /// The per second series of the run, `None` when it has none.
    pub chart: Option<&'a ChartData>,
    /// The selected window as inclusive second indices, if any.
    pub window: Option<(usize, usize)>,
    /// Whether the report covers only [`Self::window`] (the chart shows only that stretch) or
    /// the whole run (the chart shades the window).
    pub window_only: bool,
    /// Each target's method and name, by target index.
    pub targets: &'a [(String, String)],
    /// The run's requests, for the Latency section. `None` for a run without saved requests,
    /// which has no such section.
    pub log: Option<&'a SampleLog>,
    /// How many requests the run had.
    pub target_count: usize,
    /// The error groups of the covered range, most frequent first.
    pub error_groups: &'a [ErrorGroup],
    /// The first error of each status of the whole run, for the groups' samples.
    pub error_examples: &'a [ErrorExample],
    /// The request timeout, for the title of the timeout group.
    pub timeout_secs: u64,
    /// When the report is generated, as Unix seconds, for the footer.
    pub generated_at: i64,
    /// The colors.
    pub colors: &'a ReportColors,
    /// Where the Latency section reports its passes over the log and learns it was cancelled,
    /// see [`latency_units`]. When cancelled, the section is left out.
    pub progress: &'a Progress,
}

/// How many samples the Latency section of a report reads for a log of `samples` and
/// `target_count` requests: the whole log once, then once per request.
#[must_use]
pub fn latency_units(samples: usize, target_count: usize) -> u64 {
    samples as u64 * (1 + target_count as u64)
}

impl ReportInput<'_> {
    /// Whether the report covers a selected window instead of the whole run.
    fn covers_window(&self) -> bool {
        self.window_only && self.window.is_some()
    }
}

/// Builds the report of `input`, every text in the interface language.
#[must_use]
pub fn build_report(input: &ReportInput) -> Report {
    let verdict_outcomes = evaluate_thresholds(input.thresholds, &input.run_inputs);
    let failed = verdict_outcomes
        .iter()
        .filter(|outcome| !outcome.passed)
        .count();
    let verdict = (!verdict_outcomes.is_empty()).then(|| ReportVerdict {
        passed: failed == 0,
        text: if failed == 0 {
            t!("load_test.verdict.passed").into_owned()
        } else {
            t!(
                "load_test.verdict.failed",
                failed = failed,
                total = verdict_outcomes.len()
            )
            .into_owned()
        },
    });
    let thresholds = verdict_outcomes
        .iter()
        .map(|outcome| ReportThreshold {
            label: threshold_label(&outcome.threshold),
            value: threshold_value(outcome.threshold.metric, outcome.value),
            passed: outcome.passed,
        })
        .collect();

    let window = input
        .window
        .filter(|_| input.window_only)
        .map(|(first, last)| {
            t!("load_test.window.chip", range = window_label(first, last)).into_owned()
        });
    let comparison = input.compare.map(|(number, _)| {
        t!("load_test.compare.compared_with", run = run_title(number)).into_owned()
    });

    Report {
        title: title(input),
        subtitle: subtitle(input),
        window,
        comparison,
        verdict,
        thresholds,
        kpis: kpis(input),
        chart: chart(input),
        per_request: per_request(input),
        latency: latency(input),
        errors: errors(input),
        labels: ReportLabels {
            summary: t!("load_test.results.summary").into_owned(),
            thresholds: t!("load_test.report.thresholds").into_owned(),
            errors: t!("load_test.results.errors").into_owned(),
            generated_by: t!(
                "load_test.report.generated_by",
                version = brasa_cli::version_label(),
                date = local_date_time(input.generated_at)
            )
            .into_owned(),
            no_errors: if input.covers_window() {
                t!("load_test.errors.none_in_window")
            } else {
                t!("load_test.errors.none")
            }
            .into_owned(),
        },
    }
}

/// The statistics of a whole run from its final snapshot, for a run without a saved request log:
/// the histogram has the engine's fixed 10 ms buckets and there are no error groups.
#[must_use]
pub fn stats_from_snapshot(snapshot: &LoadSnapshot) -> WindowStats {
    WindowStats {
        total: snapshot.total,
        errors: (snapshot.error_rate * snapshot.total as f64).round() as u64,
        rps: snapshot.rps,
        p50: snapshot.p50,
        p95: snapshot.p95,
        p99: snapshot.p99,
        error_rate: snapshot.error_rate,
        histogram: LatencyHistogram {
            bucket_us: 10_000,
            counts: snapshot.histogram.to_vec(),
        },
        statuses: snapshot.status_counts.clone(),
        per_target: snapshot.per_target.clone(),
        target_statuses: Vec::new(),
        error_groups: Vec::new(),
    }
}

/// `"Run #3"`.
fn run_title(number: u32) -> String {
    t!("load_test.run.numbered", number = number).into_owned()
}

/// `"Run #3 · users"`, or just `"Run #3"` without a target.
fn title(input: &ReportInput) -> String {
    let run = run_title(input.run_number);
    if input.target_label.is_empty() {
        run
    } else {
        format!("{run} \u{b7} {}", input.target_label)
    }
}

/// `"Finished · Keep-alive · 2026-10-23 12:04 · 50 VUs · 60 s"`, leaving out what is unknown.
fn subtitle(input: &ReportInput) -> String {
    let mut parts = vec![input.status_label.to_string()];
    parts.extend(input.connection_label.map(str::to_string));
    parts.extend(input.started_at.map(|at| local_date_time(at as i64)));
    parts.extend(input.vus.map(|vus| format!("{} VUs", format_integer(vus))));
    parts.push(format!("{} s", format_integer(input.duration_secs)));
    parts.join(" \u{b7} ")
}

/// The status badge's text for `status`.
#[must_use]
pub fn status_label(status: &LoadTestStatus) -> String {
    match status {
        LoadTestStatus::NotStarted => t!("load_test.status.not_started"),
        LoadTestStatus::Running => t!("load_test.status.running"),
        LoadTestStatus::Finished => t!("load_test.status.finished"),
        LoadTestStatus::Stopped => t!("load_test.status.stopped"),
        LoadTestStatus::Failed(_) => t!("load_test.status.failed"),
    }
    .into_owned()
}

/// How a run connected: `"Keep-alive"` when it reused its connections, `"New connections"`
/// otherwise.
#[must_use]
pub fn connection_label(reused: bool) -> String {
    if reused {
        t!("load_test.connections.reused")
    } else {
        t!("load_test.connections.new")
    }
    .into_owned()
}

/// A threshold's goal without its metric, for example `"\u{2265} 1,800"` or `"< 0.5 %"`.
fn threshold_goal(threshold: &Threshold) -> String {
    let (limit, unit) = match threshold.metric {
        ThresholdMetric::ErrorRate => (format_trimmed(threshold.limit * 100.0), " %"),
        ThresholdMetric::Rps => (format_trimmed(threshold.limit), ""),
        _ => (format_trimmed(threshold.limit), " ms"),
    };
    format!("{} {limit}{unit}", threshold.metric.operator())
}

/// The run's value of a threshold metric as text: `"62 ms"`, `"0.4 %"`, `"1,850"`.
fn threshold_value(metric: ThresholdMetric, value: f64) -> String {
    match metric {
        ThresholdMetric::ErrorRate => format!("{} %", format_decimal(value * 100.0, 1)),
        ThresholdMetric::Rps => format_decimal(value, 0),
        _ => format_latency_ms(value),
    }
}

/// A latency in microseconds split into its number and its unit: `("38", "ms")`.
fn latency_parts(us: u32) -> (String, String) {
    let text = format_latency_ms(f64::from(us) / 1_000.0);
    match text.rsplit_once(' ') {
        Some((value, unit)) => (value.to_string(), unit.to_string()),
        None => (text, String::new()),
    }
}

/// The change from `previous` to `current` as a percentage of `previous`, `None` when
/// `previous` is zero.
fn relative_change(current: f64, previous: f64) -> Option<f64> {
    (previous > 0.0).then(|| (current - previous) / previous * 100.0)
}

/// The six KPI cells, with the deltas against the compared run and the goals of the thresholds.
fn kpis(input: &ReportInput) -> Vec<ReportKpi> {
    let summary = input.summary;
    let outcomes = evaluate_thresholds(input.thresholds, &summary.threshold_inputs());
    let goal = |metric: ThresholdMetric| -> Option<(String, bool)> {
        outcomes
            .iter()
            .find(|outcome| outcome.threshold.metric == metric)
            .map(|outcome| {
                (
                    t!(
                        "load_test.kpi.goal",
                        limit = threshold_goal(&outcome.threshold)
                    )
                    .into_owned(),
                    outcome.passed,
                )
            })
    };
    let delta = |change: Option<f64>, unit: &str, higher_is_better: bool| {
        change.map_or((None, ReportTone::Neutral), |change| {
            let tone = report_tone(delta_tone(change, higher_is_better));
            (Some(format_delta(change, unit)), tone)
        })
    };
    let previous = input.compare.map(|(_, snapshot)| snapshot);
    let relative = |current: f64, pick: fn(&LoadSnapshot) -> f64| {
        previous.and_then(|snapshot| relative_change(current, pick(snapshot)))
    };

    let make = |label: String,
                value: String,
                unit: String,
                (delta, tone): (Option<String>, ReportTone),
                goal: Option<(String, bool)>| ReportKpi {
        label,
        value,
        unit,
        delta,
        delta_tone: tone,
        goal,
    };
    let latency = |label: &str, us: u32, change: Option<f64>, metric: ThresholdMetric| {
        let (value, unit) = latency_parts(us);
        make(
            label.to_string(),
            value,
            unit,
            delta(change, " %", false),
            goal(metric),
        )
    };

    vec![
        make(
            t!("load_test.kpi.rps").into_owned(),
            format_decimal(summary.rps, 0),
            String::new(),
            delta(relative(summary.rps, |s| s.rps), " %", true),
            goal(ThresholdMetric::Rps),
        ),
        latency(
            "p50",
            summary.p50,
            relative(f64::from(summary.p50), |s| f64::from(s.p50)),
            ThresholdMetric::P50,
        ),
        latency(
            "p95",
            summary.p95,
            relative(f64::from(summary.p95), |s| f64::from(s.p95)),
            ThresholdMetric::P95,
        ),
        latency(
            "p99",
            summary.p99,
            relative(f64::from(summary.p99), |s| f64::from(s.p99)),
            ThresholdMetric::P99,
        ),
        make(
            t!("load_test.kpi.errors").into_owned(),
            format_decimal(summary.error_rate * 100.0, 1),
            "%".to_string(),
            delta(
                previous.map(|snapshot| (summary.error_rate - snapshot.error_rate) * 100.0),
                " pt",
                false,
            ),
            goal(ThresholdMetric::ErrorRate),
        ),
        make(
            t!("load_test.kpi.total").into_owned(),
            format_integer(summary.total),
            String::new(),
            // A window's total is not comparable with the whole compared run's.
            if input.covers_window() {
                (None, ReportTone::Neutral)
            } else {
                delta(
                    relative(summary.total as f64, |s| s.total as f64),
                    " %",
                    true,
                )
            },
            None,
        ),
    ]
}

/// The seconds of the chart the report shows.
fn chart_range(input: &ReportInput, points: usize) -> Range<usize> {
    match input.window.filter(|_| input.window_only) {
        Some((first, last)) => first.min(points)..(last + 1).min(points),
        None => 0..points,
    }
}

/// The largest finite value of `values`, `0.0` for none.
fn finite_max(values: &[f64]) -> f64 {
    values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(0.0, f64::max)
}

/// The "Performance over time" chart, `None` when the run has no per second data.
fn chart(input: &ReportInput) -> Option<ReportChart> {
    let data = input.chart?;
    let range = chart_range(input, data.rps.len());
    if range.is_empty() {
        return None;
    }
    let slice = |values: &[f64]| -> Vec<f64> { values.get(range.clone()).unwrap_or(&[]).to_vec() };

    let rps = slice(&data.rps);
    let vus = slice(&data.vus);
    let p50 = slice(&data.p50);
    let p95 = slice(&data.p95);
    let p99 = slice(&data.p99);
    let baseline = data.baseline.as_deref().map(slice);

    let left = rps_axis(finite_max(&rps)).with_top_suffix("/s");
    // Like the app's chart, the axis reaches the goal lines so they are drawn where they belong.
    let right = ms_axis(
        [&p50, &p95, &p99]
            .into_iter()
            .chain(baseline.as_ref())
            .map(|values| finite_max(values))
            .chain(
                input
                    .thresholds
                    .iter()
                    .filter(|threshold| is_latency(threshold.metric))
                    .map(|threshold| threshold.limit),
            )
            .fold(0.0, f64::max),
    )
    .with_top_suffix(" ms");

    let max_vus = finite_max(&vus);
    let vus_area = vus
        .iter()
        .map(|vus| {
            if max_vus > 0.0 {
                vus / max_vus * left.max * VUS_AREA_SHARE
            } else {
                0.0
            }
        })
        .collect();

    let colors = input.colors;
    let line = |label: String,
                color: Rgb,
                axis: ReportAxis,
                style: ReportSeriesStyle,
                values: Vec<f64>| ReportSeries {
        label,
        color,
        axis,
        style,
        values,
    };
    let mut series = vec![line(
        "VUs".to_string(),
        colors.vus,
        ReportAxis::Left,
        ReportSeriesStyle::Area,
        vus_area,
    )];
    if let (Some(values), Some((number, _))) = (baseline, input.compare) {
        series.push(line(
            format!("{} \u{b7} p95", run_title(number)),
            colors.baseline,
            ReportAxis::Right,
            ReportSeriesStyle::Dashed(1.25),
            values,
        ));
    }
    series.extend([
        line(
            "p99".to_string(),
            colors.p99,
            ReportAxis::Right,
            ReportSeriesStyle::Line(1.5),
            p99,
        ),
        line(
            "p95".to_string(),
            colors.p95,
            ReportAxis::Right,
            ReportSeriesStyle::Line(1.25),
            p95,
        ),
        line(
            "p50".to_string(),
            colors.p50,
            ReportAxis::Right,
            ReportSeriesStyle::Line(1.25),
            p50,
        ),
        line(
            t!("load_test.kpi.rps").into_owned(),
            colors.rps,
            ReportAxis::Left,
            ReportSeriesStyle::Line(1.5),
            rps,
        ),
    ]);

    let reference_lines = input
        .thresholds
        .iter()
        .filter(|threshold| is_latency(threshold.metric))
        .map(|threshold| ReportReferenceLine {
            axis: ReportAxis::Right,
            value: threshold.limit,
            label: t!(
                "load_test.chart.goal",
                metric = threshold_metric_label(threshold.metric),
                limit = format!("{} ms", format_trimmed(threshold.limit))
            )
            .into_owned(),
            color: colors.bad,
        })
        .collect();

    // A window only makes sense as a shade over the whole run.
    let shaded = input
        .window
        .filter(|_| !input.window_only)
        .map(|(first, last)| (first, last + 1));

    Some(ReportChart {
        title: t!("load_test.chart.title").into_owned(),
        series,
        left_max: left.max,
        right_max: right.max,
        left_ticks: left.labels,
        right_ticks: right.labels,
        x_ticks: x_labels_from(range.start as u64, range.len() as u64),
        reference_lines,
        window: shaded,
    })
}

/// Whether `metric` is a latency, drawn as a goal line on the chart's ms axis.
fn is_latency(metric: ThresholdMetric) -> bool {
    matches!(
        metric,
        ThresholdMetric::P50 | ThresholdMetric::P95 | ThresholdMetric::P99
    )
}

/// A status as text: the HTTP code, or the translated "Timeout" or "Failed".
fn status_text(status: StatusKey) -> String {
    match status {
        StatusKey::Code(code) => code.to_string(),
        StatusKey::Timeout => t!("load_test.status_key.timeout").into_owned(),
        StatusKey::Failed => t!("load_test.status_key.failed").into_owned(),
    }
}

/// The statuses of `counts` out of `total` requests, ascending as given, those with requests.
fn status_list(
    counts: &[(StatusKey, u64)],
    total: u64,
    colors: &ReportColors,
) -> Vec<ReportStatus> {
    let total = total.max(1) as f64;
    counts
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(status, count)| {
            let share = (*count as f64 / total).clamp(0.0, 1.0);
            ReportStatus {
                label: status_text(*status),
                share,
                share_text: format!("{} %", format_decimal(share * 100.0, 1)),
                count_text: format_integer(*count),
                color: colors.status(*status),
            }
        })
        .collect()
}

/// A target's label: `"POST create"`, or `"Target 3"` without a known name.
fn target_label(targets: &[(String, String)], index: usize) -> String {
    match targets.get(index) {
        Some((method, name)) if method.is_empty() => name.clone(),
        Some((method, name)) => format!("{method} {name}"),
        None => t!("load_test.table.target_n", number = index + 1).into_owned(),
    }
}

/// The per request table: a row per request, the totals row of all of them and the statuses.
fn per_request(input: &ReportInput) -> ReportTable {
    let summary = input.summary;
    let rows = summary
        .per_target
        .iter()
        .enumerate()
        .map(|(index, stats)| {
            vec![
                target_label(input.targets, index),
                format_integer(stats.count),
                format_latency_ms(f64::from(stats.p50) / 1_000.0),
                format_latency_ms(f64::from(stats.p95) / 1_000.0),
                format_latency_ms(f64::from(stats.p99) / 1_000.0),
                format!("{} %", format_decimal(stats.error_rate * 100.0, 1)),
            ]
        })
        .collect();
    let total = vec![
        t!("load_export.all_requests").into_owned(),
        format_integer(summary.total),
        format_latency_ms(f64::from(summary.p50) / 1_000.0),
        format_latency_ms(f64::from(summary.p95) / 1_000.0),
        format_latency_ms(f64::from(summary.p99) / 1_000.0),
        format!("{} %", format_decimal(summary.error_rate * 100.0, 1)),
    ];
    let total_statuses = status_list(&summary.statuses, summary.total, input.colors);
    // A run without saved requests does not know the statuses of each request.
    let statuses = (!total_statuses.is_empty()).then(|| ReportStatusColumn {
        header: t!("load_export.statuses").into_owned(),
        rows: if summary.target_statuses.is_empty() {
            Vec::new()
        } else {
            (0..summary.per_target.len())
                .map(|index| {
                    let counts = summary
                        .target_statuses
                        .get(index)
                        .map_or(&[][..], Vec::as_slice);
                    let target_total = counts.iter().map(|(_, count)| count).sum();
                    status_list(counts, target_total, input.colors)
                })
                .collect()
        },
        total: total_statuses,
    });
    ReportTable {
        title: t!("load_test.table.per_request").into_owned(),
        headers: vec![
            t!("common.request").into_owned(),
            t!("load_test.table.count").into_owned(),
            "p50".to_string(),
            "p95".to_string(),
            "p99".to_string(),
            t!("load_test.kpi.errors").into_owned(),
        ],
        align: vec![
            ReportAlign::Left,
            ReportAlign::Right,
            ReportAlign::Right,
            ReportAlign::Right,
            ReportAlign::Right,
            ReportAlign::Right,
        ],
        rows,
        total: Some(total),
        statuses,
    }
}

/// A latency in microseconds as text, `"-"` for none.
fn latency_cell(us: u32, count: u64) -> String {
    if count == 0 {
        "-".to_string()
    } else {
        format_latency_ms(f64::from(us) / 1_000.0)
    }
}

/// A y tick of the percentile chart: `"20 ms"`, `"1 s"`.
fn tick_label(ms: f64) -> String {
    if ms >= 1_000.0 {
        format!("{} s", format_trimmed(ms / 1_000.0))
    } else {
        format!("{} ms", format_trimmed(ms))
    }
}

/// The ticks of the percentile chart's axis in milliseconds: 1, 2, 3 and 5 times each power of
/// ten.
fn curve_ticks_ms() -> impl Iterator<Item = f64> {
    (-1..=6).flat_map(|power| CURVE_TICKS.iter().map(move |tick| tick * 10f64.powi(power)))
}

/// The latency by percentile chart of the requests' `curves` (microseconds), `None` when none
/// has responses.
fn curves(input: &ReportInput, curves: &[Vec<u32>]) -> Option<ReportCurves> {
    let lines: Vec<ReportCurveLine> = curves
        .iter()
        .enumerate()
        .filter(|(_, values)| !values.is_empty())
        .map(|(index, values)| ReportCurveLine {
            name: target_label(input.targets, index),
            color: input.colors.request(index),
            values_us: values.iter().map(|us| f64::from(*us)).collect(),
        })
        .collect();
    let all = lines.iter().flat_map(|line| &line.values_us);
    let fastest = all.clone().copied().fold(f64::INFINITY, f64::min) / 1_000.0;
    let slowest = all.copied().fold(0.0, f64::max) / 1_000.0;
    if lines.is_empty() {
        return None;
    }
    // The axis reaches the nearest ticks around the data, with a little room.
    let low = curve_ticks_ms()
        .take_while(|tick| *tick <= fastest * (1.0 - CURVE_MARGIN))
        .last()
        .unwrap_or(0.1);
    let high = curve_ticks_ms()
        .find(|tick| *tick >= slowest * (1.0 + CURVE_MARGIN))
        .unwrap_or(slowest.max(low * 2.0));
    Some(ReportCurves {
        title: t!("load_export.latency.curves_title").into_owned(),
        subtitle: t!("load_export.latency.curves_axes").into_owned(),
        lines,
        y_min_us: low * 1_000.0,
        y_max_us: high * 1_000.0,
        y_ticks: curve_ticks_ms()
            .filter(|tick| (low..=high).contains(tick))
            .map(|tick| (tick * 1_000.0, tick_label(tick)))
            .collect(),
        x_ticks: vec![
            t!("load_export.latency.min").into_owned(),
            "p90".to_string(),
            "p99".to_string(),
            format!("p{}", format_decimal(99.9, 1)),
        ],
    })
}

/// One request's small histogram, on its own linear axis.
fn request_histogram(
    input: &ReportInput,
    log: &SampleLog,
    options: &LatencyOptions,
    index: usize,
    stats: &LatencyPercentiles,
) -> Option<ReportRequestHistogram> {
    let view = latency_view_with(
        log,
        input.target_count,
        &LatencyOptions {
            target: Some(index),
            ..options.clone()
        },
        input.progress,
    )?;
    let histogram = &view.histogram;
    let colors = input.colors;
    let at = |us: u32| histogram.fraction(f64::from(us));
    let edge = |us: f64| format_latency_ms(us / 1_000.0);
    let (low, high) = match (histogram.edges_us.first(), histogram.edges_us.last()) {
        (Some(low), Some(high)) => (*low, *high),
        _ => (0.0, 0.0),
    };
    let series = [
        ("p50", stats.p50, colors.p50),
        ("p95", stats.p95, colors.p95),
        ("p99", stats.p99, colors.p99),
    ];
    Some(ReportRequestHistogram {
        name: target_label(input.targets, index),
        color: colors.request(index),
        count_text: plural("load_export.latency.responses", stats.count as usize),
        counts: histogram.counts.iter().map(|bin| bin[index]).collect(),
        markers: series
            .iter()
            .map(|(_, us, color)| (at(*us), *color))
            .collect(),
        x_labels: vec![edge(low), edge((low + high) / 2.0), edge(high)],
        caption: series
            .iter()
            .map(|(name, us, color)| {
                (
                    format!("{name} {}", format_latency_ms(f64::from(*us) / 1_000.0)),
                    *color,
                )
            })
            .collect(),
    })
}

/// The Latency section of the covered range: successful responses only, `None` without the
/// run's requests.
fn latency(input: &ReportInput) -> Option<ReportLatency> {
    let log = input.log?;
    let range = input
        .window
        .filter(|_| input.window_only)
        .map(|(first, last)| first as u32..last as u32 + 1);
    let targets = input.target_count.max(input.targets.len());
    let options = LatencyOptions {
        range,
        include_errors: false,
        target: None,
        scale: LatencyScale::Linear,
    };
    let view = latency_view_with(log, targets, &options, input.progress)?;

    let mut histograms = Vec::new();
    for (index, stats) in view.per_target.iter().enumerate() {
        if stats.count == 0 {
            // Its pass is skipped: count it as read, see `latency_units`.
            input.progress.advance(log.len() as u64);
            continue;
        }
        histograms.push(request_histogram(input, log, &options, index, stats)?);
    }

    let row = |name: String, color: Option<Rgb>, stats: &LatencyPercentiles| ReportLatencyRow {
        name,
        color,
        cells: vec![
            format_integer(stats.count),
            latency_cell(stats.p50, stats.count),
            latency_cell(stats.p90, stats.count),
            latency_cell(stats.p95, stats.count),
            latency_cell(stats.p99, stats.count),
            latency_cell(stats.max, stats.count),
        ],
    };
    let mut rows = vec![row(
        t!("load_export.all_requests").into_owned(),
        None,
        &view.all,
    )];
    rows.extend(view.per_target.iter().enumerate().map(|(index, stats)| {
        row(
            target_label(input.targets, index),
            Some(input.colors.request(index)),
            stats,
        )
    }));

    Some(ReportLatency {
        title: t!("load_export.latency.title").into_owned(),
        caption: t!("load_export.latency.caption").into_owned(),
        note: if view.excluded == 0 {
            t!("load_export.latency.note_none").into_owned()
        } else {
            plural("load_export.latency.note", view.excluded as usize)
        },
        histograms,
        headers: vec![
            t!("common.request").into_owned(),
            t!("load_test.table.count").into_owned(),
            "p50".to_string(),
            "p90".to_string(),
            "p95".to_string(),
            "p99".to_string(),
            t!("load_export.latency.max").into_owned(),
        ],
        rows,
        curves: curves(input, &view.curves),
    })
}

/// The title of an error group: `"429 Too Many Requests"`, `"No response in 30 s"` or
/// `"Request failed"`.
fn error_title(status: StatusKey, timeout_secs: u64) -> String {
    match status {
        StatusKey::Code(code) => match reason_phrase(code) {
            Some(reason) => format!("{code} {reason}"),
            None => code.to_string(),
        },
        StatusKey::Timeout => {
            t!("load_test.errors.timeout_title", secs = timeout_secs).into_owned()
        }
        StatusKey::Failed => t!("load_test.errors.failed_title").into_owned(),
    }
}

/// The error groups, one entry each.
fn errors(input: &ReportInput) -> Vec<ReportErrorGroup> {
    let total = input.summary.total.max(1) as f64;
    input
        .error_groups
        .iter()
        .map(|group| {
            let mut affected: Vec<(usize, u64)> = group
                .per_target
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, count)| *count > 0)
                .collect();
            affected.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
            let affected = affected
                .iter()
                .map(|(index, count)| {
                    format!(
                        "{} {}",
                        target_label(input.targets, *index),
                        format_integer(*count)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let sample = input
                .error_examples
                .iter()
                .find(|example| example.status == group.status)
                .map(|example| example.text.clone())
                .unwrap_or_default();
            ReportErrorGroup {
                code: status_text(group.status),
                title: error_title(group.status, input.timeout_secs),
                count: format_integer(group.count),
                share: format!(
                    "{} %",
                    format_decimal(group.count as f64 / total * 100.0, 2)
                ),
                span: format!(
                    "{}: {} \u{b7} {}",
                    t!("load_test.errors.first_last"),
                    mmss(u64::from(group.first_ms) / 1_000),
                    mmss(u64::from(group.last_ms) / 1_000)
                ),
                requests: format!("{}: {affected}", t!("load_test.errors.affected")),
                sample,
            }
        })
        .collect()
}

/// `unix` as local `HH:MM`, for the dashboard header and the reports.
#[must_use]
pub fn local_time(unix: i64) -> String {
    match Local.timestamp_opt(unix, 0).single() {
        Some(time) => time.format("%H:%M").to_string(),
        None => "--:--".to_string(),
    }
}

/// `unix` as local `YYYY-MM-DD HH:MM`.
#[must_use]
pub fn local_date_time(unix: i64) -> String {
    match Local.timestamp_opt(unix, 0).single() {
        Some(time) => time.format("%Y-%m-%d %H:%M").to_string(),
        None => "----/--/-- --:--".to_string(),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_load::{SampleRecord, SecondPoint, TargetStats};
    use pretty_assertions::assert_eq;

    fn snapshot(rps: f64, p99_us: u32, error_rate: f64, total: u64) -> LoadSnapshot {
        LoadSnapshot {
            elapsed: std::time::Duration::from_secs(60),
            active_vus: 0,
            total,
            rps,
            p50: 10_000,
            p95: 30_000,
            p99: p99_us,
            error_rate,
            series: (0..60)
                .map(|second| SecondPoint {
                    rps: 1_000.0 + f64::from(second),
                    vus: 50,
                    p50: 10_000,
                    p90: 20_000,
                    p95: 30_000,
                    p99: p99_us,
                })
                .collect(),
            histogram: [0; 26],
            status_counts: vec![],
            per_target: vec![],
            error_examples: vec![],
        }
    }

    fn summary() -> SummaryData {
        SummaryData {
            rps: 2_000.0,
            p50: 12_000,
            p95: 30_000,
            p99: 62_000,
            error_rate: 0.01,
            total: 1_000,
            statuses: vec![
                (StatusKey::Code(200), 900),
                (StatusKey::Code(429), 90),
                (StatusKey::Timeout, 10),
            ],
            per_target: vec![
                TargetStats {
                    count: 600,
                    p50: 10_000,
                    p95: 20_000,
                    p99: 40_000,
                    error_rate: 0.0,
                },
                TargetStats {
                    count: 400,
                    p50: 14_000,
                    p95: 40_000,
                    p99: 62_000,
                    error_rate: 0.025,
                },
            ],
            target_statuses: vec![
                vec![(StatusKey::Code(200), 600)],
                vec![
                    (StatusKey::Code(200), 300),
                    (StatusKey::Code(429), 90),
                    (StatusKey::Timeout, 10),
                ],
            ],
        }
    }

    fn groups() -> Vec<ErrorGroup> {
        vec![
            ErrorGroup {
                status: StatusKey::Code(429),
                count: 90,
                first_ms: 34_100,
                last_ms: 45_900,
                per_second: vec![],
                per_target: vec![20, 70],
            },
            ErrorGroup {
                status: StatusKey::Timeout,
                count: 10,
                first_ms: 10_000,
                last_ms: 11_000,
                per_second: vec![],
                per_target: vec![0, 10],
            },
        ]
    }

    fn examples() -> Vec<ErrorExample> {
        vec![ErrorExample {
            status: StatusKey::Code(429),
            text: "HTTP 429 \u{b7} rate limit".to_string(),
        }]
    }

    struct Fixture {
        summary: SummaryData,
        compare: LoadSnapshot,
        thresholds: Vec<Threshold>,
        chart: ChartData,
        targets: Vec<(String, String)>,
        groups: Vec<ErrorGroup>,
        examples: Vec<ErrorExample>,
        colors: ReportColors,
        log: Option<SampleLog>,
        progress: Progress,
    }

    impl Fixture {
        fn new() -> Self {
            let run = snapshot(2_000.0, 62_000, 0.01, 1_000);
            let compare = snapshot(1_900.0, 50_000, 0.02, 800);
            Self {
                summary: summary(),
                chart: ChartData::new(&run, Some(&compare)),
                compare,
                thresholds: vec![
                    Threshold {
                        metric: ThresholdMetric::P99,
                        limit: 50.0,
                    },
                    Threshold {
                        metric: ThresholdMetric::Rps,
                        limit: 1_800.0,
                    },
                ],
                targets: vec![
                    ("GET".to_string(), "list".to_string()),
                    ("POST".to_string(), "create".to_string()),
                ],
                groups: groups(),
                examples: examples(),
                colors: ReportColors::light(),
                log: Some(log()),
                progress: Progress::default(),
            }
        }

        fn input(&self) -> ReportInput<'_> {
            ReportInput {
                run_number: 3,
                target_label: "users",
                status_label: "Finished",
                connection_label: Some("Keep-alive"),
                started_at: Some(1_790_000_000),
                vus: Some(50),
                duration_secs: 60,
                summary: &self.summary,
                compare: Some((2, &self.compare)),
                thresholds: &self.thresholds,
                run_inputs: self.summary.threshold_inputs(),
                chart: Some(&self.chart),
                window: None,
                window_only: false,
                targets: &self.targets,
                log: self.log.as_ref(),
                target_count: 2,
                error_groups: &self.groups,
                error_examples: &self.examples,
                timeout_secs: 30,
                generated_at: 1_790_000_000,
                colors: &self.colors,
                progress: &self.progress,
            }
        }
    }

    #[test]
    fn the_title_subtitle_and_comparison_are_texts() {
        let fixture = Fixture::new();
        let report = build_report(&fixture.input());
        assert_eq!(report.title, "Run #3 \u{b7} users");
        assert!(
            report
                .subtitle
                .starts_with("Finished \u{b7} Keep-alive \u{b7} ")
        );
        assert!(report.subtitle.ends_with("\u{b7} 50 VUs \u{b7} 60 s"));
        assert_eq!(report.comparison.as_deref(), Some("Compared with Run #2"));
        assert_eq!(report.window, None);
        assert!(
            report
                .labels
                .generated_by
                .starts_with("Generated by Brasa ")
        );
    }

    #[test]
    fn the_verdict_counts_the_failed_thresholds_of_the_whole_run() {
        let fixture = Fixture::new();
        let report = build_report(&fixture.input());
        // p99 62 ms is over 50 ms, 2,000 requests/s meets 1,800.
        let verdict = report.verdict.expect("the run has thresholds");
        assert!(!verdict.passed);
        assert_eq!(verdict.text, "Not passed \u{b7} 1 of 2");
        assert_eq!(report.thresholds.len(), 2);
        assert_eq!(report.thresholds[0].label, "p99 < 50 ms");
        assert_eq!(report.thresholds[0].value, "62 ms");
        assert!(!report.thresholds[0].passed);
        assert!(report.thresholds[1].passed);

        let mut input = fixture.input();
        input.thresholds = &[];
        assert_eq!(build_report(&input).verdict, None);
    }

    #[test]
    fn the_kpis_carry_goals_and_deltas() {
        let fixture = Fixture::new();
        let report = build_report(&fixture.input());
        let rps = &report.kpis[0];
        assert_eq!(rps.label, "Requests/s");
        assert_eq!(rps.value, "2,000");
        assert_eq!(rps.goal, Some(("goal \u{2265} 1,800".to_string(), true)));
        // 2,000 against 1,900 is +5.3 %, better for throughput.
        assert_eq!(rps.delta.as_deref(), Some("+5.3 %"));
        assert_eq!(rps.delta_tone, ReportTone::Good);

        let p99 = &report.kpis[3];
        assert_eq!((p99.value.as_str(), p99.unit.as_str()), ("62", "ms"));
        assert_eq!(p99.goal, Some(("goal < 50 ms".to_string(), false)));
        // 62 ms against 50 ms is +24 %, worse for a latency.
        assert_eq!(p99.delta.as_deref(), Some("+24.0 %"));
        assert_eq!(p99.delta_tone, ReportTone::Bad);

        // Errors go from 2.0 % to 1.0 %: minus one point, better.
        let errors = &report.kpis[4];
        assert_eq!(errors.value, "1.0");
        assert_eq!(errors.delta.as_deref(), Some("-1.0 pt"));
        assert_eq!(errors.delta_tone, ReportTone::Good);
        assert_eq!(errors.goal, None);

        assert_eq!(report.kpis[5].label, "Total");
        assert_eq!(report.kpis[5].delta.as_deref(), Some("+25.0 %"));
    }

    #[test]
    fn without_a_comparison_there_are_no_deltas() {
        let fixture = Fixture::new();
        let mut input = fixture.input();
        input.compare = None;
        let report = build_report(&input);
        assert_eq!(report.comparison, None);
        assert!(report.kpis.iter().all(|kpi| kpi.delta.is_none()));
        let chart = report.chart.expect("the run has a chart");
        assert_eq!(chart.series.len(), 5);
    }

    #[test]
    fn the_chart_has_two_axes_the_goal_line_and_the_shaded_window() {
        let fixture = Fixture::new();
        let mut input = fixture.input();
        input.window = Some((34, 45));
        let chart = build_report(&input).chart.expect("the run has a chart");
        // The busiest second is 1,059 requests/s: a 500 step reaches 2,000.
        assert_eq!(
            chart.left_ticks,
            vec!["0", "500", "1,000", "1,500", "2,000/s"]
        );
        assert!((chart.left_max - 2_000.0).abs() < 1e-9);
        assert_eq!(chart.right_ticks.last().map(String::as_str), Some("80 ms"));
        assert_eq!(chart.x_ticks.first().map(String::as_str), Some("0s"));
        assert_eq!(chart.x_ticks.last().map(String::as_str), Some("60s"));
        assert_eq!(chart.window, Some((34, 46)));
        assert_eq!(chart.series.len(), 6);
        assert_eq!(chart.series[0].label, "VUs");
        assert_eq!(chart.series[1].label, "Run #2 \u{b7} p95");
        assert_eq!(chart.series[1].style, ReportSeriesStyle::Dashed(1.25));
        assert_eq!(chart.series[0].values.len(), 60);
        // 50 of 50 virtual users reach 90 % of the left axis.
        assert!((chart.series[0].values[0] - 1_800.0).abs() < 1e-9);
        assert_eq!(chart.reference_lines.len(), 1);
        assert_eq!(chart.reference_lines[0].label, "p99 goal 50 ms");
        assert!((chart.reference_lines[0].value - 50.0).abs() < 1e-9);
        assert_eq!(chart.reference_lines[0].axis, ReportAxis::Right);
    }

    #[test]
    fn exporting_only_the_window_crops_the_chart() {
        let fixture = Fixture::new();
        let mut input = fixture.input();
        input.window = Some((30, 41));
        input.window_only = true;
        let report = build_report(&input);
        assert_eq!(report.window.as_deref(), Some("Window 30\u{2013}42 s"));
        assert_eq!(report.labels.no_errors, "No errors in this window");
        let chart = report.chart.expect("the run has a chart");
        assert_eq!(chart.window, None);
        assert!(chart.series.iter().all(|series| series.values.len() == 12));
        assert_eq!(chart.x_ticks.first().map(String::as_str), Some("30s"));
        assert_eq!(chart.x_ticks.last().map(String::as_str), Some("42s"));
        // The window's total is not compared with the whole compared run.
        assert_eq!(report.kpis[5].delta, None);
    }

    /// 100 successful responses per request (about 300 ms and 2 s), and 3 errors of the second
    /// one in the second 5.
    fn log() -> SampleLog {
        let mut log = SampleLog::new();
        let url = log.intern("http://t/x");
        let mut push = |elapsed_ms: u32, target: u32, status: StatusKey, latency_ms: u32| {
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
        };
        for i in 0..100u32 {
            let second = i % 10;
            push(second * 1_000, 0, StatusKey::Code(200), 300 + i % 5);
            push(second * 1_000, 1, StatusKey::Code(200), 2_000 + i % 3);
        }
        for _ in 0..3 {
            push(5_000, 1, StatusKey::Code(429), 1);
        }
        log
    }

    #[test]
    fn the_per_request_table_has_a_totals_row_and_status_bars() {
        let fixture = Fixture::new();
        let table = build_report(&fixture.input()).per_request;
        assert_eq!(table.rows[1][0], "POST create");
        assert_eq!(table.rows[1][2], "14 ms");
        assert_eq!(table.rows[1][5], "2.5 %");
        let total = table.total.expect("a totals row");
        assert_eq!(total.len(), table.headers.len());
        assert_eq!(total[0], "All requests");
        assert_eq!(total[1], "1,000");
        assert_eq!(total[4], "62 ms");
        assert_eq!(total[5], "1.0 %");

        let column = table.statuses.expect("the run has statuses");
        assert_eq!(column.header, "Statuses");
        let labels: Vec<&str> = column.total.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["200", "429", "Timeout"]);
        assert_eq!(column.total[0].share_text, "90.0 %");
        assert_eq!(column.total[0].count_text, "900");
        assert_eq!(column.total[2].color, fixture.colors.failure);
        assert_ne!(fixture.colors.failure, fixture.colors.status_server);
        // The second request: 300 of 400 are 200, shares are of its own requests.
        assert_eq!(column.rows.len(), 2);
        assert_eq!(column.rows[0].len(), 1);
        assert_eq!(column.rows[1][0].share_text, "75.0 %");
        assert_eq!(column.rows[1][1].share_text, "22.5 %");
    }

    #[test]
    fn without_the_statuses_of_each_request_there_is_no_column_but_the_totals() {
        let fixture = Fixture::new();
        let mut summary = fixture.summary.clone();
        summary.target_statuses.clear();
        let mut input = fixture.input();
        input.summary = &summary;
        let column = build_report(&input)
            .per_request
            .statuses
            .expect("the totals stay");
        assert!(column.rows.is_empty());
        assert_eq!(column.total.len(), 3);
    }

    #[test]
    fn the_latency_section_reads_successful_responses_per_request() {
        let fixture = Fixture::new();
        let latency = build_report(&fixture.input())
            .latency
            .expect("the run has its requests");
        assert_eq!(
            latency.note,
            "Successful responses only, 3 errors left out."
        );
        assert_eq!(latency.histograms.len(), 2);
        let fast = &latency.histograms[0];
        assert_eq!(fast.name, "GET list");
        assert_eq!(fast.color, fixture.colors.request(0));
        assert_eq!(fast.count_text, "100 responses");
        assert_eq!(fast.counts.iter().sum::<u64>(), 100);
        assert_eq!(fast.x_labels.len(), 3);
        assert_eq!(fast.caption[0].0, "p50 302 ms");
        assert_eq!(fast.markers.len(), 3);
        // Each request has its own axis: the slow one starts near 2 s.
        assert!(latency.histograms[1].x_labels[0].ends_with(" s"));

        assert_eq!(latency.rows[0].name, "All requests");
        assert_eq!(latency.rows[0].color, None);
        assert_eq!(latency.rows[0].cells[0], "200");
        assert_eq!(latency.rows[2].color, Some(fixture.colors.request(1)));
        assert_eq!(latency.rows[2].cells[1], "2.0 s");
        assert_eq!(latency.headers.len(), latency.rows[0].cells.len() + 1);

        let curves = latency.curves.expect("two lines");
        assert_eq!(curves.lines.len(), 2);
        assert_eq!(curves.lines[0].values_us.len(), 61);
        assert_eq!(curves.x_ticks, vec!["min", "p90", "p99", "p99.9"]);
        let labels: Vec<&str> = curves.y_ticks.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec!["200 ms", "300 ms", "500 ms", "1 s", "2 s", "3 s"]
        );
        assert!((curves.y_min_us - 200_000.0).abs() < 1.0);
        assert!((curves.y_max_us - 3_000_000.0).abs() < 1.0);
    }

    #[test]
    fn the_latency_section_follows_the_window_and_needs_the_log() {
        let fixture = Fixture::new();
        // The errors are in the second 5: a window without it leaves none out.
        let mut input = fixture.input();
        input.window = Some((0, 3));
        input.window_only = true;
        let latency = build_report(&input).latency.expect("a section");
        assert_eq!(latency.note, "Successful responses only.");
        assert_eq!(latency.rows[0].cells[0], "80");
        // Only shaded, the report still covers the whole run.
        input.window_only = false;
        let latency = build_report(&input).latency.expect("a section");
        assert_eq!(latency.rows[0].cells[0], "200");

        input.log = None;
        assert_eq!(build_report(&input).latency, None);
    }

    #[test]
    fn the_latency_section_reports_every_pass_and_stops_when_cancelled() {
        let mut fixture = Fixture::new();
        let samples = fixture.log.as_ref().map_or(0, SampleLog::len);
        fixture.progress = Progress::new(1);
        fixture
            .progress
            .begin_step(0, latency_units(samples, fixture.targets.len()));
        assert!(build_report(&fixture.input()).latency.is_some());
        assert_eq!(fixture.progress.fraction(), 1.0);

        fixture.progress.cancel();
        assert_eq!(build_report(&fixture.input()).latency, None);
    }

    #[test]
    fn error_groups_have_titles_spans_and_affected_requests() {
        let fixture = Fixture::new();
        let report = build_report(&fixture.input());
        let first = &report.errors[0];
        assert_eq!(first.title, "429 Too Many Requests");
        assert_eq!(first.count, "90");
        assert_eq!(first.share, "9.00 %");
        assert_eq!(first.span, "First \u{b7} last: 00:34 \u{b7} 00:45");
        assert_eq!(
            first.requests,
            "Affected requests: POST create 70, GET list 20"
        );
        assert_eq!(first.sample, "HTTP 429 \u{b7} rate limit");
        let timeout = &report.errors[1];
        assert_eq!(timeout.code, "Timeout");
        assert_eq!(timeout.title, "No response in 30 s");
        assert_eq!(timeout.sample, "");
        assert_eq!(error_title(StatusKey::Failed, 30), "Request failed");
        assert_eq!(error_title(StatusKey::Code(599), 30), "599");
    }

    #[test]
    fn a_snapshot_stands_in_for_the_statistics_of_a_run_without_requests() {
        let mut run = snapshot(2_000.0, 62_000, 0.01, 1_000);
        run.status_counts = vec![(StatusKey::Code(200), 990), (StatusKey::Failed, 10)];
        let stats = stats_from_snapshot(&run);
        assert_eq!((stats.total, stats.errors), (1_000, 10));
        assert_eq!(stats.histogram.bucket_us, 10_000);
        assert_eq!(stats.histogram.counts.len(), 26);
        assert_eq!(stats.statuses, run.status_counts);
        assert!(stats.error_groups.is_empty());
    }

    #[test]
    fn the_light_colors_are_the_palettes() {
        let colors = ReportColors::light();
        assert_eq!(colors.p50, Rgb(0x1f, 0x8f, 0x80));
        assert_eq!(colors.p99, Rgb(0xcf, 0x5b, 0x3a));
        assert_eq!(colors.status(StatusKey::Code(204)), colors.status_ok);
        assert_eq!(colors.status(StatusKey::Failed), colors.failure);
    }

    #[test]
    fn local_times_have_a_fixed_shape() {
        // The timezone varies, so only the shape is checked.
        let time = local_time(1_790_000_000);
        assert_eq!(time.len(), 5);
        assert_eq!(time.as_bytes()[2], b':');
        let date_time = local_date_time(1_790_000_000);
        assert_eq!(date_time.len(), 16);
        assert_eq!(&date_time[4..5], "-");
        assert_eq!(&date_time[7..8], "-");
        assert_eq!(&date_time[10..11], " ");
        assert_eq!(&date_time[11..], local_time(1_790_000_000));
    }
}
