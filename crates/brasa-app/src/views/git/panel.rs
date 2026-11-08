//! The Source Control panel of the sidebar (design 2b): the workspace's changed files as a tree
//! with checkboxes that mirror the git index, and the empty states (6a git missing, 6b not a
//! repository, 6c no changes). The rows come from `state::git::change_rows`, rebuilt by
//! [`AppView::rebuild_git_rows`] when the status changes, never in render.

use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::actions::CommitFromMessage;
use crate::state::git::{
    ChangeGroup, ChangeRow, ChangeRowKind, Check, GitAvailability, RepoState, change_rows,
    fill_methods, staged_outside,
};
use crate::state::git_diff::DiffTarget;
use crate::state::git_sync::{can_commit, can_write_message, staged_secrets};
use crate::state::locale::plural;
use crate::state::number::format_integer;
use crate::state::settings::{GitCommitAction, SettingsCategory};
use crate::theme::metrics::ICON_BUTTON_HEADER;
use crate::theme::{Palette, PaletteExt};
use telaio_ui::components::{CheckState, TextArea, TriCheckbox};

use crate::views::components::{
    ConfirmDialog, EmptyState, GhostButton, IconButton, MethodBadge, NeutralButtonExt,
    PrimaryButton, SecondaryButton, SectionLabel, TreeRow, open_confirm_dialog,
};

use super::super::root::AppView;
use super::sync::open_git_output;

/// Where the "How to install Git" link points.
pub(crate) const INSTALL_GIT_URL: &str = "https://brasa.tanis.codes/docs/git.html#installing-git";

/// What a click on a row's checkbox does.
#[derive(Clone, Copy)]
enum IndexAction {
    Stage,
    Unstage,
}

impl AppView {
    /// Rebuilds [`crate::state::git::GitState::rows`] from the last status, the folded folders
    /// and the collection tree (for the method badges). Call when any of them changes.
    pub(crate) fn rebuild_git_rows(&mut self) {
        let (rows, outside, secrets) = match self.git.status() {
            Some((repo, status)) => {
                let mut rows =
                    change_rows(status, repo.prefix(), &self.git.collapsed, self.git.flat);
                if let Some(workspace) = self.state.workspace.as_ref() {
                    fill_methods(&mut rows, workspace.tree());
                }
                (
                    rows,
                    staged_outside(status, repo.prefix()),
                    staged_secrets(status),
                )
            }
            None => (Vec::new(), Vec::new(), Vec::new()),
        };
        self.git.rows = Arc::new(rows);
        self.git.staged_outside = Arc::new(outside);
        self.git.secrets = Arc::new(secrets);
    }

    /// Folds or unfolds a group or folder of the panel.
    fn toggle_git_row(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.git.collapsed.remove(&key) {
            self.git.collapsed.insert(key);
        }
        self.rebuild_git_rows();
        cx.notify();
    }

    /// Stages or unstages `paths` (repository relative) on the background executor, then reads
    /// the status again.
    fn change_index(&mut self, action: IndexAction, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        self.run_git_write(cx, move || match action {
            IndexAction::Stage => repo.stage(&paths),
            IndexAction::Unstage => repo.unstage(&paths),
        });
    }

    /// Discards the changes of `paths`, after a confirmation when the setting asks for one.
    fn discard_paths(
        &mut self,
        label: String,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        // An empty path would cover the whole repository.
        if paths.is_empty() || paths.iter().any(|path| path.is_empty()) {
            return;
        }
        if !self.state.settings.git_confirm_discard {
            self.run_git_work_tree_write(cx, move || repo.discard(&paths));
            return;
        }
        let weak = cx.weak_entity();
        open_confirm_dialog(
            window,
            cx,
            ConfirmDialog {
                title: t!("git.discard.title").into(),
                text: t!("git.discard.text", name = label).into(),
                confirm_label: t!("git.discard.confirm").into(),
                destructive: true,
                show_cancel: true,
                on_confirm: Rc::new(move |_, cx| {
                    let repo = repo.clone();
                    let paths = paths.clone();
                    let _ = weak.update(cx, |view, cx| {
                        view.run_git_work_tree_write(cx, move || repo.discard(&paths));
                    });
                }),
            },
        );
    }

    /// Opens Settings on its Git category.
    pub(crate) fn open_git_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_category = SettingsCategory::Git;
        self.open_settings(window, cx);
    }

    /// The panel header's buttons (design 7a): refresh, discard every change, the folder
    /// grouping, the log (#174, disabled until it exists) and the menu of the rest.
    fn render_git_toolbar(&self, weak: WeakEntity<Self>, cx: &App) -> AnyElement {
        let palette = cx.palette();
        let refresh = {
            let weak = weak.clone();
            IconButton::new("git-refresh", IconName::RefreshCw)
                .box_size(ICON_BUTTON_HEADER)
                .tooltip(t!("git.panel.refresh"))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |view, cx| view.refresh_git_status(cx));
                })
        };
        let all_paths: Vec<String> = self
            .git
            .rows
            .iter()
            .filter(|row| matches!(row.kind, ChangeRowKind::Group(_)))
            .flat_map(|row| row.repo_paths.iter().cloned())
            .collect();
        let discard = {
            let weak = weak.clone();
            IconButton::new("git-discard-all", IconName::Undo2)
                .box_size(ICON_BUTTON_HEADER)
                .tooltip(t!("git.panel.discard_all"))
                .disabled(all_paths.is_empty())
                .on_click(move |_, window, cx| {
                    let paths = all_paths.clone();
                    let label = t!("git.panel.all_changes").into_owned();
                    let _ =
                        weak.update(cx, |view, cx| view.discard_paths(label, paths, window, cx));
                })
        };
        let (icon, tooltip) = if self.git.flat {
            (IconName::List, t!("git.panel.group_by_folder"))
        } else {
            (IconName::ListTree, t!("git.panel.show_as_list"))
        };
        let grouping = {
            let weak = weak.clone();
            IconButton::new("git-grouping", icon)
                .box_size(ICON_BUTTON_HEADER)
                .tooltip(tooltip)
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        view.git.flat = !view.git.flat;
                        view.rebuild_git_rows();
                        cx.notify();
                    });
                })
        };
        let log = {
            let weak = weak.clone();
            IconButton::new("git-log", IconName::GitGraph)
                .box_size(ICON_BUTTON_HEADER)
                .tooltip(t!("git.panel.log"))
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.open_git_log(None, window, cx));
                })
        };
        let running = self.git.sync.running.is_some();
        let failure_output: Option<SharedString> = self
            .git
            .sync
            .failure
            .as_ref()
            .map(|failure| failure.output.clone().into());
        let more = Button::new("git-more")
            .neutral_ghost(cx, palette.fg_muted)
            .icon(Icon::new(IconName::Ellipsis))
            .w(px(ICON_BUTTON_HEADER))
            .h(px(ICON_BUTTON_HEADER))
            .tooltip(t!("git.panel.more"))
            .dropdown_menu(move |menu, _, _| {
                let item = |label: Cow<'static, str>, action: fn(&mut AppView, &mut Window, &mut Context<AppView>)| {
                    let weak = weak.clone();
                    PopupMenuItem::new(label)
                        .disabled(running)
                        .on_click(move |_, window, cx| {
                            let _ = weak.update(cx, |view, cx| action(view, window, cx));
                        })
                };
                let settings = {
                    let weak = weak.clone();
                    PopupMenuItem::new(t!("git.empty.open_settings")).on_click(move |_, window, cx| {
                        let _ = weak.update(cx, |view, cx| view.open_git_settings(window, cx));
                    })
                };
                menu.item(item(t!("git.menu.pull"), |view, window, cx| view.pull(false, window, cx)))
                    .item(item(t!("git.menu.push"), |view, window, cx| {
                        view.start_push(false, window, cx)
                    }))
                    .item(item(t!("git.menu.fetch"), |view, window, cx| view.fetch(false, window, cx)))
                    .separator()
                    .when_some(failure_output.clone(), |menu, output| {
                        menu.item(PopupMenuItem::new(t!("git.failure.show_output")).on_click(
                            move |_, window, cx| open_git_output(output.clone(), window, cx),
                        ))
                    })
                    .item(settings)
            });
        h_flex()
            .gap(px(2.0))
            .child(refresh)
            .child(discard)
            .child(grouping)
            .child(log)
            .child(more)
            .into_any_element()
    }

    /// Renders the Source Control panel.
    pub(crate) fn render_git_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let header = h_flex()
            .justify_between()
            .items_center()
            .h(px(36.0))
            .pt(px(10.0))
            .pr(px(10.0))
            .pb(px(6.0))
            .pl(px(14.0))
            .child(SectionLabel::new(t!("git.panel.title")))
            .children(
                self.git
                    .repo()
                    .map(|_| self.render_git_toolbar(weak.clone(), cx)),
            );

        v_flex()
            .size_full()
            .bg(palette.surface)
            .border_r_1()
            .border_color(palette.border)
            .child(header)
            .child(self.render_git_body(weak.clone(), &palette, cx))
            .children(self.render_commit_box(weak, &palette, cx))
            .into_any_element()
    }

    fn render_git_body(
        &self,
        weak: WeakEntity<Self>,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> AnyElement {
        if self.state.workspace.is_none() {
            return panel_empty(EmptyState::new(t!("git.empty.no_workspace")));
        }
        match &self.git.availability {
            GitAvailability::Detecting => return div().into_any_element(),
            GitAvailability::Missing(_) => return render_git_missing(weak, palette),
            GitAvailability::Ready(_) => {}
        }
        match &self.git.repo {
            RepoState::Unknown | RepoState::Repo { status: None, .. } => div().into_any_element(),
            RepoState::NotARepo => self.render_not_a_repo(weak),
            RepoState::Error(message) => panel_empty(
                EmptyState::new(t!("git.empty.error"))
                    .icon(Icon::new(IconName::CircleAlert))
                    .description(message.clone()),
            ),
            RepoState::Repo {
                status: Some(status),
                ..
            } => {
                if self.git.rows.is_empty() {
                    let description = if status.ahead > 0 {
                        plural("git.empty.ahead", status.ahead as usize)
                    } else {
                        t!("git.empty.clean").into_owned()
                    };
                    let ahead = status.ahead as usize;
                    let push = (ahead > 0).then(|| {
                        let weak = weak.clone();
                        PrimaryButton::new("git-push-ahead", plural("git.empty.push", ahead))
                            .icon(Icon::new(IconName::ArrowUp))
                            .disabled(self.git.sync.running.is_some())
                            .on_click(move |_, window, cx| {
                                let _ =
                                    weak.update(cx, |view, cx| view.start_push(false, window, cx));
                            })
                    });
                    return v_flex()
                        .flex_1()
                        .min_h_0()
                        .children(self.render_outside_notice(weak.clone(), palette))
                        .child(panel_empty(
                            EmptyState::new(t!("git.empty.no_changes"))
                                .icon(Icon::new(IconName::CircleCheck))
                                .description(description)
                                .when_some(push, |state, push| state.action(push)),
                        ))
                        .into_any_element();
                }
                let rows = self.git.rows.clone();
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .children(self.render_outside_notice(weak.clone(), palette))
                    .child(
                        div()
                            .id("git-panel-rows")
                            .flex_1()
                            .min_h_0()
                            .px(px(6.0))
                            .overflow_y_scroll()
                            .child(v_flex().gap(px(1.0)).children(rows.iter().enumerate().map(
                                |(index, row)| {
                                    render_change_row(index, row, weak.clone(), palette, cx)
                                },
                            ))),
                    )
                    .into_any_element()
            }
        }
    }

    /// 6b: the workspace is not in a repository.
    fn render_not_a_repo(&self, weak: WeakEntity<Self>) -> AnyElement {
        panel_empty(
            EmptyState::new(t!("git.empty.not_a_repo"))
                .icon(Icon::new(IconName::GitBranch))
                .description(t!("git.empty.not_a_repo_text"))
                .action(
                    PrimaryButton::new("git-init", t!("git.empty.init"))
                        .icon(Icon::new(IconName::Plus))
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |view, cx| view.init_git_repository(cx));
                        }),
                ),
        )
    }

    /// The notice over the rows when files outside the workspace are staged: they would go into
    /// the next commit although the panel does not list them.
    fn render_outside_notice(
        &self,
        weak: WeakEntity<Self>,
        palette: &Palette,
    ) -> Option<AnyElement> {
        let outside = self.git.staged_outside.clone();
        if outside.is_empty() {
            return None;
        }
        Some(
            v_flex()
                .mx(px(8.0))
                .mb(px(6.0))
                .p(px(8.0))
                .gap(px(6.0))
                .rounded(px(6.0))
                .bg(palette.warning.opacity(0.10))
                .text_size(px(12.0))
                .text_color(palette.fg_muted)
                .child(plural("git.panel.staged_outside", outside.len()))
                .child(
                    div().child(
                        SecondaryButton::new("git-unstage-outside", t!("git.panel.unstage_them"))
                            .on_click(move |_, _, cx| {
                                let paths = outside.to_vec();
                                let _ = weak.update(cx, |view, cx| {
                                    view.change_index(IndexAction::Unstage, paths, cx);
                                });
                            }),
                    ),
                )
                .into_any_element(),
        )
    }
}

impl AppView {
    /// The commit box under the rows (design 7a): the message, Amend, the summary line and the
    /// two commit buttons, with the secrets banner (6d) over them when it applies.
    fn render_commit_box(
        &self,
        weak: WeakEntity<Self>,
        palette: &Palette,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.git.status()?;
        let running = self.git.sync.running.is_some();
        let stats = self.git.commit.stats;
        let amend = self.git.commit.amend;
        let message = self.commit_input.read(cx).value();
        let blocked = self.state.settings.git_block_secrets && !self.git.secrets.is_empty();
        let enabled = can_commit(&message, stats, amend, running) && !blocked;

        let message_box = {
            let weak = weak.clone();
            div()
                .key_context("CommitMessage")
                .on_action(move |_: &CommitFromMessage, window, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        let push =
                            view.state.settings.git_commit_action == GitCommitAction::CommitAndPush;
                        view.commit_from_box(push, window, cx);
                    });
                })
                .child(TextArea::new(&self.commit_input).disabled(!can_write_message(stats, amend)))
        };
        let amend_box = {
            let weak = weak.clone();
            let state = if amend {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            TriCheckbox::new("git-amend", state)
                .label(t!("git.commit.amend"))
                .disabled(running)
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.toggle_amend(window, cx));
                })
        };
        let mono = cx.theme().mono_font_family.clone();
        let summary = h_flex()
            .gap(px(4.0))
            .font_family(mono)
            .text_size(px(11.5))
            .text_color(palette.fg_subtle)
            .child(format!(
                "{} \u{b7}",
                plural("git.commit.files", stats.files)
            ))
            .child(
                div()
                    .text_color(palette.success)
                    .child(format!("+{}", format_integer(stats.additions))),
            )
            .child(
                div()
                    .text_color(palette.danger)
                    .child(format!("\u{2212}{}", format_integer(stats.deletions))),
            );
        let commit = {
            let weak = weak.clone();
            PrimaryButton::new("git-commit", t!("git.commit.commit"))
                .full_width()
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.commit_from_box(false, window, cx));
                })
        };
        let commit_and_push = {
            let weak = weak.clone();
            SecondaryButton::new("git-commit-push", t!("git.commit.commit_and_push"))
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.commit_from_box(true, window, cx));
                })
        };
        Some(
            v_flex()
                .flex_none()
                .gap(px(8.0))
                .p(px(10.0))
                .border_t_1()
                .border_color(palette.border)
                .child(message_box)
                .when(blocked, |column| {
                    column.child(self.render_secrets_banner(weak.clone(), palette))
                })
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(14.0))
                        .gap_y(px(4.0))
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(div().flex_1().child(amend_box))
                        .child(summary),
                )
                .child(
                    h_flex()
                        .gap(px(6.0))
                        .child(div().flex_1().min_w_0().child(commit))
                        .child(div().flex_none().child(commit_and_push)),
                )
                .into_any_element(),
        )
    }

    /// 6d: staged files hold secrets, so the commit is blocked until they are unstaged or
    /// ignored.
    fn render_secrets_banner(&self, weak: WeakEntity<Self>, palette: &Palette) -> AnyElement {
        let secrets = self.git.secrets.clone();
        let names = secrets
            .iter()
            .map(|path| path.rsplit('/').next().unwrap_or(path))
            .collect::<Vec<_>>()
            .join(", ");
        let unstage = {
            let weak = weak.clone();
            let paths = secrets.to_vec();
            SecondaryButton::new("git-secrets-unstage", t!("git.secrets.unstage")).on_click(
                move |_, _, cx| {
                    let paths = paths.clone();
                    let _ = weak.update(cx, |view, cx| {
                        view.change_index(IndexAction::Unstage, paths, cx);
                    });
                },
            )
        };
        let ignore = {
            let paths = secrets.to_vec();
            GhostButton::new("git-secrets-ignore", t!("git.secrets.ignore")).on_click(
                move |_, _, cx| {
                    let paths = paths.clone();
                    let _ = weak.update(cx, |view, cx| view.ignore_secrets(paths, cx));
                },
            )
        };
        v_flex()
            .p(px(10.0))
            .gap(px(8.0))
            .rounded(px(8.0))
            .bg(palette.danger.opacity(0.10))
            .border_1()
            .border_color(palette.danger.opacity(0.35))
            .child(
                h_flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        Icon::new(IconName::ShieldAlert)
                            .small()
                            .text_color(palette.danger),
                    )
                    .child(div().flex_1().min_w_0().text_size(px(12.5)).child(
                        plural("git.secrets.text", secrets.len()).replace("%{files}", &names),
                    )),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(unstage)
                    .child(ignore),
            )
            .into_any_element()
    }

    /// "Add to .gitignore": ignores each secret file and takes it out of the index.
    fn ignore_secrets(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.run_git_write(cx, move || {
            for path in &paths {
                repo.add_to_gitignore(path)?;
            }
            repo.unstage(&paths)
        });
    }
}

/// 6a: git is not installed, too old or failed to run.
fn render_git_missing(weak: WeakEntity<AppView>, palette: &Palette) -> AnyElement {
    panel_empty(
        EmptyState::new(t!("git.empty.missing"))
            .icon(Icon::new(IconName::CircleAlert))
            .description(t!("git.empty.missing_text"))
            .action(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        SecondaryButton::new("git-open-settings", t!("git.empty.open_settings"))
                            .icon(Icon::new(IconName::Settings))
                            .on_click(move |_, window, cx| {
                                let _ =
                                    weak.update(cx, |view, cx| view.open_git_settings(window, cx));
                            }),
                    )
                    .child(
                        div()
                            .id("git-install-link")
                            .text_size(px(12.0))
                            .text_color(palette.accent_text)
                            .cursor_pointer()
                            .hover(|style| style.underline())
                            .child(t!("git.empty.install"))
                            .on_click(|_, _, cx| cx.open_url(INSTALL_GIT_URL)),
                    ),
            ),
    )
}

/// An empty state laid out inside the panel, with side padding so its text wraps.
fn panel_empty(state: EmptyState) -> AnyElement {
    div()
        .flex_1()
        .min_h_0()
        .px(px(16.0))
        .child(state)
        .into_any_element()
}

/// The status letter's color: modified amber, added and untracked green, deleted red, renamed
/// blue, conflicts red.
pub(super) fn letter_color(letter: char, palette: &Palette) -> Hsla {
    match letter {
        'A' | 'U' => palette.success,
        'D' | '!' => palette.danger,
        'R' => palette.info,
        _ => palette.warning,
    }
}

/// The checkbox state of a [`Check`].
fn check_state(check: Check) -> CheckState {
    match check {
        Check::On => CheckState::Checked,
        Check::Off => CheckState::Unchecked,
        Check::Partial => CheckState::Mixed,
    }
}

/// One row of the panel: a group header, a folder or a file.
fn render_change_row(
    index: usize,
    row: &ChangeRow,
    weak: WeakEntity<AppView>,
    palette: &Palette,
    cx: &Context<AppView>,
) -> AnyElement {
    let check = row.check;
    let paths = row.repo_paths.clone();
    let checkbox = {
        let weak = weak.clone();
        let paths = paths.clone();
        TriCheckbox::new(("git-check", index), check_state(check)).on_click(move |_, _, cx| {
            let action = match check {
                Check::On => IndexAction::Unstage,
                Check::Off | Check::Partial => IndexAction::Stage,
            };
            let paths = paths.clone();
            let _ = weak.update(cx, |view, cx| view.change_index(action, paths, cx));
        })
    };
    let discard_label = match &row.kind {
        ChangeRowKind::Group(_) => t!("git.panel.all_changes").into_owned(),
        _ => row.path.clone(),
    };
    let discard = {
        let weak = weak.clone();
        let paths = paths.clone();
        IconButton::new(("git-discard", index), IconName::Undo2)
            .box_size(22.0)
            .tooltip(t!("git.panel.discard"))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                let paths = paths.clone();
                let label = discard_label.clone();
                let _ = weak.update(cx, |view, cx| view.discard_paths(label, paths, window, cx));
            })
    };
    let mono = cx.theme().mono_font_family.clone();
    let count = |count: usize| {
        div()
            .font_family(mono.clone())
            .text_size(px(11.0))
            .text_color(palette.fg_subtle)
            .child(format_integer(count as u64))
    };

    match &row.kind {
        ChangeRowKind::Group(group) => {
            let label = match group {
                ChangeGroup::Changes => t!("git.panel.changes"),
                ChangeGroup::Unversioned => t!("git.panel.unversioned"),
            };
            let key = row.key.clone();
            TreeRow::new(("git-row", index), label)
                .folder(row.expanded)
                .leading(checkbox)
                .trailing(discard)
                .suffix(count(row.count))
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    let _ = weak.update(cx, |view, cx| view.toggle_git_row(key, cx));
                })
                .into_any_element()
        }
        ChangeRowKind::Folder => {
            let key = row.key.clone();
            TreeRow::new(("git-row", index), row.label.clone())
                .depth(row.depth)
                .folder(row.expanded)
                .leading(
                    h_flex().gap(px(6.0)).items_center().child(checkbox).child(
                        Icon::new(IconName::Folder)
                            .small()
                            .text_color(palette.fg_subtle),
                    ),
                )
                .trailing(discard)
                .suffix(count(row.count))
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    let _ = weak.update(cx, |view, cx| view.toggle_git_row(key, cx));
                })
                .into_any_element()
        }
        ChangeRowKind::File { letter, deleted } => {
            let letter_element = div()
                .w(px(12.0))
                .font_family(mono.clone())
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(letter_color(*letter, palette))
                .child(letter.to_string());
            let leading = h_flex()
                .gap(px(6.0))
                .items_center()
                .child(checkbox)
                .children(row.method.clone().map(MethodBadge::inline));
            let repo_path = row.repo_paths.first().cloned().unwrap_or_default();
            let path = row.path.clone();
            let (letter, deleted) = (*letter, *deleted);
            // A flat list (depth 1) names the folder each file is in.
            let folder = (row.depth == 1)
                .then(|| {
                    row.path
                        .rsplit_once('/')
                        .map(|(folder, _)| folder.to_string())
                })
                .flatten();
            TreeRow::new(("git-row", index), row.label.clone())
                .depth(row.depth)
                .when_some(folder, |row, folder| row.detail(folder))
                .strikethrough(deleted)
                .leading(leading)
                .trailing(discard)
                .suffix(letter_element)
                .on_click(move |_, _, cx| {
                    let (repo_path, path) = (repo_path.clone(), path.clone());
                    let _ = weak.update(cx, |view, cx| {
                        view.open_git_diff(repo_path, path, DiffTarget::WorkTree, letter, cx);
                    });
                })
                .into_any_element()
        }
    }
}
