//! Rich rows for gpui-component's `PopupMenu`: a choice with a check and a detail, an item with a
//! tag and a description, a checkbox that keeps the menu open, and an item with a shortcut hint
//! (in Postino, the load test "Compare with" and "Export" menus).
//!
//! Each function returns a `PopupMenuItem` built with `PopupMenuItem::element`, so the menu is the
//! real `PopupMenu` (keyboard navigation, submenus, dismissal) and the rows only draw the
//! content. Build the menu in the `dropdown_menu` callback of a button or a trigger:
//!
//! ```ignore
//! button.dropdown_menu(|menu, _window, _cx| {
//!     rich_menu_width(menu, 240.0)
//!         .item(menu_choice_item("Previous run", Some("yesterday".into()), true, |_, _| {}))
//!         .item(menu_choice_item("Baseline", None, false, |_, _| {}))
//! })
//! ```
//!
//! Set the menu's width with [`rich_menu_width`], or description lines cannot wrap. None of the
//! rows sets an icon or a check on the item itself, because the menu would then reserve a left
//! column for every row: the rows draw their own.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::fonts::MONO_FAMILY;
use crate::metrics::{
    RADIUS_XS, RICH_MENU_CHECK_SIZE, RICH_MENU_DETAIL_SIZE, RICH_MENU_ROW_HEIGHT,
    RICH_MENU_TAG_SIZE, RICH_MENU_TEXT_SIZE,
};

/// Space between the parts of a row.
const ROW_GAP: f32 = 8.0;
/// The horizontal space a gpui-component popup menu takes around an item's own element: the
/// list's 4 px padding and the item's 8 px padding, on each side.
const MENU_CHROME_X: f32 = 24.0;
/// Space between the icon and the text of a described row.
const DESCRIBED_GAP: f32 = 10.0;
/// Padding added to the menu's own 8 px item padding, so a row's content sits 10 px in.
const ROW_PADDING_X: f32 = 2.0;
/// Vertical padding of a described row.
const DESCRIBED_PADDING_Y: f32 = 8.0;
/// Vertical padding of a toggle row.
const TOGGLE_PADDING_Y: f32 = 6.0;
/// Size of the check mark of a choice row (and the column it takes).
const CHOICE_CHECK_SIZE: f32 = 13.0;
/// Size of the icon of a described row.
const DESCRIBED_ICON_SIZE: f32 = 15.0;
/// Size of the icon of a shortcut row.
const SHORTCUT_ICON_SIZE: f32 = 14.0;
/// Size of the check mark inside the box of a toggle row.
const TOGGLE_MARK_SIZE: f32 = 10.0;

/// A click handler of a row.
type RowHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Fixes the width of `menu` to `width` pixels (both its least and its greatest), which is what
/// lets the description of a [`menu_described_item`] wrap instead of widening the menu.
pub fn rich_menu_width(menu: PopupMenu, width: f32) -> PopupMenu {
    menu.min_w(px(width)).max_w(px(width))
}

/// The common start of a row: it fills the menu's item, is inset like the design's 10 px rows and
/// uses the menu text size.
fn row() -> Div {
    h_flex()
        .flex_1()
        .min_w_0()
        .px(px(ROW_PADDING_X))
        .gap(px(ROW_GAP))
        .text_size(px(RICH_MENU_TEXT_SIZE))
}

/// A one line choice row, 30 px high: a 13 px check in `accent_text` shown only when `selected`
/// (its column is always there, so the labels line up), the `label` and, right aligned in
/// `fg_subtle`, an optional `detail` (a date, a rate). Picking it calls `on_click` and closes the
/// menu.
pub fn menu_choice_item(
    label: impl Into<SharedString>,
    detail: Option<SharedString>,
    selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |_window, cx| {
        let palette = cx.palette();
        row()
            .h(px(RICH_MENU_ROW_HEIGHT))
            .items_center()
            .child(
                div()
                    .size(px(CHOICE_CHECK_SIZE))
                    .flex_none()
                    .children(selected.then(|| {
                        Icon::new(gpui_kit::assets::IconName::Check)
                            .with_size(px(CHOICE_CHECK_SIZE))
                            .text_color(palette.accent_text)
                    })),
            )
            .child(div().flex_1().min_w_0().truncate().child(label.clone()))
            .children(detail.clone().map(|detail| {
                div()
                    .flex_none()
                    .text_size(px(RICH_MENU_DETAIL_SIZE))
                    .text_color(palette.fg_subtle)
                    .child(detail)
            }))
    })
    .on_click(move |_, window, cx| on_click(window, cx))
}

/// A row with a 15 px `accent_text` icon, a medium `title` with a right aligned monospace `tag`
/// (a file extension such as ".md") and a wrapping `description` in `fg_muted` under them.
/// Picking it calls `on_click` and closes the menu.
///
/// `menu_width` is the width given to [`rich_menu_width`]: gpui-component lays an element row
/// out at its content's width, so the description only wraps inside a row of a fixed width.
pub fn menu_described_item(
    menu_width: f32,
    icon: impl Into<Icon>,
    title: impl Into<SharedString>,
    tag: Option<SharedString>,
    description: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let icon = icon.into();
    let title = title.into();
    let description = description.into();
    PopupMenuItem::element(move |_window, cx| {
        let palette = cx.palette();
        row()
            // `flex_none`: the row's `flex_1` basis would otherwise win over the width.
            .flex_none()
            .w(px((menu_width - MENU_CHROME_X).max(0.0)))
            .items_start()
            .gap(px(DESCRIBED_GAP))
            .py(px(DESCRIBED_PADDING_Y))
            .child(
                // One pixel down, so the icon sits on the title's line.
                div().mt(px(1.0)).flex_none().child(
                    icon.clone()
                        .with_size(px(DESCRIBED_ICON_SIZE))
                        .text_color(palette.accent_text),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(
                        h_flex()
                            .justify_between()
                            .gap(px(ROW_GAP))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(title.clone()),
                            )
                            .children(tag.clone().map(|tag| {
                                div()
                                    .flex_none()
                                    .font_family(MONO_FAMILY)
                                    .text_size(px(RICH_MENU_TAG_SIZE))
                                    .text_color(palette.fg_subtle)
                                    .child(tag)
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(RICH_MENU_DETAIL_SIZE))
                            .text_color(palette.fg_muted)
                            .child(description.clone()),
                    ),
            )
    })
    .on_click(move |_, window, cx| on_click(window, cx))
}

/// A checkbox row (a 14 px box, filled with the accent and a check while `checked`, then the
/// `label` in `fg_muted`) that toggles without closing the menu.
///
/// `PopupMenu` closes after every item it activates, so the row handles the mouse click itself and
/// stops it before the menu sees it, then redraws. It keeps its own checked state from the
/// `checked` it was built with, so the box flips at once; `on_toggle` receives the new value and
/// the app stores it for the next time it builds the menu. The keyboard (Enter on the highlighted
/// row) is the menu's own activation, which toggles and then closes.
pub fn menu_toggle_item(
    label: impl Into<SharedString>,
    checked: bool,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let label = label.into();
    let state = Rc::new(Cell::new(checked));
    let on_toggle = Rc::new(on_toggle);
    let toggle = {
        let state = state.clone();
        move |window: &mut Window, cx: &mut App| {
            let checked = !state.get();
            state.set(checked);
            on_toggle(checked, window, cx);
            window.refresh();
        }
    };
    let on_key = toggle.clone();
    PopupMenuItem::element(move |_window, cx| {
        let palette = cx.palette();
        let checked = state.get();
        let toggle = toggle.clone();
        let mut check_box = div()
            .size(px(RICH_MENU_CHECK_SIZE))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(RADIUS_XS));
        check_box = if checked {
            check_box.bg(palette.accent).child(
                Icon::new(gpui_kit::assets::IconName::Check)
                    .with_size(px(TOGGLE_MARK_SIZE))
                    .text_color(palette.accent_fg),
            )
        } else {
            check_box.border_1().border_color(palette.border_strong)
        };
        row()
            .id("telaio-menu-toggle")
            .items_center()
            .py(px(TOGGLE_PADDING_Y))
            .text_size(px(RICH_MENU_DETAIL_SIZE))
            .text_color(palette.fg_muted)
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                // Before the menu's own item gets the click: it would close the menu.
                cx.stop_propagation();
                toggle(window, cx);
            })
            .child(check_box)
            .child(div().min_w_0().child(label.clone()))
    })
    .on_click(move |_, window, cx| on_key(window, cx))
}

/// A one line row, 30 px high: a 14 px `fg_muted` `icon`, the `label` and, right aligned in
/// `fg_subtle` and the monospace font, a shortcut `hint` such as "Ctrl+Shift+C". The hint is only
/// shown, the app binds the shortcut. Picking it calls `on_click` and closes the menu.
pub fn menu_shortcut_item(
    icon: impl Into<Icon>,
    label: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let icon = icon.into();
    let label = label.into();
    let hint = hint.into();
    let on_click: RowHandler = Rc::new(on_click);
    PopupMenuItem::element(move |_window, cx| {
        let palette = cx.palette();
        row()
            .h(px(RICH_MENU_ROW_HEIGHT))
            .items_center()
            .gap(px(DESCRIBED_GAP))
            .child(
                icon.clone()
                    .with_size(px(SHORTCUT_ICON_SIZE))
                    .text_color(palette.fg_muted),
            )
            .child(div().flex_1().min_w_0().truncate().child(label.clone()))
            .child(
                div()
                    .flex_none()
                    .font_family(MONO_FAMILY)
                    .text_size(px(RICH_MENU_TAG_SIZE))
                    .text_color(palette.fg_subtle)
                    .child(hint.clone()),
            )
    })
    .on_click(move |_, window, cx| on_click(window, cx))
}
