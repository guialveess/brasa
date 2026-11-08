//! The Git integration's glue (milestone 0.5.0): detecting git, discovering the workspace's
//! repository and reading its status, always on the background executor. The results land in
//! [`AppView::git`] (`state::git`), which render reads. The Source Control panel is
//! `views/git/panel.rs`.

use std::sync::Arc;

use gpui_kit::prelude::*;
use gpui_kit::*;
use telaio_git::{DiffSource, Executable};

use crate::state::git::{GitAvailability, RepoState, needs_detection, wsl_distro_of};
use crate::state::git_sync::CommitStats;

use super::root::AppView;

mod branch_menu;
mod branches;
mod diff_tab;
mod log_tab;
mod panel;
mod sync;
mod watch;
mod write;

pub(crate) use branch_menu::{MenuContext, new_branch_menu};
pub(crate) use branches::{CheckoutChoice, CheckoutTarget};
pub(crate) use panel::INSTALL_GIT_URL;
pub(crate) use write::init_with_gitignore;

/// The git work in flight. Replacing a task drops the previous one, so only the latest result is
/// applied.
#[derive(Default)]
pub(crate) struct GitTasks {
    detect: Option<Task<()>>,
    discover: Option<Task<()>>,
    status: Option<Task<()>>,
    author: Option<Task<()>>,
    /// The write in flight (`write.rs`); the next one waits for it.
    write: Option<Task<()>>,
    /// The file watcher of the open repository (`watch.rs`).
    watcher: Option<watch::GitWatcher>,
    /// The diff tab's viewer (`diff_tab.rs`), created with its first patch.
    diff_view: Option<diff_tab::DiffView>,
    /// The diff tab's patch being read.
    diff_load: Option<Task<()>>,
    /// The commit box's summary line being read.
    commit_stats: Option<Task<()>>,
    /// The branches being read for the menu (`branches.rs`).
    branches: Option<Task<()>>,
    /// The log tab's inputs and scroll positions (`log_tab.rs`), created when it opens.
    log_view: Option<log_tab::LogView>,
    /// The log page being read.
    log_load: Option<Task<()>>,
    /// The selected commit's detail being read.
    log_detail: Option<Task<()>>,
}

impl AppView {
    /// The executable the settings ask for.
    fn git_executable(&self) -> Executable {
        match &self.state.settings.git_executable {
            Some(path) => Executable::Path(path.clone()),
            None => Executable::Auto,
        }
    }

    /// Detects git again (at startup and when its setting changes), then discovers the open
    /// workspace's repository.
    pub(crate) fn detect_git(&mut self, cx: &mut Context<Self>) {
        let executable = self.git_executable();
        // A workspace inside WSL uses the git of its distribution (`needs_detection`).
        let folder = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf());
        self.git.detected_for = wsl_distro_of(folder.as_deref());
        self.git.availability = GitAvailability::Detecting;
        self.git_tasks.detect = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { telaio_git::detect_for(&executable, folder.as_deref()) },
                )
                .await;
            let _ = this.update(cx, |view, cx| {
                view.git.availability = match result {
                    Ok(git) => {
                        match git.wsl_distro() {
                            Some(distro) => log::info!(
                                "git {} at {} in WSL {distro}",
                                git.version(),
                                git.path().display()
                            ),
                            None => {
                                log::info!("git {} at {}", git.version(), git.path().display())
                            }
                        }
                        GitAvailability::Ready(git)
                    }
                    Err(error) => {
                        log::warn!("git is not available: {error}");
                        GitAvailability::Missing(Arc::new(error))
                    }
                };
                view.discover_git_repo(cx);
                cx.notify();
            });
        }));
    }

    /// Finds the repository of the open workspace, then reads its status. Call when the
    /// workspace opens and after git is detected again. Detects git first when the workspace
    /// needs another one (a workspace in another WSL distribution).
    pub(crate) fn discover_git_repo(&mut self, cx: &mut Context<Self>) {
        let detecting = matches!(self.git.availability, GitAvailability::Detecting);
        let workspace = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root());
        if needs_detection(
            self.state.settings.git_executable.is_none(),
            detecting,
            self.git.detected_for.as_deref(),
            workspace,
        ) {
            self.detect_git(cx);
            // Clear the old repository below; the detection discovers the new one when it ends.
        }
        self.git.repo = RepoState::Unknown;
        self.git.author = None;
        self.git_tasks.status = None;
        self.git.collapsed.clear();
        self.git_tasks.watcher = None;
        self.rebuild_git_rows();
        let (Some(git), Some(workspace)) =
            (self.git.availability.git(), self.state.workspace.as_ref())
        else {
            self.git_tasks.discover = None;
            return;
        };
        let git = git.clone();
        let root = workspace.root().to_path_buf();
        self.git_tasks.discover = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { git.discover(&root) })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.git.repo = match result {
                    Ok(Some(repo)) => {
                        log::debug!(
                            "git repository at {}, workspace prefix {:?}",
                            repo.root().display(),
                            repo.prefix()
                        );
                        RepoState::Repo { repo, status: None }
                    }
                    Ok(None) => RepoState::NotARepo,
                    Err(error) => {
                        log::warn!("git repository discovery failed: {error}");
                        RepoState::Error(error.to_string())
                    }
                };
                view.watch_git(cx);
                view.refresh_git_status(cx);
                view.load_branches(cx);
                view.load_git_author(cx);
                cx.notify();
            });
        }));
    }

    /// Reads the commit author from git config for the Settings pane, on the background executor.
    fn load_git_author(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.git_tasks.author = Some(cx.spawn(async move |this, cx| {
            let reading = repo.clone();
            let result = cx.background_spawn(async move { reading.author() }).await;
            let _ = this.update(cx, |view, cx| {
                let same_repo = view
                    .git
                    .repo()
                    .is_some_and(|current| current.root() == repo.root());
                if !same_repo {
                    return;
                }
                view.git.author = match result {
                    Ok(author) => author,
                    Err(error) => {
                        log::warn!("reading the git author failed: {error}");
                        None
                    }
                };
                cx.notify();
            });
        }));
    }

    /// Reads `git status` again. The view is notified only when the status changed, so calling
    /// this on every focus or file change costs no frame while nothing changes.
    pub(crate) fn refresh_git_status(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.git_tasks.status = Some(cx.spawn(async move |this, cx| {
            let reading = repo.clone();
            let result = cx.background_spawn(async move { reading.status() }).await;
            let _ = this.update(cx, |view, cx| {
                // The workspace may have changed while git ran.
                let same_repo = view
                    .git
                    .repo()
                    .is_some_and(|current| current.root() == repo.root());
                if !same_repo {
                    return;
                }
                match result {
                    Ok(status) => {
                        if let RepoState::Repo {
                            status: current, ..
                        } = &mut view.git.repo
                            && current.as_deref() != Some(&status)
                        {
                            *current = Some(Arc::new(status));
                            view.rebuild_git_rows();
                            view.load_git_diff(cx);
                            view.refresh_git_log_if_stale(cx);
                            view.load_commit_stats(cx);
                            // A commit, checkout or fetch made anywhere moves the branches too.
                            view.load_branches(cx);
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        log::warn!("git status failed: {error}");
                        view.git.repo = RepoState::Error(error.to_string());
                        view.rebuild_git_rows();
                        cx.notify();
                    }
                }
            });
        }));
    }

    /// Reads what the next commit would include, for the commit box's summary line.
    fn load_commit_stats(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo().cloned() else {
            return;
        };
        self.git_tasks.commit_stats = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { repo.numstat(&DiffSource::HeadToIndex) })
                .await;
            let _ = this.update(cx, |view, cx| match result {
                Ok(numstat) => {
                    let stats = CommitStats::from_numstat(&numstat);
                    if view.git.commit.stats != stats {
                        view.git.commit.stats = stats;
                        cx.notify();
                    }
                }
                Err(error) => log::warn!("reading the staged changes failed: {error}"),
            });
        }));
    }
}
