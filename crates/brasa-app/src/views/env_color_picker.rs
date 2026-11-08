//! The color picker of an environment (#217, the design's "Environment Colors" page): the color
//! dot of the environment editor's header is a button that opens a popover with "Automatic" (the
//! color derived from the name) and the nine named colors. "Color..." in the editor's menu opens
//! the same popover. The choice is saved in `.brasa/project.toml`.

use gpui_kit::component::button::Button;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use brasa_workspace::ProjectWrite;
use rust_i18n::t;

use crate::state::env_color::{EnvColor, auto_color};
use crate::state::workspace_log::log_workspace_error;
use crate::theme::metrics::RADIUS_SM;
use crate::theme::{Palette, PaletteExt, BrasaColors};
use crate::views::components::{NeutralButtonExt, SectionLabel};

use super::root::AppView;

/// The width of the popover.
const PICKER_WIDTH: f32 = 248.0;
/// The side of a swatch button.
const SWATCH_SIZE: f32 = 28.0;
/// The side of the color inside a swatch.
const SWATCH_COLOR_SIZE: f32 = 20.0;
/// The side of the dot button in the editor's header.
const DOT_BUTTON_SIZE: f32 = 22.0;
/// Swatches per row.
const SWATCHES_PER_ROW: usize = 5;

impl AppView {
    /// Opens or closes the color picker of the active environment tab.
    pub(crate) fn set_env_color_picker_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.env_color_picker_open != open {
            self.env_color_picker_open = open;
            cx.notify();
        }
    }

    /// Saves `color` as the color of the environment `name`, or goes back to the automatic one
    /// with `None`. Choosing a color creates `.brasa/project.toml` when it does not exist; going
    /// back to automatic only edits an existing file.
    pub(crate) fn set_environment_color(
        &mut self,
        name: &str,
        color: Option<EnvColor>,
        cx: &mut Context<Self>,
    ) {
        self.env_color_picker_open = false;
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let write = if color.is_some() {
            ProjectWrite::Create
        } else {
            ProjectWrite::IfExists
        };
        let result = workspace.update_project(write, |config| {
            config.set_environment_color(name, color.map(EnvColor::key));
        });
        match result {
            Ok(()) => {
                log::debug!("changed the color of an environment");
                self.workspace_error = None;
                self.refresh_environments();
            }
            Err(error) => {
                log_workspace_error("save environment color", &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }
}

/// The header's color dot, a button that opens the picker of the environment `name`. `chosen` is
/// the color the user picked, `None` when it is automatic.
pub(crate) fn color_picker(
    name: &str,
    chosen: Option<EnvColor>,
    open: bool,
    weak: &WeakEntity<AppView>,
    palette: &Palette,
    cx: &mut App,
) -> AnyElement {
    let color = chosen.unwrap_or_else(|| auto_color(name));
    let label = match chosen {
        Some(color) => t!(color.label_key()),
        None => t!("environments.color.automatic"),
    };
    let tooltip = t!("environments.color.tooltip", color = label).into_owned();
    let trigger = Button::new("env-color-dot")
        .neutral_ghost(cx, palette.fg_muted)
        .w(px(DOT_BUTTON_SIZE))
        .h(px(DOT_BUTTON_SIZE))
        .px_0()
        .rounded(px(RADIUS_SM))
        .tooltip(tooltip)
        .child(
            div()
                .flex_none()
                .size(px(10.0))
                .rounded_full()
                .bg(palette.env_color(color)),
        );

    let name = name.to_string();
    let content_weak = weak.clone();
    let change_weak = weak.clone();
    Popover::new("env-color-popover")
        .w(px(PICKER_WIDTH))
        .p(px(12.0))
        .open(open)
        .on_open_change(move |open, _, cx| {
            let open = *open;
            let _ = change_weak.update(cx, |view, cx| view.set_env_color_picker_open(open, cx));
        })
        .trigger(trigger)
        .content(move |_, _, cx| render_picker(&name, chosen, content_weak.clone(), cx))
        .into_any_element()
}

/// The popover's content: "Automatic", a line, the swatches and where the choice is saved.
fn render_picker(
    name: &str,
    chosen: Option<EnvColor>,
    weak: WeakEntity<AppView>,
    cx: &mut Context<PopoverState>,
) -> Div {
    let palette = cx.palette();
    let auto = auto_color(name);

    let automatic = {
        let weak = weak.clone();
        let name = name.to_string();
        h_flex()
            .id("env-color-auto")
            .h(px(36.0))
            .px(px(8.0))
            .gap(px(10.0))
            .items_center()
            .rounded(px(RADIUS_SM))
            .cursor_pointer()
            .when(chosen.is_none(), |row| row.bg(palette.hover))
            .hover(|style| style.bg(palette.hover))
            .active(|style| style.bg(palette.pressed))
            .child(
                div()
                    .flex_none()
                    .size(px(18.0))
                    .rounded_full()
                    .border_1()
                    .border_color(palette.border_strong)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(palette.env_color(auto)),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_color(palette.fg)
                            .child(t!("environments.color.automatic")),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(palette.fg_muted)
                            .child(t!(
                                "environments.color.from_name",
                                color = t!(auto.label_key())
                            )),
                    ),
            )
            .when(chosen.is_none(), |row| {
                row.child(
                    Icon::new(IconName::Check)
                        .small()
                        .text_color(palette.accent_text),
                )
            })
            .on_click(cx.listener(move |popover, _, window, cx| {
                popover.dismiss(window, cx);
                let _ = weak.update(cx, |view, cx| view.set_environment_color(&name, None, cx));
            }))
    };

    let mut grid = v_flex().gap(px(8.0));
    for row_colors in EnvColor::ALL.chunks(SWATCHES_PER_ROW) {
        let mut row = h_flex().gap(px(16.0)).px(px(4.0));
        for &color in row_colors {
            row = row.child(swatch(
                name,
                color,
                chosen == Some(color),
                &weak,
                &palette,
                cx,
            ));
        }
        grid = grid.child(row);
    }

    v_flex()
        .gap(px(10.0))
        .child(SectionLabel::new(t!("environments.color.title")))
        .child(automatic)
        .child(div().h(px(1.0)).bg(palette.border))
        .child(grid)
        .child(
            div()
                .text_size(px(11.5))
                .text_color(palette.fg_muted)
                .child(t!("environments.color.saved_hint")),
        )
}

/// One color of the grid: a circle of the color, ringed when it is the chosen one.
fn swatch(
    name: &str,
    color: EnvColor,
    selected: bool,
    weak: &WeakEntity<AppView>,
    palette: &Palette,
    cx: &mut Context<PopoverState>,
) -> impl IntoElement {
    let fill = palette.env_color(color);
    let label = SharedString::from(t!(color.label_key()).into_owned());
    let weak = weak.clone();
    let name = name.to_string();
    div()
        .id(SharedString::from(format!("env-color-{}", color.key())))
        .flex_none()
        .size(px(SWATCH_SIZE))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .border_2()
        .border_color(if selected {
            fill
        } else {
            palette.overlay.opacity(0.0)
        })
        .hover({
            let ring = palette.border_strong;
            move |style| {
                if selected {
                    style
                } else {
                    style.border_color(ring)
                }
            }
        })
        .tooltip(move |window, cx| Tooltip::new(label.clone()).build(window, cx))
        .child(div().size(px(SWATCH_COLOR_SIZE)).rounded_full().bg(fill))
        .on_click(cx.listener(move |popover, _, window, cx| {
            popover.dismiss(window, cx);
            let _ = weak.update(cx, |view, cx| {
                view.set_environment_color(&name, Some(color), cx);
            });
        }))
}
