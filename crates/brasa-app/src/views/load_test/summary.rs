//! The Summary tab of the load test results: the KPIs (with the change against the compared run
//! and the threshold goals), the "Performance over time" chart (hover, legend toggles and window
//! selection) and the per request table with each request's responses by status. The latency
//! distribution has its own tab (`latency_tab.rs`).

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::Method;
use brasa_load::{LoadSnapshot, StatusKey, Threshold, ThresholdMetric};

use crate::state::load_results::{
    ChartSeriesKey, SummaryData, VuLabelKind, format_latency_ms, ms_axis, planned_vus, rps_axis,
    second_range_label, vu_labels, x_labels,
};
use crate::state::load_test::{
    DeltaTone, LoadTestTab, delta_tone, format_delta, format_trimmed, threshold_metric_label,
};
use crate::state::locale;
use crate::state::number::{format_decimal, format_integer};
use crate::state::response_render::reason_phrase;
use crate::theme::metrics::{RADIUS_LG, RADIUS_MD};
use crate::theme::{Palette, PaletteExt, BrasaColors};
use crate::views::components::{
    AxisSide, ChartAxis, ChartLabel, ChartSeries, DataCell, DataColumn, DataTable, LabelPlacement,
    MethodBadge, ReferenceLine, SeriesStyle, TimeSeriesChart,
};
use crate::views::root::AppView;

/// Least height of the chart, from the design v3 (a 272 px plot, its axis titles and its
/// labels). It grows into the room left under the results.
const CHART_HEIGHT: f32 = 320.0;
/// Width of the per request table's numeric columns, a little under the table's default so the
/// status bars still fit next to the configuration panel.
const NUMERIC_COLUMN_WIDTH: f32 = 54.0;
/// Width of a status bar's tooltip.
const STATUS_TOOLTIP_WIDTH: f32 = 250.0;
/// Height of a status bar.
const STATUS_BAR_HEIGHT: f32 = 8.0;
/// Opacity of a timeout or a failure in a status bar, a darker red than a 5xx next to it.
const FAILURE_OPACITY: f32 = 0.6;
/// Minimum width of one KPI cell, room for a total of nine figures ("1.234.567"); the two groups
/// of three wrap as whole groups.
const KPI_CELL_MIN_WIDTH: f32 = 136.0;
/// A per request error rate above which its cell turns `warning` (1 %).
const PER_REQUEST_WARNING_ERROR_RATE: f64 = 0.01;
/// The virtual users area reaches this fraction of the chart's height at its peak, like the
/// design: it has no axis of its own, so it stays well clear of the req/s gridlines it could be
/// read against.
const VUS_HEIGHT_FRACTION: f64 = 0.5;

impl AppView {
    /// Renders the Summary tab of the load test tab `tab_id`.
    pub(crate) fn render_load_test_summary(
        &mut self,
        tab_id: &str,
        load_test: &LoadTestTab,
        snapshot: &LoadSnapshot,
        summary: &SummaryData,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let ui_font = cx.theme().font_family.clone();
        let mono_font = cx.theme().mono_font_family.clone();
        let weak = cx.weak_entity();

        let chart_room = self.load_test_entities.chart_room.clone();
        // Grows into the room left under the results: the chart takes it all.
        v_flex()
            .flex_1()
            .gap(px(14.0))
            .child(render_kpis(&palette, load_test, summary))
            .child(render_chart(
                weak, tab_id, &palette, load_test, snapshot, chart_room,
            ))
            .child(render_per_request(
                &palette, &ui_font, &mono_font, load_test, summary,
            ))
            .into_any_element()
    }
}

/// The threshold of `metric` the run shown is checked against, if any.
fn threshold_for(load_test: &LoadTestTab, metric: ThresholdMetric) -> Option<Threshold> {
    load_test
        .run_thresholds
        .iter()
        .find(|threshold| threshold.metric == metric)
        .copied()
}

/// The goal line under a KPI ("goal < 50 ms") and whether the value meets it.
fn kpi_goal(threshold: &Threshold, value: f64) -> (String, bool) {
    let limit = match threshold.metric {
        ThresholdMetric::ErrorRate => format!("{} %", format_trimmed(threshold.limit * 100.0)),
        ThresholdMetric::Rps => format_trimmed(threshold.limit),
        _ => format!("{} ms", format_trimmed(threshold.limit)),
    };
    let text = t!(
        "load_test.kpi.goal",
        limit = format!("{} {limit}", threshold.metric.operator())
    )
    .into_owned();
    (text, threshold.metric.passes(value, threshold.limit))
}

/// One KPI cell's content.
struct Kpi {
    label: String,
    value: String,
    unit: &'static str,
    delta: Option<(String, DeltaTone)>,
    goal: Option<(String, bool)>,
}

/// The change from `previous` to `current` as a KPI delta: a percent change, or percentage points
/// for the error rate; `None` when there is nothing to compare against.
fn kpi_delta(
    previous: Option<f64>,
    current: f64,
    higher_is_better: bool,
    points: bool,
) -> Option<(String, DeltaTone)> {
    let previous = previous?;
    let change = if points {
        (current - previous) * 100.0
    } else if previous.abs() < f64::EPSILON {
        return None;
    } else {
        (current - previous) / previous * 100.0
    };
    let unit = if points { " pt" } else { " %" };
    Some((
        format_delta(change, unit),
        delta_tone(change, higher_is_better),
    ))
}

/// The six KPIs: Requests/s, p50, p95, p99, Errors and Total, each with its delta against the
/// compared run and its threshold goal.
fn render_kpis(palette: &Palette, load_test: &LoadTestTab, summary: &SummaryData) -> AnyElement {
    let previous = load_test.compare_snapshot.as_ref();
    let ms = |us: u32| f64::from(us) / 1_000.0;
    let latency_kpi =
        |label: &str, metric: ThresholdMetric, current: u32, before: Option<u32>| Kpi {
            label: label.to_string(),
            value: format_decimal(ms(current), if current < 10_000 { 1 } else { 0 }),
            unit: "ms",
            delta: kpi_delta(before.map(ms), ms(current), false, false),
            goal: threshold_for(load_test, metric)
                .map(|threshold| kpi_goal(&threshold, ms(current))),
        };
    let kpis = [
        Kpi {
            label: t!("load_test.kpi.rps").into_owned(),
            value: format_integer(summary.rps.round() as u64),
            unit: "",
            delta: kpi_delta(previous.map(|p| p.rps), summary.rps, true, false),
            goal: threshold_for(load_test, ThresholdMetric::Rps)
                .map(|threshold| kpi_goal(&threshold, summary.rps)),
        },
        latency_kpi(
            "p50",
            ThresholdMetric::P50,
            summary.p50,
            previous.map(|p| p.p50),
        ),
        latency_kpi(
            "p95",
            ThresholdMetric::P95,
            summary.p95,
            previous.map(|p| p.p95),
        ),
        latency_kpi(
            "p99",
            ThresholdMetric::P99,
            summary.p99,
            previous.map(|p| p.p99),
        ),
        Kpi {
            label: t!("load_test.kpi.errors").into_owned(),
            value: format_decimal(summary.error_rate * 100.0, 1),
            unit: "%",
            delta: kpi_delta(
                previous.map(|p| p.error_rate),
                summary.error_rate,
                false,
                true,
            ),
            goal: threshold_for(load_test, ThresholdMetric::ErrorRate)
                .map(|threshold| kpi_goal(&threshold, summary.error_rate)),
        },
        Kpi {
            label: t!("load_test.kpi.total").into_owned(),
            value: format_integer(summary.total),
            unit: "",
            delta: kpi_delta(
                previous.map(|p| p.total as f64),
                summary.total as f64,
                true,
                false,
            ),
            goal: None,
        },
    ];
    let any_goal = kpis.iter().any(|kpi| kpi.goal.is_some());

    let mut cells = kpis.into_iter().enumerate();
    let mut groups: Vec<AnyElement> = Vec::new();
    for _ in 0..2 {
        // Stretched, so a cell whose delta wrapped makes its whole row taller, labels aligned.
        let mut group = h_flex()
            .items_stretch()
            .flex_1()
            .min_w(px(KPI_CELL_MIN_WIDTH * 3.0))
            .bg(palette.raised);
        for (index, kpi) in cells.by_ref().take(3) {
            let failing = kpi.goal.as_ref().is_some_and(|(_, met)| !met);
            let mut cell = v_flex()
                .flex_1()
                .min_w(px(KPI_CELL_MIN_WIDTH))
                .overflow_hidden()
                .gap(px(4.0))
                .px(px(14.0))
                .py(px(12.0));
            if index % 3 != 0 {
                cell = cell.border_l_1().border_color(palette.border);
            }
            // The delta drops under the value when both do not fit side by side.
            let mut value_row = h_flex()
                .flex_wrap()
                .items_baseline()
                .gap_x(px(4.0))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(22.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if failing { palette.danger } else { palette.fg })
                        .child(kpi.value),
                )
                .when(!kpi.unit.is_empty(), |row| {
                    row.child(
                        div()
                            .flex_none()
                            .text_size(px(12.0))
                            .text_color(palette.fg_subtle)
                            .child(kpi.unit),
                    )
                });
            if let Some((delta, tone)) = kpi.delta {
                let color = match tone {
                    DeltaTone::Better => palette.success,
                    DeltaTone::Worse => palette.danger,
                    DeltaTone::Neutral => palette.fg_subtle,
                };
                value_row = value_row.child(
                    div()
                        .ml_auto()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(color)
                        .child(delta),
                );
            }
            cell = cell
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(kpi.label),
                )
                .child(value_row);
            if any_goal {
                let mut goal_row = h_flex().h(px(16.0)).items_center().gap(px(4.0));
                if let Some((text, met)) = kpi.goal {
                    let color = if met { palette.success } else { palette.danger };
                    let icon = if met {
                        gpui_kit::assets::IconName::Check
                    } else {
                        gpui_kit::assets::IconName::X
                    };
                    goal_row = goal_row
                        .text_size(px(11.5))
                        .text_color(color)
                        .child(Icon::new(icon).with_size(px(11.0)))
                        .child(text);
                }
                cell = cell.child(goal_row);
            }
            group = group.child(cell);
        }
        groups.push(group.into_any_element());
    }

    h_flex()
        .flex_wrap()
        .items_stretch()
        .gap(px(1.0))
        .border_1()
        .border_color(palette.border)
        .bg(palette.border)
        .rounded(px(RADIUS_LG))
        .overflow_hidden()
        .children(groups)
        .into_any_element()
}

/// The label of a chart series, as the legend and the tooltip show it.
fn series_label(key: ChartSeriesKey, load_test: &LoadTestTab) -> String {
    match key {
        ChartSeriesKey::Vus => "VUs".to_string(),
        ChartSeriesKey::Rps => t!("load_test.kpi.rps").into_owned(),
        ChartSeriesKey::P50 => "p50".to_string(),
        ChartSeriesKey::P95 => "p95".to_string(),
        ChartSeriesKey::P99 => "p99".to_string(),
        ChartSeriesKey::Baseline => format!(
            "{} \u{b7} p95",
            load_test
                .compare_with
                .map(super::dashboard::run_title)
                .unwrap_or_default()
        ),
    }
}

/// The "Performance over time" card: the title, the hint, the legend toggles and the chart.
fn render_chart(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    snapshot: &LoadSnapshot,
    room: Rc<Cell<f32>>,
) -> AnyElement {
    let Some(data) = load_test.chart.clone() else {
        return div().into_any_element();
    };
    let selectable = !load_test.is_running() && load_test.log.is_some();
    let hint = if load_test.window.is_some() {
        t!("load_test.chart.hint_clear")
    } else {
        t!("load_test.chart.hint_drag")
    };

    let legend_keys: Vec<ChartSeriesKey> = ChartSeriesKey::LEGEND
        .into_iter()
        .filter(|key| *key != ChartSeriesKey::Baseline || data.baseline.is_some())
        .collect();
    let legend = legend_keys.iter().map(|key| {
        let key = *key;
        let hidden = load_test.hidden_series.contains(&key);
        let color = palette.series_color(key);
        let swatch = if key == ChartSeriesKey::Baseline {
            h_flex()
                .w(px(12.0))
                .gap(px(2.0))
                .child(div().w(px(5.0)).h(px(2.0)).bg(color))
                .child(div().w(px(5.0)).h(px(2.0)).bg(color))
        } else {
            h_flex().child(div().w(px(12.0)).h(px(2.0)).bg(color))
        };
        let item_weak = weak.clone();
        let tab_id = tab_id.to_string();
        h_flex()
            .id(SharedString::from(format!("load-test-legend-{key:?}")))
            .items_center()
            .gap(px(6.0))
            .h(px(22.0))
            .px(px(6.0))
            .rounded(px(5.0))
            .cursor_pointer()
            .text_color(palette.fg_muted)
            .when(hidden, |item| item.opacity(0.4))
            .hover(|style| style.bg(palette.hover))
            .on_click(move |_, _, cx| {
                let _ = item_weak.update(cx, |view, cx| {
                    view.toggle_load_test_series(&tab_id, key, cx);
                });
            })
            .child(swatch)
            .child(series_label(key, load_test))
    });
    let legend: Vec<_> = legend.collect();

    let elapsed_secs = snapshot.elapsed.as_secs();
    let duration_secs = load_test.shown_duration_secs().max(elapsed_secs).max(1);
    let count = (duration_secs as usize).max(data.rps.len()).max(2);
    let latency_limits: Vec<&Threshold> = load_test
        .run_thresholds
        .iter()
        .filter(|threshold| {
            matches!(
                threshold.metric,
                ThresholdMetric::P50 | ThresholdMetric::P95 | ThresholdMetric::P99
            )
        })
        .collect();
    let max_ms = latency_limits
        .iter()
        .map(|threshold| threshold.limit)
        .fold(data.max_ms, f64::max);
    let left = rps_axis(data.max_rps);
    let right = ms_axis(max_ms);

    let mut chart = TimeSeriesChart::new("load-test-chart", count)
        .height(room.get().max(CHART_HEIGHT))
        .surface(palette.raised)
        .left_axis(
            ChartAxis::new(left.max, left.labels)
                .label_color(palette.accent_text)
                .title("req/s"),
        )
        .right_axis(ChartAxis::new(right.max, right.labels).title("ms"))
        .x_labels(x_labels(duration_secs))
        .time_label(|index| second_range_label(index).into())
        .selection(load_test.window);
    for key in ChartSeriesKey::ALL {
        if load_test.hidden_series.contains(&key) {
            continue;
        }
        let color = palette.series_color(key);
        let label = series_label(key, load_test);
        let order = ChartSeriesKey::LEGEND
            .iter()
            .position(|legend| *legend == key)
            .unwrap_or(0) as i32;
        let series = match key {
            ChartSeriesKey::Vus => ChartSeries::new(label, color, data.vus.clone())
                .style(SeriesStyle::area(color.opacity(0.08), color.opacity(0.35)))
                .scale_max(vu_scale(load_test, data.max_vus))
                .dot(false)
                .format(|value| format_integer(value.round() as u64).into()),
            ChartSeriesKey::Rps => ChartSeries::new(label, color, data.rps.clone())
                .style(SeriesStyle::line(2.25))
                .format(|value| format_integer(value.round().max(0.0) as u64).into()),
            ChartSeriesKey::P50 | ChartSeriesKey::P95 | ChartSeriesKey::P99 => {
                let values = match key {
                    ChartSeriesKey::P50 => data.p50.clone(),
                    ChartSeriesKey::P95 => data.p95.clone(),
                    _ => data.p99.clone(),
                };
                let width = if key == ChartSeriesKey::P95 {
                    1.75
                } else {
                    1.5
                };
                ChartSeries::new(label, color, values)
                    .side(AxisSide::Right)
                    .style(SeriesStyle::line(width))
                    .format(|value| format_latency_ms(value).into())
            }
            ChartSeriesKey::Baseline => {
                let Some(values) = data.baseline.clone() else {
                    continue;
                };
                ChartSeries::new(label, color.opacity(0.6), values)
                    .side(AxisSide::Right)
                    .style(SeriesStyle::dashed(1.5, 4.0, 4.0))
                    .dot(false)
                    .format(|value| format_latency_ms(value).into())
            }
        };
        chart = chart.series(series.tooltip_order(order));
    }
    if !load_test.hidden_series.contains(&ChartSeriesKey::Vus) {
        chart = with_vu_labels(chart, palette, load_test, &data.vus, count);
    }
    for threshold in latency_limits {
        chart = chart.reference_line(ReferenceLine::new(
            AxisSide::Right,
            threshold.limit,
            t!(
                "load_test.chart.goal",
                metric = threshold_metric_label(threshold.metric),
                limit = format!("{} ms", format_trimmed(threshold.limit))
            ),
            palette.danger,
        ));
    }
    if selectable {
        let tab_id = tab_id.to_string();
        chart = chart.on_select(move |selection, _, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.set_load_test_window(&tab_id, selection, cx);
            });
        });
    }

    v_flex()
        .flex_1()
        .border_1()
        .border_color(palette.border)
        .rounded(px(RADIUS_LG))
        .bg(palette.raised)
        .px(px(16.0))
        .pt(px(12.0))
        .pb(px(10.0))
        .gap(px(8.0))
        .child(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(14.0))
                .gap_y(px(4.0))
                .text_size(px(12.0))
                .child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px(13.0))
                        .child(t!("load_test.chart.title")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(120.0))
                        .text_color(palette.fg_subtle)
                        .when(selectable, |hint_div| hint_div.child(hint)),
                )
                .children(legend),
        )
        .child(
            // The chart is laid out of the flow, so the slot's height comes from the room left
            // in the results, never from the chart. The canvas measures that height and draws one
            // more frame only when it changed, for the chart to take it.
            div()
                .relative()
                .flex_1()
                .min_h(px(CHART_HEIGHT))
                .child(
                    canvas(
                        move |bounds, window, _| {
                            let height = f32::from(bounds.size.height);
                            if (room.get() - height).abs() > 0.5 {
                                room.set(height);
                                window.refresh();
                            }
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(div().absolute().top_0().left_0().right_0().child(chart)),
        )
        .into_any_element()
}

/// The scale of the virtual users area, so its peak (the configured count, or the most seen)
/// sits at [`VUS_HEIGHT_FRACTION`] of the plot, a live ramp-up included.
fn vu_scale(load_test: &LoadTestTab, max_vus: f64) -> f64 {
    let peak = load_test.run_vus.map_or(0.0, f64::from).max(max_vus);
    peak.max(1.0) / VUS_HEIGHT_FRACTION
}

/// Adds the virtual users labels to `chart` (the area has no axis of its own, so its labels give
/// its values) and, ahead of a live run, the rest of the configured profile as a dashed outline.
fn with_vu_labels(
    mut chart: TimeSeriesChart,
    palette: &Palette,
    load_test: &LoadTestTab,
    vus: &[f64],
    count: usize,
) -> TimeSeriesChart {
    let running = load_test.is_running();
    let target = load_test.run_vus;
    let max_vus = vus
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0.0, f64::max);
    let scale = vu_scale(load_test, max_vus);
    if running
        && let (Some(target), Some(ramp_up)) = (target, load_test.run_ramp_up_secs)
        && let Some(edge) = vus.len().checked_sub(1)
    {
        let color = palette.series_color(ChartSeriesKey::Vus).opacity(0.45);
        chart = chart.series(
            ChartSeries::new(
                t!("load_test.chart.planned"),
                color,
                planned_vus(count, edge, target, ramp_up),
            )
            .style(SeriesStyle::dashed(1.0, 3.0, 3.0))
            .scale_max(scale)
            .dot(false)
            .format(|value| format_integer(value.round() as u64).into())
            .tooltip_order(i32::MAX),
        );
    }
    let labels = vu_labels(vus, target.filter(|_| running));
    for label in &labels {
        let (text, placement) = match label.kind {
            VuLabelKind::Plateau => (
                format!("{} VUs", format_integer(label.vus)),
                LabelPlacement::BeforeBelow,
            ),
            VuLabelKind::ThinPlateau => (
                format!("{} VUs", format_integer(label.vus)),
                LabelPlacement::BeforeAbove,
            ),
            VuLabelKind::LiveEdge { target } => (
                format!(
                    "{} \u{2192} {} VUs",
                    format_integer(label.vus),
                    format_integer(target)
                ),
                LabelPlacement::AfterAbove,
            ),
        };
        chart = chart.label(
            ChartLabel::new(label.index, f64::from(label.vus), text)
                .scale_max(scale)
                .placement(placement),
        );
    }
    // A quiet caption at the foot of the ramp, once the run has one to show.
    if !running
        && let Some(ramp_up) = load_test.run_ramp_up_secs.filter(|secs| *secs > 0)
        && !labels.is_empty()
    {
        chart = chart.label(
            ChartLabel::new(0, 0.0, t!("load_test.chart.ramp_up", secs = ramp_up))
                .placement(LabelPlacement::AfterOnBase)
                .boxed(false),
        );
    }
    chart
}

/// The color of one status' bar and label: the HTTP status range colors, `danger` for a timeout
/// or a failure.
pub(crate) fn status_key_color(palette: &Palette, status: &StatusKey) -> Hsla {
    match status {
        StatusKey::Code(code) => palette.status_colors(Some(*code)).0,
        StatusKey::Timeout | StatusKey::Failed => palette.danger,
    }
}

/// The label of one status: the code, or the translated "Timeout" or "Failed".
pub(crate) fn status_key_label(status: &StatusKey) -> String {
    match status {
        StatusKey::Code(code) => code.to_string(),
        StatusKey::Timeout => t!("load_test.status_key.timeout").into_owned(),
        StatusKey::Failed => t!("load_test.status_key.failed").into_owned(),
    }
}

/// Each target's HTTP method and display name, by target index: a generic "Target N" for a run
/// saved before the labels were kept.
pub(crate) fn target_row_label(load_test: &LoadTestTab, index: usize) -> (Option<Method>, String) {
    match load_test.target_rows.get(index) {
        Some((method, name)) => (Some(method.clone()), name.clone()),
        None => (
            None,
            t!("load_test.table.target_n", number = index + 1).into_owned(),
        ),
    }
}

/// The color of a status' segment in a status bar: its range color, and a darker red for a
/// timeout or a failure so it stands apart from a 5xx next to it.
fn status_segment_color(palette: &Palette, status: &StatusKey) -> Hsla {
    match status {
        StatusKey::Code(_) => status_key_color(palette, status),
        StatusKey::Timeout | StatusKey::Failed => palette.danger.opacity(FAILURE_OPACITY),
    }
}

/// A status with its reason phrase, as the status bar's tooltip lists it: `200 OK`, `418`,
/// `Timeout`.
fn status_with_phrase(status: &StatusKey) -> String {
    match status {
        StatusKey::Code(code) => match reason_phrase(*code) {
            Some(phrase) => format!("{code} {phrase}"),
            None => code.to_string(),
        },
        _ => status_key_label(status),
    }
}

/// A share of `total` as a percentage, with two decimals below 1 % so a rare status does not
/// read as zero.
fn share_text(count: u64, total: u64) -> String {
    let share = count as f64 / total.max(1) as f64 * 100.0;
    format!(
        "{} %",
        format_decimal(share, if share < 1.0 { 2 } else { 1 })
    )
}

/// The status bar of one row of the per request table: a segment per status range (2xx, 3xx,
/// 4xx, 5xx, then timeouts and failures), with the exact codes, counts and shares in a tooltip.
fn status_bar(
    id: SharedString,
    title: String,
    statuses: &[(StatusKey, u64)],
    palette: &Palette,
    mono_font: &SharedString,
) -> AnyElement {
    let total: u64 = statuses.iter().map(|(_, count)| count).sum();
    let mut segments: Vec<(Hsla, u64)> = Vec::new();
    for (status, count) in statuses {
        let color = status_segment_color(palette, status);
        match segments.iter_mut().find(|(existing, _)| *existing == color) {
            Some((_, sum)) => *sum += count,
            None => segments.push((color, *count)),
        }
    }
    let mut by_count = statuses.to_vec();
    by_count.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let tooltip_palette = palette.clone();
    let tooltip_font = mono_font.clone();
    let tooltip_title = t!(
        "load_test.status_bar.title",
        name = title,
        responses = locale::plural("load_test.status_bar.responses", total as usize)
    )
    .into_owned();

    div()
        .id(id)
        .w_full()
        .h(px(STATUS_BAR_HEIGHT))
        .flex()
        .gap(px(1.0))
        .rounded(px(STATUS_BAR_HEIGHT / 2.0))
        .overflow_hidden()
        .bg(palette.surface)
        .children(segments.into_iter().map(|(color, count)| {
            div()
                .h_full()
                .flex_grow(count as f32)
                .flex_basis(px(0.0))
                .bg(color)
        }))
        .tooltip(move |window, cx| {
            let palette = tooltip_palette.clone();
            let mono_font = tooltip_font.clone();
            let title = tooltip_title.clone();
            let rows = by_count.clone();
            Tooltip::element(move |_, _| {
                v_flex()
                    .w(px(STATUS_TOOLTIP_WIDTH))
                    .gap(px(5.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(palette.fg)
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(palette.fg_muted)
                            .child(title.clone()),
                    )
                    .children(rows.iter().map(|(status, count)| {
                        h_flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(8.0))
                                    .rounded(px(2.0))
                                    .bg(status_segment_color(&palette, status)),
                            )
                            .child(div().flex_1().truncate().child(status_with_phrase(status)))
                            .child(
                                div()
                                    .font_family(mono_font.clone())
                                    .child(format_integer(*count)),
                            )
                            .child(
                                div()
                                    .w(px(52.0))
                                    .text_right()
                                    .font_family(mono_font.clone())
                                    .text_color(palette.fg_muted)
                                    .child(share_text(*count, total)),
                            )
                    }))
            })
            .bg(tooltip_palette.overlay)
            .border_color(tooltip_palette.border_strong)
            .rounded(px(RADIUS_MD))
            .px(px(10.0))
            .build(window, cx)
        })
        .into_any_element()
}

/// The per request table: request, Total, p50, p95, p99, Errors and the responses by status, then
/// a totals row for all requests.
fn render_per_request(
    palette: &Palette,
    ui_font: &SharedString,
    mono_font: &SharedString,
    load_test: &LoadTestTab,
    summary: &SummaryData,
) -> AnyElement {
    let ms = |us: u32| format_latency_ms(f64::from(us) / 1_000.0);
    let errors_cell = |error_rate: f64| {
        let cell = DataCell::text(format!("{} %", format_decimal(error_rate * 100.0, 1)));
        if error_rate > PER_REQUEST_WARNING_ERROR_RATE {
            cell.color(palette.warning)
        } else {
            cell
        }
    };
    let has_statuses = !summary.target_statuses.is_empty();
    let mut table = DataTable::new("load-test-per-request-table")
        .column(DataColumn::new(t!("load_test.table.per_request")))
        .column(DataColumn::numeric(t!("load_test.kpi.total")).width(NUMERIC_COLUMN_WIDTH))
        .column(DataColumn::numeric("p50").width(NUMERIC_COLUMN_WIDTH))
        .column(DataColumn::numeric("p95").width(NUMERIC_COLUMN_WIDTH))
        .column(DataColumn::numeric("p99").width(NUMERIC_COLUMN_WIDTH))
        .column(DataColumn::numeric(t!("load_test.kpi.errors")).width(NUMERIC_COLUMN_WIDTH));
    // The status bars share the leftover width with the names.
    if has_statuses {
        table = table.column(DataColumn::new(t!("load_test.requests.status")));
    }
    for (index, stats) in summary.per_target.iter().enumerate() {
        let (method, name) = target_row_label(load_test, index);
        let mut cells = vec![
            DataCell::element(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap(px(8.0))
                    .children(method.map(MethodBadge::label))
                    .child(div().min_w_0().truncate().child(name.clone())),
            ),
            DataCell::text(format_integer(stats.count)),
            DataCell::text(ms(stats.p50)),
            DataCell::text(ms(stats.p95)),
            DataCell::text(ms(stats.p99)),
            errors_cell(stats.error_rate),
        ];
        if let Some(statuses) = summary.target_statuses.get(index) {
            cells.push(DataCell::element(status_bar(
                format!("load-test-status-bar-{index}").into(),
                name,
                statuses,
                palette,
                mono_font,
            )));
        }
        table = table.row(cells);
    }
    let all_requests = t!("load_test.table.all_requests").into_owned();
    let mut total = vec![
        DataCell::element(
            div()
                .min_w_0()
                .truncate()
                .font_family(ui_font.clone())
                .child(all_requests.clone()),
        ),
        DataCell::text(format_integer(summary.total)),
        DataCell::text(ms(summary.p50)),
        DataCell::text(ms(summary.p95)),
        DataCell::text(ms(summary.p99)),
        errors_cell(summary.error_rate),
    ];
    if has_statuses {
        total.push(DataCell::element(status_bar(
            "load-test-status-bar-all".into(),
            all_requests,
            &summary.statuses,
            palette,
            mono_font,
        )));
    }
    table = table.total_row(total);
    // The table's own `mono` would use telaio's monospace font; the user's choice is inherited
    // from here instead, like the rest of the dashboard.
    div()
        .font_family(mono_font.clone())
        .child(table)
        .into_any_element()
}
