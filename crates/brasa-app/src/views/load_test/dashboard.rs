//! The load test tab's right side: the empty state and history list before a first run, and,
//! once a run exists, the results of the design's option 6a: the header (title, status, "Compare
//! with" and "Export"), the thresholds verdict, the Summary / Requests / Errors tabs (with the
//! selected window's chip) and the shown tab (`summary.rs`, `requests_tab.rs`, `errors_tab.rs`).

use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_load::history::RunRecordHeader;
use brasa_load::{LoadSnapshot, ThresholdMetric};

use crate::state::format::relative_day;
use crate::state::load_results::{ResultsTab, SummaryData, mmss, window_label};
use crate::state::load_test::{LoadTestFailure, LoadTestStatus, LoadTestTab, threshold_label};
use crate::state::number::{format_decimal, format_integer};
use crate::theme::{Palette, PaletteExt};
use crate::views::components::{
    Card, Chip, EmptyState, InlineMessage, InlineMessageKind, NeutralButtonExt, ProgressBar,
    UnderlineTabItem, UnderlineTabs, menu_choice_item, menu_described_item, menu_shortcut_item,
    menu_toggle_item, rich_menu_width,
};

use super::export::ExportFormat;
use super::run::unix_now;
use crate::views::root::AppView;

/// Width of the "Compare with" menu, from the design.
const COMPARE_MENU_WIDTH: f32 = 240.0;
/// Width of the "Export" menu, a little over the design's 320 px: the "Copy summary as Markdown"
/// row does not wrap or truncate, and in Spanish and Italian its shortcut hint overflowed it.
const EXPORT_MENU_WIDTH: f32 = 344.0;
/// Minimum width of the header's progress bar while a run is in progress.
const PROGRESS_MIN_WIDTH: f32 = 160.0;
/// Height of the header's buttons.
const HEADER_BUTTON_HEIGHT: f32 = 28.0;

impl AppView {
    /// Renders the right side of the load test tab `tab_id`: the empty state (and history)
    /// before a first run, or the results once a snapshot exists.
    pub(crate) fn render_load_test_dashboard(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(load_test) = self
            .state
            .tabs
            .active()
            .and_then(|tab| tab.load_test())
            .cloned()
        else {
            return div().flex_1().into_any_element();
        };
        let palette = cx.palette();
        let weak = cx.weak_entity();

        let Some(snapshot) = load_test.snapshot.clone() else {
            return render_empty_state(weak, tab_id, &palette, &load_test);
        };
        let summary = summary_data(&load_test, &snapshot);

        let mut column = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .px(px(20.0))
            .pt(px(16.0))
            .gap(px(14.0))
            .child(render_header(
                weak.clone(),
                tab_id,
                &palette,
                &load_test,
                &snapshot,
                cx,
            ));
        if !load_test.is_running() && !load_test.run_thresholds.is_empty() {
            column = column.child(render_verdict(&palette, &load_test, &snapshot));
        }
        column = column.child(render_tabs(weak, tab_id, &palette, &load_test, &summary));

        match load_test.results_tab {
            ResultsTab::Requests => column
                .pb(px(16.0))
                .child(self.render_load_test_requests(tab_id, &load_test, &summary, window, cx))
                .into_any_element(),
            tab => {
                let content = match tab {
                    ResultsTab::Summary => {
                        self.render_load_test_summary(tab_id, &load_test, &snapshot, &summary, cx)
                    }
                    ResultsTab::Latency => self.render_load_test_latency(tab_id, &load_test, cx),
                    _ => self.render_load_test_errors(tab_id, &load_test, &snapshot, cx),
                };
                // The header and the tabs scroll away with the content, like the design.
                v_flex()
                    .id("load-test-dashboard")
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(column.h_auto().pb(px(24.0)).child(content))
                    .into_any_element()
            }
        }
    }
}

/// What the Summary shows: the window's statistics once computed, the snapshot's otherwise (a
/// past run without saved requests, or before the first computation). While a run is live the
/// KPIs always come from the snapshot, whose requests per second is the last second's.
pub(crate) fn summary_data(load_test: &LoadTestTab, snapshot: &LoadSnapshot) -> SummaryData {
    match load_test.current_analysis() {
        Some(stats) => {
            let mut data = SummaryData::from_stats(stats);
            if load_test.is_running() {
                data.rps = snapshot.rps;
                data.p50 = snapshot.p50;
                data.p95 = snapshot.p95;
                data.p99 = snapshot.p99;
                data.error_rate = snapshot.error_rate;
                data.total = snapshot.total;
            }
            data
        }
        None => SummaryData::from_snapshot(snapshot),
    }
}

/// The empty state before a first run: an explanatory placeholder, plus a Failed run's reason, plus
/// the target's run history if any (`click to view` is wired by the history rows' own click
/// handler, see [`render_history_list`]).
fn render_empty_state(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
) -> AnyElement {
    let mut column = v_flex()
        .id("load-test-empty-state")
        .flex_1()
        .h_full()
        .min_w_0()
        .min_h_0()
        .overflow_y_scrollbar()
        .px(px(20.0))
        .py(px(16.0))
        .gap(px(16.0));

    if let LoadTestStatus::Failed(failure) = &load_test.status {
        column = column.child(InlineMessage::new(
            InlineMessageKind::Danger,
            failure_message(failure),
        ));
    }

    column = column.child(EmptyState::new(t!("load_test.dashboard.empty")).compact(true));

    if !load_test.history.is_empty() {
        column = column.child(render_history_list(weak, tab_id, palette, load_test));
    }

    column.into_any_element()
}

/// The text shown for a run that could not be started, in the current language.
fn failure_message(failure: &LoadTestFailure) -> String {
    match failure {
        LoadTestFailure::NoWorkspace => t!("load_test.failure.no_workspace").into_owned(),
        LoadTestFailure::TabClosed => t!("load_test.failure.tab_closed").into_owned(),
        LoadTestFailure::NoTarget => t!("load_test.failure.no_target").into_owned(),
        LoadTestFailure::EmptyFolder => t!("load_test.failure.empty_folder").into_owned(),
        LoadTestFailure::Load(error) => t!("load_test.failure.load", error = error).into_owned(),
    }
}

/// `"Run #N"`, the number only.
pub(crate) fn run_title(number: u32) -> String {
    t!("load_test.run.numbered", number = number).into_owned()
}

/// `"Run #N \u{b7} today"`, for the history list.
fn run_with_day(number: u32, started_at_unix: u64, now: i64) -> String {
    format!(
        "{} \u{b7} {}",
        run_title(number),
        relative_day(started_at_unix as i64, now)
    )
}

/// How a run connected: "Keep-alive" when it reused its connections, "New connections" otherwise.
pub(crate) fn connection_mode_label(reused: bool) -> String {
    if reused {
        t!("load_test.connections.reused")
    } else {
        t!("load_test.connections.new")
    }
    .into_owned()
}

/// The history list of previous runs for this tab's target, click to view.
fn render_history_list(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
) -> AnyElement {
    let now = unix_now() as i64;

    Card::new()
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .child(t!("load_test.history.title")),
        )
        .children(load_test.history.iter().map(|header| {
            let number = header.number;
            let tab_id = tab_id.to_string();
            let row_weak = weak.clone();
            h_flex()
                .id(("load-test-history-row", number as usize))
                .h(px(30.0))
                .items_center()
                .gap(px(10.0))
                .cursor_pointer()
                .rounded(px(6.0))
                .px(px(6.0))
                .hover(|style| style.bg(palette.hover))
                .child(div().flex_1().min_w_0().child(run_with_day(
                    number,
                    header.started_at_unix,
                    now,
                )))
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_subtle)
                        .child(connection_mode_label(header.reuse_connections)),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_subtle)
                        .child(format!("{} req/s", format_decimal(header.rps, 0))),
                )
                .on_click(move |_, _window, cx| {
                    let _ = row_weak.update(cx, |view, cx| {
                        view.view_load_test_history(&tab_id, number, cx);
                    });
                })
        }))
        .into_any_element()
}

/// The status badge's label and colors: running in accent, finished in success, stopped in
/// warning, failed in danger.
fn status_badge_colors(palette: &Palette, status: &LoadTestStatus) -> (String, Hsla, Hsla) {
    let (label, fg, bg) = match status {
        LoadTestStatus::NotStarted => (
            t!("load_test.status.not_started"),
            palette.fg_muted,
            palette.hover,
        ),
        LoadTestStatus::Running => (
            t!("load_test.status.running"),
            palette.accent_text,
            palette.accent_subtle,
        ),
        LoadTestStatus::Finished => (
            t!("load_test.status.finished"),
            palette.success,
            palette.success_subtle,
        ),
        LoadTestStatus::Stopped => (
            t!("load_test.status.stopped"),
            palette.warning,
            palette.warning_subtle,
        ),
        LoadTestStatus::Failed(_) => (
            t!("load_test.status.failed"),
            palette.danger,
            palette.danger_subtle,
        ),
    };
    (label.into_owned(), fg, bg)
}

/// The header: "Run #N", the status badge and the connection mode with the start day, then the
/// progress while running, or the "Compare with" and "Export" menus once the run has ended.
fn render_header(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    snapshot: &LoadSnapshot,
    cx: &App,
) -> AnyElement {
    let run_label = load_test
        .run_number
        .map(run_title)
        .unwrap_or_else(|| t!("load_test.run.plain").into_owned());
    let (badge_label, badge_fg, badge_bg) = status_badge_colors(palette, &load_test.status);
    let mut subtitle = Vec::new();
    if let Some(reused) = load_test.run_reuse_connections {
        subtitle.push(connection_mode_label(reused));
    }
    if let Some(started) = load_test.run_started_at {
        subtitle.push(format!(
            "{} {}",
            relative_day(started as i64, unix_now() as i64),
            crate::state::load_report::local_time(started as i64)
        ));
    }

    let mut row = h_flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(12.0))
        .gap_y(px(8.0))
        .child(
            div()
                .flex_none()
                .text_size(px(16.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(run_label),
        )
        .child(
            h_flex()
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .h(px(22.0))
                .px(px(8.0))
                .rounded(px(6.0))
                .bg(badge_bg)
                .text_color(badge_fg)
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().size(px(6.0)).rounded_full().bg(badge_fg))
                .child(badge_label),
        )
        .when(!subtitle.is_empty(), |row| {
            row.child(
                div()
                    .flex_none()
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(subtitle.join(" \u{b7} ")),
            )
        })
        .child(div().flex_1());

    if load_test.is_running() {
        let duration_secs = load_test.shown_duration_secs();
        let progress = if duration_secs > 0 {
            (snapshot.elapsed.as_secs_f64() / duration_secs as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        row = row
            .child(ProgressBar::new(progress as f32).min_width(PROGRESS_MIN_WIDTH))
            .child(
                div()
                    .flex_none()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(format!(
                        "{} / {}",
                        mmss(snapshot.elapsed.as_secs()),
                        mmss(Duration::from_secs(duration_secs).as_secs())
                    )),
            );
    } else {
        row = row
            .child(render_compare_menu(
                weak.clone(),
                tab_id,
                palette,
                load_test,
                cx,
            ))
            .child(render_export_menu(weak, tab_id, palette, load_test, cx));
    }
    row.into_any_element()
}

/// The previous runs of the same target the "Compare with" menu offers: every one but the run
/// shown.
fn compare_candidates(load_test: &LoadTestTab) -> Vec<RunRecordHeader> {
    load_test
        .history
        .iter()
        .filter(|header| Some(header.number) != load_test.run_number)
        .cloned()
        .collect()
}

/// The "Compare with <run>" button and its menu: one row per previous run of the same target
/// (with its day and requests per second) and "No comparison".
fn render_compare_menu(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    cx: &App,
) -> AnyElement {
    let candidates = compare_candidates(load_test);
    let selected = load_test
        .compare_with
        .filter(|number| candidates.iter().any(|header| header.number == *number));
    let value = selected
        .map(run_title)
        .unwrap_or_else(|| t!("load_test.compare.nothing").into_owned());
    let now = unix_now() as i64;
    let tab_id = tab_id.to_string();

    header_button("load-test-compare-with", palette, false, cx)
        .child(
            h_flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    Icon::new(gpui_kit::assets::IconName::GitCompare)
                        .small()
                        .text_color(palette.fg_muted),
                )
                .child(
                    div()
                        .text_color(palette.fg_muted)
                        .child(t!("load_test.compare.title")),
                )
                .child(value)
                .child(
                    Icon::new(IconName::ChevronDown)
                        .small()
                        .text_color(palette.fg_muted),
                ),
        )
        .dropdown_menu(move |menu, _, _| {
            let mut menu = rich_menu_width(menu, COMPARE_MENU_WIDTH);
            for header in &candidates {
                let number = header.number;
                let item_weak = weak.clone();
                let tab_id = tab_id.clone();
                let detail = format!(
                    "{} \u{b7} {} req/s",
                    relative_day(header.started_at_unix as i64, now),
                    format_integer(header.rps.round() as u64)
                );
                menu = menu.item(menu_choice_item(
                    run_title(number),
                    Some(detail.into()),
                    selected == Some(number),
                    move |_, cx| {
                        let _ = item_weak.update(cx, |view, cx| {
                            view.set_load_test_compare_with(&tab_id, Some(number), cx);
                        });
                    },
                ));
            }
            let none_weak = weak.clone();
            let none_tab_id = tab_id.clone();
            menu.item(menu_choice_item(
                t!("load_test.compare.none"),
                None,
                selected.is_none(),
                move |_, cx| {
                    let _ = none_weak.update(cx, |view, cx| {
                        view.set_load_test_compare_with(&none_tab_id, None, cx);
                    });
                },
            ))
        })
        .into_any_element()
}

/// The "Export" button and its menu: Markdown, PDF, CSV and JSON files, "Only the window" when a
/// window is selected, and "Copy summary as Markdown".
fn render_export_menu(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    cx: &App,
) -> AnyElement {
    use gpui_kit::assets::IconName as Lucide;

    let tab_id = tab_id.to_string();
    let window = load_test.window;
    let window_only = load_test.export_window_only;
    let whole_run = load_test.snapshot.as_ref().map_or(0, |s| s.total);
    // The selected window's requests, unless "Only the window" is off.
    let rows = match load_test.current_analysis() {
        Some(stats) if window.is_none() || window_only => stats.total,
        _ => whole_run,
    };
    let entries: [(Lucide, &str, &str, String, ExportFormat); 4] = [
        (
            Lucide::FileText,
            "Markdown",
            ".md",
            t!("load_test.export.markdown").into_owned(),
            ExportFormat::Markdown,
        ),
        (
            Lucide::FileChartColumn,
            "PDF",
            ".pdf",
            t!("load_test.export.pdf").into_owned(),
            ExportFormat::Pdf,
        ),
        (
            Lucide::Sheet,
            "CSV",
            ".csv",
            crate::state::locale::plural("load_test.export.csv", rows as usize),
            ExportFormat::Csv,
        ),
        (
            Lucide::Braces,
            "JSON",
            ".json",
            t!("load_test.export.json").into_owned(),
            ExportFormat::Json,
        ),
    ];

    header_button("load-test-export", palette, true, cx)
        .child(
            h_flex()
                .items_center()
                .gap(px(6.0))
                .child(Icon::new(Lucide::Share).small())
                .child(t!("load_test.export.button"))
                .child(
                    Icon::new(IconName::ChevronDown)
                        .small()
                        .text_color(palette.fg_muted),
                ),
        )
        .dropdown_menu(move |menu, _, _| {
            let mut menu = rich_menu_width(menu, EXPORT_MENU_WIDTH);
            for (icon, title, extension, description, format) in &entries {
                let item_weak = weak.clone();
                let tab_id = tab_id.clone();
                let format = *format;
                menu = menu.item(menu_described_item(
                    EXPORT_MENU_WIDTH,
                    Icon::new(*icon),
                    *title,
                    Some((*extension).into()),
                    description.clone(),
                    move |window, cx| {
                        let _ = item_weak.update(cx, |view, cx| {
                            view.export_load_test(&tab_id, format, window, cx);
                        });
                    },
                ));
            }
            menu = menu.separator();
            if let Some((first, last)) = window {
                let toggle_weak = weak.clone();
                let toggle_tab_id = tab_id.clone();
                menu = menu.item(menu_toggle_item(
                    t!(
                        "load_test.export.only_window",
                        range = window_label(first, last)
                    ),
                    window_only,
                    move |checked, _, cx| {
                        let _ = toggle_weak.update(cx, |view, cx| {
                            view.edit_load_test(&toggle_tab_id, cx, |load_test| {
                                load_test.export_window_only = checked;
                            });
                        });
                    },
                ));
            }
            let copy_weak = weak.clone();
            let copy_tab_id = tab_id.clone();
            menu.item(menu_shortcut_item(
                Icon::new(Lucide::Clipboard),
                t!("load_test.export.copy_markdown"),
                copy_shortcut_label(),
                move |window, cx| {
                    let _ = copy_weak.update(cx, |view, cx| {
                        view.copy_load_test_summary(&copy_tab_id, window, cx);
                    });
                },
            ))
        })
        .into_any_element()
}

/// The "Copy summary as Markdown" shortcut as shown, with the platform's modifier.
fn copy_shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "\u{2318}\u{21e7}C"
    } else {
        "Ctrl+Shift+C"
    }
}

/// A bordered header button, 28 px high: neutral, or a little stronger for the primary
/// "Export".
fn header_button(id: &'static str, palette: &Palette, strong: bool, cx: &App) -> Button {
    let (rest, border) = if strong {
        (palette.hover_over(palette.raised), palette.border_strong)
    } else {
        (palette.raised, palette.border)
    };
    Button::new(id)
        .neutral_bordered(cx, rest, border, palette.fg)
        .border_1()
        .h(px(HEADER_BUTTON_HEIGHT))
        .px(px(10.0))
        .rounded(px(7.0))
        .text_size(px(12.5))
        .when(strong, |button| button.font_weight(FontWeight::MEDIUM))
}

/// The thresholds verdict: "Thresholds passed" or "Not passed · N of M", then one chip per
/// threshold with the whole run's value.
fn render_verdict(
    palette: &Palette,
    load_test: &LoadTestTab,
    snapshot: &LoadSnapshot,
) -> AnyElement {
    let inputs = SummaryData::from_snapshot(snapshot).threshold_inputs();
    let outcomes = brasa_load::evaluate_thresholds(&load_test.run_thresholds, &inputs);
    let failed = outcomes.iter().filter(|outcome| !outcome.passed).count();
    let (text, icon, fg, bg) = if failed == 0 {
        (
            t!("load_test.verdict.passed").into_owned(),
            gpui_kit::assets::IconName::CircleCheck,
            palette.success,
            palette.success_subtle,
        )
    } else {
        (
            t!(
                "load_test.verdict.failed",
                failed = failed,
                total = outcomes.len()
            )
            .into_owned(),
            gpui_kit::assets::IconName::CircleX,
            palette.danger,
            palette.danger_subtle,
        )
    };

    h_flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .text_size(px(12.5))
        .child(
            h_flex()
                .items_center()
                .gap(px(6.0))
                .h(px(24.0))
                .px(px(9.0))
                .rounded(px(6.0))
                .bg(bg)
                .text_color(fg)
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .child(Icon::new(icon).with_size(px(12.0)))
                .child(text),
        )
        .children(outcomes.iter().map(|outcome| {
            let color = if outcome.passed {
                palette.success
            } else {
                palette.danger
            };
            let icon = if outcome.passed {
                gpui_kit::assets::IconName::Check
            } else {
                gpui_kit::assets::IconName::X
            };
            h_flex()
                .items_center()
                .gap(px(6.0))
                .h(px(24.0))
                .px(px(9.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(palette.border)
                .child(Icon::new(icon).with_size(px(12.0)).text_color(color))
                .child(
                    div()
                        .text_color(palette.fg_muted)
                        .child(threshold_label(&outcome.threshold)),
                )
                .child(
                    div()
                        .font_family(telaio_ui::fonts::MONO_FAMILY)
                        .text_size(px(12.0))
                        .text_color(color)
                        .child(threshold_value(outcome.threshold.metric, outcome.value)),
                )
        }))
        .into_any_element()
}

/// A threshold metric's value as shown next to it: latencies with their unit, the error rate in
/// percent, requests per second as a whole number.
pub(crate) fn threshold_value(metric: ThresholdMetric, value: f64) -> String {
    match metric {
        ThresholdMetric::Rps => format_integer(value.round().max(0.0) as u64),
        ThresholdMetric::ErrorRate => format!("{} %", format_decimal(value * 100.0, 1)),
        _ => crate::state::load_results::format_latency_ms(value),
    }
}

/// The Summary / Requests / Errors tabs, with the request and error counts of the window shown,
/// and the selected window's chip on the right.
fn render_tabs(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    summary: &SummaryData,
) -> AnyElement {
    let errors = summary
        .statuses
        .iter()
        .filter(|(status, _)| status.is_error())
        .map(|(_, count)| count)
        .sum::<u64>();
    let item = |tab: ResultsTab, label: String| {
        let item_weak = weak.clone();
        let tab_id = tab_id.to_string();
        UnderlineTabItem::new(label)
            .selected(load_test.results_tab == tab)
            .on_click(move |_, cx| {
                let _ = item_weak.update(cx, |view, cx| {
                    view.set_load_test_results_tab(&tab_id, tab, cx);
                });
            })
    };
    let mut errors_item = item(
        ResultsTab::Errors,
        t!("load_test.results.errors").into_owned(),
    )
    .count(format_integer(errors));
    if errors > 0 {
        errors_item = errors_item.count_color(palette.danger);
    }
    let mut tabs = UnderlineTabs::new("load-test-results-tabs")
        .height(34.0)
        .item(item(
            ResultsTab::Summary,
            t!("load_test.results.summary").into_owned(),
        ))
        .item(item(
            ResultsTab::Latency,
            t!("load_test.results.latency").into_owned(),
        ))
        .item(
            item(
                ResultsTab::Requests,
                t!("load_test.results.requests").into_owned(),
            )
            .count(format_integer(summary.total)),
        )
        .item(errors_item);
    if let Some((first, last)) = load_test.window {
        let chip_weak = weak.clone();
        let tab_id = tab_id.to_string();
        tabs = tabs.suffix(
            Chip::new(
                "load-test-window-chip",
                t!("load_test.window.chip", range = window_label(first, last)),
            )
            .icon(gpui_kit::assets::IconName::Scan)
            .on_dismiss(move |_, cx| {
                let _ = chip_weak.update(cx, |view, cx| {
                    view.set_load_test_window(&tab_id, None, cx);
                });
            }),
        );
    }
    tabs.into_any_element()
}
