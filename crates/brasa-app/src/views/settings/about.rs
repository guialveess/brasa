//! The "About" pane of the Settings modal: the version, the release links, the What's new tab
//! and the updater. The
//! updater logic is `state::update` and `views/update.rs`; this only renders it and forwards the
//! clicks to the same flows the status bar and the command palette use. Everything it shows is
//! captured once per frame from [`AppView`] as plain data ([`AboutPane`]), with no IO.

use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use brasa_cli::display_identity;
use rust_i18n::t;

use crate::state::about::{self, AboutInfo};
use crate::state::update::{CheckStatus, ManualUpdate, ReadyUpdate, updater_enabled};
use crate::theme::Palette;
use crate::theme::metrics::{CONTROL_HEIGHT, RADIUS_LG};
use crate::views::components::{PrimaryButton, SecondaryButton, switch_halo};
use crate::views::root::AppView;
use telaio_ui::components::{LabeledRow, SectionHeading};

/// What the pane needs from [`AppView`], cloned out so nothing borrows the view while rendering.
pub(super) struct AboutPane {
    info: AboutInfo,
    enabled: bool,
    check_updates: bool,
    ready: Option<ReadyUpdate>,
    manual: ManualUpdate,
    status: CheckStatus,
}

impl AboutPane {
    /// Captures the pane's data from `view`.
    pub(super) fn capture(view: &AppView) -> Self {
        Self {
            info: view.about_info.clone(),
            enabled: updater_enabled(),
            check_updates: view.state.settings.check_updates,
            ready: view.state.update.ready().cloned(),
            manual: view.manual_update(),
            status: view.state.update.status(),
        }
    }
}

/// The pane's body sections: the header row, the What's new row and the updates section.
pub(super) fn render_about(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    mono_font_family: &SharedString,
    pane: &AboutPane,
) -> Vec<AnyElement> {
    vec![
        telaio_app::about::header(
            &display_identity(),
            &pane.info.platform,
            palette,
            mono_font_family,
        ),
        render_whats_new(weak.clone()),
        render_updates(weak, palette, mono_font_family, pane),
    ]
}

/// The row that opens the What's new tab (#189), closing the Settings modal so it shows.
fn render_whats_new(weak: WeakEntity<AppView>) -> AnyElement {
    let button = SecondaryButton::new("about-whats-new", t!("whats_new.palette_action"))
        .icon(Icon::new(gpui_kit::assets::IconName::Sparkles))
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            let _ = weak.update(cx, |view, cx| view.open_whats_new_tab(cx));
        });
    LabeledRow::new(t!("whats_new.tab"), button)
        .description(t!("settings.about.whats_new_description"))
        .into_any_element()
}

/// The About footer's note: whether updates are checked or the build has no updater.
pub(super) fn footer_text() -> SharedString {
    if updater_enabled() {
        t!("settings.about.footer.updater")
    } else {
        t!("settings.about.footer.offline")
    }
    .into()
}

/// The "Updates" section: variant 3a with an updater, 3b without.
fn render_updates(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    mono_font_family: &SharedString,
    pane: &AboutPane,
) -> AnyElement {
    let title = SectionHeading::new(t!("update.setting.title"));
    let body = if pane.enabled {
        render_managed_updates(weak, palette, mono_font_family, pane)
    } else {
        render_distro_updates(palette, mono_font_family, &pane.info)
    };
    v_flex()
        .gap(px(12.0))
        .child(title)
        .children(body)
        .into_any_element()
}

/// Variant 3a: the ready banner or the check status, the automatic check switch and "Check now".
fn render_managed_updates(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    mono_font_family: &SharedString,
    pane: &AboutPane,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    if let Some(ready) = &pane.ready {
        rows.push(render_ready_banner(
            weak.clone(),
            palette,
            ready,
            pane.manual,
        ));
    } else if let Some(row) = render_status_row(palette, pane.status, pane.info.version) {
        rows.push(row);
    }

    let auto_weak = weak.clone();
    let switch = switch_halo(
        "settings-check-updates-halo",
        Switch::new("settings-check-updates")
            .checked(pane.check_updates)
            .on_click(move |checked, window, cx| {
                let checked = *checked;
                let _ =
                    auto_weak.update(cx, |view, cx| view.set_check_updates(checked, window, cx));
            }),
        palette,
    )
    .into_any_element();
    rows.push(
        LabeledRow::new(t!("update.setting.check"), switch)
            .description(t!("update.setting.check_description"))
            .into_any_element(),
    );

    let busy = pane.status == CheckStatus::Checking || pane.ready.is_some();
    let check = SecondaryButton::new("about-check-now", t!("shell.palette.check_for_updates"))
        .icon(Icon::new(gpui_kit::assets::IconName::RefreshCw))
        .disabled(busy)
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |view, cx| view.check_for_updates_manually(window, cx));
        });
    let mut leading = v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(2.0))
        .child(div().child(t!("settings.about.check_now")));
    // No download directory (a portable copy) means there is nothing to say about downloads.
    if let Some(dir) = &pane.info.download_dir {
        leading = leading.child(
            h_flex()
                .gap(px(4.0))
                .text_color(palette.fg_muted)
                .text_size(px(12.0))
                .child(t!("settings.about.downloads_to"))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font_family.clone())
                        .child(dir.display().to_string()),
                ),
        );
    }
    rows.push(
        h_flex()
            .items_center()
            .justify_between()
            .gap(px(16.0))
            .child(leading)
            .child(div().flex_none().h(px(CONTROL_HEIGHT)).child(check))
            .into_any_element(),
    );
    rows
}

/// The accent banner shown once an update is downloaded and verified, or, for a portable copy,
/// as soon as a newer version exists.
fn render_ready_banner(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    ready: &ReadyUpdate,
    manual: ManualUpdate,
) -> AnyElement {
    let version = ready.version.as_str();
    let (title, text) = if ready.is_manual_download() {
        let text = match manual {
            ManualUpdate::PortableZip => t!("settings.about.available.text"),
            ManualUpdate::Download => t!("settings.about.available.text_linux"),
            ManualUpdate::InstallScript => t!("settings.about.available.text_script"),
        };
        (
            t!("settings.about.available.title", version = version),
            text,
        )
    } else {
        (
            t!("settings.about.ready.title", version = version),
            t!("settings.about.ready.text"),
        )
    };
    let button = if ready.is_manual_download() && manual == ManualUpdate::InstallScript {
        PrimaryButton::new("about-copy-command", t!("settings.about.available.copy")).on_click(
            move |_, window, cx| {
                let _ = weak.update(cx, |view, cx| view.copy_update_command(window, cx));
            },
        )
    } else if ready.is_manual_download() {
        let notes_url = ready.notes_url.clone();
        PrimaryButton::new("about-download", t!("settings.about.available.download"))
            .on_click(move |_, _, cx| cx.open_url(&notes_url))
    } else {
        PrimaryButton::new("about-restart", t!("settings.about.ready.restart")).on_click(
            move |_, window, cx| {
                let _ = weak.update(cx, |view, cx| view.request_update_restart(window, cx));
            },
        )
    };
    h_flex()
        .items_center()
        .gap(px(12.0))
        .p(px(14.0))
        .rounded(px(RADIUS_LG))
        .bg(palette.accent_subtle)
        .border_1()
        .border_color(palette.accent.opacity(0.35))
        .child(
            Icon::new(gpui_kit::assets::IconName::Download)
                .size(px(18.0))
                .text_color(palette.accent_text),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .child(
                    div()
                        .text_color(palette.fg_muted)
                        .text_size(px(12.0))
                        .child(text),
                ),
        )
        .child(button)
        .into_any_element()
}

/// A neutral row with the last check's result. `None` before any check finished.
fn render_status_row(palette: &Palette, status: CheckStatus, version: &str) -> Option<AnyElement> {
    let (icon, text) = match status {
        CheckStatus::Idle => return None,
        CheckStatus::Checking => (
            gpui_kit::assets::IconName::RefreshCw,
            t!("update.checking").into_owned(),
        ),
        CheckStatus::UpToDate => (
            gpui_kit::assets::IconName::Check,
            t!("update.up_to_date", version = version).into_owned(),
        ),
        CheckStatus::Failed => (
            gpui_kit::assets::IconName::CircleAlert,
            t!("settings.about.status.failed").into_owned(),
        ),
    };
    Some(
        h_flex()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(10.0))
            .rounded(px(RADIUS_LG))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .text_color(palette.fg_muted)
            .child(Icon::new(icon).small())
            .child(text)
            .into_any_element(),
    )
}

/// Variant 3b: no updater in this build, so updates come from somewhere else.
fn render_distro_updates(
    palette: &Palette,
    mono_font_family: &SharedString,
    info: &AboutInfo,
) -> Vec<AnyElement> {
    let (title, text) = if cfg!(target_os = "linux") {
        (
            t!("settings.about.managed.title"),
            t!("settings.about.managed.text"),
        )
    } else {
        (
            t!("settings.about.unavailable.title"),
            t!("settings.about.unavailable.text"),
        )
    };
    let mut rows = vec![
        h_flex()
            .items_start()
            .gap(px(12.0))
            .p(px(14.0))
            .rounded(px(RADIUS_LG))
            .bg(palette.surface)
            .border_1()
            .border_color(palette.border)
            .child(
                div().mt(px(1.0)).child(
                    Icon::new(gpui_kit::assets::IconName::Package)
                        .size(px(18.0))
                        .text_color(palette.fg_muted),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.0))
                    .child(div().font_weight(FontWeight::MEDIUM).child(title))
                    .child(
                        div()
                            .text_color(palette.fg_muted)
                            .line_height(relative(1.5))
                            .child(text),
                    )
                    .when(cfg!(target_os = "linux"), |this| {
                        this.child(
                            div().pt(px(6.0)).child(
                                SecondaryButton::new(
                                    "about-install-instructions",
                                    t!("settings.about.managed.link"),
                                )
                                .icon(Icon::new(gpui_kit::assets::IconName::ExternalLink))
                                .on_click(|_, _, cx| cx.open_url(about::INSTALL_URL)),
                            ),
                        )
                    }),
            )
            .into_any_element(),
    ];
    if let Some(exe) = &info.exe {
        rows.push(
            h_flex()
                .items_center()
                .gap(px(8.0))
                .text_color(palette.fg_muted)
                .text_size(px(12.0))
                .child(Icon::new(gpui_kit::assets::IconName::Info).small())
                .child(t!("settings.about.installed_from"))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font_family.clone())
                        .text_color(palette.fg)
                        .child(exe.display().to_string()),
                )
                .into_any_element(),
        );
    }
    rows
}
