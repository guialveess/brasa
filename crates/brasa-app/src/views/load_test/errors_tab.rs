//! The Errors tab of the load test results: one card per kind of error in the window shown, with
//! its count and share, a per second sparkline over the whole run, the first and last time it
//! happened, the requests it affected, an example response and "View requests".

use gpui_kit::component::button::Button;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_load::{ErrorGroup, LoadSnapshot, StatusKey};

use crate::state::load_results::mmss;
use crate::state::load_test::LoadTestTab;
use crate::state::number::{format_decimal, format_integer};
use crate::state::response_render::reason_phrase;
use crate::theme::metrics::RADIUS_LG;
use crate::theme::{Palette, PaletteExt};
use crate::views::components::{EmptyState, NeutralButtonExt};
use crate::views::root::AppView;

use super::summary::{status_key_color, target_row_label};

/// The most bars a sparkline draws: longer runs add up several seconds per bar, so a long run
/// never builds thousands of elements per card.
const SPARKLINE_MAX_BARS: usize = 120;
/// Height of a sparkline.
const SPARKLINE_HEIGHT: f32 = 28.0;
/// Width of the first/last and affected requests column, from the design.
const DETAILS_WIDTH: f32 = 300.0;

impl AppView {
    /// Renders the Errors tab of the load test tab `tab_id`.
    pub(crate) fn render_load_test_errors(
        &mut self,
        tab_id: &str,
        load_test: &LoadTestTab,
        snapshot: &LoadSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        if load_test.log_unavailable {
            return EmptyState::new(t!("load_test.requests.unavailable"))
                .compact(true)
                .into_any_element();
        }
        let Some(stats) = load_test.current_analysis() else {
            return EmptyState::new(t!("load_test.requests.loading"))
                .compact(true)
                .into_any_element();
        };
        if stats.error_groups.is_empty() {
            let text = if load_test.window.is_some() {
                t!("load_test.errors.none_in_window")
            } else {
                t!("load_test.errors.none")
            };
            return div()
                .border_1()
                .border_color(palette.border)
                .rounded(px(RADIUS_LG))
                .p(px(40.0))
                .text_center()
                .text_color(palette.fg_subtle)
                .child(text)
                .into_any_element();
        }
        let mono = cx.theme().mono_font_family.clone();
        let timeout_secs = self.send_options.timeout.as_secs();
        let weak = cx.weak_entity();
        let total = stats.total.max(1);

        v_flex()
            .gap(px(14.0))
            .children(stats.error_groups.iter().map(|group| {
                render_group(
                    weak.clone(),
                    tab_id,
                    &palette,
                    &mono,
                    load_test,
                    snapshot,
                    group,
                    total,
                    timeout_secs,
                    cx,
                )
            }))
            .into_any_element()
    }
}

/// The title of an error kind: `"429 Too Many Requests"`, "No response in 30 s" or "Request
/// failed".
pub(crate) fn error_title(status: StatusKey, timeout_secs: u64) -> String {
    match status {
        StatusKey::Code(code) => match reason_phrase(code) {
            Some(reason) => format!("{code} {reason}"),
            None => code.to_string(),
        },
        StatusKey::Timeout => t!(
            "load_test.errors.timeout_title",
            secs = format_integer(timeout_secs)
        )
        .into_owned(),
        StatusKey::Failed => t!("load_test.errors.failed_title").into_owned(),
    }
}

/// The short code shown at the start of a group: the HTTP code, or "Timeout" / "Failed".
fn error_code(status: StatusKey) -> String {
    super::summary::status_key_label(&status)
}

/// The counts of `per_second` added up into at most [`SPARKLINE_MAX_BARS`] bars, with the
/// range of seconds each covers.
fn sparkline_bars(per_second: &[u64]) -> Vec<(u64, usize, usize)> {
    let seconds = per_second.len().max(1);
    let per_bar = seconds.div_ceil(SPARKLINE_MAX_BARS).max(1);
    per_second
        .chunks(per_bar)
        .enumerate()
        .map(|(index, chunk)| {
            let first = index * per_bar;
            (chunk.iter().sum(), first, first + chunk.len() - 1)
        })
        .collect()
}

/// One error kind's card.
#[allow(clippy::too_many_arguments)]
fn render_group(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    mono: &SharedString,
    load_test: &LoadTestTab,
    snapshot: &LoadSnapshot,
    group: &ErrorGroup,
    total: u64,
    timeout_secs: u64,
    cx: &App,
) -> AnyElement {
    let color = status_key_color(palette, &group.status);
    let share = group.count as f64 / total as f64 * 100.0;
    let bars = sparkline_bars(&group.per_second);
    let peak = bars
        .iter()
        .map(|(count, _, _)| *count)
        .max()
        .unwrap_or(0)
        .max(1);
    let window = load_test.window;
    let seconds = group.per_second.len();
    let affected: Vec<String> = group
        .per_target
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(index, count)| {
            let (method, name) = target_row_label(load_test, index);
            let method = method
                .map(|method| format!("{method} "))
                .unwrap_or_default();
            format!("{method}{name} {}", format_integer(*count))
        })
        .collect();
    let example = snapshot
        .error_examples
        .iter()
        .find(|example| example.status == group.status)
        .map(|example| example.text.clone());
    let status = group.status;
    let tab_id = tab_id.to_string();

    let detail_row = |label: String, value: AnyElement| {
        h_flex()
            .justify_between()
            .gap(px(12.0))
            .child(div().flex_none().child(label))
            .child(value)
    };

    v_flex()
        .border_1()
        .border_color(palette.border)
        .rounded(px(RADIUS_LG))
        .bg(palette.raised)
        .px(px(16.0))
        .py(px(14.0))
        .gap(px(10.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .w(px(66.0))
                        .flex_none()
                        .font_family(mono.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(error_code(group.status)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(error_title(group.status, timeout_secs)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format_integer(group.count)),
                )
                .child(
                    div()
                        .w(px(56.0))
                        .text_right()
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(format!("{} %", format_decimal(share, 2))),
                )
                .child(
                    Button::new(SharedString::from(format!(
                        "load-test-error-view-{status:?}"
                    )))
                    .neutral_bordered(
                        cx,
                        palette.raised,
                        palette.border_strong,
                        palette.accent_text,
                    )
                    .border_1()
                    .h(px(26.0))
                    .px(px(10.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(t!("load_test.errors.view_requests"))
                            .child(
                                Icon::new(gpui_kit::assets::IconName::ArrowRight)
                                    .with_size(px(12.0)),
                            ),
                    )
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.view_load_test_error_requests(&tab_id, status, cx);
                        });
                    }),
                ),
        )
        .child(
            h_flex()
                .items_end()
                .gap(px(16.0))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(6.0))
                        .child(
                            h_flex()
                                .h(px(SPARKLINE_HEIGHT))
                                .items_end()
                                .gap(px(1.0))
                                .children(bars.into_iter().map(|(count, first, last)| {
                                    let in_window = window
                                        .is_some_and(|(start, end)| last >= start && first <= end);
                                    let (height, bar) = if count > 0 {
                                        ((count as f32 / peak as f32).max(0.08), color)
                                    } else if in_window {
                                        (0.04, palette.border)
                                    } else {
                                        (0.04, palette.hover)
                                    };
                                    div().flex_1().h(relative(height)).rounded(px(1.0)).bg(bar)
                                })),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .font_family(mono.clone())
                                .text_size(px(10.5))
                                .text_color(palette.fg_subtle)
                                .child("0s")
                                .child(format!("{}s", format_integer(seconds as u64 / 2)))
                                .child(format!("{}s", format_integer(seconds as u64))),
                        ),
                )
                .child(
                    v_flex()
                        .w(px(DETAILS_WIDTH))
                        .flex_none()
                        .gap(px(4.0))
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(detail_row(
                            t!("load_test.errors.first_last").into_owned(),
                            div()
                                .font_family(mono.clone())
                                .text_color(palette.fg)
                                .child(format!(
                                    "{} \u{b7} {}",
                                    mmss(u64::from(group.first_ms) / 1_000),
                                    mmss(u64::from(group.last_ms) / 1_000)
                                ))
                                .into_any_element(),
                        ))
                        .child(detail_row(
                            t!("load_test.errors.affected").into_owned(),
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(mono.clone())
                                .text_size(px(11.0))
                                .child(affected.join("   "))
                                .into_any_element(),
                        )),
                ),
        )
        .children(example.map(|example| {
            div()
                .font_family(mono.clone())
                .text_size(px(12.0))
                .text_color(palette.fg_muted)
                .bg(palette.bg)
                .border_1()
                .border_color(palette.border)
                .rounded(px(7.0))
                .px(px(10.0))
                .py(px(8.0))
                .child(example)
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Named imports, not `super::*`: gpui's `test` attribute macro would shadow `#[test]`.
    use super::{StatusKey, error_title, sparkline_bars};
    use pretty_assertions::assert_eq;

    #[test]
    fn sparklines_add_up_long_runs() {
        let short = sparkline_bars(&[1, 0, 2]);
        assert_eq!(short, vec![(1, 0, 0), (0, 1, 1), (2, 2, 2)]);
        let long = sparkline_bars(&vec![1; 600]);
        assert_eq!(long.len(), 120);
        assert_eq!(long[0], (5, 0, 4));
    }

    #[test]
    fn error_titles() {
        assert_eq!(
            error_title(StatusKey::Code(429), 30),
            "429 Too Many Requests"
        );
        assert_eq!(error_title(StatusKey::Timeout, 5), "No response in 5 s");
    }
}
