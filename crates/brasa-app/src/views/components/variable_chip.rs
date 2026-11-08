//! [`VariableChip`]: a standalone rendering of a `{{ name }}` marker, and [`highlight_style`], the
//! shared [`HighlightStyle`] builder [`super::url_bar::UrlBar`] reuses to style the same two
//! variants as spans inside a longer line of text.
//!
//! Custom element: a defined variable is a small `accent_subtle` chip (an ordinary styled
//! `div`), but an undefined one needs a wavy underline, which a plain `div`'s `Styled::underline`
//! cannot draw (it is a solid underline only); [`gpui::StyledText`] with a
//! [`gpui::UnderlineStyle`] can, exactly the fallback described in
//! `docs/gpui-notes.md`.

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::theme::Palette;
use crate::theme::PaletteExt;
use crate::theme::metrics::RADIUS_XS;

/// The [`HighlightStyle`] for a `{{ }}` marker span: `accent_subtle` background plus
/// `accent_text` foreground when `defined`, `danger` text with a wavy underline otherwise.
/// Shared by [`VariableChip`]'s undefined variant and `UrlBar`'s not-focused span rendering.
pub fn highlight_style(defined: bool, palette: &Palette) -> HighlightStyle {
    if defined {
        HighlightStyle {
            background_color: Some(palette.accent_subtle),
            color: Some(palette.accent_text),
            ..Default::default()
        }
    } else {
        HighlightStyle {
            color: Some(palette.danger),
            underline: Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(palette.danger),
                wavy: true,
            }),
            ..Default::default()
        }
    }
}

/// A standalone `{{ }}` marker, in its defined (chip) or undefined (wavy underline) variant.
/// Displays `text` exactly as given, with no added or removed whitespace: callers pass the
/// marker's own source substring (for example `"{{baseUrl}}"`, byte-for-byte), never a
/// reformatted `name`, so the chip never puts spaces in a marker that had none.
#[derive(IntoElement)]
pub struct VariableChip {
    text: SharedString,
    defined: bool,
}

impl VariableChip {
    /// A chip showing `text` (the marker's exact source substring, braces included), styled as
    /// defined or undefined.
    pub fn new(text: impl Into<SharedString>, defined: bool) -> Self {
        Self {
            text: text.into(),
            defined,
        }
    }
}

impl RenderOnce for VariableChip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let text = self.text;

        if self.defined {
            div()
                .font_family(mono_font)
                .text_size(px(12.5))
                .rounded(px(RADIUS_XS))
                .bg(palette.accent_subtle)
                .text_color(palette.accent_text)
                .px(px(4.0))
                .py(px(1.0))
                .child(text)
                .into_any_element()
        } else {
            let style = highlight_style(false, &palette);
            div()
                .font_family(mono_font)
                .text_size(px(12.5))
                .child(StyledText::new(text.clone()).with_highlights(vec![(0..text.len(), style)]))
                .into_any_element()
        }
    }
}
