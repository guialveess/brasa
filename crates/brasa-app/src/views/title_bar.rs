//! The title bar: the workspace switcher, the search trigger, the environment pill and the
//! settings gear. No logo and no wordmark (the window title and the Dock/taskbar already carry
//! the name and icon; repeating them here was the one thing left that read as a straight copy of
//! the app this was forked from). The Import menu lives in the sidebar's header, next to "new
//! folder". The window controls at the far right are drawn automatically by gpui-kit's
//! `TitleBar`, so nothing here adds them. The search trigger opens the command palette
//! (`views/command_palette.rs`); the settings gear opens Settings (`views/settings.rs`).

use std::path::Path;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::state;
use crate::theme::Palette;
use crate::theme::PaletteExt;
use crate::theme::metrics::{ENV_PILL_HEIGHT, RADIUS_MD, SEARCH_TRIGGER_WIDTH, TITLE_BAR_HEIGHT};
use crate::views::components::{NeutralButtonExt, SectionLabel};

use super::root::AppView;
use super::{new_workspace, sidebar};

impl AppView {
    /// Renders the whole title bar's content.
    pub(crate) fn render_title_bar(
        &mut self,
        weak: WeakEntity<Self>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();

        // gpui-kit's own default (34px) felt cramped; the window controls stretch with it.
        let bar = TitleBar::new().h(px(TITLE_BAR_HEIGHT)).child(
            h_flex()
                .size_full()
                .items_center()
                .gap_2()
                // gpui-kit's `TitleBar` gives the left edge room for the traffic lights but adds
                // none on the right: without this the env pill and the gear sit flush against
                // the window's physical edge on macOS, where `WindowControls` renders nothing.
                .pr(px(16.0))
                .child(no_drag(self.render_workspace_switcher(
                    weak.clone(),
                    &palette,
                    cx,
                )))
                .child(div().flex_1())
                .child(no_drag(self.render_env_picker(weak.clone(), cx)))
                .child(no_drag(render_settings_gear(weak.clone(), &palette))),
        );

        // The search field is centered on the whole window, not in the gap between the side
        // groups: the environment pill changes width with the active environment's name, which
        // would otherwise move the field sideways. The overlay lives outside `TitleBar` because
        // the bar lays the window controls out beside its content, so centering inside the
        // content would be off by half of their width. The overlay has no handlers, so it does
        // not take mouse input away from the bar.
        div()
            .relative()
            .w_full()
            .flex_none()
            .child(bar)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(no_drag(render_search_trigger(weak, &palette, mono_font))),
            )
            .into_any_element()
    }

    /// Renders the workspace switcher: the folder icon, the open workspace's folder name (or a
    /// placeholder), a chevron, and its popover (design 1b of "Console & Workspace History",
    /// #193). The popover is rebuilt from [`Self::recent_workspaces`] on every render, so a
    /// workspace removed with its X disappears while the popover stays open.
    fn render_workspace_switcher(
        &self,
        weak: WeakEntity<Self>,
        palette: &Palette,
        cx: &App,
    ) -> AnyElement {
        let label = self
            .state
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.root().file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| t!("shell.title_bar.no_workspace").into_owned());

        let trigger = Button::new("workspace-switcher")
            .neutral_ghost(cx, palette.fg_muted)
            .h(px(ENV_PILL_HEIGHT))
            .px_2()
            .rounded(px(RADIUS_MD - 2.0))
            .text_color(palette.fg_muted)
            .icon(Icon::new(IconName::Folder).small())
            .label(label)
            .dropdown_caret(true);

        Popover::new("workspace-switcher-popover")
            .w(px(WORKSPACE_MENU_WIDTH))
            .p(px(6.0))
            .trigger(trigger)
            .content(move |_, _, cx| render_workspace_menu(weak.clone(), cx))
            .into_any_element()
    }

    /// Removes `path` from the recent workspace list, never from disk, and refreshes the list
    /// the switcher and the welcome tab show.
    fn forget_recent_workspace(&mut self, path: &Path, cx: &mut Context<Self>) {
        state::config::forget_workspace(path);
        self.recent_workspaces = state::config::recent_workspaces();
        log::debug!("removed a workspace from the recent list");
        cx.notify();
    }
}

/// Width of the workspace switcher's popover.
const WORKSPACE_MENU_WIDTH: f32 = 400.0;
/// Height of a recent workspace row: its name over its path.
const WORKSPACE_ROW_HEIGHT: f32 = 44.0;
/// Side of the X that removes a workspace from the recent list.
const WORKSPACE_FORGET_SIZE: f32 = 28.0;
/// Height of the "Open folder" and "New workspace" rows.
const WORKSPACE_ACTION_ROW_HEIGHT: f32 = 30.0;

/// The switcher popover's content: the "Recent" rows, then "Open folder..." and "New
/// workspace...". Reads the recent list from the app view, which `open_workspace_at` and
/// `forget_recent_workspace` keep up to date, so no file is read here.
fn render_workspace_menu(weak: WeakEntity<AppView>, cx: &mut Context<PopoverState>) -> Div {
    let palette = cx.palette();
    let mono_font = cx.theme().mono_font_family.clone();
    let (recent, current_path) = match weak.upgrade() {
        Some(view) => {
            let view = view.read(cx);
            (view.recent_workspaces.clone(), view.workspace_path.clone())
        }
        None => (Vec::new(), None),
    };
    let home = dirs::home_dir();
    let only_current = recent
        .iter()
        .all(|path| current_path.as_deref() == Some(path.as_path()));

    let rows = recent.into_iter().enumerate().map(|(index, path)| {
        let is_current = current_path.as_deref() == Some(path.as_path());
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let shortened = state::format::shorten_path(&path, home.as_deref());
        let open_weak = weak.clone();
        let forget_weak = weak.clone();
        let open_path = path.clone();

        let trailing = if is_current {
            div()
                .flex_none()
                .px_2()
                .text_size(px(11.0))
                .text_color(palette.fg_subtle)
                .child(t!("shell.title_bar.open_label"))
                .into_any_element()
        } else {
            let fg = palette.fg;
            let hover = palette.hover;
            let pressed = palette.pressed;
            let tooltip = SharedString::from(t!("shell.title_bar.remove_recent").into_owned());
            div()
                .id(("workspace-forget", index))
                .flex_none()
                .size(px(WORKSPACE_FORGET_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(RADIUS_MD - 2.0))
                .cursor_pointer()
                .text_color(palette.fg_subtle)
                .hover(move |style| style.bg(hover).text_color(fg))
                .active(move |style| style.bg(pressed).text_color(fg))
                .child(Icon::new(IconName::Close).with_size(px(14.0)))
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .on_click(move |_, _, cx| {
                    // The row under the button opens the workspace on click: keep it out.
                    cx.stop_propagation();
                    let _ =
                        forget_weak.update(cx, |view, cx| view.forget_recent_workspace(&path, cx));
                })
                .into_any_element()
        };

        h_flex()
            .id(("workspace-recent", index))
            .h(px(WORKSPACE_ROW_HEIGHT))
            .items_center()
            .gap(px(10.0))
            .pl_2()
            .pr_1()
            .rounded(px(RADIUS_MD - 2.0))
            .cursor_pointer()
            .when(is_current, |row| row.bg(palette.list_active))
            .hover(|style| style.bg(palette.hover))
            .active(|style| style.bg(palette.pressed))
            .child(if is_current {
                Icon::new(IconName::Check)
                    .small()
                    .text_color(palette.accent_text)
            } else {
                Icon::new(IconName::Folder)
                    .small()
                    .text_color(palette.fg_muted)
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(div().truncate().text_color(palette.fg).child(name))
                    .child(
                        div()
                            .truncate()
                            .font_family(mono_font.clone())
                            .text_size(px(11.0))
                            .text_color(palette.fg_subtle)
                            .child(shortened),
                    ),
            )
            .child(trailing)
            .on_click(cx.listener(move |popover, _, window, cx| {
                popover.dismiss(window, cx);
                if !is_current {
                    let _ = open_weak.update(cx, |view, cx| {
                        view.open_workspace_at(&open_path, window, cx)
                    });
                }
            }))
    });
    let rows: Vec<_> = rows.collect();

    let open_weak = weak.clone();
    let new_weak = weak;
    v_flex()
        .text_sm()
        .child(
            div()
                .px_2()
                .pt(px(6.0))
                .pb(px(4.0))
                .child(SectionLabel::new(t!("shell.title_bar.recent"))),
        )
        .children(rows)
        .when(only_current, |menu| {
            menu.child(
                div()
                    .p_2()
                    .text_size(px(12.0))
                    .text_color(palette.fg_subtle)
                    .child(t!("shell.title_bar.no_other_recent")),
            )
        })
        .child(div().h(px(1.0)).my(px(6.0)).bg(palette.border))
        .child(
            action_row("workspace-open-folder", Lucide::FolderOpen, &palette)
                .child(div().flex_1().child(t!("common.open_folder")))
                .child(
                    div()
                        .font_family(mono_font)
                        .text_size(px(11.0))
                        .text_color(palette.fg_subtle)
                        .child(modifier_hint('O')),
                )
                .on_click(cx.listener(move |popover, _, window, cx| {
                    popover.dismiss(window, cx);
                    sidebar::pick_workspace_folder(open_weak.clone(), window, cx);
                })),
        )
        .child(
            action_row("workspace-new", Lucide::FolderPlus, &palette)
                .child(div().flex_1().child(t!("common.new_workspace")))
                .on_click(cx.listener(move |popover, _, window, cx| {
                    popover.dismiss(window, cx);
                    new_workspace::open_new_workspace_dialog(new_weak.clone(), window, cx);
                })),
        )
}

/// A one-line row of the switcher popover with a leading icon, and the hover and pressed
/// backgrounds.
fn action_row(id: &'static str, icon: Lucide, palette: &Palette) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(WORKSPACE_ACTION_ROW_HEIGHT))
        .items_center()
        .gap(px(10.0))
        .px_2()
        .rounded(px(RADIUS_MD - 2.0))
        .cursor_pointer()
        .text_color(palette.fg)
        .hover(|style| style.bg(palette.hover))
        .active(|style| style.bg(palette.pressed))
        .child(Icon::new(icon).small().text_color(palette.fg_muted))
}

/// Wraps an interactive title bar control so the mouse reaches it. `TitleBar` marks the whole bar
/// as a window drag area, and on Windows gpui answers `WM_NCHITTEST` with `HTCAPTION` for any
/// point whose hit test includes that area, so the system eats the clicks and hovers of every
/// control inside it. Occluding the control keeps the bar's area out of the hit test there.
fn no_drag(control: impl IntoElement) -> impl IntoElement {
    div().occlude().child(control)
}

/// The "B" logo square: 20px, radius 8, `accent` background, `accent_fg` text.
/// The `group` name of the search trigger, so its key hint follows the trigger's hover.
const SEARCH_TRIGGER_GROUP: &str = "title-bar-search-trigger";

/// The centered search trigger: opens the command palette (`views/command_palette.rs`). Hovered,
/// it takes an `accent` border with a soft `accent` glow around it, and its text and key hint step
/// up one token (`fg_muted`, `border_strong`); pressed, it takes the focused input look of the URL
/// bar (`accent` border and a 3 px `accent_subtle` ring), since releasing it focuses the palette's
/// search field.
fn render_search_trigger(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    mono_font: SharedString,
) -> impl IntoElement {
    let border_strong = palette.border_strong;
    let fg_muted = palette.fg_muted;
    let accent = palette.accent;
    let accent_subtle = palette.accent_subtle;
    h_flex()
        .id("title-bar-search-trigger")
        .group(SEARCH_TRIGGER_GROUP)
        .w(px(SEARCH_TRIGGER_WIDTH))
        .h(px(ENV_PILL_HEIGHT))
        .items_center()
        .gap_2()
        .px_2p5()
        .cursor_pointer()
        .rounded(px(RADIUS_MD - 1.0))
        .bg(palette.raised)
        .border_1()
        .border_color(palette.border)
        .text_color(palette.fg_subtle)
        .hover(move |style| {
            style
                .border_color(accent)
                .text_color(fg_muted)
                .shadow(search_glow(accent))
        })
        .active(move |style| {
            style.border_color(accent).shadow(vec![
                BoxShadow::new(px(0.0), px(0.0), accent_subtle).spread_radius(px(3.0)),
            ])
        })
        .child(Icon::new(IconName::Search).small())
        .child(div().flex_1().child(t!("shell.title_bar.search_hint")))
        .child(
            div()
                .font_family(mono_font)
                .text_size(px(11.0))
                .border_1()
                .border_color(palette.border)
                .group_hover(SEARCH_TRIGGER_GROUP, move |style| {
                    style.border_color(border_strong)
                })
                .rounded(px(4.0))
                .px(px(4.0))
                .child(modifier_hint('K')),
        )
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |view, cx| view.open_command_palette(window, cx));
        })
}

/// The hover glow of the search trigger: a close and a wider layer of `accent`, centered on the
/// border, so the light fades out softly around it.
fn search_glow(accent: Hsla) -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.0), px(0.0), accent.opacity(0.2)).blur_radius(px(6.0)),
        BoxShadow::new(px(0.0), px(0.0), accent.opacity(0.1)).blur_radius(px(18.0)),
    ]
}

/// The settings gear: opens the Settings modal (`views/settings.rs`).
fn render_settings_gear(weak: WeakEntity<AppView>, palette: &Palette) -> impl IntoElement {
    div()
        .id("title-bar-settings-gear")
        .size(px(ENV_PILL_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .rounded(px(RADIUS_MD - 2.0))
        .text_color(palette.fg_muted)
        .hover(|style| style.bg(palette.hover))
        .active(|style| style.bg(palette.pressed))
        .child(Icon::new(IconName::Settings).with_size(px(16.0)))
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |view, cx| view.open_settings(window, cx));
        })
}

/// `"Ctrl+<key>"` (`"Cmd+<key>"` on macOS): the key hints of the search trigger and the
/// switcher's "Open folder..." row.
fn modifier_hint(key: char) -> String {
    let modifier = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    format!("{modifier}+{key}")
}
