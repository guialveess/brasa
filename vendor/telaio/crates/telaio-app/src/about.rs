//! The header of an About pane: the logo square with the app's initial, the name, the version and
//! platform, and the release notes and GitHub buttons. Everything the pane shows below it
//! (updates, install source) is the app's own.

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_ui::Palette;
use telaio_ui::components::SecondaryButton;

use crate::AppIdentity;

/// The About header for `identity`, running on `platform` (for example `Linux x86_64`). The
/// version is the identity's, the release notes button opens its tag on GitHub, and the GitHub
/// button opens its repository. Texts follow the active language.
///
/// `mono_font_family` is the font of the version numbers. Cheap to build: no IO and nothing
/// proportional to data, so it can be called from `render`.
pub fn header(
    identity: &AppIdentity,
    platform: &str,
    palette: &Palette,
    mono_font_family: &SharedString,
) -> AnyElement {
    let notes_url = identity.release_notes_url();
    let repository = identity.repository;
    let initial: String = identity.name.chars().take(1).collect();
    h_flex()
        .items_center()
        .gap(px(14.0))
        .child(
            div()
                .flex_none()
                .size(px(44.0))
                .rounded(px(11.0))
                .bg(palette.accent)
                .text_color(palette.accent_fg)
                .text_size(px(22.0))
                .font_weight(FontWeight::BOLD)
                .flex()
                .items_center()
                .justify_center()
                .child(initial),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(3.0))
                .child(
                    div()
                        .text_size(px(16.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(identity.name),
                )
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .text_color(palette.fg_muted)
                        .child(t!("telaio_app.about.version"))
                        .child(
                            div()
                                .font_family(mono_font_family.clone())
                                .text_color(palette.fg)
                                .child(identity.version),
                        )
                        .child(format!("\u{b7} {platform}")),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .gap(px(6.0))
                .child(
                    SecondaryButton::new(
                        "about-release-notes",
                        t!("telaio_app.about.release_notes"),
                    )
                    .icon(Icon::new(gpui_kit::assets::IconName::FileText))
                    .on_click(move |_, _, cx| cx.open_url(&notes_url)),
                )
                .child(
                    SecondaryButton::new("about-github", "GitHub")
                        .icon(Icon::new(gpui_kit::assets::IconName::Github))
                        .on_click(move |_, _, cx| cx.open_url(repository)),
                ),
        )
        .into_any_element()
}
