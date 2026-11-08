//! [`TextArea`]: a multi line plain text input with the theme's look, the app's edit menu and
//! the hover border of a [`super::HoverInput`].
//!
//! The text, the placeholder and an auto height live on the [`TextareaState`], built by the app:
//!
//! ```ignore
//! // Grows with its text from 3 to 10 lines, then scrolls.
//! let notes = cx.new(|cx| {
//!     TextareaState::new(window, cx).placeholder("Notes").auto_grow(3, 10)
//! });
//! // Or a fixed height, set on the element: `TextArea::new(&notes).h(px(120.))`.
//! ```
//!
//! The text scrolls inside the area with gpui-component's own scrollbar.

use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::edit_menu::edit_menu;
use crate::PaletteExt as _;

/// A multi line plain text input for `state`. Its height follows the state's auto grow rows
/// unless [`Self::h`] fixes it.
#[derive(IntoElement)]
pub struct TextArea {
    state: Entity<TextareaState>,
    height: Option<DefiniteLength>,
    readonly: bool,
    disabled: bool,
}

impl TextArea {
    /// The area for `state`.
    pub fn new(state: &Entity<TextareaState>) -> Self {
        Self {
            state: state.clone(),
            height: None,
            readonly: false,
            disabled: false,
        }
    }

    /// Fixes the height of the area, in pixels or relative to its parent. The text scrolls when
    /// it is taller.
    pub fn h(mut self, height: impl Into<DefiniteLength>) -> Self {
        self.height = Some(height.into());
        self
    }

    /// Whether the text can be selected and copied but not changed. Set here, not on the state:
    /// the element applies its own value to the state on every render.
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    /// Whether the area is disabled: dimmed, not focusable, no edit menu actions.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for TextArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state;
        let id = ("text-area", state.entity_id());
        let hovered = window.use_keyed_state(id, cx, |_, _| false);
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        let palette = cx.palette();
        let border = if *hovered.read(cx) && !self.disabled {
            palette.border_strong
        } else {
            palette.border
        };
        div()
            .id(id)
            .w_full()
            .on_hover(move |is_hovered, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    if *hovered != *is_hovered {
                        *hovered = *is_hovered;
                        cx.notify();
                    }
                });
            })
            .child(
                Textarea::new(&state)
                    .context_menu(edit_menu(&state, cx))
                    .readonly(self.readonly)
                    .disabled(self.disabled)
                    .when_some(self.height, |area, height| area.h(height))
                    // A focused area keeps the border of its own focus style.
                    .when(!focused, |area| area.border_color(border)),
            )
    }
}
