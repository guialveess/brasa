//! [`SyntaxTheme`]: the syntax colors of the code editors, passed by the app.
//!
//! telaio ships no syntax colors: the app lists the tree-sitter captures it colors, one theme
//! per mode, and [`crate::ThemeFamily::syntax`] writes them as they are into the gpui-kit
//! theme's `highlight.syntax`. A capture the theme leaves out shows in the text color, so
//! changing or adding a color never needs a change in telaio.
//!
//! ```ignore
//! const SYNTAX_LIGHT: SyntaxTheme = SyntaxTheme(&[
//!     ("comment", SyntaxStyle::color("#5c606e").italic()),
//!     ("keyword", SyntaxStyle::color("#7a4fd6")),
//!     ("string", SyntaxStyle::color("#1d7a4f")),
//! ]);
//! ThemeFamily::new("App").syntax(SYNTAX_LIGHT, SYNTAX_DARK)
//! ```
//!
//! The captures are the field names of gpui-component's `SyntaxColors` (`keyword`, `comment`,
//! `function`, `property`, `string`, `number`, `boolean`, `constant`, `type`, `tag`,
//! `attribute`, `operator`, `punctuation`, `variable`, ...). A grammar's dotted capture with no
//! entry of its own, such as `function.method`, takes the style of its first part. An unknown
//! name is ignored by gpui-component.

use gpui_kit::Hsla;

use crate::palette::hex;

/// How one capture is drawn: a `#rrggbb` color, optionally in italics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntaxStyle {
    /// The color, `#rrggbb` or `#rrggbbaa`.
    pub color: &'static str,
    /// Whether the text is set in italics.
    pub italic: bool,
}

impl SyntaxStyle {
    /// Upright text in `color`.
    pub const fn color(color: &'static str) -> Self {
        Self {
            color,
            italic: false,
        }
    }

    /// The same style in italics.
    pub const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }

    /// The parsed color, see [`hex`].
    pub fn hsla(&self) -> Hsla {
        hex(self.color)
    }
}

/// The syntax colors of one theme mode: capture name and style pairs. The default is empty,
/// every capture in the text color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyntaxTheme(pub &'static [(&'static str, SyntaxStyle)]);

impl SyntaxTheme {
    /// The style of `capture`, or `None` when the theme leaves it out. Exact names only, no
    /// fallback to the first part of a dotted name. For a view that draws colored code by hand,
    /// such as a preview.
    pub fn get(&self, capture: &str) -> Option<SyntaxStyle> {
        self.0
            .iter()
            .find(|(name, _)| *name == capture)
            .map(|(_, style)| *style)
    }

    /// The `highlight.syntax` object of the gpui-kit theme JSON.
    pub(crate) fn to_json(self) -> serde_json::Value {
        let entries = self.0.iter().map(|(name, style)| {
            let mut value = serde_json::json!({ "color": style.color });
            if style.italic {
                value["font_style"] = "italic".into();
            }
            ((*name).to_string(), value)
        });
        serde_json::Value::Object(entries.collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{SyntaxStyle, SyntaxTheme};
    use gpui_kit::FontStyle;
    use gpui_kit::component::highlighter::SyntaxColors;

    const THEME: SyntaxTheme = SyntaxTheme(&[
        ("keyword", SyntaxStyle::color("#7a4fd6")),
        ("comment", SyntaxStyle::color("#5c606e").italic()),
    ]);

    #[test]
    fn get_finds_exact_names_only() {
        assert_eq!(THEME.get("keyword"), Some(SyntaxStyle::color("#7a4fd6")));
        assert!(THEME.get("comment").is_some_and(|style| style.italic));
        assert_eq!(THEME.get("keyword.modifier"), None);
        assert_eq!(SyntaxTheme::default().get("keyword"), None);
    }

    #[test]
    fn json_reaches_gpui_components_syntax_colors() {
        #[allow(clippy::expect_used)]
        let colors: SyntaxColors =
            serde_json::from_value(THEME.to_json()).expect("the JSON parses as SyntaxColors");
        let keyword = colors.style("keyword").map(|style| style.color);
        assert_eq!(keyword, Some(Some(THEME.0[0].1.hsla())));
        let comment = colors.style("comment");
        assert_eq!(
            comment.and_then(|style| style.font_style),
            Some(FontStyle::Italic)
        );
        // A dotted capture falls back to its first part.
        assert!(colors.style("keyword.modifier").is_some());
        assert!(colors.style("string").is_none());
    }
}
