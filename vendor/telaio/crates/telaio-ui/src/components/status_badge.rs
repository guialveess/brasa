//! [`StatusBadge`]: a pill with a colored dot and a label on a subtle background (in Postino the
//! response status, in Vano a category or "Safe to remove").
//!
//! Custom element for the dot-plus-text pill (no gpui-kit widget matches its shape); a busy badge
//! wraps gpui-kit's own [`Spinner`] in place of the dot.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::metrics::{RADIUS_SM, STATUS_BADGE_HEIGHT};
use crate::{PaletteExt, Tone};

/// The badge's colors: a [`Tone`] of the palette, or three given ones.
#[derive(Debug, Clone, Copy)]
enum Colors {
    Tone(Tone),
    Custom { dot: Hsla, fg: Hsla, bg: Hsla },
}

/// A colored dot (or a spinner, while [`Self::busy`]) plus a text label, on a `*-subtle`
/// background. The colors come from a [`Tone`] ([`Tone::Neutral`] by default) through
/// [`crate::Palette::tone_colors`], or from [`Self::colors`].
#[derive(IntoElement)]
pub struct StatusBadge {
    label: SharedString,
    colors: Colors,
    busy: bool,
}

impl StatusBadge {
    /// A neutral badge showing `label`.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            colors: Colors::Tone(Tone::Neutral),
            busy: false,
        }
    }

    /// Colors the badge with `tone`.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.colors = Colors::Tone(tone);
        self
    }

    /// Colors the badge with a `dot`, a label `fg` and a `bg`, for colors outside the palette's
    /// tones (a category color).
    pub fn colors(mut self, dot: Hsla, fg: Hsla, bg: Hsla) -> Self {
        self.colors = Colors::Custom { dot, fg, bg };
        self
    }

    /// Shows a spinner in place of the dot, for a state still in progress ("Sending...").
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
}

impl RenderOnce for StatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (dot, fg, bg) = match self.colors {
            Colors::Tone(tone) => {
                let (fg, bg) = cx.palette().tone_colors(tone);
                (fg, fg, bg)
            }
            Colors::Custom { dot, fg, bg } => (dot, fg, bg),
        };

        let marker: AnyElement = if self.busy {
            Spinner::new().color(dot).into_any_element()
        } else {
            div()
                .flex_none()
                .size(px(6.0))
                .rounded(px(3.0))
                .bg(dot)
                .into_any_element()
        };

        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.0))
            .h(px(STATUS_BADGE_HEIGHT))
            .px_2()
            .rounded(px(RADIUS_SM))
            .bg(bg)
            .text_size(px(12.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(fg)
            .child(marker)
            .child(self.label)
    }
}
