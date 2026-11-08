//! The color palette: [`Tokens`], the raw values of one theme mode, which each app passes as
//! data; [`Palette`], the parsed [`Hsla`] colors plus the values derived from them; and
//! [`PaletteExt`], the extension trait components use to reach the active one.
//!
//! An app writes its hex and shadow literals once, in a `Tokens` per mode (or uses
//! [`Tokens::DEFAULT_LIGHT`] and [`Tokens::DEFAULT_DARK`]). Both [`Palette`] and the gpui-kit
//! theme JSON built in [`crate::theme`] (through [`theme_colors_and_highlight`]) are derived
//! from them, so there is no second, hand-written copy of the color values.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, BoxShadow, Global, Hsla, Rgba, px};

/// The raw string values of one theme mode: `#rrggbb`, or `#rrggbbaa` for a translucent color.
///
/// A malformed literal does not panic, it parses as opaque black (see [`hex`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    /// Background of editor and content panels.
    pub bg: &'static str,
    /// Sidebar, title bar and tab bar background.
    pub surface: &'static str,
    /// Inputs, cards and the code editor background.
    pub raised: &'static str,
    /// Menus, popovers and modals background.
    pub overlay: &'static str,
    /// Default separator and border color.
    pub border: &'static str,
    /// Border color for controls (inputs, buttons).
    pub border_strong: &'static str,
    /// Primary text color.
    pub fg: &'static str,
    /// Secondary text, inactive tabs.
    pub fg_muted: &'static str,
    /// Placeholders, icons, line numbers.
    pub fg_subtle: &'static str,
    /// Primary button, focus ring, active tab.
    pub accent: &'static str,
    /// Text drawn on an `accent` background.
    pub accent_fg: &'static str,
    /// Text and links drawn in the accent color.
    pub accent_text: &'static str,
    /// Selection background, chips.
    pub accent_subtle: &'static str,
    /// Hover background, translucent so it shows on every surface (Postino #138).
    pub hover: &'static str,
    /// Success color.
    pub success: &'static str,
    /// Success badge background.
    pub success_subtle: &'static str,
    /// Warning color.
    pub warning: &'static str,
    /// Warning badge background.
    pub warning_subtle: &'static str,
    /// Danger color.
    pub danger: &'static str,
    /// Danger badge background.
    pub danger_subtle: &'static str,
    /// Info color.
    pub info: &'static str,
    /// Info badge background.
    pub info_subtle: &'static str,
    /// The dimmed backdrop behind a modal (`rgba(10,11,16,.45)` in the default tokens).
    pub scrim: &'static str,
    /// The two elevation shadow layers, in CSS `box-shadow` order. Horizontal offset is always 0
    /// in this design, so it is not stored.
    pub shadow: [ShadowLayer; 2],
}

/// One layer of the elevation shadow, as CSS values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowLayer {
    /// Vertical offset, in pixels.
    pub offset_y: f32,
    /// CSS blur, in pixels (twice gpui's `blur_radius`, see [`shadow_layer`]).
    pub blur: f32,
    /// Spread, in pixels.
    pub spread: f32,
    /// RGB channels and a 0.0..1.0 alpha.
    pub color: (u8, u8, u8, f32),
}

impl Tokens {
    /// The default light tokens, Postino's indigo palette.
    pub const DEFAULT_LIGHT: Tokens = Tokens {
        bg: "#fcfcfd",
        surface: "#f4f5f7",
        raised: "#ffffff",
        overlay: "#ffffff",
        border: "#e3e5ea",
        border_strong: "#cdd0d8",
        fg: "#1b1c22",
        fg_muted: "#5c606e",
        fg_subtle: "#8b8e9a",
        accent: "#4f57d8",
        accent_fg: "#ffffff",
        accent_text: "#3f47c4",
        accent_subtle: "#e8e9fd",
        // `fg` at 7.5%: a translucent layer, so a hover shows on every surface (Postino #138).
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
                color: (20, 22, 40, 0.14),
            },
            ShadowLayer {
                offset_y: 0.0,
                blur: 0.0,
                spread: 1.0,
                color: (20, 22, 40, 0.06),
            },
        ],
    };

    /// The default dark tokens, Postino's indigo palette.
    pub const DEFAULT_DARK: Tokens = Tokens {
        bg: "#15161a",
        surface: "#1b1c21",
        raised: "#23252c",
        overlay: "#262830",
        border: "#2c2e36",
        border_strong: "#3b3e48",
        fg: "#e9eaee",
        fg_muted: "#a0a3ae",
        fg_subtle: "#6f7280",
        accent: "#5b63e6",
        accent_fg: "#ffffff",
        accent_text: "#a3a9ff",
        accent_subtle: "#272b4d",
        // White at 5%: a translucent layer, so a hover shows on every surface (Postino #138).
        // The opaque #25272e used before was almost `raised` and darker than `overlay`,
        // invisible on both.
        hover: "#ffffff0d",
        success: "#4fc88f",
        success_subtle: "#15302a",
        warning: "#e8b75a",
        warning_subtle: "#352b15",
        danger: "#f27171",
        danger_subtle: "#3b1e21",
        info: "#62b3f0",
        info_subtle: "#162a3b",
        scrim: "#0a0b1073",
        shadow: [
            ShadowLayer {
                offset_y: 12.0,
                blur: 32.0,
                spread: 0.0,
                color: (0, 0, 0, 0.45),
            },
            ShadowLayer {
                offset_y: 0.0,
                blur: 0.0,
                spread: 1.0,
                color: (255, 255, 255, 0.04),
            },
        ],
    };
}

/// One [`Hsla`] field per token, plus the values derived from them. Components read colors only
/// from here, through [`PaletteExt::palette`], never from `cx.theme()` directly and never from a
/// literal.
#[derive(Debug, Clone)]
pub struct Palette {
    /// Whether this is a dark mode palette.
    pub dark: bool,
    /// Background of editor and content panels.
    pub bg: Hsla,
    /// Sidebar, title bar and tab bar background.
    pub surface: Hsla,
    /// Inputs, cards and the code editor background.
    pub raised: Hsla,
    /// Menus, popovers and modals background (their own surface fill, JSON key
    /// `"popover.background"`). Not the dialog backdrop scrim, that is [`Self::scrim`].
    pub overlay: Hsla,
    /// The dimmed backdrop behind a modal. The app paints it over the window's content while a
    /// dialog is open; the theme's own `"overlay"` key, which `Dialog` paints, is transparent
    /// (see [`theme_colors_and_highlight`]).
    pub scrim: Hsla,
    /// Default separator and border color.
    pub border: Hsla,
    /// Border color for controls (inputs, buttons).
    pub border_strong: Hsla,
    /// Primary text color.
    pub fg: Hsla,
    /// Secondary text, inactive tabs.
    pub fg_muted: Hsla,
    /// Placeholders, icons, line numbers.
    pub fg_subtle: Hsla,
    /// Primary button, focus ring, active tab.
    pub accent: Hsla,
    /// Text drawn on an `accent` background.
    pub accent_fg: Hsla,
    /// Text and links drawn in the accent color.
    pub accent_text: Hsla,
    /// Selection background, chips.
    pub accent_subtle: Hsla,
    /// Background of a selected list or tree row: `accent_subtle` at [`LIST_ACTIVE_ALPHA`], what
    /// gpui-component makes of `list.active.background`, for rows that draw their selection
    /// themselves.
    pub list_active: Hsla,
    /// Hover background for rows, buttons and controls. Translucent (Postino #138): it darkens
    /// (light) or lightens (dark) whatever surface is under it, so the same token reads on `bg`,
    /// `surface`, `raised` and `overlay` alike.
    pub hover: Hsla,
    /// Pressed background for rows, icon buttons and other hand-rolled clickable `div`s. Not a
    /// token of its own, so it is derived rather than a second literal to keep in sync by hand:
    /// the `hover` layer at [`PRESSED_ALPHA_FACTOR`] times its alpha, translucent like it.
    pub pressed: Hsla,
    /// `PrimaryButton`'s hover fill: `accent` shifted one [`SHADE_STEP`] toward white in the
    /// dark theme, toward black in the light theme (see [`shade`]), so hover reads as "the same
    /// accent, a touch stronger".
    pub accent_hover: Hsla,
    /// `PrimaryButton`'s pressed fill: `accent` shifted two [`SHADE_STEP`]s, see
    /// [`Self::accent_hover`].
    pub accent_pressed: Hsla,
    /// `DangerButton`'s hover fill: `danger` shifted the same way as [`Self::accent_hover`].
    pub danger_hover: Hsla,
    /// `DangerButton`'s pressed fill: `danger` shifted two [`SHADE_STEP`]s, see
    /// [`Self::danger_hover`].
    pub danger_pressed: Hsla,
    /// Success color.
    pub success: Hsla,
    /// Success badge background.
    pub success_subtle: Hsla,
    /// Warning color.
    pub warning: Hsla,
    /// Warning badge background.
    pub warning_subtle: Hsla,
    /// Danger color.
    pub danger: Hsla,
    /// Danger badge background.
    pub danger_subtle: Hsla,
    /// Info color.
    pub info: Hsla,
    /// Info badge background.
    pub info_subtle: Hsla,
    /// The elevation shadow for `overlay`-level surfaces (menus, popovers, modals). There is no
    /// `ThemeConfigColors` field for it, so components apply it with gpui's own `.shadow(...)`.
    pub shadow: Vec<BoxShadow>,
}

/// A semantic color pair: what [`Palette::tone_colors`] maps to a foreground and a subtle
/// background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// `fg_muted` on `hover`, for a state with no meaning of its own.
    #[default]
    Neutral,
    /// `success` on `success_subtle`.
    Success,
    /// `info` on `info_subtle`.
    Info,
    /// `warning` on `warning_subtle`.
    Warning,
    /// `danger` on `danger_subtle`.
    Danger,
}

impl Palette {
    /// The palette built from `tokens`. `dark` picks which way the derived button shades go:
    /// lighter in the dark theme, darker in the light theme ([`shade`]'s doc comment).
    pub fn new(tokens: &Tokens, dark: bool) -> Self {
        let sign = if dark { 1.0 } else { -1.0 };
        let accent = hex(tokens.accent);
        let danger = hex(tokens.danger);
        let hover = hex(tokens.hover);
        Self {
            dark,
            bg: hex(tokens.bg),
            surface: hex(tokens.surface),
            raised: hex(tokens.raised),
            overlay: hex(tokens.overlay),
            scrim: hex(tokens.scrim),
            border: hex(tokens.border),
            border_strong: hex(tokens.border_strong),
            fg: hex(tokens.fg),
            fg_muted: hex(tokens.fg_muted),
            fg_subtle: hex(tokens.fg_subtle),
            accent_fg: hex(tokens.accent_fg),
            accent_text: hex(tokens.accent_text),
            accent_subtle: hex(tokens.accent_subtle),
            list_active: hex(tokens.accent_subtle).opacity(LIST_ACTIVE_ALPHA),
            hover,
            pressed: Hsla {
                a: hover.a * PRESSED_ALPHA_FACTOR,
                ..hover
            },
            accent_hover: shade(accent, sign * SHADE_STEP),
            accent_pressed: shade(accent, sign * SHADE_STEP * 2.0),
            danger_hover: shade(danger, sign * SHADE_STEP),
            danger_pressed: shade(danger, sign * SHADE_STEP * 2.0),
            accent,
            success: hex(tokens.success),
            success_subtle: hex(tokens.success_subtle),
            warning: hex(tokens.warning),
            warning_subtle: hex(tokens.warning_subtle),
            danger,
            danger_subtle: hex(tokens.danger_subtle),
            info: hex(tokens.info),
            info_subtle: hex(tokens.info_subtle),
            shadow: tokens.shadow.iter().map(shadow_layer).collect(),
        }
    }

    /// The palette of [`Tokens::DEFAULT_LIGHT`].
    pub fn default_light() -> Self {
        Self::new(&Tokens::DEFAULT_LIGHT, false)
    }

    /// The palette of [`Tokens::DEFAULT_DARK`].
    pub fn default_dark() -> Self {
        Self::new(&Tokens::DEFAULT_DARK, true)
    }

    /// `hover` composited over the opaque `surface`: the hover fill of a control whose resting
    /// background is `surface`. A `.hover()` style replaces the background instead of layering
    /// over it, so a raised control given the bare translucent `hover` would show what is under
    /// it. Controls with a transparent resting background use `hover` itself.
    pub fn hover_over(&self, surface: Hsla) -> Hsla {
        surface.blend(self.hover)
    }

    /// `pressed` composited over the opaque `surface`, see [`Self::hover_over`].
    pub fn pressed_over(&self, surface: Hsla) -> Hsla {
        surface.blend(self.pressed)
    }

    /// The foreground and background of `tone`, see [`Tone`]'s variants.
    pub fn tone_colors(&self, tone: Tone) -> (Hsla, Hsla) {
        match tone {
            Tone::Neutral => (self.fg_muted, self.hover),
            Tone::Success => (self.success, self.success_subtle),
            Tone::Info => (self.info, self.info_subtle),
            Tone::Warning => (self.warning, self.warning_subtle),
            Tone::Danger => (self.danger, self.danger_subtle),
        }
    }
}

/// The light and dark palettes the app installed ([`crate::theme::install`]), a gpui global.
/// Each is shared as an [`Rc`], so reading one per render is a pointer copy.
#[derive(Debug, Clone)]
pub struct Palettes {
    /// The light mode palette.
    pub light: Rc<Palette>,
    /// The dark mode palette.
    pub dark: Rc<Palette>,
}

impl Palettes {
    /// Both palettes built from `light` and `dark`.
    pub fn new(light: &Tokens, dark: &Tokens) -> Self {
        Self {
            light: Rc::new(Palette::new(light, false)),
            dark: Rc::new(Palette::new(dark, true)),
        }
    }
}

impl Default for Palettes {
    fn default() -> Self {
        Self::new(&Tokens::DEFAULT_LIGHT, &Tokens::DEFAULT_DARK)
    }
}

impl Global for Palettes {}

/// Gives any `App` (and anything that derefs to it, such as `Context<T>`, the same way
/// `cx.theme()` already works) one-call access to the palette matching the active theme mode.
/// Every component reaches the palette through this trait.
pub trait PaletteExt {
    /// The light or dark [`Palette`], matching `cx.theme().is_dark()`.
    fn palette(&self) -> Rc<Palette>;

    /// Both installed palettes, for previews that show a mode other than the active one (the
    /// theme cards). The default palettes when the app installed none.
    fn palettes(&self) -> Palettes;
}

impl PaletteExt for App {
    fn palette(&self) -> Rc<Palette> {
        let dark = self.theme().is_dark();
        match self.try_global::<Palettes>() {
            Some(palettes) if dark => palettes.dark.clone(),
            Some(palettes) => palettes.light.clone(),
            None if dark => Rc::new(Palette::default_dark()),
            None => Rc::new(Palette::default_light()),
        }
    }

    fn palettes(&self) -> Palettes {
        self.try_global::<Palettes>().cloned().unwrap_or_default()
    }
}

/// The `"colors"` and `"highlight"` JSON fragments for one mode's `ThemeConfig`, built straight
/// from `tokens`, never a separate literal. The `"syntax"` part of `"highlight"` comes from the
/// app's [`crate::SyntaxTheme`], added by [`crate::ThemeFamily`].
pub(crate) fn theme_colors_and_highlight(
    tokens: &Tokens,
    dark: bool,
) -> (serde_json::Value, serde_json::Value) {
    // Only the derived colors are read out of this: every other JSON color below comes straight
    // from `tokens`.
    let palette = Palette::new(tokens, dark);
    let colors = serde_json::json!({
        "background": tokens.bg,
        "sidebar.background": tokens.surface,
        "title_bar.background": tokens.surface,
        "tab_bar.background": tokens.surface,
        "secondary.background": tokens.raised,
        "popover.background": tokens.overlay,
        // A different field from `popover.background` above: this is `Dialog`'s own backdrop
        // scrim color (`overlay_color()` in gpui-component's `dialog/dialog.rs`, reading
        // `cx.theme().overlay`), not the modal/popover surface fill. Transparent because
        // gpui-kit 0.8.0 paints that backdrop over the whole window, client side shadow margin
        // included, which showed as a gray frame around the window; the app paints
        // `Palette::scrim` over the content instead.
        "overlay": "#00000000",
        "border": tokens.border,
        // The design defines no scrollbar, so the thumb is a neutral border tone that darkens
        // to the muted text color on hover. The track is transparent so it shows whatever
        // surface it sits on: `bg` painted a darker strip over every raised surface (command
        // palettes, menus, modals).
        "scrollbar.background": "#00000000",
        "scrollbar.thumb.background": tokens.border_strong,
        "scrollbar.thumb.hover.background": tokens.fg_subtle,
        "input.border": tokens.border_strong,
        "foreground": tokens.fg,
        "muted.foreground": tokens.fg_muted,
        "primary.background": tokens.accent,
        "primary.foreground": tokens.accent_fg,
        "link": tokens.accent_text,
        "accent.background": tokens.accent_subtle,
        // Text selection in every input: the accent at the most alpha gpui-component allows a
        // selection (0.3, `Theme::apply_config`'s `clamp_alpha`). `accent_subtle` is almost the
        // input's own background, so a selection showed no highlight at all.
        "selection.background": format!("{}{SELECTION_ALPHA_HEX}", opaque_hex(tokens.accent)),
        "list.active.background": tokens.accent_subtle,
        "list.hover.background": tokens.hover,
        "button.hover.background": tokens.hover,
        "table.hover.background": tokens.hover,
        "success.background": tokens.success,
        "warning.background": tokens.warning,
        "danger.background": tokens.danger,
        "info.background": tokens.info,
        // `PrimaryButton`, `SecondaryButton` and `DangerButton` (`components::buttons`) use
        // gpui-component's own `Primary`, `Secondary` and `Danger` button variants instead of
        // `Ghost` restyled through `Styled`, so their hover and pressed states come from here
        // rather than `Ghost`'s hardcoded, accent tinted formula, which looked like the button
        // had gone disabled on hover.
        "button.primary.background": tokens.accent,
        "button.primary.foreground": tokens.accent_fg,
        "button.primary.hover.background": hex_string(palette.accent_hover),
        "button.primary.active.background": hex_string(palette.accent_pressed),
        "button.secondary.background": tokens.raised,
        "button.secondary.foreground": tokens.fg,
        // Over `raised`, the secondary button's own fill: a hover style replaces the fill.
        "button.secondary.hover.background": hex_string(palette.hover_over(palette.raised)),
        "button.secondary.active.background": hex_string(palette.pressed_over(palette.raised)),
        "button.danger.background": tokens.danger,
        "button.danger.foreground": tokens.accent_fg,
        "button.danger.hover.background": hex_string(palette.danger_hover),
        "button.danger.active.background": hex_string(palette.danger_pressed),
        // gpui-kit's own switch reads these two fields for its unchecked track and its thumb,
        // which otherwise fall back to formulas derived from `secondary` and `background` that
        // do not match the design's `border_strong` track and `accent_fg` thumb.
        "switch.background": tokens.border_strong,
        "switch.thumb.background": tokens.accent_fg,
    });
    // The focus color of every gpui-component control. Unset, it was gpui-component's blue.
    // Inserted after the macro, which reaches its recursion limit with one more entry.
    let mut colors = colors;
    colors["ring"] = serde_json::Value::from(tokens.accent);
    let highlight = serde_json::json!({
        // The line number column of every code editor, the same `bg` as the box the editors
        // sit in (Brasa's own choice: one flat color for the whole editor, not the raised look
        // other inputs get). Unset, it took gpui-component's input background, a lighter strip
        // whose edge read as a divider right where a cursor at the start of a line is drawn.
        "editor.gutter.background": tokens.bg,
    });
    (colors, highlight)
}

/// Builds a [`BoxShadow`] from one shadow layer. The tokens are CSS values: a CSS blur is twice
/// the gaussian's standard deviation, while gpui's `blur_radius` is the standard deviation itself
/// (`gaussian(y, blur_radius)` in its shadow shader), so the blur is halved to look as designed.
fn shadow_layer(layer: &ShadowLayer) -> BoxShadow {
    let (r, g, b, a) = layer.color;
    BoxShadow::new(px(0.0), px(layer.offset_y), rgba(r, g, b, a))
        .blur_radius(px(layer.blur / 2.0))
        .spread_radius(px(layer.spread))
}

/// Parses a `#rrggbb` literal into an opaque [`Hsla`], or a `#rrggbbaa` one into a translucent
/// one. A malformed literal falls back to opaque black instead of panicking.
pub fn hex(value: &str) -> Hsla {
    let digits = value.trim_start_matches('#');
    let parsed = u32::from_str_radix(digits, 16).unwrap_or(0);
    let (rgb, alpha) = if digits.len() == 8 {
        (parsed >> 8, (parsed & 0xff) as f32 / 255.0)
    } else {
        (parsed, 1.0)
    };
    rgba(
        ((rgb >> 16) & 0xff) as u8,
        ((rgb >> 8) & 0xff) as u8,
        (rgb & 0xff) as u8,
        alpha,
    )
}

/// The `#rrggbb` part of a `#rrggbb` or `#rrggbbaa` literal, so an alpha can be appended to it.
fn opaque_hex(value: &str) -> &str {
    value.get(..7).unwrap_or(value)
}

/// Builds an [`Hsla`] from 8-bit RGB channels and a 0.0..1.0 alpha.
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Hsla {
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
    .into()
}

/// The alpha byte appended to the accent's `#rrggbb` for the text selection highlight: `4d` is
/// 0.3, the cap gpui-component applies to `selection.background`.
const SELECTION_ALPHA_HEX: &str = "4d";

/// The alpha gpui-component caps `list.active.background` at, so [`Palette::list_active`] matches
/// the rows `ListItem` selects itself.
const LIST_ACTIVE_ALPHA: f32 = 0.2;

/// How much stronger [`Palette::pressed`] is than the translucent `hover` layer it is derived
/// from: about the step the opaque tokens used to have (hover blended two-thirds of the way
/// toward `border_strong`).
const PRESSED_ALPHA_FACTOR: f32 = 2.2;

/// One `shade` step, in HSL lightness. `accent_hover` and `danger_hover` are one step,
/// `accent_pressed` and `danger_pressed` two.
const SHADE_STEP: f32 = 0.07;

/// Shifts `color`'s HSL lightness by `delta` (clamped to 0.0..=1.0), keeping hue, saturation and
/// alpha. There is no token for a solid button's hover or pressed fill, so [`Palette::new`]
/// derives one from `accent` and `danger` themselves instead of a hand-picked literal: a
/// positive `delta` (dark theme) reads as "a touch lighter", a negative one (light theme) as "a
/// touch darker", the ordinary direction a solid UI color shifts on hover.
fn shade(color: Hsla, delta: f32) -> Hsla {
    Hsla {
        l: (color.l + delta).clamp(0.0, 1.0),
        ..color
    }
}

/// Formats an [`Hsla`] as a `#rrggbb` string, or `#rrggbbaa` when it is translucent
/// ([`Palette::pressed`]), for the [`Palette`] colors computed at runtime ([`shade`]) rather
/// than copied from a [`Tokens`] literal, so [`theme_colors_and_highlight`] can still hand them
/// to the JSON theme family as plain strings like every other color in it.
fn hex_string(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let byte = |channel: f32| (channel * 255.0).round() as u8;
    let rgb = format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.r),
        byte(rgba.g),
        byte(rgba.b)
    );
    if byte(rgba.a) == 0xff {
        rgb
    } else {
        format!("{rgb}{:02x}", byte(rgba.a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn default_light_palette_matches_its_tokens() {
        let palette = Palette::default_light();
        assert!(!palette.dark);
        assert_eq!(to_hex(palette.bg), "#fcfcfd");
        assert_eq!(to_hex(palette.accent), "#4f57d8");
    }

    #[test]
    fn default_dark_palette_matches_its_tokens() {
        let palette = Palette::default_dark();
        assert!(palette.dark);
        assert_eq!(to_hex(palette.bg), "#15161a");
        assert_eq!(to_hex(palette.accent), "#5b63e6");
    }

    #[test]
    fn hex_parses_translucent_literals_and_falls_back_to_black() {
        assert!((hex("#0a0b1073").a - 0x73 as f32 / 255.0).abs() < 0.001);
        assert_eq!(hex("#ffffff").a, 1.0);
        assert_eq!(to_hex(hex("not a color")), "#000000");
    }

    #[test]
    fn tone_colors_map_every_tone() {
        let palette = Palette::default_light();
        assert_eq!(
            palette.tone_colors(Tone::Success),
            (palette.success, palette.success_subtle)
        );
        assert_eq!(
            palette.tone_colors(Tone::Info),
            (palette.info, palette.info_subtle)
        );
        assert_eq!(
            palette.tone_colors(Tone::Warning),
            (palette.warning, palette.warning_subtle)
        );
        assert_eq!(
            palette.tone_colors(Tone::Danger),
            (palette.danger, palette.danger_subtle)
        );
        assert_eq!(
            palette.tone_colors(Tone::Neutral),
            (palette.fg_muted, palette.hover)
        );
    }

    /// `pressed` has no token of its own: it is the `hover` layer, same color, more opaque, so a
    /// hover-then-press sequence is visibly distinct.
    #[test]
    fn pressed_is_a_stronger_hover_layer() {
        for palette in [Palette::default_light(), Palette::default_dark()] {
            assert!(palette.hover.a < 1.0);
            assert!(palette.pressed.a > palette.hover.a);
            assert!(palette.pressed.a < 1.0);
            assert_eq!(
                Hsla {
                    a: 1.0,
                    ..palette.pressed
                },
                Hsla {
                    a: 1.0,
                    ..palette.hover
                },
                "pressed keeps the hover color"
            );
        }
    }

    /// The translucent `hover` and `pressed` (Postino #138) must show on every surface a control
    /// sits on. The opaque dark `hover` they replaced was 2 levels above `raised` and 1 below
    /// `overlay`, invisible on both.
    #[test]
    fn hover_and_pressed_show_on_every_surface() {
        // The mean of the R, G and B differences, in 0..=255 levels.
        fn distance(a: Hsla, b: Hsla) -> f32 {
            let (a, b) = (Rgba::from(a), Rgba::from(b));
            ((a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs()) / 3.0 * 255.0
        }
        for palette in [Palette::default_light(), Palette::default_dark()] {
            for surface in [palette.bg, palette.surface, palette.raised, palette.overlay] {
                let hovered = surface.blend(palette.hover);
                let pressed = surface.blend(palette.pressed);
                assert!(distance(surface, hovered) >= 9.0, "hover on {surface:?}");
                assert!(distance(hovered, pressed) >= 9.0, "pressed on {surface:?}");
            }
        }
    }

    /// The derived button shades must go toward white in the dark theme and toward black in the
    /// light theme, two full steps by the pressed state, so a solid button's hover and press
    /// read as "the same color, a touch stronger" instead of jumping to an unrelated hue.
    #[test]
    fn accent_and_danger_button_shades_lighten_dark_and_darken_light() {
        let light = Palette::default_light();
        assert!(light.accent_hover.l < light.accent.l);
        assert!(light.accent_pressed.l < light.accent_hover.l);
        assert!(light.danger_hover.l < light.danger.l);
        assert!(light.danger_pressed.l < light.danger_hover.l);

        let dark = Palette::default_dark();
        assert!(dark.accent_hover.l > dark.accent.l);
        assert!(dark.accent_pressed.l > dark.accent_hover.l);
        assert!(dark.danger_hover.l > dark.danger.l);
        assert!(dark.danger_pressed.l > dark.danger_hover.l);
    }

    /// The theme JSON's derived button colors must be the same shade `Palette` itself exposes,
    /// in the exact `#rrggbb` form `ThemeConfigColors` parses, not a second, drifting
    /// computation.
    #[test]
    fn theme_json_button_hover_matches_the_palette() {
        for (tokens, dark) in [(Tokens::DEFAULT_LIGHT, false), (Tokens::DEFAULT_DARK, true)] {
            let (colors, _highlight) = theme_colors_and_highlight(&tokens, dark);
            let palette = Palette::new(&tokens, dark);
            assert_eq!(
                colors["button.primary.hover.background"],
                hex_string(palette.accent_hover)
            );
            assert_eq!(
                colors["button.danger.active.background"],
                hex_string(palette.danger_pressed)
            );
            assert_eq!(
                colors["button.secondary.hover.background"],
                hex_string(palette.hover_over(palette.raised))
            );
            assert_eq!(
                colors["selection.background"],
                format!("{}4d", tokens.accent)
            );
            assert_eq!(
                colors["ring"], tokens.accent,
                "the focus color is the accent"
            );
        }
    }
}
