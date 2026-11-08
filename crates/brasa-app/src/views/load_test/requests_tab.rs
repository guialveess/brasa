//! The Requests tab of the load test results: the search box, the status, latency and virtual
//! user filters and "Only errors", then every request of the run (or of the selected window) in
//! a virtualized, sortable table. The rows are filtered and sorted in the background
//! (`results_data.rs`); the table only reads the rows on screen.

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_load::StatusKey;

use crate::state::load_results::{
    RequestColumn, StatusFilter, SummaryData, format_latency_ms, format_size,
    latency_filter_options, mmss_millis, status_filters, url_path,
};
use crate::state::load_test::LoadTestTab;
use crate::state::locale;
use crate::state::number::format_integer;
use crate::theme::metrics::RADIUS_LG;
use crate::theme::{Palette, PaletteExt};
use crate::views::components::{
    DataAlign, DataCell, DataColumn, EmptyState, MethodBadge, NeutralButtonExt, SegmentedControl,
    SegmentedItem, SortOrder, VirtualDataTable,
};
use crate::views::root::AppView;

use super::summary::{status_key_label, target_row_label};

/// Height of the filter bar's controls, from the design.
pub(crate) const FILTER_HEIGHT: f32 = 28.0;
/// Width of the search box, from the design.
const SEARCH_WIDTH: f32 = 230.0;
/// The table's columns' widths from the design: #, time, VU, (request is flexible), status,
/// latency, size.
const ID_WIDTH: f32 = 76.0;
const TIME_WIDTH: f32 = 92.0;
const VU_WIDTH: f32 = 56.0;
const STATUS_WIDTH: f32 = 72.0;
const LATENCY_WIDTH: f32 = 160.0;
const SIZE_WIDTH: f32 = 72.0;
/// Width of the latency value next to its bar.
const LATENCY_VALUE_WIDTH: f32 = 64.0;

impl AppView {
    /// Renders the Requests tab of the load test tab `tab_id`.
    pub(crate) fn render_load_test_requests(
        &mut self,
        tab_id: &str,
        load_test: &LoadTestTab,
        summary: &SummaryData,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        if load_test.log_unavailable {
            return EmptyState::new(t!("load_test.requests.unavailable"))
                .compact(true)
                .into_any_element();
        }
        let weak = cx.weak_entity();
        let rows = load_test.requests.as_ref().map(|(_, rows)| rows.clone());
        let row_count = rows.as_ref().map_or(0, |rows| rows.len());
        let sorted_column = column_label(load_test.request_sort.column);
        let count_text = if row_count > 0 {
            format!(
                "{} \u{b7} {}",
                locale::plural("load_test.requests.count", row_count),
                t!(
                    "load_test.requests.sorted_by",
                    column = sorted_column.to_lowercase()
                )
            )
        } else {
            locale::plural("load_test.requests.count", row_count)
        };

        let filters = h_flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .children(
                self.load_test_entities
                    .request_search
                    .as_ref()
                    .map(|search| {
                        h_flex()
                            .w(px(SEARCH_WIDTH))
                            .h(px(FILTER_HEIGHT))
                            .items_center()
                            .gap(px(8.0))
                            .px(px(10.0))
                            .rounded(px(7.0))
                            .border_1()
                            .border_color(palette.border_strong)
                            .bg(palette.raised)
                            .child(
                                Icon::new(IconName::Search)
                                    .small()
                                    .text_color(palette.fg_subtle),
                            )
                            .child(
                                Input::new(search)
                                    .flex_1()
                                    .px_0()
                                    .py_0()
                                    .h(px(FILTER_HEIGHT - 2.0))
                                    .bordered(false)
                                    .text_size(px(12.5))
                                    .bg(palette.bg.opacity(0.0)),
                            )
                    }),
            )
            .child(render_status_filter(weak.clone(), tab_id, load_test))
            .child(render_latency_filter(
                weak.clone(),
                tab_id,
                &palette,
                load_test,
                cx,
            ))
            .child(render_vu_filter(
                weak.clone(),
                tab_id,
                &palette,
                load_test,
                cx,
            ))
            .child(render_only_errors(
                weak.clone(),
                tab_id,
                &palette,
                load_test,
            ))
            .child(div().flex_1())
            .child(div().text_color(palette.fg_muted).child(count_text));

        let table = render_table(
            weak,
            tab_id,
            &palette,
            load_test,
            summary,
            rows,
            &self.load_test_entities.requests_scroll,
            cx,
        );

        v_flex()
            .flex_1()
            .min_h_0()
            .gap(px(12.0))
            .child(filters)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .border_1()
                    .border_color(palette.border)
                    .rounded(px(RADIUS_LG))
                    .bg(palette.raised)
                    .overflow_hidden()
                    .child(table),
            )
            .into_any_element()
    }
}

/// The header label of a column, also the "sorted by" text.
fn column_label(column: RequestColumn) -> String {
    match column {
        RequestColumn::Id => "#".to_string(),
        RequestColumn::Time => t!("load_test.requests.time").into_owned(),
        RequestColumn::Vu => "VU".to_string(),
        RequestColumn::Request => t!("common.request").into_owned(),
        RequestColumn::Status => t!("load_test.requests.status").into_owned(),
        RequestColumn::Latency => t!("load_test.requests.latency").into_owned(),
        RequestColumn::Size => t!("load_test.requests.size").into_owned(),
    }
}

/// The label of a status filter segment.
fn status_filter_label(filter: StatusFilter) -> String {
    match filter {
        StatusFilter::All => t!("load_test.requests.all").into_owned(),
        StatusFilter::Timeout => t!("load_test.status_key.timeout").into_owned(),
        StatusFilter::Failed => t!("load_test.status_key.failed").into_owned(),
        StatusFilter::Class(_) => filter.class_label().unwrap_or_default(),
    }
}

/// The status filter: "All" and one segment per kind of status the run had.
fn render_status_filter(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    load_test: &LoadTestTab,
) -> AnyElement {
    let statuses = load_test
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.status_counts.clone())
        .unwrap_or_default();
    let mut control = SegmentedControl::new("load-test-status-filter");
    for filter in status_filters(&statuses) {
        let item_weak = weak.clone();
        let tab_id = tab_id.to_string();
        control = control.item(
            SegmentedItem::new(status_filter_label(filter))
                .selected(load_test.request_filter.status == filter)
                .on_click(move |_, cx| {
                    let _ = item_weak.update(cx, |view, cx| {
                        view.edit_load_test_request_filter(&tab_id, cx, |request_filter| {
                            request_filter.status = filter;
                        });
                    });
                }),
        );
    }
    control.into_any_element()
}

/// A bordered filter button: a muted `label`, the current `value` and a chevron.
pub(crate) fn filter_button(
    id: &'static str,
    palette: &Palette,
    label: String,
    value: String,
    cx: &App,
) -> Button {
    Button::new(id)
        .neutral_bordered(cx, palette.raised, palette.border_strong, palette.fg)
        .border_1()
        .h(px(FILTER_HEIGHT))
        .px(px(10.0))
        .rounded(px(7.0))
        .text_size(px(12.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(6.0))
                .child(div().text_color(palette.fg_muted).child(label))
                .child(value)
                .child(
                    Icon::new(IconName::ChevronDown)
                        .small()
                        .text_color(palette.fg_muted),
                ),
        )
}

/// A checkbox drawn as a bordered filter pill, next to the [`filter_button`]s: the whole pill is
/// the click target and lights up on hover like them. A halo around the box alone would clash
/// with the pill's border, and one around box and label hid the box.
pub(crate) fn filter_checkbox(
    id: &'static str,
    palette: &Palette,
    label: String,
    checked: bool,
) -> Checkbox {
    let (hover, pressed) = (
        palette.hover_over(palette.raised),
        palette.pressed_over(palette.raised),
    );
    Checkbox::new(id)
        .label(SharedString::from(label))
        .checked(checked)
        .h(px(FILTER_HEIGHT))
        .items_center()
        .px(px(10.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.raised)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
}

/// The latency filter: any latency, or at least one of three values near the run's p50, p95
/// and p99.
fn render_latency_filter(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    cx: &App,
) -> AnyElement {
    let options = load_test
        .snapshot
        .as_ref()
        .map(|snapshot| latency_filter_options(snapshot.p50, snapshot.p95, snapshot.p99))
        .unwrap_or_default();
    let label = |ms: Option<u32>| match ms {
        None => t!("load_test.requests.any").into_owned(),
        Some(ms) => format!("\u{2265} {} ms", format_integer(u64::from(ms))),
    };
    let current = load_test.request_filter.min_latency_ms;
    let tab_id = tab_id.to_string();
    filter_button(
        "load-test-latency-filter",
        palette,
        t!("load_test.requests.latency").into_owned(),
        label(current),
        cx,
    )
    .dropdown_menu(move |mut menu, _, _| {
        for option in std::iter::once(None).chain(options.iter().copied().map(Some)) {
            let item_weak = weak.clone();
            let tab_id = tab_id.clone();
            menu = menu.item(
                PopupMenuItem::new(label(option))
                    .checked(current == option)
                    .on_click(move |_, _, cx| {
                        let _ = item_weak.update(cx, |view, cx| {
                            view.edit_load_test_request_filter(&tab_id, cx, |filter| {
                                filter.min_latency_ms = option;
                            });
                        });
                    }),
            );
        }
        menu
    })
    .into_any_element()
}

/// The virtual user filter: every virtual user, or one of them.
fn render_vu_filter(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    cx: &App,
) -> AnyElement {
    let vus = load_test.run_vus.unwrap_or(0);
    let label = |vu: Option<u32>| match vu {
        None => t!("load_test.requests.all").into_owned(),
        Some(vu) => format!("VU {vu}"),
    };
    let current = load_test.request_filter.vu;
    let tab_id = tab_id.to_string();
    filter_button(
        "load-test-vu-filter",
        palette,
        "VU".to_string(),
        label(current),
        cx,
    )
    .dropdown_menu(move |menu, _, _| {
        let mut menu = menu.scrollable(true).max_h(px(320.0));
        for option in std::iter::once(None).chain((1..=vus).map(Some)) {
            let item_weak = weak.clone();
            let tab_id = tab_id.clone();
            menu = menu.item(
                PopupMenuItem::new(label(option))
                    .checked(current == option)
                    .on_click(move |_, _, cx| {
                        let _ = item_weak.update(cx, |view, cx| {
                            view.edit_load_test_request_filter(&tab_id, cx, |filter| {
                                filter.vu = option;
                            });
                        });
                    }),
            );
        }
        menu
    })
    .into_any_element()
}

/// "Only errors".
fn render_only_errors(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
) -> AnyElement {
    let tab_id = tab_id.to_string();
    filter_checkbox(
        "load-test-only-errors",
        palette,
        t!("load_test.requests.only_errors").into_owned(),
        load_test.request_filter.only_errors,
    )
    .on_click(move |checked, _, cx| {
        let checked = *checked;
        let _ = weak.update(cx, |view, cx| {
            view.edit_load_test_request_filter(&tab_id, cx, |filter| {
                filter.only_errors = checked;
            });
        });
    })
    .into_any_element()
}

/// The lowest latency threshold, in microseconds: slower requests are drawn in `warning`.
fn slow_from_us(load_test: &LoadTestTab) -> Option<u32> {
    load_test
        .run_thresholds
        .iter()
        .filter(|threshold| {
            matches!(
                threshold.metric,
                brasa_load::ThresholdMetric::P50
                    | brasa_load::ThresholdMetric::P95
                    | brasa_load::ThresholdMetric::P99
            )
        })
        .map(|threshold| (threshold.limit * 1_000.0) as u32)
        .min()
}

/// The virtualized table of requests.
#[allow(clippy::too_many_arguments)]
fn render_table(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    summary: &SummaryData,
    rows: Option<crate::state::load_results::Shared<Vec<u32>>>,
    scroll: &UniformListScrollHandle,
    cx: &App,
) -> AnyElement {
    let row_count = rows.as_ref().map_or(0, |rows| rows.len());
    let log = load_test.log.clone();
    let labels: Vec<(Option<brasa_core::Method>, String)> =
        (0..load_test.target_rows.len().max(1))
            .map(|index| target_row_label(load_test, index))
            .collect();
    let slow_from = slow_from_us(load_test);
    // The latency bars are relative to the p99, so the slow tail saturates instead of squashing
    // every other bar.
    let full_us = summary.p99.max(1);
    let colors = RowColors {
        danger: palette.danger,
        warning: palette.warning,
        accent: palette.accent_text,
        fg: palette.fg,
        muted: palette.fg_muted,
        subtle: palette.fg_subtle,
        track: palette.surface,
        url: palette.fg,
        success: palette.success,
        info: palette.info,
    };
    let background_rows = rows.clone();
    let background_log = log.clone();
    let error_tint = palette.danger.opacity(0.05);
    let mono = cx.theme().mono_font_family.clone();

    let sort = load_test.request_sort;
    let sorted_index = RequestColumn::ALL
        .iter()
        .position(|column| *column == sort.column)
        .unwrap_or(0);
    let order = if sort.descending {
        SortOrder::Descending
    } else {
        SortOrder::Ascending
    };
    let empty = if load_test.requests.is_none() {
        t!("load_test.requests.loading")
    } else {
        t!("load_test.requests.empty")
    };
    let tab_id = tab_id.to_string();

    let table = VirtualDataTable::new("load-test-requests-table", row_count, move |range, _, _| {
        let (Some(rows), Some(log)) = (rows.as_ref(), log.as_ref()) else {
            return Vec::new();
        };
        range
            .filter_map(|position| rows.get(position).copied())
            .filter_map(|index| {
                let sample = log.samples().get(index as usize)?;
                let (method, _) = labels
                    .get(sample.target as usize)
                    .cloned()
                    .unwrap_or((None, String::new()));
                let slow = slow_from.is_some_and(|limit| sample.latency_us >= limit);
                let (bar, value) = if sample.is_error {
                    (colors.danger, colors.danger)
                } else if slow {
                    (colors.warning, colors.warning)
                } else {
                    (colors.accent, colors.fg)
                };
                let fraction = (sample.latency_us as f32 / full_us as f32).clamp(0.0, 1.0);
                Some(vec![
                    DataCell::text(format!("#{}", format_integer(u64::from(index) + 1)))
                        .color(colors.subtle),
                    DataCell::text(mmss_millis(u64::from(sample.elapsed_ms))).color(colors.muted),
                    DataCell::text(format!("VU {}", sample.vu + 1)).color(colors.muted),
                    DataCell::element(
                        h_flex()
                            .min_w_0()
                            .items_center()
                            .gap(px(8.0))
                            .children(
                                method.map(|method| {
                                    div().flex_none().child(MethodBadge::label(method))
                                }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(colors.url)
                                    .child(url_path(log.url(sample.url)).to_string()),
                            ),
                    ),
                    DataCell::text(status_text(sample.status))
                        .color(status_color(sample.status, &colors)),
                    DataCell::element(
                        h_flex()
                            .items_center()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .h(px(4.0))
                                    .rounded(px(2.0))
                                    .bg(colors.track)
                                    .overflow_hidden()
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(fraction))
                                            .rounded(px(2.0))
                                            .bg(bar),
                                    ),
                            )
                            .child(
                                div()
                                    .w(px(LATENCY_VALUE_WIDTH))
                                    .flex_none()
                                    .text_right()
                                    .text_color(value)
                                    .child(format_latency_ms(
                                        f64::from(sample.latency_us) / 1_000.0,
                                    )),
                            ),
                    ),
                    DataCell::text(if sample.size == 0 && sample.is_error {
                        "\u{2013}".to_string()
                    } else {
                        format_size(u64::from(sample.size))
                    })
                    .color(colors.muted),
                ])
            })
            .collect()
    })
    .column(DataColumn::new("#").width(ID_WIDTH).sortable())
    .column(
        DataColumn::new(column_label(RequestColumn::Time))
            .width(TIME_WIDTH)
            .sortable(),
    )
    .column(DataColumn::new("VU").width(VU_WIDTH).sortable())
    .column(DataColumn::new(column_label(RequestColumn::Request)).sortable())
    .column(
        DataColumn::new(column_label(RequestColumn::Status))
            .width(STATUS_WIDTH)
            .sortable(),
    )
    .column(
        DataColumn::new(column_label(RequestColumn::Latency))
            .width(LATENCY_WIDTH)
            .align(DataAlign::Right)
            .sortable(),
    )
    .column(
        DataColumn::new(column_label(RequestColumn::Size))
            .width(SIZE_WIDTH)
            .align(DataAlign::Right)
            .sortable(),
    )
    .mono(true)
    .sort(Some((sorted_index, order)))
    .on_sort(move |column, _, cx| {
        let Some(column) = RequestColumn::ALL.get(column).copied() else {
            return;
        };
        let _ = weak.update(cx, |view, cx| {
            view.sort_load_test_requests(&tab_id, column, cx);
        });
    })
    .row_background(move |position| {
        let index = background_rows.as_ref()?.get(position).copied()?;
        let sample = background_log.as_ref()?.samples().get(index as usize)?;
        sample.is_error.then_some(error_tint)
    })
    .track_scroll(scroll)
    .empty_message(empty);

    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .font_family(mono)
        .child(table)
        .into_any_element()
}

/// The colors a row reads, copied out of the palette so the row closure owns them.
#[derive(Clone, Copy)]
struct RowColors {
    danger: Hsla,
    warning: Hsla,
    accent: Hsla,
    fg: Hsla,
    muted: Hsla,
    subtle: Hsla,
    track: Hsla,
    url: Hsla,
    success: Hsla,
    info: Hsla,
}

/// A status as the table shows it.
fn status_text(status: StatusKey) -> String {
    status_key_label(&status)
}

/// A status' text color, the same as in "Responses by status".
fn status_color(status: StatusKey, colors: &RowColors) -> Hsla {
    match status {
        StatusKey::Code(code) if (200..300).contains(&code) => colors.success,
        StatusKey::Code(code) if (300..400).contains(&code) => colors.info,
        StatusKey::Code(code) if (400..500).contains(&code) => colors.warning,
        StatusKey::Code(code) if code < 200 => colors.muted,
        _ => colors.danger,
    }
}
