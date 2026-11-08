//! Text for the PDF: the four faces, how a piece of text looks and measuring it.

use krilla::text::Font;
use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::{FontRef, MetadataProvider};

use super::{PdfError, PdfFonts};
use crate::export::report::Rgb;

/// Which of the four faces a piece of text is set in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Weight {
    Regular,
    Medium,
    Semibold,
    Mono,
}

/// How a piece of text looks: face, size in points and color.
#[derive(Debug, Clone, Copy)]
pub(super) struct Style {
    pub(super) weight: Weight,
    pub(super) size: f32,
    pub(super) color: Rgb,
}

impl Style {
    pub(super) const fn new(weight: Weight, size: f32, color: Rgb) -> Self {
        Self {
            weight,
            size,
            color,
        }
    }
}

/// A font ready to draw with and to measure with.
pub(super) struct Face<'a> {
    pub(super) font: Font,
    pub(super) charmap: Charmap<'a>,
    pub(super) metrics: GlyphMetrics<'a>,
    pub(super) units_per_em: f32,
    /// The ellipsis to cut text with: `…` when the font has it, three dots otherwise.
    pub(super) ellipsis: &'static str,
}

impl<'a> Face<'a> {
    pub(super) fn new(bytes: &'a [u8], name: &'static str) -> Result<Self, PdfError> {
        let font = Font::new(bytes.to_vec().into(), 0).ok_or(PdfError::Font(name))?;
        let file = FontRef::new(bytes).map_err(|_| PdfError::Font(name))?;
        let charmap = file.charmap();
        let ellipsis = if charmap.map('\u{2026}').is_some() {
            "\u{2026}"
        } else {
            "..."
        };
        let units_per_em = f32::from(
            file.metrics(Size::unscaled(), LocationRef::default())
                .units_per_em,
        );
        if units_per_em <= 0.0 {
            return Err(PdfError::Font(name));
        }
        Ok(Self {
            font,
            charmap,
            metrics: file.glyph_metrics(Size::unscaled(), LocationRef::default()),
            units_per_em,
            ellipsis,
        })
    }

    /// The advance of one character, in font units.
    pub(super) fn advance(&self, c: char) -> f32 {
        let glyph = self.charmap.map(c).unwrap_or_default();
        self.metrics.advance_width(glyph).unwrap_or(0.0)
    }

    /// The width of `text` at `size` points. Kerning is ignored, which is off by a fraction of a
    /// point on a line.
    pub(super) fn width(&self, text: &str, size: f32) -> f32 {
        text.chars().map(|c| self.advance(c)).sum::<f32>() / self.units_per_em * size
    }

    /// `text` cut with an ellipsis so that it is at most `max` points wide. A `max` measured from
    /// the same text, after a few additions and subtractions, still keeps it whole.
    pub(super) fn fit(&self, text: &str, size: f32, max: f32) -> String {
        if self.width(text, size) <= max + 0.01 {
            return text.to_string();
        }
        let ellipsis = self.width(self.ellipsis, size);
        if max < ellipsis {
            return String::new();
        }
        let budget = max - ellipsis;
        let mut used = 0.0;
        let mut out = String::new();
        for c in text.chars() {
            let w = self.advance(c) / self.units_per_em * size;
            if used + w > budget {
                break;
            }
            used += w;
            out.push(c);
        }
        let mut out = out.trim_end().to_string();
        out.push_str(self.ellipsis);
        out
    }
}

/// The four faces of a report.
pub(super) struct Fonts<'a> {
    pub(super) regular: Face<'a>,
    pub(super) medium: Face<'a>,
    pub(super) semibold: Face<'a>,
    pub(super) mono: Face<'a>,
}

impl<'a> Fonts<'a> {
    pub(super) fn new(fonts: &PdfFonts<'a>) -> Result<Self, PdfError> {
        Ok(Self {
            regular: Face::new(fonts.regular, "regular")?,
            medium: Face::new(fonts.medium, "medium")?,
            semibold: Face::new(fonts.semibold, "semibold")?,
            mono: Face::new(fonts.mono, "mono")?,
        })
    }

    pub(super) fn face(&self, weight: Weight) -> &Face<'a> {
        match weight {
            Weight::Regular => &self.regular,
            Weight::Medium => &self.medium,
            Weight::Semibold => &self.semibold,
            Weight::Mono => &self.mono,
        }
    }
}

/// `text` with control characters (line breaks, tabs) replaced by spaces.
pub(super) fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
