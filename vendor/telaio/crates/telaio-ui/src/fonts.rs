//! The bundled Geist and Geist Mono fonts (`assets/fonts`, SIL Open Font License 1.1).

use std::borrow::Cow;

use gpui_kit::App;

/// The UI font family name.
pub const UI_FAMILY: &str = "Geist";
/// The monospace font family name.
pub const MONO_FAMILY: &str = "Geist Mono";

/// Geist Regular (400), the UI font's body weight.
pub const GEIST_REGULAR: &[u8] = include_bytes!("../assets/fonts/Geist-Regular.ttf");
/// Geist Medium (500).
pub const GEIST_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Geist-Medium.ttf");
/// Geist SemiBold (600).
pub const GEIST_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Geist-SemiBold.ttf");
/// Geist Bold (700).
pub const GEIST_BOLD: &[u8] = include_bytes!("../assets/fonts/Geist-Bold.ttf");
/// Geist Mono Regular (400).
pub const GEIST_MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/GeistMono-Regular.ttf");
/// Geist Mono Medium (500).
pub const GEIST_MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/GeistMono-Medium.ttf");
/// Geist Mono SemiBold (600).
pub const GEIST_MONO_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/GeistMono-SemiBold.ttf");

/// Every bundled font file, the ones [`register`] adds to the text system. An app that embeds the
/// fonts elsewhere (a PDF export, for example) reads the same bytes instead of bundling its own.
pub const ALL: [&[u8]; 7] = [
    GEIST_REGULAR,
    GEIST_MEDIUM,
    GEIST_SEMIBOLD,
    GEIST_BOLD,
    GEIST_MONO_REGULAR,
    GEIST_MONO_MEDIUM,
    GEIST_MONO_SEMIBOLD,
];

/// Registers the bundled Geist and Geist Mono TTFs ([`ALL`]) with `cx`'s text system.
/// [`crate::install`] calls it; an app that needs more fonts registers them itself. A failure here
/// (the platform text system rejecting well-formed, bundled font bytes) is not worth aborting
/// startup over: the app still runs, falling back to gpui's own font resolution for both families.
pub fn register(cx: &App) {
    let fonts: Vec<Cow<'static, [u8]>> = ALL.into_iter().map(Cow::Borrowed).collect();
    let _ = cx.text_system().add_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::ALL;

    #[test]
    fn every_bundled_font_is_a_truetype_file() {
        for font in ALL {
            // A TrueType file starts with the sfnt version 1.0.
            assert_eq!(font.get(..4), Some([0u8, 1, 0, 0].as_slice()));
        }
    }
}
