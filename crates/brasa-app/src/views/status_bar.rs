//! The status bar at the bottom of the main window: the workspace path with a `hard-drive` icon
//! (a click opens the folder in the file manager) and, inside a git repository, the branch with a
//! `git-branch` icon in the accent color and its ahead and behind counts, the git command running
//! ("Pushing…"), a spacer, "Unsaved changes" when the active tab is
//! dirty, and "UTF-8".

use gpui_kit::component::button::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use brasa_workspace::git_branch;
use rust_i18n::t;
use telaio_ui::components::{SearchableMenu, open_tooltip};

use crate::state;
use crate::state::update::{ManualUpdate, ReadyUpdate};
use crate::theme::metrics::{STATUS_BAR_BUTTON, STATUS_BAR_HEIGHT};
use crate::theme::{Palette, PaletteExt};

use super::components::{IconButton, NeutralButtonExt};
use super::root::AppView;

impl AppView {
    /// Reads the open workspace's git branch again into [`AppView::git_branch`]. Call when the
    /// workspace opens and when the window gets the focus back, never from render.
    pub(crate) fn refresh_git_branch(&mut self) {
        self.git_branch = self
            .state
            .workspace
            .as_ref()
            .and_then(|workspace| git_branch(workspace.root()));
    }

    /// Renders the status bar.
    pub(crate) fn render_status_bar(&self, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette();
        let active_tab = self.state.tabs.active();
        let weak = cx.weak_entity();

        h_flex()
            .flex_none()
            .h(px(STATUS_BAR_HEIGHT))
            .items_center()
            .gap(px(16.0))
            .px_3()
            .bg(palette.surface)
            .border_t_1()
            .border_color(palette.border)
            .text_size(px(11.5))
            .text_color(palette.fg_subtle)
            .children(self.state.workspace.as_ref().map(|workspace| {
                let path_label =
                    state::format::shorten_path(workspace.root(), dirs::home_dir().as_deref());
                let root = workspace.root().to_path_buf();
                let tooltip = SharedString::from(t!("shell.status_bar.open_folder"));
                // git's own answer when it runs, else what `.git/HEAD` says.
                let branch = self.git.branch_label().or_else(|| self.git_branch.clone());
                h_flex()
                    .items_center()
                    .gap(px(16.0))
                    .child(
                        h_flex()
                            .id("status-bar-workspace")
                            .items_center()
                            .gap(px(6.0))
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .child(Icon::new(gpui_kit::assets::IconName::HardDrive).small())
                            .child(path_label)
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip.clone()).build(window, cx)
                            })
                            .on_click(move |_, _, cx| cx.open_with_system(&root)),
                    )
                    .children(branch.map(|branch| {
                        let (ahead, behind) = self
                            .git
                            .status()
                            .map_or((0, 0), |(_, status)| (status.ahead, status.behind));
                        let count = |icon, count: u32| {
                            (count > 0).then(|| {
                                h_flex()
                                    .items_center()
                                    .gap(px(2.0))
                                    .child(Icon::new(icon).xsmall())
                                    .child(state::number::format_integer(u64::from(count)))
                            })
                        };
                        let content = h_flex()
                            .items_center()
                            .gap(px(6.0))
                            .text_color(palette.accent_text)
                            .child(Icon::new(gpui_kit::assets::IconName::GitBranch).small())
                            .child(branch)
                            .child(Icon::new(gpui_kit::assets::IconName::ChevronUp).xsmall())
                            .children(count(gpui_kit::assets::IconName::ArrowUp, ahead))
                            .children(count(gpui_kit::assets::IconName::ArrowDown, behind));
                        // With git and a repository, the branch opens the branch menu (7b).
                        if self.git.repo().is_some() {
                            SearchableMenu::new("status-bar-branch-menu", &self.branch_menu)
                                .trigger(
                                    Button::new("status-bar-branch")
                                        .neutral_ghost(cx, palette.accent_text)
                                        .compact()
                                        .h(px(20.0))
                                        .px(px(6.0))
                                        .rounded(px(4.0))
                                        .text_size(px(11.5))
                                        .tooltip(t!("git.menu.tooltip"))
                                        .child(content),
                                )
                                .into_any_element()
                        } else {
                            content.into_any_element()
                        }
                    }))
                    .children(self.git.sync.running.map(|op| {
                        h_flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(Icon::new(gpui_kit::assets::IconName::LoaderCircle).small())
                            .child(op.progress_label())
                    }))
            }))
            .child(div().flex_1())
            .children(self.state.update.ready().map(|ready| {
                if ready.is_manual_download() {
                    render_update_download(weak.clone(), ready, self.manual_update(), &palette)
                        .into_any_element()
                } else {
                    render_update_ready(weak.clone(), ready, &palette).into_any_element()
                }
            }))
            .children(
                active_tab
                    .filter(|tab| tab.dirty)
                    .map(|_| div().child(t!("shell.status_bar.unsaved"))),
            )
            .child(div().child("UTF-8"))
            .child(
                IconButton::new("status-bar-shortcuts", gpui_kit::assets::IconName::Keyboard)
                    .box_size(STATUS_BAR_BUTTON)
                    .tooltip(open_tooltip())
                    .on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |view, cx| view.open_shortcuts(window, cx));
                    }),
            )
            .into_any_element()
    }
}

/// The discreet "update ready" item: the restart click and the release notes link. Built from
/// the stored update, no work beyond a few small clones.
fn render_update_ready(
    weak: WeakEntity<AppView>,
    ready: &ReadyUpdate,
    palette: &Palette,
) -> impl IntoElement {
    let notes_url = ready.notes_url.clone();
    h_flex()
        .items_center()
        .gap(px(12.0))
        .child(
            h_flex()
                .id("status-bar-update-restart")
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .text_color(palette.accent_text)
                .hover(|style| style.underline())
                .child(Icon::new(gpui_kit::assets::IconName::RefreshCw).small())
                .child(t!("update.status.ready", version = ready.version.as_str()))
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.request_update_restart(window, cx));
                }),
        )
        .child(
            div()
                .id("status-bar-update-notes")
                .cursor_pointer()
                .hover(|style| style.underline())
                .child(t!("update.status.notes"))
                .on_click(move |_, _, cx| cx.open_url(&notes_url)),
        )
}

/// The portable copy's "update available" item: nothing was downloaded, so a click opens the
/// release page where the new zip is.
fn render_update_download(
    weak: WeakEntity<AppView>,
    ready: &ReadyUpdate,
    manual: ManualUpdate,
    palette: &Palette,
) -> impl IntoElement {
    let notes_url = ready.notes_url.clone();
    let version = ready.version.as_str();
    let label = if manual == ManualUpdate::InstallScript {
        t!("update.status.available_script", version = version)
    } else {
        t!("update.status.available", version = version)
    };
    h_flex()
        .id("status-bar-update-download")
        .items_center()
        .gap(px(6.0))
        .cursor_pointer()
        .text_color(palette.accent_text)
        .hover(|style| style.underline())
        .child(Icon::new(gpui_kit::assets::IconName::Download).small())
        .child(label)
        .on_click(move |_, window, cx| {
            if manual == ManualUpdate::InstallScript {
                let _ = weak.update(cx, |view, cx| view.copy_update_command(window, cx));
            } else {
                cx.open_url(&notes_url);
            }
        })
}
