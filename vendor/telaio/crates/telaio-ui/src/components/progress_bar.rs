//! [`ProgressBar`]: a thin track with a filled part, as in the header of Postino's load test run.
//!
//! No animation: the caller gives the value and re-renders when it changes, so an idle window
//! does no work.

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::metrics::PROGRESS_BAR_HEIGHT;
use crate::{PaletteExt, Tone};

/// What colors the filled part.
#[derive(Debug, Clone, Copy)]
enum Fill {
    Accent,
    Tone(Tone),
    Custom(Hsla),
}

/// A progress bar: a `surface` track with a `border` outline and a filled part covering `value`
/// of it. The fill is the accent by default, or a [`Tone`], or any color.
///
/// The bar takes the free width of its row (`flex_1`), so put it in an `h_flex`, or give its
/// parent a width. [`Self::min_width`] makes a narrow row wrap instead of squeezing it to a stub.
#[derive(IntoElement)]
pub struct ProgressBar {
    value: f32,
    fill: Fill,
    min_width: Option<f32>,
}

impl ProgressBar {
    /// A bar filled `value` of the way, from 0 (empty) to 1 (full). Values outside the range are
    /// clamped, and NaN counts as 0.
    pub fn new(value: f32) -> Self {
        Self {
            value: clamp_progress(value),
            fill: Fill::Accent,
            min_width: None,
        }
    }

    /// Fills the bar with the foreground color of `tone`, see [`crate::Palette::tone_colors`]
    /// (success, warning, danger, ...).
    pub fn tone(mut self, tone: Tone) -> Self {
        self.fill = Fill::Tone(tone);
        self
    }

    /// Fills the bar with `color`, for a color outside the palette's tones.
    pub fn color(mut self, color: Hsla) -> Self {
        self.fill = Fill::Custom(color);
        self
    }

    /// The narrowest the bar gets, in pixels.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// The value shown, after clamping.
    pub fn value(&self) -> f32 {
        self.value
    }
}

/// `value` limited to 0 through 1, with NaN as 0.
pub fn clamp_progress(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

impl RenderOnce for ProgressBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let color = match self.fill {
            Fill::Accent => palette.accent,
            Fill::Tone(tone) => palette.tone_colors(tone).0,
            Fill::Custom(color) => color,
        };
        div()
            .flex_1()
            .when_some(self.min_width, |bar, width| bar.min_w(px(width)))
            .h(px(PROGRESS_BAR_HEIGHT))
            .rounded(px(PROGRESS_BAR_HEIGHT / 2.0))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .overflow_hidden()
            .child(div().h_full().w(relative(self.value)).bg(color))
    }
}

#[cfg(test)]
mod tests {
    use super::{ProgressBar, clamp_progress};

    #[test]
    fn values_are_clamped_to_the_unit_range() {
        assert_eq!(clamp_progress(-0.5), 0.0);
        assert_eq!(clamp_progress(0.25), 0.25);
        assert_eq!(clamp_progress(7.0), 1.0);
        assert_eq!(clamp_progress(f32::NAN), 0.0);
        assert_eq!(ProgressBar::new(1.5).value(), 1.0);
    }
}
