//! Brasa's visual foundations, on top of telaio: the palette ([`LIGHT_TOKENS`] and
//! [`DARK_TOKENS`], plus the domain colors in [`palette`]), layout metrics, and the "Brasa
//! Light"/"Brasa Dark" theme family built from the settings' fonts.
//!
//! [`install`] does the startup work through `telaio_ui::install`: register the bundled fonts, load
//! the theme family into gpui-kit's `ThemeRegistry` and install the palettes `cx.palette()`
//! reads. [`apply_settings`] does the same live after a Settings change.

pub mod metrics;
pub mod palette;
pub mod syntax;

// Re-exported for views (`cx.palette()`); every component in `views/components/` uses it.
pub use palette::{Palette, PaletteExt, BrasaColors};

use gpui_kit::App;
use telaio_ui::{ShadowLayer, Tokens};

use crate::state::settings::Settings;

/// The light mode tokens of the app's own palette: a warm near-white with an orange accent.
const LIGHT_TOKENS: Tokens = Tokens {
    bg: "#fdfcfb",
    surface: "#f6f4f1",
    raised: "#ffffff",
    overlay: "#ffffff",
    border: "#e7e3de",
    border_strong: "#d2ccc5",
    fg: "#1a1a1d",
    fg_muted: "#5c5a56",
    fg_subtle: "#8b887f",
    accent: "#fd572a",
    accent_fg: "#ffffff",
    accent_text: "#c8431d",
    accent_subtle: "#ffe7dd",
    hover: "#1b1c2213",
    success: "#1d8657",
    success_subtle: "#e2f4ea",
    warning: "#9a6500",
    warning_subtle: "#fbf0d9",
    danger: "#cf3535",
    danger_subtle: "#fde7e7",
    info: "#1c6fb0",
    info_subtle: "#e1eef9",
    scrim: "#0a0b1073",
    shadow: [
        ShadowLayer {
            offset_y: 12.0,
            blur: 32.0,
            spread: 0.0,
            color: (20, 16, 10, 0.14),
        },
        ShadowLayer {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: (20, 16, 10, 0.06),
        },
    ],
};

/// The dark mode tokens: near black with the same orange accent.
const DARK_TOKENS: Tokens = Tokens {
    bg: "#0a0a0d",
    surface: "#131318",
    raised: "#19191f",
    overlay: "#1f1f26",
    border: "#28282f",
    border_strong: "#38383f",
    fg: "#efeef0",
    fg_muted: "#9fa0a8",
    fg_subtle: "#6e6f78",
    accent: "#fd572a",
    accent_fg: "#ffffff",
    accent_text: "#ff8a63",
    accent_subtle: "#2c1811",
    hover: "#ffffff0d",
    success: "#4fc88f",
    success_subtle: "#16302a",
    warning: "#e8b75a",
    warning_subtle: "#352c16",
    danger: "#f2726c",
    danger_subtle: "#3c1e1e",
    info: "#62b3f0",
    info_subtle: "#16293b",
    scrim: "#05050873",
    shadow: [
        ShadowLayer {
            offset_y: 12.0,
            blur: 32.0,
            spread: 0.0,
            color: (0, 0, 0, 0.55),
        },
        ShadowLayer {
            offset_y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: (255, 255, 255, 0.04),
        },
    ],
};

/// Loads the Brasa Light/Dark theme family built from `settings`'s font choices and makes it
/// the registry's default light and dark themes. Call once, after `gpui_kit::init`, before
/// opening any window; the window's own `ThemeChoice::apply` call is what makes the loaded
/// family actually visible.
pub fn install(cx: &mut App, settings: &Settings) {
    telaio_ui::install(cx, &theme_family(settings));
}

/// Rebuilds the "Brasa Light"/"Brasa Dark" themes from `settings` and assigns them as the
/// active `Theme`'s light/dark themes, live (every Settings change updates the global `Theme`
/// immediately). The caller still has to apply the theme choice afterwards to repaint the
/// window with the new config.
pub fn apply_settings(cx: &mut App, settings: &Settings) {
    telaio_ui::apply(cx, &theme_family(settings));
}

/// The theme family: [`LIGHT_TOKENS`] and [`DARK_TOKENS`], Brasa's [`syntax`] colors and the
/// settings' fonts.
fn theme_family(settings: &Settings) -> telaio_ui::ThemeFamily {
    telaio_ui::ThemeFamily::new("Brasa")
        .tokens(LIGHT_TOKENS, DARK_TOKENS)
        .syntax(syntax::LIGHT, syntax::DARK)
        .fonts(telaio_ui::Fonts {
            ui_family: settings.ui_font.clone().into(),
            ui_size: settings.ui_font_size,
            mono_family: settings.mono_font.clone().into(),
            mono_size: settings.mono_font_size,
        })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use gpui_kit::component::ThemeSet;
    use pretty_assertions::assert_eq;

    #[test]
    fn family_json_parses_as_a_theme_set_with_both_modes() {
        let settings = Settings::default();
        let json = theme_family(&settings).to_json().to_string();

        let theme_set: ThemeSet =
            serde_json::from_str(&json).expect("the theme family must parse as a ThemeSet");

        assert_eq!(theme_set.themes.len(), 2);
        assert_eq!(theme_set.themes[0].name.as_ref(), "Brasa Light");
        assert!(!theme_set.themes[0].mode.is_dark());
        assert_eq!(theme_set.themes[1].name.as_ref(), "Brasa Dark");
        assert!(theme_set.themes[1].mode.is_dark());
    }

    #[test]
    fn family_json_carries_the_settings_fonts() {
        let settings = Settings::default();
        let json = theme_family(&settings).to_json();

        assert_eq!(json["themes"][0]["font.family"], "Geist");
        assert_eq!(json["themes"][0]["mono_font.family"], "Geist Mono");
        assert_eq!(json["themes"][0]["font.size"], 13.0);
        assert_eq!(json["themes"][0]["mono_font.size"], 12.5);
    }

    #[test]
    fn family_json_colors_match_the_design_tokens() {
        let settings = Settings::default();
        let json = theme_family(&settings).to_json();

        assert_eq!(json["themes"][0]["colors"]["background"], "#fdfcfb");
        assert_eq!(json["themes"][0]["colors"]["primary.background"], "#fd572a");
        assert_eq!(json["themes"][1]["colors"]["background"], "#0a0a0d");
        assert_eq!(json["themes"][1]["colors"]["primary.background"], "#fd572a");
    }
}
