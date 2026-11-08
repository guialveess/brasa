//! The gpui-kit theme family built from an app's [`Tokens`] and [`SyntaxTheme`]s.
//!
//! [`install`] does the startup work: register the bundled fonts, build the "<Name> Light" and
//! "<Name> Dark" theme family JSON from the tokens, load it into gpui-kit's `ThemeRegistry` so
//! `Theme::change` and `Theme::sync_system_appearance` resolve to it instead of
//! gpui-component's built-in default, and install the matching [`Palettes`] global that
//! [`crate::PaletteExt::palette`] reads. [`apply`] does the same live, for a font or palette
//! change after startup.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{App, SharedString, Window};

use crate::fonts;
use crate::palette::{Palettes, Tokens, theme_colors_and_highlight};
use crate::syntax::SyntaxTheme;

/// General radius for gpui-kit's own stock widgets (inputs, buttons), matching the design's
/// radius scale ([`crate::metrics::RADIUS_MD`]).
const GENERAL_RADIUS: u64 = 17;
/// Large-element radius (dialogs, notifications), matching the design's modal radius
/// ([`crate::metrics::RADIUS_MODAL`]).
const LARGE_RADIUS: u64 = 24;

/// The UI and monospace fonts of a theme family.
#[derive(Debug, Clone, PartialEq)]
pub struct Fonts {
    /// The UI font family.
    pub ui_family: SharedString,
    /// The UI font size, in pixels.
    pub ui_size: f32,
    /// The monospace font family, for code and values.
    pub mono_family: SharedString,
    /// The monospace font size, in pixels.
    pub mono_size: f32,
}

impl Default for Fonts {
    /// Geist 13 and Geist Mono 12.5, the bundled families ([`crate::fonts`]).
    fn default() -> Self {
        Self {
            ui_family: fonts::UI_FAMILY.into(),
            ui_size: 13.0,
            mono_family: fonts::MONO_FAMILY.into(),
            mono_size: 12.5,
        }
    }
}

/// Everything a theme family is built from: a name, the tokens and syntax colors of both modes
/// and the fonts.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeFamily {
    /// The family name. The two themes are registered as "<name> Light" and "<name> Dark".
    pub name: SharedString,
    /// The light mode tokens.
    pub light: Tokens,
    /// The dark mode tokens.
    pub dark: Tokens,
    /// The light mode syntax colors of the code editors, empty by default.
    pub syntax_light: SyntaxTheme,
    /// The dark mode syntax colors of the code editors, empty by default.
    pub syntax_dark: SyntaxTheme,
    /// The UI and monospace fonts.
    pub fonts: Fonts,
}

impl ThemeFamily {
    /// A family named `name` with the default tokens and fonts, and no syntax colors.
    pub fn new(name: impl Into<SharedString>) -> Self {
        Self {
            name: name.into(),
            light: Tokens::DEFAULT_LIGHT,
            dark: Tokens::DEFAULT_DARK,
            syntax_light: SyntaxTheme::default(),
            syntax_dark: SyntaxTheme::default(),
            fonts: Fonts::default(),
        }
    }

    /// Replaces the tokens of both modes.
    pub fn tokens(mut self, light: Tokens, dark: Tokens) -> Self {
        self.light = light;
        self.dark = dark;
        self
    }

    /// Sets the syntax colors of both modes. See [`SyntaxTheme`].
    pub fn syntax(mut self, light: SyntaxTheme, dark: SyntaxTheme) -> Self {
        self.syntax_light = light;
        self.syntax_dark = dark;
        self
    }

    /// Replaces the fonts.
    pub fn fonts(mut self, fonts: Fonts) -> Self {
        self.fonts = fonts;
        self
    }

    /// The name the light theme is registered under.
    pub fn light_name(&self) -> String {
        format!("{} Light", self.name)
    }

    /// The name the dark theme is registered under.
    pub fn dark_name(&self) -> String {
        format!("{} Dark", self.name)
    }

    /// The theme family JSON gpui-kit loads, with both `ThemeConfig` entries. This is the single
    /// place the family is assembled: there is no hand-written theme JSON file.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name.as_ref(),
            "themes": [self.config_json(false), self.config_json(true)],
        })
    }

    /// One mode's `ThemeConfig` entry.
    fn config_json(&self, dark: bool) -> serde_json::Value {
        let (name, mode, tokens, syntax) = if dark {
            (self.dark_name(), "dark", &self.dark, self.syntax_dark)
        } else {
            (self.light_name(), "light", &self.light, self.syntax_light)
        };
        let (colors, mut highlight) = theme_colors_and_highlight(tokens, dark);
        highlight["syntax"] = syntax.to_json();
        serde_json::json!({
            "name": name,
            "mode": mode,
            "font.family": self.fonts.ui_family.as_ref(),
            "font.size": self.fonts.ui_size,
            "mono_font.family": self.fonts.mono_family.as_ref(),
            "mono_font.size": self.fonts.mono_size,
            "radius": GENERAL_RADIUS,
            "radius.lg": LARGE_RADIUS,
            "colors": colors,
            "highlight": highlight,
        })
    }
}

/// Registers the bundled fonts, loads `family` and makes it the registry's default light and
/// dark themes, and installs its [`Palettes`]. Call once, after `gpui_kit::init`, before opening
/// any window: `ThemeRegistry` is set up as a global inside `init`, and the window's own
/// `Theme::change` or `Theme::sync_system_appearance` call (see [`ThemeChoice::apply`]) is what
/// makes the loaded family visible.
pub fn install(cx: &mut App, family: &ThemeFamily) {
    fonts::register(cx);

    let json = family.to_json().to_string();
    // Built by this module from a fixed, well-formed shape (the tests below cover it), so a
    // parse failure here would be a bug in this module, not a runtime condition.
    #[allow(clippy::expect_used)]
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(&json)
        .expect("ThemeFamily::to_json always builds a valid ThemeSet, see the tests in theme.rs");

    let registry = ThemeRegistry::global(cx);
    let light = registry.themes().get(family.light_name().as_str()).cloned();
    let dark = registry.themes().get(family.dark_name().as_str()).cloned();
    let theme = Theme::global_mut(cx);
    // No 3 px half alpha band around a focused control: it shows the 1 px `ring` border alone,
    // the focus look of the design system, which also cannot be clipped by a scroll area.
    theme.focus_ring = false;
    if let Some(light) = light {
        theme.light_theme = light;
    }
    if let Some(dark) = dark {
        theme.dark_theme = dark;
    }
    cx.set_global(Palettes::new(&family.light, &family.dark));
}

/// Rebuilds both `ThemeConfig`s from `family` and assigns them as the active `Theme`'s light and
/// dark themes, live, and replaces the [`Palettes`] global. Unlike [`install`], this does not go
/// through `ThemeRegistry::load_themes_from_str`: that call silently no-ops when a theme name it
/// already knows is loaded again, which would make a font or palette change invisible. Parsing
/// each mode's `ThemeConfig` directly and assigning it sidesteps that: `Theme::change` always
/// re-reads those two fields.
///
/// The caller still has to call [`ThemeChoice::apply`] (or `Theme::change` or
/// `Theme::sync_system_appearance`) afterwards to repaint the window with the new config.
pub fn apply(cx: &mut App, family: &ThemeFamily) {
    // Built from the fixed, well-formed shape `config_json` always produces (the same guarantee
    // `install` relies on, covered by the tests below).
    #[allow(clippy::expect_used)]
    let light: ThemeConfig = serde_json::from_value(family.config_json(false))
        .expect("ThemeFamily::config_json always builds a valid ThemeConfig");
    #[allow(clippy::expect_used)]
    let dark: ThemeConfig = serde_json::from_value(family.config_json(true))
        .expect("ThemeFamily::config_json always builds a valid ThemeConfig");

    let theme = Theme::global_mut(cx);
    theme.light_theme = Rc::new(light);
    theme.dark_theme = Rc::new(dark);
    cx.set_global(Palettes::new(&family.light, &family.dark));
}

/// Which theme mode the app follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Follow the OS light or dark setting.
    #[default]
    System,
    /// Always use the light theme.
    Light,
    /// Always use the dark theme.
    Dark,
}

impl ThemeChoice {
    /// The three choices, in the order the theme cards show them.
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    /// Switches the active theme to this choice and repaints `window`. With
    /// [`ThemeChoice::System`] the app must also call `Theme::sync_system_appearance` from a
    /// `observe_window_appearance` callback to follow later OS changes.
    pub fn apply(self, window: Option<&mut Window>, cx: &mut App) {
        match self {
            ThemeChoice::System => Theme::sync_system_appearance(window, cx),
            ThemeChoice::Light => Theme::change(ThemeMode::Light, window, cx),
            ThemeChoice::Dark => Theme::change(ThemeMode::Dark, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::ThemeSet;
    use pretty_assertions::assert_eq;

    #[test]
    fn family_json_parses_as_a_theme_set_with_both_modes() {
        let family = ThemeFamily::new("Postino");
        let json = family.to_json().to_string();

        #[allow(clippy::expect_used)]
        let theme_set: ThemeSet =
            serde_json::from_str(&json).expect("to_json must parse as a ThemeSet");

        assert_eq!(theme_set.themes.len(), 2);
        assert_eq!(theme_set.themes[0].name.as_ref(), "Postino Light");
        assert!(!theme_set.themes[0].mode.is_dark());
        assert_eq!(theme_set.themes[1].name.as_ref(), "Postino Dark");
        assert!(theme_set.themes[1].mode.is_dark());
    }

    #[test]
    fn family_json_carries_the_fonts() {
        let family = ThemeFamily::new("Postino");
        let json = family.to_json();

        assert_eq!(json["themes"][0]["font.family"], "Geist");
        assert_eq!(json["themes"][0]["mono_font.family"], "Geist Mono");
        assert_eq!(json["themes"][0]["font.size"], 13.0);
        assert_eq!(json["themes"][0]["mono_font.size"], 12.5);
    }

    #[test]
    fn family_json_colors_match_the_tokens() {
        let family = ThemeFamily::new("Postino");
        let json = family.to_json();

        assert_eq!(json["themes"][0]["colors"]["background"], "#fcfcfd");
        assert_eq!(json["themes"][0]["colors"]["primary.background"], "#4f57d8");
        assert_eq!(json["themes"][1]["colors"]["background"], "#15161a");
        assert_eq!(json["themes"][1]["colors"]["primary.background"], "#5b63e6");
    }

    #[test]
    fn each_mode_gets_its_own_syntax_theme() {
        use crate::syntax::SyntaxStyle;
        const LIGHT: SyntaxTheme = SyntaxTheme(&[("keyword", SyntaxStyle::color("#7a4fd6"))]);
        const DARK: SyntaxTheme =
            SyntaxTheme(&[("comment", SyntaxStyle::color("#a0a3ae").italic())]);
        let json = ThemeFamily::new("Postino").syntax(LIGHT, DARK).to_json();

        let light_syntax = &json["themes"][0]["highlight"]["syntax"];
        assert_eq!(light_syntax["keyword"]["color"], "#7a4fd6");
        assert!(light_syntax.get("comment").is_none());
        let dark_syntax = &json["themes"][1]["highlight"]["syntax"];
        assert_eq!(dark_syntax["comment"]["font_style"], "italic");
        // The rest of `highlight` still comes from the tokens.
        assert_eq!(
            json["themes"][1]["highlight"]["editor.gutter.background"],
            "#23252c"
        );
    }

    #[test]
    fn a_family_without_syntax_colors_has_an_empty_syntax_object() {
        let json = ThemeFamily::new("Postino").to_json();
        assert_eq!(
            json["themes"][0]["highlight"]["syntax"],
            serde_json::json!({})
        );
    }

    #[test]
    fn custom_tokens_reach_the_json() {
        let light = Tokens {
            accent: "#171717",
            ..Tokens::DEFAULT_LIGHT
        };
        let family = ThemeFamily::new("Vano").tokens(light, Tokens::DEFAULT_DARK);
        let json = family.to_json();

        assert_eq!(json["themes"][0]["name"], "Vano Light");
        assert_eq!(json["themes"][0]["colors"]["primary.background"], "#171717");
    }

    #[test]
    fn every_config_parses_as_a_theme_config() {
        let family = ThemeFamily::new("Postino");
        for dark in [false, true] {
            let config: Result<ThemeConfig, _> = serde_json::from_value(family.config_json(dark));
            assert!(config.is_ok(), "dark = {dark}");
        }
    }
}
