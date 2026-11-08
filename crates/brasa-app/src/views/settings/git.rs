//! The "Git" pane of the Settings modal (design 5a): the git executable and what was detected,
//! syncing, pulling, the default commit action, the safety checks and the read only commit
//! author. The settings live in `state::settings`; changing the executable detects git again.

use std::path::PathBuf;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_git::{DetectError, MIN_VERSION};
use telaio_ui::components::{LabeledRow, SectionHeading};

use crate::state::git::GitAvailability;
use crate::state::settings::{GitCommitAction, GitPull, Settings};
use crate::theme::Palette;
use crate::views::components::{
    GhostButton, SecondaryButton, SegmentedControl, SegmentedItem, switch_halo,
};
use crate::views::git::INSTALL_GIT_URL;
use crate::views::root::AppView;

use super::selects::{SettingsSelect, render_select};

/// What the pane needs from [`AppView`], cloned out so nothing borrows the view while rendering.
pub(super) struct GitPane {
    availability: GitAvailability,
    author: Option<telaio_git::Author>,
}

impl GitPane {
    /// Captures the pane's data from `view`.
    pub(super) fn capture(view: &AppView) -> Self {
        Self {
            availability: view.git.availability.clone(),
            author: view.git.author.clone(),
        }
    }
}

/// "2.30", the oldest git version, for the hints.
fn min_version() -> String {
    format!("{}.{}", MIN_VERSION.major, MIN_VERSION.minor)
}

impl AppView {
    /// Picks the git executable (`None` for auto-detect) and detects git again.
    pub(super) fn set_git_executable(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.settings.git_executable == path {
            return;
        }
        self.state.settings.git_executable = path;
        self.apply_settings_live(window, cx);
        self.detect_git(cx);
    }

    pub(super) fn set_git_fetch_automatically(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.git_fetch_automatically = enabled;
        self.apply_settings_live(window, cx);
    }

    fn set_git_pull(&mut self, choice: GitPull, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.git_pull = choice;
        self.apply_settings_live(window, cx);
    }

    fn set_git_commit_action(
        &mut self,
        choice: GitCommitAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.git_commit_action = choice;
        self.apply_settings_live(window, cx);
    }

    fn set_git_block_secrets(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.git_block_secrets = enabled;
        self.apply_settings_live(window, cx);
    }

    fn set_git_confirm_discard(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.git_confirm_discard = enabled;
        self.apply_settings_live(window, cx);
    }
}

/// The pane's sections: executable, sync, pull, safety and the commit author.
pub(super) fn render_git(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    settings: &Settings,
    select: &SettingsSelect,
    mono_font_family: &SharedString,
    pane: &GitPane,
) -> Vec<AnyElement> {
    vec![
        render_executable(weak.clone(), palette, select, pane),
        render_sync(weak.clone(), palette, settings),
        render_pull(weak.clone(), settings.git_pull),
        render_commit_action(weak.clone(), settings.git_commit_action),
        render_safety(weak, palette, settings, mono_font_family),
        render_author(palette, mono_font_family, pane),
    ]
}

fn render_executable(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    select: &SettingsSelect,
    pane: &GitPane,
) -> AnyElement {
    let browse_weak = weak.clone();
    let detect_weak = weak;
    let controls = h_flex()
        .gap(px(8.0))
        .child(div().flex_1().min_w_0().child(render_select(
            "settings-git-executable",
            select,
            None,
        )))
        .child(
            SecondaryButton::new("settings-git-browse", t!("settings.git.browse"))
                .on_click(move |_, _, cx| pick_executable(browse_weak.clone(), cx)),
        )
        .child(
            SecondaryButton::new("settings-git-detect", t!("settings.git.detect_again"))
                .icon(Icon::new(gpui_kit::assets::IconName::RefreshCw))
                .on_click(move |_, _, cx| {
                    let _ = detect_weak.update(cx, |view, cx| view.detect_git(cx));
                }),
        );

    let (icon, color, text, hint) = match &pane.availability {
        GitAvailability::Detecting => (
            gpui_kit::assets::IconName::RefreshCw,
            palette.fg_muted,
            t!("settings.git.detecting").into_owned(),
            None,
        ),
        GitAvailability::Ready(git) => (
            gpui_kit::assets::IconName::CircleCheck,
            palette.success,
            match git.wsl_distro() {
                Some(distro) => t!(
                    "settings.git.found_wsl",
                    version = git.version().to_string(),
                    distro = distro
                ),
                None => t!("settings.git.found", version = git.version().to_string()),
            }
            .into_owned(),
            Some(t!("settings.git.needs", min = min_version()).into_owned()),
        ),
        GitAvailability::Missing(error) => (
            gpui_kit::assets::IconName::TriangleAlert,
            palette.danger,
            detect_error_text(error),
            None,
        ),
    };
    let missing = matches!(pane.availability, GitAvailability::Missing(_));
    let status = h_flex()
        .items_center()
        .gap(px(8.0))
        .text_size(px(12.0))
        .text_color(color)
        .child(Icon::new(icon).small())
        .child(div().min_w_0().child(text))
        .children(hint.map(|hint| {
            div()
                .text_color(palette.fg_subtle)
                .child(format!("\u{b7} {hint}"))
        }));

    v_flex()
        .gap(px(10.0))
        .child(
            SectionHeading::new(t!("settings.git.executable"))
                .description(t!("settings.git.executable_description")),
        )
        .child(controls)
        .child(status)
        .when(missing, |this| {
            this.child(
                h_flex().child(
                    GhostButton::new("settings-git-install", t!("settings.git.install_help"))
                        .icon(Icon::new(gpui_kit::assets::IconName::ExternalLink))
                        .on_click(|_, _, cx| cx.open_url(INSTALL_GIT_URL)),
                ),
            )
        })
        .into_any_element()
}

/// The translated message for a failed detection.
fn detect_error_text(error: &DetectError) -> String {
    match error {
        DetectError::NotFound => t!("settings.git.not_found").into_owned(),
        DetectError::XcodeToolsMissing => t!("settings.git.xcode_missing").into_owned(),
        DetectError::TooOld { version, .. } => t!(
            "settings.git.too_old",
            version = version.to_string(),
            min = min_version()
        )
        .into_owned(),
        DetectError::Failed(message) => {
            t!("settings.git.failed", error = message.as_str()).into_owned()
        }
    }
}

/// Opens the native file picker and, once a file is picked, makes it the git executable.
fn pick_executable(weak: WeakEntity<AppView>, cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(mut paths))) = receiver.await else {
            return;
        };
        let Some(path) = paths.pop() else {
            return;
        };
        cx.update(|cx| {
            let Some(window) = cx.active_window() else {
                return;
            };
            let _ = window.update(cx, |_, window, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.set_git_executable(Some(path), window, cx)
                });
            });
        });
    })
    .detach();
}

fn switch(
    id: &'static str,
    checked: bool,
    palette: &Palette,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    switch_halo(
        SharedString::from(format!("{id}-halo")),
        Switch::new(id)
            .checked(checked)
            .on_click(move |checked, window, cx| on_change(*checked, window, cx)),
        palette,
    )
    .into_any_element()
}

fn render_sync(weak: WeakEntity<AppView>, palette: &Palette, settings: &Settings) -> AnyElement {
    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.git.sync_title")))
        .child(
            LabeledRow::new(
                t!("settings.git.fetch"),
                switch(
                    "settings-git-fetch",
                    settings.git_fetch_automatically,
                    palette,
                    move |checked, window, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.set_git_fetch_automatically(checked, window, cx)
                        });
                    },
                ),
            )
            .description(t!("settings.git.fetch_description")),
        )
        .into_any_element()
}

fn render_pull(weak: WeakEntity<AppView>, current: GitPull) -> AnyElement {
    let mut control = SegmentedControl::new("settings-git-pull");
    for choice in GitPull::ALL {
        let weak = weak.clone();
        control = control.item(
            SegmentedItem::new(choice.label())
                .selected(choice == current)
                .on_click(move |window, cx| {
                    let _ = weak.update(cx, |view, cx| view.set_git_pull(choice, window, cx));
                }),
        );
    }
    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.git.pull_title")))
        .child(
            LabeledRow::new(t!("settings.git.pull_label"), control)
                .description(t!("settings.git.pull_description")),
        )
        .into_any_element()
}

fn render_commit_action(weak: WeakEntity<AppView>, current: GitCommitAction) -> AnyElement {
    let mut control = SegmentedControl::new("settings-git-commit-action");
    for choice in GitCommitAction::ALL {
        let weak = weak.clone();
        control = control.item(
            SegmentedItem::new(choice.label())
                .selected(choice == current)
                .on_click(move |window, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        view.set_git_commit_action(choice, window, cx)
                    });
                }),
        );
    }
    LabeledRow::new(t!("settings.git.commit_action"), control)
        .description(t!("settings.git.commit_action_description"))
        .into_any_element()
}

fn render_safety(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    settings: &Settings,
    _mono_font_family: &SharedString,
) -> AnyElement {
    let secrets_weak = weak.clone();
    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.git.safety_title")))
        .child(
            LabeledRow::new(
                t!("settings.git.block_secrets"),
                switch(
                    "settings-git-block-secrets",
                    settings.git_block_secrets,
                    palette,
                    move |checked, window, cx| {
                        let _ = secrets_weak.update(cx, |view, cx| {
                            view.set_git_block_secrets(checked, window, cx)
                        });
                    },
                ),
            )
            .description(t!("settings.git.block_secrets_description")),
        )
        .child(
            LabeledRow::new(
                t!("settings.git.confirm_discard"),
                switch(
                    "settings-git-confirm-discard",
                    settings.git_confirm_discard,
                    palette,
                    move |checked, window, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.set_git_confirm_discard(checked, window, cx)
                        });
                    },
                ),
            )
            .description(t!("settings.git.confirm_discard_description")),
        )
        .into_any_element()
}

fn render_author(palette: &Palette, mono_font_family: &SharedString, pane: &GitPane) -> AnyElement {
    let value = match &pane.author {
        Some(author) => v_flex()
            .items_end()
            .child(div().child(author.name.clone()))
            .child(
                div()
                    .font_family(mono_font_family.clone())
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(author.email.clone()),
            )
            .into_any_element(),
        None => div()
            .text_color(palette.fg_muted)
            .child(t!("settings.git.author_not_configured"))
            .into_any_element(),
    };
    LabeledRow::new(t!("settings.git.author"), value)
        .description(t!("settings.git.author_description"))
        .into_any_element()
}
