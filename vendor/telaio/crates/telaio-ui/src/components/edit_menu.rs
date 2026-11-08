//! [`edit_menu`]: the right-click menu of a text field (Cut, Copy, Paste, Select all) with
//! translated labels. gpui-component's built-in menu reads its labels from its own locale files,
//! which have no Spanish or Galician and no Italian for these items, so every `Input`, `Textarea`
//! and `Editor` in the app replaces it with this one through their `context_menu` builder.

use gpui_kit::base::input::{InputBaseState, InputModeKind};
use gpui_kit::component::input::{Copy, Cut, Paste, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::*;
use rust_i18n::t;

/// Builds the menu for the field backed by `state`, with the same items and disabled rules as
/// gpui-component's own: Cut and Copy need a selection (and an unmasked value), Cut and Paste
/// need an editable field. The builder runs while `state` is being updated, where reading it
/// panics, so those rules are read here, at render time, and a right-click sees the selection
/// of the last frame. Both reads are O(1).
pub fn edit_menu<M: InputModeKind>(
    state: &Entity<InputBaseState<M>>,
    cx: &App,
) -> impl Fn(NativeMenu, &mut Window, &mut App) -> NativeMenu + 'static {
    let state = state.read(cx);
    let presentation = state.presentation();
    let editable = presentation.is_editable();
    let copyable = !state.selected_range().is_empty() && !presentation.is_masked();
    move |menu, _, _| {
        menu.menu_with_disabled(
            t!("telaio.edit_menu.cut"),
            !(editable && copyable),
            Box::new(Cut),
        )
        .menu_with_disabled(t!("telaio.edit_menu.copy"), !copyable, Box::new(Copy))
        .menu_with_disabled(t!("telaio.edit_menu.paste"), !editable, Box::new(Paste))
        .separator()
        .menu(t!("telaio.edit_menu.select_all"), Box::new(SelectAll))
    }
}
