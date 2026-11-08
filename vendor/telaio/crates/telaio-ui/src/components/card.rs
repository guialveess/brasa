//! [`Card`]: a raised panel with a border and radius 10.
//!
//! Custom element: a plain styled `div` wrapper, since a "card" here is just a bordered,
//! `raised` container with no behavior of its own.

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::RADIUS_LG;

/// A `raised`, bordered container, radius 10, as in `docs/design-system.md`.
#[derive(IntoElement)]
pub struct Card {
    children: Vec<AnyElement>,
}

impl Card {
    /// An empty card; add content with [`gpui::ParentElement::child`]/`children`.
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

impl ParentElement for Card {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Card {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        v_flex()
            .p_4()
            .gap_4()
            .rounded(px(RADIUS_LG))
            .border_1()
            .border_color(palette.border)
            .bg(palette.raised)
            .children(self.children)
    }
}
