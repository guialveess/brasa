//! [`EmptyState`]: the centered placeholder of a panel with nothing to show yet (in Postino, the
//! response viewer before a request is sent and the load test dashboard before a first run).
//!
//! A plain styled `div`. Every text is passed in already translated, so the component reads no
//! locale. With only a title it is the bare muted line of those panels; an icon, an explanation,
//! a key hint and an action are optional additions.

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::fonts::MONO_FAMILY;
use crate::metrics::{
    EMPTY_STATE_COMPACT_PADDING, EMPTY_STATE_DESCRIPTION_MAX_WIDTH, EMPTY_STATE_ICON_SIZE,
};

/// A centered placeholder: optional icon, a title in `fg_muted`, an optional explanation in
/// `fg_subtle`, an optional monospace hint (a key combination) and an optional action (a button).
///
/// By default it fills its parent and centers its content both ways. [`Self::compact`] makes it
/// take only its own height, with vertical padding, for a placeholder inside a scrolling page.
#[derive(IntoElement)]
pub struct EmptyState {
    icon: Option<Icon>,
    title: SharedString,
    description: Option<SharedString>,
    hint: Option<SharedString>,
    action: Option<AnyElement>,
    compact: bool,
}

impl EmptyState {
    /// A placeholder showing `title`.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            title: title.into(),
            description: None,
            hint: None,
            action: None,
            compact: false,
        }
    }

    /// Shows `icon` above the title.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Adds an explanation under the title.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds a monospace hint under the text, for example the shortcut that fills the panel.
    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Adds an action under everything else, usually a button.
    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }

    /// Takes only its own height, plus vertical padding, instead of filling its parent.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
}

impl RenderOnce for EmptyState {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        v_flex()
            .items_center()
            .justify_center()
            .gap_2()
            .text_center()
            .when(self.compact, |this| {
                this.w_full().py(px(EMPTY_STATE_COMPACT_PADDING))
            })
            .when(!self.compact, |this| this.size_full())
            .children(self.icon.map(|icon| {
                icon.size(px(EMPTY_STATE_ICON_SIZE))
                    .text_color(palette.fg_subtle)
            }))
            .child(div().text_color(palette.fg_muted).child(self.title))
            .children(self.description.map(|description| {
                div()
                    .max_w(px(EMPTY_STATE_DESCRIPTION_MAX_WIDTH))
                    .text_size(px(12.0))
                    .text_color(palette.fg_subtle)
                    .child(description)
            }))
            .children(self.hint.map(|hint| {
                div()
                    .font_family(MONO_FAMILY)
                    .text_size(px(10.5))
                    .text_color(palette.fg_subtle)
                    .child(hint)
            }))
            .children(self.action.map(|action| div().mt_2().child(action)))
    }
}
