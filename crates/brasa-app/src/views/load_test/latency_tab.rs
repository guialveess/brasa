//! The Latency tab of the load test results: the request selector, the histogram scale and
//! "Include errors", then the latency distribution (stacked by request, with each peak labeled,
//! or one request with its p50, p95 and p99), the percentiles of each request and the latency by
//! percentile curve. The data comes from `brasa_load::latency_view`, computed in the background
//! (`results_data.rs`); nothing here costs in proportion to the run.

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_load::latency::{CURVE_POINTS, curve_percentile};
use brasa_load::{LatencyPercentiles, LatencyScale, LatencyView, StackedHistogram};

use super::requests_tab::{FILTER_HEIGHT, filter_checkbox};
use super::summary::target_row_label;
use crate::state::load_results::{ChartSeriesKey, format_latency_ms, plain_axis};
use crate::state::load_test::{DeltaTone, LoadTestTab};
use crate::state::locale;
use crate::state::number::{format_decimal, format_integer};
use crate::theme::metrics::{CHART_TOOLTIP_OFFSET, CHART_TOOLTIP_TOP, RADIUS_LG, RADIUS_MD};
use crate::theme::{Palette, PaletteExt, BrasaColors};
use crate::views::components::{
    ChartAxis, ChartSeries, EmptyState, NeutralButtonExt, SegmentedControl, SegmentedItem,
    SeriesStyle, TimeSeriesChart,
};
use crate::views::root::AppView;

/// Height of the histogram's plot.
const HISTOGRAM_HEIGHT: f32 = 220.0;
/// Width of the strip left of the histogram, for its y labels.
const HISTOGRAM_GUTTER: f32 = 48.0;
/// Height of the row above the histogram's plot for its axis title.
const AXIS_TITLE_HEIGHT: f32 = 22.0;
/// Height of the latency by percentile chart.
const CURVE_HEIGHT: f32 = 220.0;
/// Width of the request selector's menu.
const REQUEST_MENU_WIDTH: f32 = 260.0;
/// Width of the histogram's hover tooltip.
const HISTOGRAM_TOOLTIP_WIDTH: f32 = 246.0;
/// Width of a peak label, centered on its bar.
const PEAK_LABEL_WIDTH: f32 = 184.0;
/// A peak this close to an edge of the plot (as a fraction of it) starts or ends its label at
/// the bar instead of centering it.
const PEAK_EDGE_FRACTION: f32 = 0.3;
/// Two peak labels closer than this fraction of the plot go on separate rows: a label is about
/// a third of a typical plot wide.
const PEAK_LABEL_SPACING: f32 = 0.34;
/// Height of one row of peak or percentile marker labels.
const MARK_ROW_HEIGHT: f32 = 16.0;
/// A percentile marker past this fraction of the plot puts its label on its left.
const MARK_FLIP_FRACTION: f32 = 0.85;
/// Base widths of the percentiles table and of the curve: side by side when both fit, the curve
/// below the table otherwise.
const TABLE_MIN_WIDTH: f32 = 520.0;
const CURVE_MIN_WIDTH: f32 = 340.0;
/// Width of the percentiles table's numeric columns.
const PERCENTILE_COLUMN_WIDTH: f32 = 54.0;
/// Height of a percentiles table row with the change against the compared run under each value.
const PERCENTILE_COMPARED_ROW_HEIGHT: f32 = 44.0;
/// Height of a percentiles table row.
const PERCENTILE_ROW_HEIGHT: f32 = 34.0;
/// Opacity of a request's curve while another request is focused.
const DIMMED_OPACITY: f32 = 0.25;
/// Opacity of the selected row's accent tint.
const SELECTED_ROW_OPACITY: f32 = 0.14;

impl AppView {
    /// Renders the Latency tab of the load test tab `tab_id`.
    pub(crate) fn render_load_test_latency(
        &mut self,
        tab_id: &str,
        load_test: &LoadTestTab,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if load_test.log_unavailable {
            return EmptyState::new(t!("load_test.requests.unavailable"))
                .compact(true)
                .into_any_element();
        }
        let Some(data) = load_test.current_latency() else {
            return EmptyState::new(t!("load_test.latency.loading"))
                .compact(true)
                .into_any_element();
        };
        let view = &data.view;
        let compare = data
            .compare
            .as_ref()
            .zip(load_test.compare_with.map(super::dashboard::run_title));
        let compare = compare.as_ref().map(|(compare, title)| Compared {
            view: compare,
            title,
        });
        let palette = cx.palette();
        let ui_font = cx.theme().font_family.clone();
        let mono_font = cx.theme().mono_font_family.clone();
        let weak = cx.weak_entity();
        let names: Vec<String> = (0..view.per_target.len())
            .map(|index| target_row_label(load_test, index).1)
            .collect();

        v_flex()
            .gap(px(14.0))
            .child(render_toolbar(
                weak.clone(),
                tab_id,
                &palette,
                load_test,
                view,
                &names,
                cx,
            ))
            .child(render_histogram(
                weak.clone(),
                self.latency_hover,
                &palette,
                &mono_font,
                load_test,
                view,
                compare,
                &names,
            ))
            .child(
                h_flex()
                    .min_w_0()
                    .flex_wrap()
                    .items_start()
                    .gap(px(14.0))
                    .child(
                        div()
                            .flex_grow(1.15)
                            .flex_basis(px(TABLE_MIN_WIDTH))
                            .min_w_0()
                            .child(render_percentiles(
                                weak, tab_id, &palette, &ui_font, &mono_font, load_test, view,
                                compare, &names,
                            )),
                    )
                    .child(
                        div()
                            .flex_grow(1.0)
                            .flex_basis(px(CURVE_MIN_WIDTH))
                            .min_w_0()
                            .child(render_curve(&palette, load_test, view, compare, &names)),
                    ),
            )
            .into_any_element()
    }
}

/// The run compared with, and its title (`Run #2`).
#[derive(Clone, Copy)]
struct Compared<'a> {
    view: &'a LatencyView,
    title: &'a str,
}

/// The text of a latency in microseconds.
fn latency_text(us: u32) -> String {
    format_latency_ms(f64::from(us) / 1_000.0)
}

/// A request's color swatch.
fn swatch(color: Hsla) -> Div {
    div().flex_none().size(px(8.0)).rounded(px(2.0)).bg(color)
}

/// A bordered card like the Summary's chart card.
fn panel(palette: &Palette) -> Div {
    v_flex()
        .border_1()
        .border_color(palette.border)
        .rounded(px(RADIUS_LG))
        .bg(palette.raised)
        .px(px(16.0))
        .pt(px(12.0))
        .pb(px(10.0))
        .gap(px(8.0))
}

/// A card's title row: the title, a hint, then `extra` on the right.
fn panel_title(title: String, hint: String, palette: &Palette) -> Div {
    h_flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(12.0))
        .gap_y(px(4.0))
        .text_size(px(12.0))
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::MEDIUM)
                .text_size(px(13.0))
                .child(title),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(120.0))
                .truncate()
                .text_color(palette.fg_subtle)
                .child(hint),
        )
}

/// The toolbar: the request selector, Linear or Log, "Include errors", a live badge during a
/// run, and on the right how many responses the histogram counts.
fn render_toolbar(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    view: &LatencyView,
    names: &[String],
    cx: &App,
) -> AnyElement {
    let choice = load_test.latency_options;
    let selected_color = choice
        .target
        .map_or(palette.fg_subtle, |target| palette.request_color(target));
    let selected_label = choice.target.map_or_else(
        || t!("load_test.table.all_requests").into_owned(),
        |target| names.get(target).cloned().unwrap_or_default(),
    );

    let menu_tab_id = tab_id.to_string();
    let menu_weak = weak.clone();
    let options: Vec<(Option<usize>, String, u64)> = std::iter::once((
        None,
        t!("load_test.table.all_requests").into_owned(),
        view.all.count,
    ))
    .chain(
        view.per_target
            .iter()
            .enumerate()
            .map(|(index, stats)| (Some(index), names[index].clone(), stats.count)),
    )
    .collect();
    let menu_palette = palette.clone();
    let selector = Button::new("load-test-latency-request")
        .neutral_bordered(cx, palette.raised, palette.border_strong, palette.fg)
        .border_1()
        .h(px(FILTER_HEIGHT))
        .px(px(10.0))
        .rounded(px(7.0))
        .text_size(px(12.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_color(palette.fg_muted)
                        .child(t!("load_test.latency.request")),
                )
                .child(swatch(selected_color))
                .child(selected_label)
                .child(
                    Icon::new(IconName::ChevronDown)
                        .small()
                        .text_color(palette.fg_muted),
                ),
        )
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu
                .min_w(px(REQUEST_MENU_WIDTH))
                .scrollable(true)
                .max_h(px(320.0));
            for (target, label, count) in &options {
                let target = *target;
                let item_weak = menu_weak.clone();
                let tab_id = menu_tab_id.clone();
                let color = target.map_or(menu_palette.fg_subtle, |index| {
                    menu_palette.request_color(index)
                });
                let label = label.clone();
                let count = format_integer(*count);
                let subtle = menu_palette.fg_subtle;
                menu = menu.item(
                    PopupMenuItem::element(move |_, _| {
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap(px(8.0))
                            .child(swatch(color))
                            .child(div().flex_1().min_w_0().truncate().child(label.clone()))
                            .child(div().text_color(subtle).child(count.clone()))
                    })
                    .checked(choice.target == target)
                    .on_click(move |_, _, cx| {
                        let _ = item_weak.update(cx, |view, cx| {
                            view.edit_load_test_latency(&tab_id, cx, |choice| {
                                choice.target = target;
                            });
                        });
                    }),
                );
            }
            menu
        });

    let mut scale = SegmentedControl::new("load-test-latency-scale");
    for (option, label) in [
        (LatencyScale::Linear, t!("load_test.latency.linear")),
        (LatencyScale::Log, t!("load_test.latency.log")),
    ] {
        let item_weak = weak.clone();
        let tab_id = tab_id.to_string();
        scale = scale.item(
            SegmentedItem::new(label.into_owned())
                .selected(choice.scale == option)
                .on_click(move |_, cx| {
                    let _ = item_weak.update(cx, |view, cx| {
                        view.edit_load_test_latency(&tab_id, cx, |choice| choice.scale = option);
                    });
                }),
        );
    }

    let errors_tab_id = tab_id.to_string();
    let include_errors = filter_checkbox(
        "load-test-latency-errors",
        palette,
        t!("load_test.latency.include_errors").into_owned(),
        choice.include_errors,
    )
    .on_click(move |checked, _, cx| {
        let checked = *checked;
        let _ = weak.update(cx, |view, cx| {
            view.edit_load_test_latency(&errors_tab_id, cx, |choice| {
                choice.include_errors = checked;
            });
        });
    });

    let live = load_test.is_running().then(|| {
        h_flex()
            .items_center()
            .gap(px(6.0))
            .h(px(24.0))
            .px(px(9.0))
            .rounded(px(4.0))
            .bg(palette.info.opacity(0.14))
            .text_color(palette.info)
            .text_size(px(12.0))
            .child(div().size(px(6.0)).rounded_full().bg(palette.info))
            .child(t!("load_test.latency.live"))
    });

    let mut summary = locale::plural("load_test.status_bar.responses", view.shown as usize);
    if view.excluded > 0 {
        summary = format!(
            "{summary} \u{b7} {}",
            locale::plural("load_test.latency.excluded", view.excluded as usize)
        );
    }

    h_flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .child(selector)
        .child(scale)
        .child(include_errors)
        .children(live)
        .child(div().flex_1())
        .child(
            div()
                .text_size(px(12.0))
                .text_color(palette.fg_muted)
                .child(summary),
        )
        .into_any_element()
}

/// The most x axis labels of the histogram.
const MAX_X_LABELS: usize = 7;
/// Width of one x axis label of the histogram, centered on its tick.
const X_LABEL_WIDTH: f32 = 60.0;

/// A tick of the histogram's latency axis, in microseconds, as text: whole milliseconds, or one
/// or two decimals for a tick that needs them (`450.5 ms`), seconds from one second up.
fn tick_text(us: f64) -> String {
    if us <= 0.0 {
        return "0".to_string();
    }
    let ms = us / 1_000.0;
    if ms >= 1_000.0 {
        let seconds = ms / 1_000.0;
        let decimals = if (seconds * 10.0).fract().abs() > 1e-6 {
            2
        } else {
            1
        };
        let decimals = if seconds.fract().abs() < 1e-6 {
            0
        } else {
            decimals
        };
        return format!("{} s", format_decimal(seconds, decimals));
    }
    let decimals = (0..2)
        .find(|decimals| {
            let scaled = ms * 10f64.powi(*decimals);
            (scaled - scaled.round()).abs() < 1e-6
        })
        .unwrap_or(2) as usize;
    format!("{} ms", format_decimal(ms, decimals))
}

/// The histogram's x axis ticks, in microseconds: round values inside the axis, at most
/// [`MAX_X_LABELS`]. A linear axis steps by a round multiple of its bin width; a log one marks
/// the 1, 2, 3 and 5 of each power of ten that it spans, or the decades when they are too many.
fn histogram_ticks(histogram: &StackedHistogram) -> Vec<f64> {
    let (Some(&low), Some(&high)) = (histogram.edges_us.first(), histogram.edges_us.last()) else {
        return Vec::new();
    };
    match histogram.scale {
        LatencyScale::Linear => {
            let step = f64::from(histogram.step_us.max(1));
            let tick = [
                1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1_000.0,
            ]
            .into_iter()
            .map(|factor| factor * step)
            .find(|tick| (((high - low) / tick).floor() as usize) < MAX_X_LABELS)
            .unwrap_or(high - low);
            let first = (low / tick).ceil() * tick;
            (0..)
                .map(|index| first + f64::from(index) * tick)
                .take_while(|value| *value <= high + 1e-6)
                .collect()
        }
        LatencyScale::Log => {
            let in_range = |multipliers: &[f64]| -> Vec<f64> {
                let first = low.log10().floor() as i32;
                let last = high.log10().ceil() as i32;
                (first..=last)
                    .flat_map(|power| {
                        multipliers
                            .iter()
                            .map(move |multiplier| multiplier * 10f64.powi(power))
                    })
                    .filter(|value| *value >= low && *value <= high)
                    .collect()
            };
            let ticks = in_range(&[1.0, 2.0, 3.0, 5.0]);
            if ticks.len() <= MAX_X_LABELS {
                ticks
            } else {
                in_range(&[1.0])
            }
        }
    }
}

/// The latency distribution card: the bars stacked by request, the peak labels (all requests)
/// or the p50, p95 and p99 markers (one request), and the hovered bin's tooltip.
#[allow(clippy::too_many_arguments)]
fn render_histogram(
    weak: WeakEntity<AppView>,
    hover: Option<usize>,
    palette: &Palette,
    mono_font: &SharedString,
    load_test: &LoadTestTab,
    view: &LatencyView,
    compare: Option<Compared>,
    names: &[String],
) -> AnyElement {
    let histogram = &view.histogram;
    let choice = load_test.latency_options;
    let shown: Vec<usize> = match choice.target {
        Some(target) => vec![target],
        None => (0..view.per_target.len()).collect(),
    };
    let hint = match histogram.scale {
        LatencyScale::Linear => t!(
            "load_test.latency.bin_linear",
            width = tick_text(f64::from(histogram.step_us))
        )
        .into_owned(),
        LatencyScale::Log => t!("load_test.latency.bin_log").into_owned(),
    };
    let legend = shown.iter().map(|target| {
        h_flex()
            .items_center()
            .gap(px(6.0))
            .text_color(palette.fg_muted)
            .child(swatch(palette.request_color(*target)))
            .child(
                div()
                    .font_family(mono_font.clone())
                    .child(names.get(*target).cloned().unwrap_or_default()),
            )
    });
    let compare_legend = compare.map(|compare| {
        h_flex()
            .items_center()
            .gap(px(6.0))
            .text_color(palette.fg_muted)
            .child(
                div()
                    .flex_none()
                    .size(px(10.0))
                    .rounded(px(2.0))
                    .border_1()
                    .border_dashed()
                    .border_color(palette.fg_muted),
            )
            .child(t!("load_test.latency.same_scale", run = compare.title))
    });
    let title = panel_title(
        t!("load_test.distribution.title").into_owned(),
        hint,
        palette,
    )
    .children(legend)
    .children(compare_legend);

    let bins = histogram.counts.len();
    if bins == 0 {
        return panel(palette)
            .child(title)
            .child(
                div()
                    .h(px(HISTOGRAM_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette.fg_muted)
                    .child(t!("load_test.latency.no_responses")),
            )
            .into_any_element();
    }
    // The compared run, scaled to this run's number of responses so the shapes compare.
    let compare_counts: Option<Vec<f64>> = compare
        .filter(|compare| compare.view.histogram.counts.len() == bins && compare.view.shown > 0)
        .map(|compare| {
            let scale = view.shown as f64 / compare.view.shown as f64;
            (0..bins)
                .map(|bin| compare.view.histogram.total(bin) as f64 * scale)
                .collect()
        });
    let peak = (0..bins)
        .map(|bin| histogram.total(bin) as f64)
        .chain(compare_counts.iter().flatten().copied())
        .fold(0.0, f64::max);
    // Room above the tallest bar for the peak labels.
    let headroom = if choice.target.is_none() { 1.3 } else { 1.0 };
    let axis = plain_axis(peak * headroom);
    let y_max = axis.max.max(1.0);

    // One column per bin, the requests stacked bottom up in target order.
    let columns: Vec<Stateful<Div>> = (0..bins)
        .map(|bin| {
            let counts = &histogram.counts[bin];
            let weak = weak.clone();
            let segments = shown.iter().rev().filter_map(|target| {
                let count = counts.get(*target).copied().unwrap_or(0);
                (count > 0).then(|| {
                    div()
                        .w_full()
                        .h(relative((count as f64 / y_max) as f32))
                        .bg(palette.request_color(*target))
                })
            });
            div()
                .id(("load-test-latency-bin", bin))
                .flex_1()
                .h_full()
                .px(px(if bins > 48 { 0.0 } else { 0.5 }))
                .flex()
                .flex_col()
                .justify_end()
                .when(hover == Some(bin), |column| column.bg(palette.hover))
                .on_mouse_move(move |_, _, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        if view.latency_hover != Some(bin) {
                            view.latency_hover = Some(bin);
                            cx.notify();
                        }
                    });
                })
                .children(segments)
        })
        .collect();

    // Labels: each request's peak over all requests, or one request's percentiles.
    let mut marks: Vec<AnyElement> = Vec::new();
    if choice.target.is_none() {
        let mut peaks: Vec<(f32, usize)> = shown
            .iter()
            .filter_map(|target| {
                let (bin, count) = (0..bins)
                    .map(|bin| (bin, histogram.counts[bin][*target]))
                    .max_by_key(|(_, count)| *count)?;
                (count > 0).then_some(((bin as f32 + 0.5) / bins as f32, *target))
            })
            .collect();
        peaks.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Each label takes the first row whose last label is far enough to its left.
        let mut row_ends: Vec<f32> = Vec::new();
        for (fraction, target) in peaks {
            let row = match row_ends
                .iter()
                .position(|end| fraction - end >= PEAK_LABEL_SPACING)
            {
                Some(row) => {
                    row_ends[row] = fraction;
                    row
                }
                None => {
                    row_ends.push(fraction);
                    row_ends.len() - 1
                }
            };
            let label = format!(
                "{} \u{b7} p50 {}",
                names.get(target).cloned().unwrap_or_default(),
                latency_text(view.per_target[target].p50)
            );
            // Centered on its bar in the middle of the plot, and starting or ending at it near an
            // edge, so it never sticks out of the card.
            let anchored = div()
                .absolute()
                .top(px(2.0 + row as f32 * MARK_ROW_HEIGHT * 1.25))
                .w(px(PEAK_LABEL_WIDTH))
                .flex();
            let anchored = if fraction < PEAK_EDGE_FRACTION {
                anchored.left(relative(fraction)).ml(px(-8.0))
            } else if fraction > 1.0 - PEAK_EDGE_FRACTION {
                anchored
                    .right(relative(1.0 - fraction))
                    .mr(px(-8.0))
                    .justify_end()
            } else {
                anchored
                    .left(relative(fraction))
                    .ml(px(-PEAK_LABEL_WIDTH / 2.0))
                    .justify_center()
            };
            marks.push(
                anchored
                    .child(
                        div()
                            .px(px(4.0))
                            .rounded(px(3.0))
                            .bg(palette.raised)
                            .truncate()
                            .font_family(mono_font.clone())
                            .text_size(px(11.0))
                            .text_color(palette.request_color(target))
                            .child(label),
                    )
                    .into_any_element(),
            );
        }
    } else if let Some(stats) = choice.target.and_then(|target| view.per_target.get(target)) {
        let percentiles = [
            ("p50", stats.p50, palette.series_color(ChartSeriesKey::P50)),
            ("p95", stats.p95, palette.series_color(ChartSeriesKey::P95)),
            ("p99", stats.p99, palette.series_color(ChartSeriesKey::P99)),
        ];
        for (row, (name, us, color)) in percentiles.into_iter().enumerate() {
            let fraction = histogram.fraction(f64::from(us)) as f32;
            marks.push(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .left(relative(fraction))
                    .w(px(1.0))
                    .bg(color.opacity(0.8))
                    .into_any_element(),
            );
            let label = div()
                .absolute()
                .top(px(2.0 + row as f32 * MARK_ROW_HEIGHT))
                .px(px(3.0))
                .bg(palette.raised)
                .font_family(mono_font.clone())
                .text_size(px(10.5))
                .text_color(color)
                .whitespace_nowrap()
                .child(format!("{name} {}", latency_text(us)));
            let label = if fraction > MARK_FLIP_FRACTION {
                label.right(relative(1.0 - fraction)).mr(px(4.0))
            } else {
                label.left(relative(fraction)).ml(px(4.0))
            };
            marks.push(label.into_any_element());
        }
    }

    // The hovered bin's tooltip.
    let total = view.shown.max(1);
    let tooltip = hover.filter(|bin| *bin < bins).map(|bin| {
        let fraction = (bin as f32 + 0.5) / bins as f32;
        let counts = &histogram.counts[bin];
        let rows: Vec<AnyElement> = shown
            .iter()
            .filter(|target| counts.get(**target).copied().unwrap_or(0) > 0)
            .map(|target| {
                let count = counts[*target];
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(swatch(palette.request_color(*target)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(mono_font.clone())
                            .text_color(palette.fg_muted)
                            .child(names.get(*target).cloned().unwrap_or_default()),
                    )
                    .child(
                        div()
                            .font_family(mono_font.clone())
                            .child(format_integer(count)),
                    )
                    .child(
                        div()
                            .w(px(52.0))
                            .text_right()
                            .font_family(mono_font.clone())
                            .text_color(palette.fg_subtle)
                            .child(share_text(count, total)),
                    )
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty().then(|| {
            div()
                .text_color(palette.fg_muted)
                .child(t!("load_test.latency.no_responses"))
        });
        let range = format!(
            "{}\u{2013}{}",
            tick_text(histogram.edges_us[bin]),
            tick_text(histogram.edges_us[bin + 1])
        );
        let title = t!(
            "load_test.latency.bin_tooltip",
            range = range,
            count = format_integer(histogram.total(bin)),
            total = format_integer(total)
        )
        .into_owned();
        v_flex()
            .absolute()
            .top(px(CHART_TOOLTIP_TOP))
            .w(px(HISTOGRAM_TOOLTIP_WIDTH))
            .gap(px(4.0))
            .py(px(8.0))
            .px(px(10.0))
            .rounded(px(RADIUS_MD))
            .border_1()
            .border_color(palette.border_strong)
            .bg(palette.overlay)
            .shadow(palette.shadow.clone())
            .text_size(px(12.0))
            .text_color(palette.fg)
            .map(|tooltip| {
                // Right of the bin on the left half, left of it on the right half.
                if fraction <= 0.5 {
                    tooltip
                        .left(relative(fraction))
                        .ml(px(CHART_TOOLTIP_OFFSET))
                } else {
                    tooltip
                        .right(relative(1.0 - fraction))
                        .mr(px(CHART_TOOLTIP_OFFSET))
                }
            })
            .child(
                div()
                    .font_family(mono_font.clone())
                    .text_size(px(11.0))
                    .text_color(palette.fg_muted)
                    .pb(px(2.0))
                    .child(title),
            )
            .children(rows)
            .children(empty)
            .children(compare.map(|compare| {
                let count = compare
                    .view
                    .histogram
                    .counts
                    .get(bin)
                    .map_or(0, |counts| counts.iter().sum::<u64>());
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(8.0))
                            .rounded(px(2.0))
                            .border_1()
                            .border_dashed()
                            .border_color(palette.fg_muted),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(palette.fg_muted)
                            .child(compare.title.to_string()),
                    )
                    .child(
                        div()
                            .w(px(52.0))
                            .text_right()
                            .font_family(mono_font.clone())
                            .text_color(palette.fg_subtle)
                            .child(share_text(count, compare.view.shown)),
                    )
            }))
    });

    let ticks = axis.labels.len();
    let grid = (1..ticks).map(|tick| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(tick as f32 / (ticks - 1).max(1) as f32))
            .h(px(1.0))
            .bg(palette.border)
    });
    // One label taller than the plot, so each label centers on its line.
    let y_labels = axis
        .labels
        .iter()
        .rev()
        .map(|label| div().child(label.clone()));

    panel(palette)
        .child(title)
        .child(
            h_flex()
                .gap(px(0.0))
                .child(
                    v_flex()
                        .w(px(HISTOGRAM_GUTTER))
                        .pr(px(6.0))
                        .h(px(HISTOGRAM_HEIGHT + AXIS_TITLE_HEIGHT))
                        .items_end()
                        .font_family(mono_font.clone())
                        .text_size(px(10.5))
                        .text_color(palette.fg_muted)
                        .child(
                            // Left aligned and free to run past the gutter, like the chart's.
                            div()
                                .w_full()
                                .h(px(AXIS_TITLE_HEIGHT))
                                .whitespace_nowrap()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("load_test.latency.axis")),
                        )
                        .child(
                            v_flex()
                                .h(px(HISTOGRAM_HEIGHT + 14.0))
                                .mt(px(-7.0))
                                .justify_between()
                                .items_end()
                                .children(y_labels),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().h(px(AXIS_TITLE_HEIGHT)))
                        .child(
                            div()
                                .id("load-test-latency-histogram")
                                .relative()
                                .h(px(HISTOGRAM_HEIGHT))
                                .flex()
                                .items_end()
                                .border_b_1()
                                .border_color(palette.border_strong)
                                .cursor_crosshair()
                                .on_hover(move |hovered, _, cx| {
                                    if !*hovered {
                                        let _ = weak.update(cx, |view, cx| {
                                            if view.latency_hover.take().is_some() {
                                                cx.notify();
                                            }
                                        });
                                    }
                                })
                                .children(grid)
                                .children(columns)
                                .children(compare_counts.map(|counts| {
                                    compare_outline(counts, y_max, palette.fg.opacity(0.7))
                                }))
                                .children(marks)
                                .children(tooltip),
                        ),
                ),
        )
        .child(
            div().w_full().pl(px(HISTOGRAM_GUTTER)).child(
                div()
                    .relative()
                    .h(px(14.0))
                    .font_family(mono_font.clone())
                    .text_size(px(10.5))
                    .text_color(palette.fg_subtle)
                    .children(histogram_ticks(histogram).into_iter().map(|us| {
                        div()
                            .absolute()
                            .left(relative(histogram.fraction(us) as f32))
                            .ml(px(-X_LABEL_WIDTH / 2.0))
                            .w(px(X_LABEL_WIDTH))
                            .text_center()
                            .whitespace_nowrap()
                            .child(tick_text(us))
                    })),
            ),
        )
        .into_any_element()
}

/// The compared run's histogram as a dashed step outline over the bars: `counts` per bin, on an
/// axis from 0 to `y_max`.
fn compare_outline(counts: Vec<f64>, y_max: f64, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let width = f32::from(bounds.size.width);
            let height = f32::from(bounds.size.height);
            let bins = counts.len().max(1) as f32;
            let at = |x: f32, y: f32| point(bounds.origin.x + px(x), bounds.origin.y + px(y));
            let mut builder = PathBuilder::stroke(px(1.25)).dash_array(&[px(3.0), px(3.0)]);
            builder.move_to(at(0.0, height));
            for (bin, count) in counts.iter().enumerate() {
                let y = height - (count / y_max).clamp(0.0, 1.0) as f32 * height;
                builder.line_to(at(bin as f32 / bins * width, y));
                builder.line_to(at((bin + 1) as f32 / bins * width, y));
            }
            builder.line_to(at(width, height));
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .size_full()
}

/// A share of `total` as a percentage, with two decimals below 1 %.
fn share_text(count: u64, total: u64) -> String {
    let share = count as f64 / total.max(1) as f64 * 100.0;
    format!(
        "{} %",
        format_decimal(share, if share < 1.0 { 2 } else { 1 })
    )
}

/// The percentiles card: Count, p50, p90, p95, p99 and max of all requests, then of each one.
/// Clicking a row focuses that request.
#[allow(clippy::too_many_arguments)]
fn render_percentiles(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    ui_font: &SharedString,
    mono_font: &SharedString,
    load_test: &LoadTestTab,
    view: &LatencyView,
    compare: Option<Compared>,
    names: &[String],
) -> AnyElement {
    let selected = load_test.latency_options.target;
    let line = |cells: Vec<AnyElement>| {
        let mut row = h_flex().w_full().items_center().gap(px(8.0)).px(px(16.0));
        for (index, cell) in cells.into_iter().enumerate() {
            row = row.child(if index == 0 {
                div().flex_1().min_w_0().child(cell)
            } else {
                div()
                    .flex_none()
                    .w(px(PERCENTILE_COLUMN_WIDTH))
                    .text_right()
                    .child(cell)
            });
        }
        row
    };
    let header = line(
        [
            t!("load_test.latency.request").into_owned(),
            t!("load_test.table.count").into_owned(),
            "p50".to_string(),
            "p90".to_string(),
            "p95".to_string(),
            "p99".to_string(),
            "max".to_string(),
        ]
        .into_iter()
        .map(|label| div().truncate().child(label).into_any_element())
        .collect(),
    )
    .h(px(30.0))
    .border_t_1()
    .border_b_1()
    .border_color(palette.border)
    .text_size(px(11.5))
    .font_weight(FontWeight::MEDIUM)
    .text_color(palette.fg_subtle);

    let row = |target: Option<usize>,
               stats: &LatencyPercentiles,
               before: Option<&LatencyPercentiles>,
               first: bool| {
        let (color, name, font) = match target {
            None => (
                palette.fg_subtle,
                t!("load_test.table.all_requests").into_owned(),
                ui_font.clone(),
            ),
            Some(index) => (
                palette.request_color(index),
                names.get(index).cloned().unwrap_or_default(),
                mono_font.clone(),
            ),
        };
        let mut cells = vec![
            h_flex()
                .items_center()
                .gap(px(8.0))
                .child(swatch(color))
                .child(div().min_w_0().truncate().font_family(font).child(name))
                .into_any_element(),
            div().child(format_integer(stats.count)).into_any_element(),
        ];
        let values = [stats.p50, stats.p90, stats.p95, stats.p99, stats.max];
        let previous =
            before.map(|before| [before.p50, before.p90, before.p95, before.p99, before.max]);
        cells.extend(values.into_iter().enumerate().map(|(index, us)| {
            let delta = previous.map(|previous| {
                let (text, tone) = latency_delta(us, previous[index]);
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(match tone {
                        DeltaTone::Better => palette.success,
                        DeltaTone::Worse => palette.danger,
                        DeltaTone::Neutral => palette.fg_subtle,
                    })
                    .child(text)
            });
            v_flex()
                .items_end()
                .child(latency_text(us))
                .children(delta)
                .into_any_element()
        }));
        let item_weak = weak.clone();
        let tab_id = tab_id.to_string();
        line(cells)
            .id(SharedString::from(format!(
                "load-test-latency-row-{}",
                target.map_or(-1, |index| index as i64)
            )))
            .h(px(if before.is_some() {
                PERCENTILE_COMPARED_ROW_HEIGHT
            } else {
                PERCENTILE_ROW_HEIGHT
            }))
            .when(!first, |row| row.border_t_1().border_color(palette.border))
            .when(target.is_none(), |row| {
                row.font_weight(FontWeight::SEMIBOLD)
            })
            .when(selected == target, |row| {
                row.bg(palette.accent.opacity(SELECTED_ROW_OPACITY))
            })
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .on_click(move |_, _, cx| {
                let _ = item_weak.update(cx, |view, cx| {
                    view.edit_load_test_latency(&tab_id, cx, |choice| choice.target = target);
                });
            })
    };

    let before_all = compare.map(|compare| &compare.view.all);
    let mut rows = vec![row(None, &view.all, before_all, true)];
    for (index, stats) in view.per_target.iter().enumerate() {
        let before = compare.and_then(|compare| compare.view.per_target.get(index));
        rows.push(row(Some(index), stats, before, false));
    }
    let hint = match compare {
        Some(compare) => t!("load_test.latency.delta_hint", run = compare.title).into_owned(),
        None => t!("load_test.latency.click_hint").into_owned(),
    };

    v_flex()
        .border_1()
        .border_color(palette.border)
        .rounded(px(RADIUS_LG))
        .bg(palette.raised)
        .overflow_hidden()
        .font_family(mono_font.clone())
        .text_size(px(12.0))
        .child(
            panel_title(
                t!("load_test.latency.percentiles").into_owned(),
                hint,
                palette,
            )
            .font_family(ui_font.clone())
            .h(px(40.0))
            .px(px(16.0)),
        )
        .child(header)
        .children(rows)
        .into_any_element()
}

/// The change from `before` to `now` (microseconds) as the percentiles table writes it under a
/// value, `+12 ms` or `\u{2212}0.3 s`, and whether it is better: lower is better, and less than a
/// millisecond either way is no change.
fn latency_delta(now: u32, before: u32) -> (String, DeltaTone) {
    let change_ms = (f64::from(now) - f64::from(before)) / 1_000.0;
    if change_ms.abs() < 1.0 {
        return ("\u{b1}0".to_string(), DeltaTone::Neutral);
    }
    let sign = if change_ms > 0.0 { "+" } else { "\u{2212}" };
    let tone = if change_ms > 0.0 {
        DeltaTone::Worse
    } else {
        DeltaTone::Better
    };
    (
        format!("{sign}{}", format_latency_ms(change_ms.abs())),
        tone,
    )
}

/// The decades a log axis spans, from the one at or below `low` to the one at or above `high`,
/// both in milliseconds: at least one.
fn decades(low_ms: f64, high_ms: f64) -> (i32, i32) {
    let bottom = low_ms.max(0.001).log10().floor() as i32;
    let top = (high_ms.max(0.001).log10().ceil() as i32).max(bottom + 1);
    (bottom, top)
}

/// The text of a decade on the curve's axis: `1 ms`, `100 ms`, `10 s`.
fn decade_text(power: i32) -> String {
    let ms = 10f64.powi(power);
    if ms >= 1_000.0 {
        format!("{} s", format_integer((ms / 1_000.0).round() as u64))
    } else if ms >= 1.0 {
        format!("{} ms", format_integer(ms.round() as u64))
    } else {
        format!("{} ms", format_decimal(ms, (-power) as usize))
    }
}

/// The name of curve point `index`: `min`, `p90`, `p99.5`.
fn curve_point_label(index: usize) -> String {
    if index == 0 {
        return "min".to_string();
    }
    // Two decimals at most, and only the ones it needs: `p90`, `p99.9`, `p96.84`.
    let percentile = (curve_percentile(index) * 100.0).round() / 100.0;
    let decimals = (0..2)
        .find(|decimals| {
            let scaled = percentile * 10f64.powi(*decimals);
            (scaled - scaled.round()).abs() < 1e-6
        })
        .unwrap_or(2) as usize;
    format!("p{}", format_decimal(percentile, decimals))
}

/// The latency by percentile card: one line per request on log axes, the focused request (if
/// any) drawn over the others dimmed. A [`TimeSeriesChart`] over the curve's evenly spaced
/// points, fed the logarithms of the latencies.
fn render_curve(
    palette: &Palette,
    load_test: &LoadTestTab,
    view: &LatencyView,
    compare: Option<Compared>,
    names: &[String],
) -> AnyElement {
    let selected = load_test.latency_options.target;
    let compare_curves = compare.map_or(&[][..], |compare| compare.view.curves.as_slice());
    let finite = view
        .curves
        .iter()
        .chain(compare_curves)
        .flatten()
        .map(|us| f64::from(*us) / 1_000.0)
        .filter(|ms| *ms > 0.0);
    let (low, high) = finite.fold((f64::INFINITY, 0.0f64), |(low, high), ms| {
        (low.min(ms), high.max(ms))
    });
    let hint = match compare {
        Some(compare) => t!("load_test.latency.log_axes_compare", run = compare.title).into_owned(),
        None => t!("load_test.latency.log_axes").into_owned(),
    };
    let title = panel_title(t!("load_test.latency.curve").into_owned(), hint, palette);
    if !low.is_finite() {
        return panel(palette).child(title).into_any_element();
    }
    let (bottom, top) = decades(low, high);
    let labels: Vec<String> = (bottom..=top).map(decade_text).collect();
    let mut chart = TimeSeriesChart::new("load-test-latency-curve", CURVE_POINTS)
        .height(CURVE_HEIGHT)
        .gutters(56.0, 12.0)
        .surface(palette.raised)
        .left_axis(ChartAxis::new(f64::from(top - bottom), labels))
        .x_labels(["min", "p90", "p99", "p99.9"])
        .time_label(|index| curve_point_label(index).into());
    // The focused request last, so it is drawn on top.
    let mut order: Vec<usize> = (0..view.curves.len()).collect();
    if let Some(target) = selected {
        order.retain(|index| *index != target);
        order.push(target);
    }
    let log_values = |curve: &[u32]| -> Vec<f64> {
        curve
            .iter()
            .map(|us| (f64::from(*us).max(1.0) / 1_000.0).log10() - f64::from(bottom))
            .collect()
    };
    // The compared run's lines first, dashed, under this run's.
    if let Some(compare) = compare {
        for target in &order {
            let Some(curve) = compare_curves
                .get(*target)
                .filter(|curve| !curve.is_empty())
            else {
                continue;
            };
            let dimmed = selected.is_some_and(|selected| selected != *target);
            let color = palette
                .request_color(*target)
                .opacity(if dimmed { 0.12 } else { 0.6 });
            chart = chart.series(
                ChartSeries::new(
                    format!(
                        "{} \u{b7} {}",
                        names.get(*target).cloned().unwrap_or_default(),
                        compare.title
                    ),
                    color,
                    log_values(curve),
                )
                .style(SeriesStyle::dashed(1.25, 4.0, 3.0))
                .dot(false)
                .tooltip_order(1_000 + *target as i32)
                .format(move |value| {
                    format_latency_ms(10f64.powf(value + f64::from(bottom))).into()
                }),
            );
        }
    }
    for target in order {
        let curve = &view.curves[target];
        if curve.is_empty() {
            continue;
        }
        let values = log_values(curve);
        let focused = selected == Some(target);
        let color = palette.request_color(target);
        let color = if selected.is_some() && !focused {
            color.opacity(DIMMED_OPACITY)
        } else {
            color
        };
        let width = if focused { 2.25 } else { 1.75 };
        chart = chart.series(
            ChartSeries::new(
                names.get(target).cloned().unwrap_or_default(),
                color,
                values,
            )
            .style(SeriesStyle::line(width))
            .tooltip_order(target as i32)
            .format(move |value| format_latency_ms(10f64.powf(value + f64::from(bottom))).into()),
        );
    }
    panel(palette).child(title).child(chart).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::prelude::v1::test;

    #[test]
    fn the_curve_axis_spans_whole_decades() {
        assert_eq!(decades(300.0, 2_000.0), (2, 4));
        assert_eq!(decades(100.0, 100.0), (2, 3));
        assert_eq!(decades(0.4, 9.0), (-1, 1));
        assert_eq!(decade_text(2), "100 ms");
        assert_eq!(decade_text(4), "10 s");
        assert_eq!(decade_text(-1), "0.1 ms");
    }

    fn linear(edges_us: Vec<f64>, step_us: u32) -> StackedHistogram {
        StackedHistogram {
            edges_us,
            counts: Vec::new(),
            scale: LatencyScale::Linear,
            step_us,
        }
    }

    #[test]
    fn linear_ticks_are_round_multiples_of_the_bin_width() {
        // 0 to 2.1 s in 50 ms bins: a tick every 500 ms.
        let ticks = histogram_ticks(&linear(vec![0.0, 2_100_000.0], 50_000));
        assert_eq!(
            ticks,
            vec![0.0, 500_000.0, 1_000_000.0, 1_500_000.0, 2_000_000.0]
        );
        // 450 to 454 ms in 0.5 ms bins: a tick every millisecond, never the same text twice.
        let ticks = histogram_ticks(&linear(vec![450_000.0, 454_000.0], 500));
        let texts: Vec<String> = ticks.iter().map(|us| tick_text(*us)).collect();
        assert_eq!(
            texts,
            vec!["450 ms", "451 ms", "452 ms", "453 ms", "454 ms"]
        );
    }

    #[test]
    fn log_ticks_mark_one_two_three_and_five() {
        let histogram = StackedHistogram {
            scale: LatencyScale::Log,
            ..linear(vec![9_000.0, 1_100_000.0], 0)
        };
        let texts: Vec<String> = histogram_ticks(&histogram)
            .iter()
            .map(|us| tick_text(*us))
            .collect();
        assert_eq!(texts, vec!["10 ms", "100 ms", "1 s"]);
        assert_eq!(tick_text(0.0), "0");
        assert_eq!(tick_text(450_500.0), "450.5 ms");
        assert_eq!(tick_text(1_500_000.0), "1.5 s");
    }

    #[test]
    fn curve_points_are_named_by_percentile() {
        assert_eq!(curve_point_label(0), "min");
        assert_eq!(curve_point_label(20), "p90");
        assert_eq!(curve_point_label(40), "p99");
        assert_eq!(curve_point_label(60), "p99.9");
        assert_eq!(curve_point_label(30), "p96.84");
    }
}
