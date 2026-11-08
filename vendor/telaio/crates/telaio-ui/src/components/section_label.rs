//! [`SectionLabel`]: the section heading used for "Collections" and every gallery section title.
//!
//! Custom element: a plain styled `div`, since this is just fixed typography with no
//! interactivity.
//!
//! Deviation: the design's 0.06em letter-spacing ("tracking") has no equivalent in gpui's text
//! style (`gpui-pre-0.3.6/src/style.rs` has no letter-spacing field), so this renders without
//! it.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;

/// An 11px/600 heading in sentence case, colored `fg_subtle` (section titles such as the
/// sidebar's "Collections").
#[derive(IntoElement)]
pub struct SectionLabel {
    text: SharedString,
}

impl SectionLabel {
    /// A label showing `text` as given: write it in sentence case ("Recent files").
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for SectionLabel {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        div()
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.fg_subtle)
            .child(self.text)
    }
}
