//! Brasa's palette: telaio's [`Palette`] (its default tokens are Brasa's own, listed in
//! `docs/design-system.md`) plus the domain colors no other app has, through [`BrasaColors`]:
//! one color per HTTP method, the status code mapping and the environment tiers.
//!
//! The method colors are written once, in [`LIGHT`] and [`DARK`] below (keep them in sync with
//! `docs/design-system.md`).

use gpui_kit::Hsla;

use brasa_core::Method;
pub use telaio_ui::{Palette, PaletteExt, Tone};

use crate::state::env_color::EnvColor;
use crate::state::load_results::ChartSeriesKey;

/// The method colors of one theme mode.
struct MethodTokens {
    get: &'static str,
    post: &'static str,
    put: &'static str,
    patch: &'static str,
    delete: &'static str,
}

const LIGHT: MethodTokens = MethodTokens {
    get: "#1d8657",
    post: "#9a6500",
    put: "#1c6fb0",
    patch: "#7a4fd6",
    delete: "#cf3535",
};

/// The load test chart's latency series colors that are not palette tokens (p95 uses `warning`).
struct ChartTokens {
    p50: &'static str,
    p99: &'static str,
}

const CHART_LIGHT: ChartTokens = ChartTokens {
    p50: "#1f8f80",
    p99: "#cf5b3a",
};

const CHART_DARK: ChartTokens = ChartTokens {
    p50: "#5fc9b8",
    p99: "#f08060",
};

/// The colors that tell the requests of a load test apart in the Latency tab and the report, in
/// order; a run with more requests cycles through them. From the design v3: in the dark theme
/// the first two are `accent_text` and `info`, in the light one `accent` and a lighter `info`.
/// The environment colors that are not palette tokens (#217), from the design's "Environment
/// Colors" page. Red, amber, green, blue and violet reuse `danger`, `warning`, `success`, `info`
/// and the PATCH color.
struct EnvTokens {
    orange: &'static str,
    teal: &'static str,
    pink: &'static str,
    grey: &'static str,
}

const ENV_LIGHT: EnvTokens = EnvTokens {
    orange: "#b4530f",
    teal: "#0f7a74",
    pink: "#b03a78",
    grey: "#6b6f7c",
};

const ENV_DARK: EnvTokens = EnvTokens {
    orange: "#f59a5b",
    teal: "#4cc9c0",
    pink: "#f3a2c6",
    grey: "#a0a3ae",
};

/// The brand mark's five concentric rings, outer to inner: a glowing ember emerging from the
/// background, a hot pale core at the center. Dark mode goes from barely-there rust to a warm
/// near-white; light mode has no pale center to go to (it would vanish against a light page), so
/// it runs the other way, pale at the edge to a dark ember core.
const EMBER_RINGS_LIGHT: [&str; 5] = ["#ffe4d6", "#ffbfa0", "#fd8a5c", "#fd572a", "#c23f1a"];
const EMBER_RINGS_DARK: [&str; 5] = ["#2a140c", "#5c2210", "#a83a14", "#fd572a", "#ffd9ad"];

const REQUESTS_LIGHT: [&str; 6] = [
    "#fd572a", "#1c78b8", "#b0479a", "#3f8a3a", "#a8641c", "#7a4fd6",
];

const REQUESTS_DARK: [&str; 6] = [
    "#ff8a63", "#62b3f0", "#e58fd0", "#8fd18a", "#e0a96d", "#b9a2ff",
];

const DARK: MethodTokens = MethodTokens {
    get: "#4fc88f",
    post: "#e8b75a",
    put: "#62b3f0",
    patch: "#b597f7",
    delete: "#f27171",
};

/// The tone of a response status code: 2xx `Success`, 3xx `Info`, 4xx `Warning`, 5xx `Danger`.
/// `None` ("Not sent" or a send failure) and anything below 200 are `Neutral`.
pub fn status_tone(status: Option<u16>) -> Tone {
    match status {
        Some(code) if (200..300).contains(&code) => Tone::Success,
        Some(code) if (300..400).contains(&code) => Tone::Info,
        Some(code) if (400..500).contains(&code) => Tone::Warning,
        Some(code) if code >= 500 => Tone::Danger,
        _ => Tone::Neutral,
    }
}

/// Brasa's domain colors, read from the active [`Palette`] (`cx.palette().method_color(..)`).
pub trait BrasaColors {
    /// The color a method is drawn in: the five standard methods get their own color, HEAD,
    /// OPTIONS and custom methods fall back to `fg_muted`. Used by the `MethodBadge` component.
    fn method_color(&self, method: &Method) -> Hsla;

    /// The method badge's background: the method color at 14% alpha, matching the design's
    /// `color-mix(... 14%, transparent)`. Used by the `MethodBadge` component's `pill` variant.
    fn method_badge_bg(&self, method: &Method) -> Hsla;

    /// The foreground and background for a response status code, see [`status_tone`].
    fn status_colors(&self, status: Option<u16>) -> (Hsla, Hsla);

    /// The dot color of an environment, from `state::env_color::EnvColor`. "No
    /// environment" has no [`EnvColor`] of its own (`state::env_color`'s doc comment): views draw
    /// its hollow ring straight from `fg_subtle` instead of calling this. Used by the `EnvPill`
    /// and `EnvMenu` components.
    fn env_color(&self, color: EnvColor) -> Hsla;

    /// The color of the tree-sitter `capture` (`"property"`, `"string"`, ...) in the active
    /// mode's [`super::syntax`] theme, or `fg` when the theme leaves it out. For views that draw
    /// code by hand, such as the Settings font preview; code editors get the theme itself.
    fn syntax_color(&self, capture: &str) -> Hsla;

    /// The color of one load test chart series: `accent_text` for requests per second, two
    /// chart colors of Brasa's own for p50 and p99, `warning` for p95 (the compared run's p95
    /// too, drawn dashed) and `fg_subtle` for the virtual users area.
    fn series_color(&self, key: ChartSeriesKey) -> Hsla;

    /// The color of request `index` of a load test (its target index), cycling through six.
    fn request_color(&self, index: usize) -> Hsla;

    /// The brand mark's five concentric ring colors, outer to inner. Used by
    /// `views::welcome::render_ember_mark`.
    fn ember_rings(&self) -> [Hsla; 5];
}

impl BrasaColors for Palette {
    fn method_color(&self, method: &Method) -> Hsla {
        let tokens = if self.dark { &DARK } else { &LIGHT };
        match method {
            Method::Get => telaio_ui::palette::hex(tokens.get),
            Method::Post => telaio_ui::palette::hex(tokens.post),
            Method::Put => telaio_ui::palette::hex(tokens.put),
            Method::Patch => telaio_ui::palette::hex(tokens.patch),
            Method::Delete => telaio_ui::palette::hex(tokens.delete),
            Method::Head | Method::Options | Method::Custom(_) => self.fg_muted,
        }
    }

    fn method_badge_bg(&self, method: &Method) -> Hsla {
        self.method_color(method).opacity(0.14)
    }

    fn status_colors(&self, status: Option<u16>) -> (Hsla, Hsla) {
        self.tone_colors(status_tone(status))
    }

    fn env_color(&self, color: EnvColor) -> Hsla {
        let tokens = if self.dark { &ENV_DARK } else { &ENV_LIGHT };
        match color {
            EnvColor::Red => self.danger,
            EnvColor::Orange => telaio_ui::palette::hex(tokens.orange),
            EnvColor::Amber => self.warning,
            EnvColor::Green => self.success,
            EnvColor::Teal => telaio_ui::palette::hex(tokens.teal),
            EnvColor::Blue => self.info,
            EnvColor::Violet => self.method_color(&Method::Patch),
            EnvColor::Pink => telaio_ui::palette::hex(tokens.pink),
            EnvColor::Grey => telaio_ui::palette::hex(tokens.grey),
        }
    }

    fn syntax_color(&self, capture: &str) -> Hsla {
        let theme = if self.dark {
            super::syntax::DARK
        } else {
            super::syntax::LIGHT
        };
        theme.get(capture).map_or(self.fg, |style| style.hsla())
    }

    fn series_color(&self, key: ChartSeriesKey) -> Hsla {
        let tokens = if self.dark { &CHART_DARK } else { &CHART_LIGHT };
        match key {
            ChartSeriesKey::Vus => self.fg_subtle,
            ChartSeriesKey::Rps => self.accent_text,
            ChartSeriesKey::P50 => telaio_ui::palette::hex(tokens.p50),
            ChartSeriesKey::P95 | ChartSeriesKey::Baseline => self.warning,
            ChartSeriesKey::P99 => telaio_ui::palette::hex(tokens.p99),
        }
    }

    fn request_color(&self, index: usize) -> Hsla {
        let tokens = if self.dark {
            &REQUESTS_DARK
        } else {
            &REQUESTS_LIGHT
        };
        telaio_ui::palette::hex(tokens[index % tokens.len()])
    }

    fn ember_rings(&self) -> [Hsla; 5] {
        let tokens = if self.dark {
            &EMBER_RINGS_DARK
        } else {
            &EMBER_RINGS_LIGHT
        };
        tokens.map(telaio_ui::palette::hex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::Rgba;
    use pretty_assertions::assert_eq;

    /// Round-trips an [`Hsla`] back to a `#rrggbb` string for comparison against the design's
    /// own hex literals, sidestepping float precision noise from the HSL conversion.
    fn to_hex(color: Hsla) -> String {
        let rgba = Rgba::from(color);
        format!(
            "#{:02x}{:02x}{:02x}",
            (rgba.r * 255.0).round() as u8,
            (rgba.g * 255.0).round() as u8,
            (rgba.b * 255.0).round() as u8,
        )
    }

    #[test]
    fn palettes_match_design_tokens() {
        let light = Palette::default_light();
        assert_eq!(to_hex(light.bg), "#fcfcfd");
        assert_eq!(to_hex(light.accent), "#4f57d8");
        assert_eq!(to_hex(light.method_color(&Method::Post)), "#9a6500");
        let dark = Palette::default_dark();
        assert_eq!(to_hex(dark.bg), "#15161a");
        assert_eq!(to_hex(dark.accent), "#5b63e6");
        assert_eq!(to_hex(dark.method_color(&Method::Post)), "#e8b75a");
    }

    #[test]
    fn syntax_color_follows_the_mode_and_falls_back_to_fg() {
        let light = Palette::default_light();
        assert_eq!(to_hex(light.syntax_color("property")), "#3f47c4");
        assert_eq!(light.syntax_color("operator"), light.fg);
        let dark = Palette::default_dark();
        assert_eq!(to_hex(dark.syntax_color("property")), "#a3a9ff");
    }

    #[test]
    fn series_colors_follow_the_mode() {
        let dark = Palette::default_dark();
        assert_eq!(to_hex(dark.series_color(ChartSeriesKey::P50)), "#5fc9b8");
        assert_eq!(to_hex(dark.series_color(ChartSeriesKey::P99)), "#f08060");
        assert_eq!(dark.series_color(ChartSeriesKey::Rps), dark.accent_text);
        let light = Palette::default_light();
        assert_eq!(to_hex(light.series_color(ChartSeriesKey::P50)), "#1f8f80");
    }

    #[test]
    fn request_colors_start_with_the_accent_and_info_and_cycle() {
        // Brasa's own tokens, not telaio's defaults: `REQUESTS_DARK`/`REQUESTS_LIGHT` are tuned
        // to this app's accent, which `Palette::default_dark`/`default_light` do not carry.
        let dark = Palette::new(&super::super::DARK_TOKENS, true);
        assert_eq!(dark.request_color(0), dark.accent_text);
        assert_eq!(dark.request_color(1), dark.info);
        assert_eq!(dark.request_color(6), dark.request_color(0));
        let light = Palette::new(&super::super::LIGHT_TOKENS, false);
        assert_eq!(to_hex(light.request_color(2)), "#b0479a");
    }

    #[test]
    fn method_color_falls_back_to_fg_muted() {
        let palette = Palette::default_light();
        assert_eq!(to_hex(palette.method_color(&Method::Get)), "#1d8657");
        assert_eq!(palette.method_color(&Method::Head), palette.fg_muted);
        assert_eq!(
            palette.method_color(&Method::Custom("PURGE".to_string())),
            palette.fg_muted
        );
    }

    #[test]
    fn method_badge_bg_is_method_color_at_14_percent() {
        let palette = Palette::default_light();
        let badge = palette.method_badge_bg(&Method::Get);
        assert_eq!(badge.a, palette.method_color(&Method::Get).a * 0.14);
    }

    #[test]
    fn status_colors_map_ranges_to_tokens() {
        let palette = Palette::default_light();
        assert_eq!(
            palette.status_colors(Some(204)),
            (palette.success, palette.success_subtle)
        );
        assert_eq!(
            palette.status_colors(Some(301)),
            (palette.info, palette.info_subtle)
        );
        assert_eq!(
            palette.status_colors(Some(404)),
            (palette.warning, palette.warning_subtle)
        );
        assert_eq!(
            palette.status_colors(Some(503)),
            (palette.danger, palette.danger_subtle)
        );
        assert_eq!(
            palette.status_colors(None),
            (palette.fg_muted, palette.hover)
        );
    }

    #[test]
    fn env_color_maps_every_variant() {
        let palette = Palette::default_light();
        assert_eq!(palette.env_color(EnvColor::Red), palette.danger);
        assert_eq!(palette.env_color(EnvColor::Amber), palette.warning);
        assert_eq!(palette.env_color(EnvColor::Green), palette.success);
        assert_eq!(palette.env_color(EnvColor::Blue), palette.info);
        assert_eq!(to_hex(palette.env_color(EnvColor::Violet)), "#7a4fd6");
        assert_eq!(to_hex(palette.env_color(EnvColor::Teal)), "#0f7a74");
        let dark = Palette::default_dark();
        assert_eq!(to_hex(dark.env_color(EnvColor::Orange)), "#f59a5b");
        assert_eq!(to_hex(dark.env_color(EnvColor::Grey)), "#a0a3ae");
    }
}
