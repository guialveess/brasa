//! Commit, push, pull and fetch (#171, #172). Each command waits for the write in flight (the
//! queue of `write.rs`), shows its progress in the status bar, and a failure becomes a
//! notification in plain language with git's own output one click away (design 6e).

use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_git::{FailureKind, GitError, PullMode, PushTarget};

use crate::state::git_sync::{
    GitFailure, GitOp, auto_fetch_due, can_commit, can_write_message, staged_secrets,
};
use crate::state::locale::plural;
use crate::state::settings::GitPull;
use crate::theme::PaletteExt;
use crate::views::components::{
    ConfirmDialog, ModalPlacement, NeutralButtonExt, PrimaryButton, framed_dialog, modal_body,
    modal_footer, modal_header, open_confirm_dialog,
};

use super::super::root::AppView;

/// How many commits the "Commit and Push" confirmation lists before "and N more".
const LISTED_COMMITS: usize = 10;

impl AppView {
    /// Runs `work` as `op` after the write in flight, with the op in the status bar, then reads
    /// the status again. `done` gets the result on success; a failure goes to
    /// [`Self::show_git_failure`] unless `quiet` (an automatic fetch never nags).
    pub(super) fn run_git_op<T: Send + 'static>(
        &mut self,
        op: GitOp,
        quiet: bool,
        work: impl FnOnce() -> Result<T, GitError> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.git_tasks.write.take();
        self.git_tasks.write = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = this.update(cx, |view, cx| {
                view.git.sync.running = Some(op);
                cx.notify();
            });
            let result = cx.background_spawn(async move { work() }).await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.git.sync.running = None;
                match result {
                    Ok(value) => {
                        view.git.sync.failure = None;
                        done(view, value, window, cx);
                    }
                    Err(error) => {
                        log::warn!("git {op:?} failed: {error}");
                        let failure = GitFailure {
                            op,
                            kind: error.kind(),
                            output: error.output(),
                        };
                        if !quiet {
                            view.show_git_failure(failure.clone(), window, cx);
                        }
                        view.git.sync.failure = Some(failure);
                    }
                }
                view.refresh_git_status(cx);
                cx.notify();
            });
        }));
    }

    /// Commits the staged changes with the commit box's message, then pushes when `push` (the
    /// push asks first, listing the commits). Ctrl+Enter and the two buttons call this.
    pub(crate) fn commit_from_box(
        &mut self,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((repo, status)) = self.git.status() else {
            return;
        };
        let message = self.commit_input.read(cx).value().to_string();
        let amend = self.git.commit.amend;
        if !can_commit(
            &message,
            self.git.commit.stats,
            amend,
            self.git.sync.running.is_some(),
        ) {
            return;
        }
        if self.state.settings.git_block_secrets && !staged_secrets(status).is_empty() {
            return;
        }
        let repo = repo.clone();
        self.run_git_op(
            GitOp::Commit,
            false,
            move || repo.commit(&message, amend),
            move |view, (), window, cx| {
                log::info!("committed{}", if amend { " (amend)" } else { "" });
                view.commit_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                view.git.commit.amend = false;
                if push {
                    view.start_push(true, window, cx);
                }
            },
            window,
            cx,
        );
    }

    /// The palette's Commit: commits when the message and the staged files allow it, otherwise
    /// shows the Source Control panel with the focus in the message.
    pub(crate) fn commit_from_palette(
        &mut self,
        push: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = self.commit_input.read(cx).value().to_string();
        let ready = can_commit(
            &message,
            self.git.commit.stats,
            self.git.commit.amend,
            self.git.sync.running.is_some(),
        ) && !(self.state.settings.git_block_secrets && !self.git.secrets.is_empty());
        if ready {
            self.commit_from_box(push, window, cx);
            return;
        }
        let section = crate::state::navigation::NavSection::SourceControl;
        if !self.nav.is_active(section) {
            self.nav.click(section);
        }
        // With nothing staged the message box is disabled, so there is nothing to focus.
        if can_write_message(self.git.commit.stats, self.git.commit.amend) {
            self.commit_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    /// Ticks or clears Amend. Ticking it with an empty message brings back the last commit's.
    pub(crate) fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.git.commit.amend = !self.git.commit.amend;
        cx.notify();
        let empty = self.commit_input.read(cx).value().trim().is_empty();
        let Some(repo) = self
            .git
            .repo()
            .cloned()
            .filter(|_| self.git.commit.amend && empty)
        else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { repo.last_commit_message() })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match result {
                Ok(Some(message)) => {
                    let still_empty = view.commit_input.read(cx).value().trim().is_empty();
                    if view.git.commit.amend && still_empty {
                        view.commit_input
                            .update(cx, |input, cx| input.set_value(message, window, cx));
                    }
                }
                Ok(None) => {}
                Err(error) => log::warn!("reading the last commit message failed: {error}"),
            });
        })
        .detach();
    }

    /// Finds where to push and, when `confirm`, lists the commits in a confirmation first.
    pub(crate) fn start_push(
        &mut self,
        confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let target = repo.push_target()?;
                    let commits = match &target {
                        Some(target) if confirm => repo.unpushed_commits(target)?,
                        _ => Vec::new(),
                    };
                    Ok::<_, GitError>((target, commits))
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match result {
                Ok((None, _)) => {
                    window.push_notification(
                        Notification::warning(t!("git.sync.no_remote").into_owned()),
                        cx,
                    );
                }
                Ok((Some(target), commits)) if confirm => {
                    view.confirm_push(target, commits, window, cx);
                }
                Ok((Some(target), _)) => view.push(target, window, cx),
                Err(error) => {
                    log::warn!("finding the push target failed: {error}");
                    let failure = GitFailure {
                        op: GitOp::Push,
                        kind: error.kind(),
                        output: error.output(),
                    };
                    view.show_git_failure(failure, window, cx);
                }
            });
        })
        .detach();
    }

    /// "Commit and Push…": the commits about to leave, and a Push button.
    fn confirm_push(
        &mut self,
        target: PushTarget,
        commits: Vec<telaio_git::CommitSummary>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let destination = format!("{}/{}", target.remote, target.branch);
        let mut lines: Vec<String> = commits
            .iter()
            .take(LISTED_COMMITS)
            .map(|commit| format!("{}  {}", commit.oid, commit.subject))
            .collect();
        if commits.len() > LISTED_COMMITS {
            lines.push(plural("git.push.more", commits.len() - LISTED_COMMITS));
        }
        let intro = plural("git.push.text", commits.len()).replace("%{target}", &destination);
        let weak = cx.weak_entity();
        open_confirm_dialog(
            window,
            cx,
            ConfirmDialog {
                title: t!("git.push.title").into(),
                text: format!("{intro}\n\n{}", lines.join("\n")).into(),
                confirm_label: t!("git.push.confirm").into(),
                destructive: false,
                show_cancel: true,
                on_confirm: Rc::new(move |window, cx| {
                    let target = target.clone();
                    let _ = weak.update(cx, |view, cx| view.push(target, window, cx));
                }),
            },
        );
    }

    /// Pushes to `target`; the first push of a branch sets its upstream.
    fn push(&mut self, target: PushTarget, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        let destination = format!("{}/{}", target.remote, target.branch);
        self.run_git_op(
            GitOp::Push,
            false,
            move || repo.push(&target),
            move |_, (), window, cx| {
                log::info!("pushed to {destination}");
                window.push_notification(
                    Notification::success(t!("git.sync.pushed", target = destination).into_owned()),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Pulls as the Pull setting says, and pushes afterwards when `then_push` ("Pull and push"
    /// after a rejected push).
    pub(crate) fn pull(&mut self, then_push: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        let mode = match self.state.settings.git_pull {
            GitPull::FollowConfig => PullMode::FollowConfig,
            GitPull::Merge => PullMode::Merge,
            GitPull::Rebase => PullMode::Rebase,
        };
        self.run_git_op(
            GitOp::Pull,
            false,
            move || repo.pull(mode),
            move |view, (), window, cx| {
                log::info!("pulled");
                // The pull may have added, changed or removed requests and environments.
                view.reload_workspace_from_disk("a pull", cx);
                if then_push {
                    view.start_push(false, window, cx);
                }
            },
            window,
            cx,
        );
    }

    /// Fetches from the remote. `auto` is the fetch on focus: silent when it fails.
    pub(crate) fn fetch(&mut self, auto: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.git.sync.last_fetch = Some(Instant::now());
        self.run_git_op(
            GitOp::Fetch,
            auto,
            move || repo.fetch(),
            |_, (), _, _| log::debug!("fetched"),
            window,
            cx,
        );
    }

    /// The window got the focus back: fetch when the setting asks for it and the last fetch is
    /// at least five minutes old. An event, never a timer.
    pub(crate) fn maybe_auto_fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_upstream = self
            .git
            .status()
            .is_some_and(|(_, status)| status.upstream.is_some());
        if auto_fetch_due(
            self.state.settings.git_fetch_automatically,
            has_upstream,
            self.git.sync.running.is_some(),
            self.git.sync.last_fetch,
            Instant::now(),
        ) {
            self.fetch(true, window, cx);
        }
    }

    /// 6e: a notification with the failure in plain language, "Pull and push" for a rejected
    /// push, and "Show git output".
    pub(super) fn show_git_failure(
        &mut self,
        failure: GitFailure,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let upstream = self
            .git
            .status()
            .and_then(|(_, status)| status.upstream.clone());
        let title = failure.title();
        let message = failure.message(upstream.as_deref());
        let rejected = failure.op == GitOp::Push && failure.kind == FailureKind::Rejected;
        let output: SharedString = failure.output.into();
        let weak = cx.weak_entity();
        let notification = Notification::error(message)
            .title(title)
            .autohide(false)
            .content(move |note, _, cx| {
                let palette = cx.palette();
                let show_output = {
                    let output = output.clone();
                    Button::new("git-failure-output")
                        .label(t!("git.failure.show_output"))
                        .neutral_ghost(cx, palette.fg_muted)
                        .small()
                        .on_click(cx.listener(move |note, _, window, cx| {
                            note.dismiss(window, cx);
                            open_git_output(output.clone(), window, cx);
                        }))
                };
                let _ = note;
                h_flex()
                    .mt(px(8.0))
                    .gap(px(6.0))
                    .when(rejected, |row| {
                        let weak = weak.clone();
                        row.child(
                            PrimaryButton::new(
                                "git-failure-pull-push",
                                t!("git.failure.pull_and_push"),
                            )
                            .icon(Icon::new(gpui_kit::assets::IconName::ArrowDown))
                            .on_click(cx.listener(
                                move |note, _, window, cx| {
                                    note.dismiss(window, cx);
                                    let _ = weak.update(cx, |view, cx| view.pull(true, window, cx));
                                },
                            )),
                        )
                    })
                    .child(show_output)
                    .into_any_element()
            });
        window.push_notification(notification, cx);
    }
}

/// "Show git output": what git printed, verbatim and selectable.
pub(crate) fn open_git_output(output: SharedString, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, window, _| {
        let output = output.clone();
        framed_dialog(
            dialog,
            560.0,
            None,
            ModalPlacement::DEFAULT,
            window,
            move |_, cx| {
                let palette = cx.palette();
                let mono = cx.theme().mono_font_family.clone();
                v_flex()
                    .child(modal_header(t!("git.failure.output_title"), cx))
                    .child(
                        modal_body().child(
                            div()
                                .id("git-output")
                                .max_h(px(320.0))
                                .overflow_y_scroll()
                                .p(px(10.0))
                                .rounded(px(6.0))
                                .bg(palette.bg)
                                .border_1()
                                .border_color(palette.border)
                                .font_family(mono)
                                .text_size(px(12.0))
                                .text_color(palette.fg_muted)
                                .whitespace_normal()
                                .child(output.clone()),
                        ),
                    )
                    .child(
                        modal_footer(cx).child(
                            PrimaryButton::new("git-output-close", t!("common.close"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                    )
                    .into_any_element()
            },
        )
    });
}
