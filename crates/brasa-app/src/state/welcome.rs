//! The welcome tab (GitHub #90): when it opens at startup, and the recent workspaces it lists.
//! Plain Rust, unit tested without a window. `views/welcome.rs` renders it.

use std::path::{Path, PathBuf};

use super::format::{elide_path, shorten_path};

/// How many recent workspaces the welcome tab lists.
pub const RECENT_LIMIT: usize = 4;

/// Longest path, in characters, shown next to a recent workspace. Longer ones get their middle
/// elided, like the workspace switcher's.
const PATH_MAX_CHARS: usize = 44;

/// The quick start guide the welcome tab links to.
pub const QUICK_START_URL: &str = "https://brasa.tanis.codes/docs/quick-start.html";

/// The id of the welcome tab. Never a request id: those end in `.brasa`.
pub const TAB_ID: &str = "welcome";

/// What the welcome tab shows that comes from outside the app: the recent workspaces, read when
/// the tab opens, never while rendering.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WelcomeTab {
    /// The most recent workspaces that still exist, most recent first, at most
    /// [`RECENT_LIMIT`].
    pub recents: Vec<RecentWorkspace>,
}

/// One recent workspace of the welcome tab.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentWorkspace {
    /// The folder name.
    pub name: String,
    /// The path, `~`-shortened and elided to fit.
    pub path_label: String,
    /// The path to open on click.
    pub path: PathBuf,
    /// Its request count and last modified time, read from disk in the background
    /// ([`load_stats`]) after the card first shows: scanning every recent workspace would make
    /// opening the tab itself touch the disk, which its own doc comment rules out.
    pub stats: Option<RecentWorkspaceStats>,
}

/// What a recent workspace's card shows besides its name and path, read with [`load_stats`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecentWorkspaceStats {
    /// How many `.brasa` requests it holds, [`crate::state::load_test::count_requests`] of its
    /// tree.
    pub request_count: usize,
    /// The workspace folder's own last modified time, Unix seconds, when the file system reports
    /// one.
    pub modified_unix_seconds: Option<i64>,
}

/// Scans `path` the same way opening it would ([`brasa_workspace::Workspace::open`]) to count its
/// requests, and reads the folder's own modified time. `None` when `path` no longer exists or
/// fails to open (the recent list already drops those before this runs, but it can still lose a
/// race with something deleting the folder). Blocking: call on the background executor.
pub fn load_stats(path: &Path) -> Option<RecentWorkspaceStats> {
    let workspace = brasa_workspace::Workspace::open(path).ok()?;
    let request_count = super::load_test::count_requests(workspace.tree());
    let modified_unix_seconds = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs() as i64);
    Some(RecentWorkspaceStats {
        request_count,
        modified_unix_seconds,
    })
}

impl WelcomeTab {
    /// The welcome tab for `recents` (the recent workspaces, most recent first), checking which
    /// still exist on disk.
    pub fn load(recents: &[PathBuf], home: Option<&Path>) -> Self {
        Self {
            recents: recent_entries(recents, home, |path| path.is_dir()),
        }
    }
}

/// The first [`RECENT_LIMIT`] of `paths` for which `exists` holds, with their labels.
fn recent_entries(
    paths: &[PathBuf],
    home: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<RecentWorkspace> {
    paths
        .iter()
        .filter(|path| exists(path))
        .take(RECENT_LIMIT)
        .map(|path| RecentWorkspace {
            name: path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            path_label: elide_path(&shorten_path(path, home), PATH_MAX_CHARS),
            path: path.clone(),
            stats: None,
        })
        .collect()
}

/// Whether the welcome tab opens at startup.
///
/// - Never when Brasa was started with a path (a command line argument, or a file the OS
///   opened with it): the user asked for something specific.
/// - Always when there is no workspace to open (the first start, or every remembered folder is
///   gone), whatever `show_welcome` says, so closing it never leads back to an empty window.
/// - Otherwise as `show_welcome` (the setting) says, next to the reopened workspace.
pub fn show_at_startup(launched_with_path: bool, has_workspace: bool, show_welcome: bool) -> bool {
    if launched_with_path {
        return false;
    }
    !has_workspace || show_welcome
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn startup_with_a_path_never_shows_it() {
        assert!(!show_at_startup(true, true, true));
        assert!(!show_at_startup(true, false, true));
        assert!(!show_at_startup(true, true, false));
    }

    #[test]
    fn startup_without_a_workspace_always_shows_it() {
        assert!(show_at_startup(false, false, false));
        assert!(show_at_startup(false, false, true));
    }

    #[test]
    fn startup_with_a_remembered_workspace_follows_the_setting() {
        assert!(show_at_startup(false, true, true));
        assert!(!show_at_startup(false, true, false));
    }

    #[test]
    fn recents_skip_missing_folders_and_stop_at_the_limit() {
        let paths: Vec<PathBuf> = ["a", "gone", "b", "c", "d", "e"]
            .iter()
            .map(|name| PathBuf::from("/home/ana/dev").join(name))
            .collect();
        let entries = recent_entries(&paths, Some(Path::new("/home/ana")), |path| {
            !path.ends_with("gone")
        });
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c", "d"]);
        assert_eq!(entries[0].path, PathBuf::from("/home/ana/dev/a"));
    }

    #[cfg(unix)]
    #[test]
    fn recent_paths_are_shortened_and_elided() {
        let long = PathBuf::from("/home/ana/clients/acme/projects/2026/backend/api/requests");
        let entries = recent_entries(&[long], Some(Path::new("/home/ana")), |_| true);
        assert_eq!(entries[0].name, "requests");
        assert!(entries[0].path_label.starts_with("~/"));
        assert!(entries[0].path_label.contains('…'));
        assert!(entries[0].path_label.ends_with("/requests"));
        assert!(entries[0].path_label.chars().count() <= PATH_MAX_CHARS);
    }

    #[test]
    fn load_lists_only_existing_folders() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing");
        let welcome = WelcomeTab::load(&[missing, dir.path().to_path_buf()], None);
        assert_eq!(welcome.recents.len(), 1);
        assert_eq!(welcome.recents[0].path, dir.path());
    }
}
