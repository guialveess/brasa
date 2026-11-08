//! The keyboard shortcuts cheat sheet: a centered modal listing an app's shortcuts by area, each
//! key drawn as a small chip. The app owns the list (its key bindings) and the action that opens
//! the sheet; this module owns the types, the keys that open it on each OS and the modal. Closed
//! with the `x`, Escape or a click on the backdrop, all handled by gpui-component's `Dialog`.

use std::rc::Rc;

use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::PaletteExt;
use crate::metrics::{RADIUS_SM, SHORTCUTS_HEIGHT, SHORTCUTS_KEY_SIZE, SHORTCUTS_WIDTH};

use super::modal::{ModalPlacement, framed_dialog, modal_header};
use super::section_label::SectionLabel;

/// The modifier key label, matching the usual key bindings: `Cmd` on macOS, `Ctrl` elsewhere.
#[cfg(target_os = "macos")]
pub const MODIFIER_KEY: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
pub const MODIFIER_KEY: &str = "Ctrl";

/// The keystroke to bind to the app's "open the cheat sheet" action: `F1`, or `Cmd+Shift+/` on
/// macOS, where `F1` is the brightness key unless `Fn` is held.
#[cfg(target_os = "macos")]
pub const OPEN_SHORTCUTS_KEYSTROKE: &str = "cmd-shift-/";
#[cfg(not(target_os = "macos"))]
pub const OPEN_SHORTCUTS_KEYSTROKE: &str = "f1";

/// One shortcut: what it does and the keys to press, one chip per key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// The translated action label.
    pub label: String,
    /// The keys, in the order they are pressed together (`["Ctrl", "K"]`).
    pub keys: Vec<String>,
}

impl Shortcut {
    /// A shortcut labeled `label`, pressing `keys` together.
    pub fn new(label: impl ToString, keys: &[&str]) -> Self {
        Self {
            label: label.to_string(),
            keys: keys.iter().map(|key| key.to_string()).collect(),
        }
    }

    /// The row for the cheat sheet itself, with telaio's own label and [`open_keys`].
    pub fn open_sheet() -> Self {
        Self {
            label: t!("telaio.shortcuts.open").to_string(),
            keys: open_keys(),
        }
    }
}

/// A titled list of shortcuts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutGroup {
    /// The translated group title.
    pub title: String,
    /// The group's shortcuts.
    pub shortcuts: Vec<Shortcut>,
}

/// The keys that open the cheat sheet, as chips: [`OPEN_SHORTCUTS_KEYSTROKE`] for display.
pub fn open_keys() -> Vec<String> {
    let keys: &[&str] = if cfg!(target_os = "macos") {
        &["Cmd", "Shift", "/"]
    } else {
        &["F1"]
    };
    keys.iter().map(|key| key.to_string()).collect()
}

/// [`open_keys`] as one string for a hint or a tooltip (`"F1"`, `"Cmd+Shift+/"`).
pub fn open_hint() -> String {
    open_keys().join("+")
}

/// The tooltip of a button that opens the cheat sheet: "Keyboard shortcuts (F1)".
pub fn open_tooltip() -> String {
    t!("telaio.shortcuts.tooltip", keys = open_hint()).to_string()
}

/// Opens the cheat sheet listing `groups`. A no-op when a dialog is already open, so the key
/// never stacks a second modal on top.
pub fn open_shortcuts_dialog(groups: Vec<ShortcutGroup>, window: &mut Window, cx: &mut App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let groups = Rc::new(groups);
    window.open_dialog(cx, move |dialog, window, _cx| {
        let groups = groups.clone();
        framed_dialog(
            dialog,
            SHORTCUTS_WIDTH,
            Some(SHORTCUTS_HEIGHT),
            ModalPlacement::Centered,
            window,
            move |window, cx| {
                let scroll_handle = window
                    .use_keyed_state("telaio-shortcuts-scroll", cx, |_, _| {
                        ScrollHandle::default()
                    })
                    .read(cx)
                    .clone();
                render_sheet(&groups, &scroll_handle, cx)
            },
        )
    });
}

/// The modal's content: the header and the scrollable list of groups.
fn render_sheet(groups: &[ShortcutGroup], scroll_handle: &ScrollHandle, cx: &App) -> AnyElement {
    let palette = cx.palette();
    // The scrollbar shows on the modal's own surface, with no track of its own.
    let track = palette.overlay.opacity(0.0);
    v_flex()
        .size_full()
        .min_h_0()
        .child(modal_header(t!("telaio.shortcuts.title"), cx))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(
                    v_flex()
                        .id("telaio-shortcuts-body")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(scroll_handle)
                        .p(px(20.0))
                        .gap(px(22.0))
                        .children(groups.iter().map(|group| render_group(group, cx))),
                )
                .child(
                    div().absolute().inset_0().child(
                        Scrollbar::vertical(scroll_handle)
                            .mode(ScrollbarMode::Always)
                            .styles(|styles| {
                                styles
                                    .track(|style| style.bg(track).border_color(track))
                                    .track_hover(|style| style.bg(track).border_color(track))
                                    .track_active(|style| style.bg(track).border_color(track))
                            }),
                    ),
                ),
        )
        .into_any_element()
}

/// One group: its title and a row per shortcut.
fn render_group(group: &ShortcutGroup, cx: &App) -> AnyElement {
    v_flex()
        .gap(px(6.0))
        .child(SectionLabel::new(group.title.clone()))
        .children(group.shortcuts.iter().map(|shortcut| {
            h_flex()
                .items_center()
                .justify_between()
                .gap(px(16.0))
                .py(px(5.0))
                .child(div().min_w_0().child(shortcut.label.clone()))
                .child(
                    h_flex()
                        .flex_none()
                        .gap(px(4.0))
                        .children(shortcut.keys.iter().map(|key| key_chip(key, cx))),
                )
        }))
        .into_any_element()
}

/// One key as a small chip.
fn key_chip(key: &str, cx: &App) -> AnyElement {
    let palette = cx.palette();
    div()
        .min_w(px(SHORTCUTS_KEY_SIZE))
        .h(px(SHORTCUTS_KEY_SIZE))
        .px(px(6.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_SM))
        .bg(palette.raised)
        .border_1()
        .border_color(palette.border_strong)
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(px(11.0))
        .text_color(palette.fg_muted)
        .child(key.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it brings gpui's own `test` attribute in through `gpui_kit::*`.
    #[cfg(not(target_os = "macos"))]
    use super::OPEN_SHORTCUTS_KEYSTROKE;
    use super::{MODIFIER_KEY, Shortcut, open_hint, open_keys};

    #[test]
    fn a_shortcut_keeps_its_keys_in_order() {
        let shortcut = Shortcut::new("Save", &[MODIFIER_KEY, "S"]);
        assert_eq!(shortcut.label, "Save");
        assert_eq!(
            shortcut.keys,
            vec![MODIFIER_KEY.to_string(), "S".to_string()]
        );
    }

    #[test]
    fn the_sheet_row_uses_the_open_keys() {
        let row = Shortcut::open_sheet();
        assert_eq!(row.keys, open_keys());
        assert!(!row.label.is_empty());
    }

    #[test]
    fn the_open_hint_joins_the_keys() {
        assert_eq!(open_hint(), open_keys().join("+"));
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(open_hint(), "F1");
            assert_eq!(OPEN_SHORTCUTS_KEYSTROKE, "f1");
        }
    }
}
