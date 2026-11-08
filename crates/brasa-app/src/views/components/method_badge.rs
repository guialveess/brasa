//! [`MethodBadge`]: the colored HTTP method label used in the sidebar tree, open tabs and the
//! Components gallery.
//!
//! Custom element: no gpui-kit widget renders mono, fixed-width, colored text with an optional
//! tinted background pill, so this is a plain styled `div`, colored only from
//! [`crate::theme::Palette`] through [`PaletteExt`].

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use brasa_core::Method;

use crate::theme::metrics::{METHOD_LABEL_WIDTH, RADIUS_SM};
use crate::theme::{PaletteExt, BrasaColors};

/// Which of the two design variants a [`MethodBadge`] renders as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MethodBadgeVariant {
    /// Mono 10/600 colored text, right aligned in a [`METHOD_LABEL_WIDTH`]-wide column so the
    /// names next to it line up while the badge stays close to its name (sidebar tree).
    Label,
    /// The same text at its natural width, for a spot with no column to align (open tabs).
    Inline,
    /// The same text on a 14%-alpha background pill (Components "Methods").
    Pill,
}

/// A colored HTTP method label, in the `label` or `pill` variant.
#[derive(IntoElement)]
pub struct MethodBadge {
    method: Method,
    variant: MethodBadgeVariant,
}

impl MethodBadge {
    /// A `label` badge for `method`: mono 10/600 colored text, fixed width, no background.
    /// Used in the sidebar tree and the open-tabs bar.
    pub fn label(method: Method) -> Self {
        Self {
            method,
            variant: MethodBadgeVariant::Label,
        }
    }

    /// An `inline` badge for `method`: like [`Self::label`] but at its natural width. Used in the
    /// open-tabs bar, where there is no name column to line up.
    pub fn inline(method: Method) -> Self {
        Self {
            method,
            variant: MethodBadgeVariant::Inline,
        }
    }

    /// A `pill` badge for `method`: the same text on a 14%-alpha tinted background.
    pub fn pill(method: Method) -> Self {
        Self {
            method,
            variant: MethodBadgeVariant::Pill,
        }
    }
}

impl RenderOnce for MethodBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let color = palette.method_color(&self.method);
        let label = self.method.to_string();
        let mono_font = cx.theme().mono_font_family.clone();

        let text = div()
            .flex_none()
            .whitespace_nowrap()
            .font_family(mono_font)
            .font_weight(FontWeight::SEMIBOLD)
            .text_size(px(10.0))
            .text_color(color)
            .child(label);

        match self.variant {
            MethodBadgeVariant::Label => div()
                .flex_none()
                .min_w(px(METHOD_LABEL_WIDTH))
                .flex()
                .justify_end()
                .child(text),
            MethodBadgeVariant::Inline => div().flex_none().child(text),
            MethodBadgeVariant::Pill => div()
                .flex_none()
                .rounded(px(RADIUS_SM))
                .bg(palette.method_badge_bg(&self.method))
                .px_1p5()
                .py(px(2.0))
                .child(text),
        }
    }
}
