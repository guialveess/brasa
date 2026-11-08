//! telaio: the design system and standard components shared by Postino, Vano and Nastro, on top
//! of gpui-kit.
//!
//! - [`palette`]: [`Tokens`] (the raw values an app passes as data), [`Palette`] (parsed colors
//!   plus derived values) and [`PaletteExt`] (`cx.palette()`).
//! - [`theme`]: the gpui-kit theme family built from the tokens, [`install`] and [`apply`].
//! - [`metrics`]: control heights, the radius scale and component sizes.
//! - [`fonts`]: the bundled Geist and Geist Mono.
//! - [`components`]: buttons, modals, tabs, badges, tables and the rest.
//!
//! The components' own strings (Close, Cancel, Light, ...) come from `locales/telaio.yml` in
//! English, Spanish, Galician and Italian. `rust_i18n::set_locale` is process wide, so they follow
//! the language the app sets.

rust_i18n::i18n!("locales", fallback = "en");

pub mod components;
pub mod fonts;
mod locale;
pub mod metrics;
pub mod palette;
pub mod syntax;
pub mod theme;

/// telaio's version, from its `Cargo.toml`, for an app's About screen.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// telaio's repository URL, from its `Cargo.toml`.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

pub use palette::{Palette, PaletteExt, Palettes, ShadowLayer, Tokens, Tone};
pub use syntax::{SyntaxStyle, SyntaxTheme};
pub use theme::{Fonts, ThemeChoice, ThemeFamily, apply, install};
