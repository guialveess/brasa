//! Branch operations (#173): reading the branches for the menu, checking out (with design 6f's
//! choice when there are uncommitted changes, and the automatic stash restored when coming back),
//! creating, renaming and deleting branches, and the starred ones. Every command runs on the
//! background executor through the queue of `sync.rs`.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::InputState;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_git::{FailureKind, GitError, Repo, auto_stash_message};

use crate::state::git::BranchList;
use crate::state::git_sync::GitOp;
use crate::state::locale::plural;
use crate::state::settings::save_settings;
use crate::theme::{Palette, PaletteExt};
use crate::views::components::{
    ConfirmDialog, ModalPlacement, PrimaryButton, SecondaryButton, framed_dialog, modal_body,
    modal_footer, modal_header, open_confirm_dialog,
};
use crate::views::sidebar::{NameSubmit, open_name_dialog};

use super::super::root::AppView;

/// The start of the message of the stashes Brasa creates itself, followed by the branch name.
/// It must stay `brasa-auto:` so the automatic stashes of earlier versions are still found.
const AUTO_STASH_PREFIX: &str = "brasa-auto:";

/// What to check out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckoutTarget {
    /// A local branch, or a remote one (`origin/x`), which checks out its local tracking branch.
    Branch(String),
    /// A commit, leaving `HEAD` detached.
    Revision(String),
}

impl CheckoutTarget {
    /// The name shown in the dialog.
    fn label(&self) -> &str {
        match self {
            CheckoutTarget::Branch(name) | CheckoutTarget::Revision(name) => name,
        }
    }
}

/// What happens to uncommitted changes on checkout (design 6f).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CheckoutChoice {
    /// Stash them, tagged with the branch they belong to, and restore them on the way back.
    #[default]
    Stash,
    /// Take them along, when they do not conflict.
    Bring,
}

/// What a checkout did, for the log and the notification.
struct CheckoutOutcome {
    /// Whether changes stashed earlier for the target branch were restored.
    restored: bool,
}

impl AppView {
    /// Reads the branches and the recent checkouts for the branch menu.
    pub(crate) fn load_branches(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.git_tasks.branches = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let branches = repo.branches()?;
                    let recent = repo.recent_branches(20)?;
                    Ok::<_, GitError>(BranchList { branches, recent })
                })
                .await;
            let _ = this.update(cx, |view, cx| match result {
                Ok(list) => {
                    view.git.branches = Some(Arc::new(list));
                    view.rebuild_branch_menu(cx);
                    cx.notify();
                }
                Err(error) => log::warn!("reading the branches failed: {error}"),
            });
        }));
    }

    /// Checks out `target`. With uncommitted changes it asks first what to do with them (6f).
    pub(crate) fn request_checkout(
        &mut self,
        target: CheckoutTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, status)) = self.git.status() else {
            return;
        };
        if status.entries.is_empty() {
            self.checkout(target, None, window, cx);
            return;
        }
        let changes = status.entries.len();
        let current = self.git.branch_label().unwrap_or_default();
        self.checkout_choice = CheckoutChoice::default();
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let weak = weak.clone();
            let target = target.clone();
            let current = current.clone();
            framed_dialog(
                dialog,
                440.0,
                None,
                ModalPlacement::DEFAULT,
                window,
                move |_, cx| {
                    let choice = weak
                        .upgrade()
                        .map(|view| view.read(cx).checkout_choice)
                        .unwrap_or_default();
                    let palette = cx.palette();
                    let option = |id: &'static str, value: CheckoutChoice, title, text| {
                        let weak = weak.clone();
                        checkout_option(id, choice == value, title, text, &palette).on_click(
                            move |_, _, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    view.checkout_choice = value;
                                    cx.notify();
                                });
                            },
                        )
                    };
                    let confirm = {
                        let weak = weak.clone();
                        let target = target.clone();
                        let current = current.clone();
                        PrimaryButton::new("git-checkout-confirm", t!("git.checkout.confirm"))
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let target = target.clone();
                                let current = current.clone();
                                let _ = weak.update(cx, |view, cx| {
                                    let stash_from = (view.checkout_choice
                                        == CheckoutChoice::Stash)
                                        .then_some(current);
                                    view.checkout(target, stash_from, window, cx);
                                });
                            })
                    };
                    v_flex()
                        .child(modal_header(
                            t!("git.checkout.title", name = target.label()),
                            cx,
                        ))
                        .child(
                            modal_body()
                                .child(
                                    div().text_color(palette.fg_muted).child(
                                        plural("git.checkout.text", changes)
                                            .replace("%{branch}", &current),
                                    ),
                                )
                                .child(option(
                                    "git-checkout-stash",
                                    CheckoutChoice::Stash,
                                    t!("git.checkout.stash"),
                                    t!("git.checkout.stash_text", branch = current.as_str()),
                                ))
                                .child(option(
                                    "git-checkout-bring",
                                    CheckoutChoice::Bring,
                                    t!("git.checkout.bring"),
                                    t!("git.checkout.bring_text", name = target.label()),
                                )),
                        )
                        .child(
                            modal_footer(cx)
                                .child(
                                    SecondaryButton::new(
                                        "git-checkout-cancel",
                                        t!("common.cancel"),
                                    )
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(confirm),
                        )
                        .into_any_element()
                },
            )
        });
    }

    /// Checks out `target`, stashing the changes first under `stash_from`'s name when given, and
    /// restoring the changes stashed earlier for the target branch when the tree is clean.
    fn checkout(
        &mut self,
        target: CheckoutTarget,
        stash_from: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        let clean = stash_from.is_some()
            || self
                .git
                .status()
                .is_some_and(|(_, status)| status.entries.is_empty());
        let label = target.label().to_string();
        self.run_git_op(
            GitOp::Checkout,
            false,
            move || checkout_with_stash(&repo, &target, stash_from.as_deref(), clean),
            move |view, outcome, window, cx| {
                log::info!("checked out a branch");
                view.after_head_moved(cx);
                if outcome.restored {
                    window.push_notification(
                        Notification::info(t!("git.checkout.restored", name = label).into_owned()),
                        cx,
                    );
                }
            },
            window,
            cx,
        );
    }

    /// After a checkout or a branch change: the files on disk may be others, read them again.
    fn after_head_moved(&mut self, cx: &mut Context<Self>) {
        self.reload_workspace_from_disk("a checkout", cx);
        self.refresh_git_branch();
        self.load_branches(cx);
    }

    /// Asks for a new branch's name, then creates it from `start` (the current branch when
    /// `None`) and checks it out. `name` pre-fills the field (the menu's search text).
    pub(crate) fn new_branch_dialog(
        &mut self,
        start: Option<String>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("git.branch.name_placeholder")));
        input.update(cx, |state, cx| state.set_value(name, window, cx));
        let title = match &start {
            Some(start) => t!("git.branch.new_from", name = start.as_str()),
            None => t!("git.branch.new"),
        };
        let weak = cx.weak_entity();
        let submit: NameSubmit = Rc::new(move |name, cx| {
            let start = start.clone();
            let _ = weak.update(cx, |view, cx| view.create_branch(name, start, cx));
        });
        open_name_dialog(
            title.into(),
            t!("common.create").into(),
            input,
            submit,
            window,
            cx,
        );
    }

    /// Creates `name` from `start` and checks it out, without a window: the queue's `done`
    /// notifies, and a bad name fails with git's message.
    pub(crate) fn create_branch(
        &mut self,
        name: String,
        start: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.run_git_branch_write(cx, move || {
            if !repo.check_branch_name(&name)? {
                return Err(GitError::Failed {
                    command: "git check-ref-format --branch".to_string(),
                    code: Some(1),
                    stderr: format!("'{name}' is not a valid branch name"),
                    stdout: String::new(),
                });
            }
            repo.create_branch(&name, start.as_deref(), true)
        });
    }

    /// Asks for `name`'s new name, then renames it.
    pub(crate) fn rename_branch_dialog(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("git.branch.name_placeholder")));
        input.update(cx, |state, cx| state.set_value(name.clone(), window, cx));
        let weak = cx.weak_entity();
        let submit: NameSubmit = Rc::new(move |new_name, cx| {
            let old = name.clone();
            let _ = weak.update(cx, |view, cx| {
                let Some(repo) = view.git.repo().cloned() else {
                    return;
                };
                view.run_git_branch_write(cx, move || repo.rename_branch(&old, &new_name));
            });
        });
        open_name_dialog(
            t!("git.branch.rename").into(),
            t!("common.rename").into(),
            input,
            submit,
            window,
            cx,
        );
    }

    /// Deletes `name` after a confirmation; when git refuses because it is not merged, asks
    /// again before forcing it.
    pub(crate) fn delete_branch_dialog(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        open_confirm_dialog(
            window,
            cx,
            ConfirmDialog {
                title: t!("git.branch.delete_title").into(),
                text: t!("git.branch.delete_text", name = name.as_str()).into(),
                confirm_label: t!("common.delete").into(),
                destructive: true,
                show_cancel: true,
                on_confirm: Rc::new(move |window, cx| {
                    let name = name.clone();
                    let _ = weak.update(cx, |view, cx| view.delete_branch(name, false, window, cx));
                }),
            },
        );
    }

    fn delete_branch(
        &mut self,
        name: String,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        let deleting = name.clone();
        let previous = self.git_tasks.write.take();
        self.git_tasks.write = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = cx
                .background_spawn(async move { repo.delete_branch(&deleting, force) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                match result {
                    Ok(()) => log::info!("deleted a branch"),
                    Err(error) if error.kind() == FailureKind::NotMerged && !force => {
                        view.confirm_force_delete(name, window, cx);
                    }
                    Err(error) => {
                        log::warn!("deleting a branch failed: {error}");
                        view.show_git_failure(
                            crate::state::git_sync::GitFailure {
                                op: GitOp::Branch,
                                kind: error.kind(),
                                output: error.output(),
                            },
                            window,
                            cx,
                        );
                    }
                }
                view.load_branches(cx);
            });
        }));
    }

    /// The branch is not merged: deleting it loses its commits.
    fn confirm_force_delete(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let weak = cx.weak_entity();
        open_confirm_dialog(
            window,
            cx,
            ConfirmDialog {
                title: t!("git.branch.force_title").into(),
                text: t!("git.branch.force_text", name = name.as_str()).into(),
                confirm_label: t!("git.branch.force_confirm").into(),
                destructive: true,
                show_cancel: true,
                on_confirm: Rc::new(move |window, cx| {
                    let name = name.clone();
                    let _ = weak.update(cx, |view, cx| view.delete_branch(name, true, window, cx));
                }),
            },
        );
    }

    /// Stars or unstars `name` in the branch menu.
    pub(crate) fn toggle_favorite_branch(&mut self, name: String, cx: &mut Context<Self>) {
        let favorites = &mut self.state.settings.git_favorite_branches;
        if let Some(index) = favorites.iter().position(|favorite| *favorite == name) {
            favorites.remove(index);
        } else {
            favorites.push(name);
        }
        save_settings(&self.state.settings);
        self.rebuild_branch_menu(cx);
        cx.notify();
    }

    /// Runs a branch write (create, rename) in the queue, then reads the branches and the files
    /// again. A failure shows in the error banner with git's own message.
    fn run_git_branch_write(
        &mut self,
        cx: &mut Context<Self>,
        write: impl FnOnce() -> Result<(), GitError> + Send + 'static,
    ) {
        let previous = self.git_tasks.write.take();
        self.git_tasks.write = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = this.update(cx, |view, cx| {
                view.git.sync.running = Some(GitOp::Branch);
                cx.notify();
            });
            let result = cx.background_spawn(async move { write() }).await;
            let _ = this.update(cx, |view, cx| {
                view.git.sync.running = None;
                if let Err(error) = result {
                    log::warn!("git branch command failed: {error}");
                    view.workspace_error = Some(error.to_string());
                }
                view.after_head_moved(cx);
                view.refresh_git_status(cx);
                cx.notify();
            });
        }));
    }
}

/// The checkout itself, on the background executor: stash, switch (putting the stash back if the
/// switch fails), then restore the target's own stash when the tree was left clean.
fn checkout_with_stash(
    repo: &Repo,
    target: &CheckoutTarget,
    stash_from: Option<&str>,
    clean: bool,
) -> Result<CheckoutOutcome, GitError> {
    let stashed = match stash_from {
        Some(branch) => repo.stash_push(&auto_stash_message(AUTO_STASH_PREFIX, branch))?,
        None => false,
    };
    let switched = match target {
        CheckoutTarget::Branch(name) => repo.checkout(name),
        CheckoutTarget::Revision(rev) => repo.checkout_detached(rev),
    };
    if let Err(error) = switched {
        if stashed {
            // Nothing moved: give the changes back where they were.
            if let Some(index) =
                repo.find_auto_stash(AUTO_STASH_PREFIX, stash_from.unwrap_or_default())?
            {
                repo.stash_pop(index)?;
            }
        }
        return Err(error);
    }
    let mut restored = false;
    if clean && let CheckoutTarget::Branch(name) = target {
        // A remote name checks out its local branch, whose stash is under the local name.
        let local = local_name(repo, name);
        if let Some(index) = repo.find_auto_stash(AUTO_STASH_PREFIX, &local)? {
            repo.stash_pop(index)?;
            restored = true;
        }
    }
    Ok(CheckoutOutcome { restored })
}

/// The local branch a checkout of `name` ends on: `name` itself, or for `origin/x`, `x`.
fn local_name(repo: &Repo, name: &str) -> String {
    match repo.status() {
        Ok(status) => match status.head {
            telaio_git::Head::Branch(branch) | telaio_git::Head::Unborn(branch) => branch,
            telaio_git::Head::Detached(_) => name.to_string(),
        },
        Err(_) => name.to_string(),
    }
}

/// One of 6f's two choices: a card with a title and an explanation, outlined in the accent color
/// when chosen.
fn checkout_option(
    id: &'static str,
    selected: bool,
    title: impl Into<SharedString>,
    text: impl Into<SharedString>,
    palette: &Palette,
) -> Stateful<Div> {
    v_flex()
        .id(id)
        .gap(px(2.0))
        .p(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .cursor_pointer()
        .when(selected, |card| {
            card.border_color(palette.accent)
                .bg(palette.accent.opacity(0.10))
        })
        .when(!selected, |card| {
            card.border_color(palette.border)
                .hover(|style| style.border_color(palette.border_strong))
        })
        .child(div().font_weight(FontWeight::MEDIUM).child(title.into()))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(palette.fg_muted)
                .child(text.into()),
        )
}
