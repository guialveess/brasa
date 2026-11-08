//! Remembers recently opened workspace folders in the OS config directory, most recent first,
//! so the app can offer to reopen one
//! automatically at startup and show them in the workspace switcher menu. Also remembers each
//! recent workspace's last active environment, restored when it is opened again, and the last
//! version that ran, for the What's new tab (`state::whats_new`).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::whats_new::LastSeen;

/// Brasa's folder inside the OS config directory, `<config dir>/brasa`, where the settings and
/// the files of this module live. `None` when the OS does not report the config directory.
pub fn config_dir() -> Option<PathBuf> {
    telaio_app::paths::config_dir(&brasa_cli::IDENTITY)
}

/// Maximum number of remembered workspaces.
const MAX_RECENT_WORKSPACES: usize = 10;

/// The path, relative to the config directory ([`config_dir`]), of the file listing recently opened
/// workspaces, one absolute path per line, most recent first.
const RECENT_WORKSPACES_FILE: &str = "recent-workspaces.txt";

/// The path of the single-workspace file this module replaces. Read once to migrate existing
/// users the first time [`RECENT_WORKSPACES_FILE`] does not exist yet, then ignored.
const LEGACY_LAST_WORKSPACE_FILE: &str = "last-workspace.txt";

/// The path, relative to the config directory ([`config_dir`]), of the file holding each recent workspace's
/// last active environment: a TOML table from absolute workspace path to environment name. It
/// lives here rather than in the workspace so picking an environment never shows up in the
/// workspace's git diff.
const LAST_ENVIRONMENTS_FILE: &str = "last-environments.toml";

/// The path, relative to the config directory ([`config_dir`]), of the file holding the last
/// stable version that ran, one line.
const LAST_VERSION_FILE: &str = "last-version.txt";

/// The files whose presence means Brasa ran here before, though it never recorded a version
/// (versions before 0.6.0).
const EARLIER_RUN_FILES: [&str; 3] = [
    super::settings::SETTINGS_FILE,
    RECENT_WORKSPACES_FILE,
    LEGACY_LAST_WORKSPACE_FILE,
];

/// The last version that ran. Read once at startup.
pub fn last_seen_version() -> LastSeen {
    match config_dir() {
        Some(base) => last_seen_version_at(&base),
        None => LastSeen::FirstRun,
    }
}

/// Records `version` as the last one that ran, when it differs from the recorded one. Errors are
/// logged and otherwise ignored, like the other files of this module.
pub fn record_version(version: &str) {
    if let Some(base) = config_dir() {
        record_version_at(&base, version);
    }
}

/// The testable core of [`last_seen_version`], taking the config base directory explicitly.
fn last_seen_version_at(base: &Path) -> LastSeen {
    if let Ok(content) = fs::read_to_string(base.join(LAST_VERSION_FILE)) {
        return LastSeen::Version(content.trim().to_string());
    }
    if EARLIER_RUN_FILES
        .iter()
        .any(|file| base.join(file).exists())
    {
        LastSeen::Unrecorded
    } else {
        LastSeen::FirstRun
    }
}

/// The testable core of [`record_version`], taking the config base directory explicitly.
fn record_version_at(base: &Path, version: &str) {
    if last_seen_version_at(base) == LastSeen::Version(version.to_string()) {
        return;
    }
    if let Err(error) = fs::create_dir_all(base) {
        log::warn!("could not create {}: {error}", base.display());
        return;
    }
    let path = base.join(LAST_VERSION_FILE);
    if let Err(error) = fs::write(&path, format!("{version}\n")) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

/// Returns the list of recently opened workspaces, most recent first, migrating from the legacy
/// single-workspace file the first time the new file does not exist yet.
pub fn recent_workspaces() -> Vec<PathBuf> {
    match config_dir() {
        Some(base) => read_recent_workspaces(&base),
        None => Vec::new(),
    }
}

/// Records `root` as the most recently opened workspace: moves it to the front of the list (or
/// inserts it), drops any duplicate, and keeps at most [`MAX_RECENT_WORKSPACES`] entries. Failing
/// to persist this is not worth interrupting the user over, so any error (a missing config
/// directory, a read-only filesystem, ...) is silently ignored.
pub fn record_workspace(root: &Path) {
    if let Some(base) = config_dir() {
        record_workspace_at(&base, root);
    }
}

/// The testable core of [`record_workspace`], taking the config base directory explicitly.
fn record_workspace_at(base: &Path, root: &Path) {
    let mut recents = read_recent_workspaces(base);
    recents.retain(|existing| existing != root);
    recents.insert(0, root.to_path_buf());
    recents.truncate(MAX_RECENT_WORKSPACES);
    write_recent_workspaces(base, &recents);
}

/// Removes `root` from the recent workspace list. Only the list changes: the folder itself is
/// never touched. Like [`record_workspace`], failing to persist it is silently ignored.
pub fn forget_workspace(root: &Path) {
    if let Some(base) = config_dir() {
        forget_workspace_at(&base, root);
    }
}

/// The testable core of [`forget_workspace`], taking the config base directory explicitly.
fn forget_workspace_at(base: &Path, root: &Path) {
    let mut recents = read_recent_workspaces(base);
    let before = recents.len();
    recents.retain(|existing| existing != root);
    if recents.len() != before {
        write_recent_workspaces(base, &recents);
    }
}

/// The workspace to reopen at startup: the first entry of [`recent_workspaces`] that still
/// exists on disk, or `None` if there are no recent workspaces or none of them exist anymore.
pub fn load_last_workspace() -> Option<PathBuf> {
    first_existing(recent_workspaces())
}

/// The first path in `paths` that is still a directory on disk, in order. Split out from
/// [`load_last_workspace`] so it can be tested without touching the real OS config directory.
fn first_existing(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.into_iter().find(|path| path.is_dir())
}

/// The environment that was active when the workspace at `root` was last used, if any was. The
/// caller checks it still exists.
pub fn last_environment(root: &Path) -> Option<String> {
    let base = config_dir()?;
    last_environment_at(&base, &workspace_key(root))
}

/// Remembers `name` (`None` for "No environment") as the active environment of the workspace at
/// `root`. Like [`record_workspace`], failing to persist it is silently ignored.
pub fn record_environment(root: &Path, name: Option<&str>) {
    if let Some(base) = config_dir() {
        record_environment_at(&base, &workspace_key(root), name);
    }
}

/// The key a workspace is remembered under: its absolute path, as [`record_workspace`] stores
/// it, so a workspace opened through a relative path matches its recent list entry. `dunce`
/// avoids the verbatim `\\?\C:\...` form `std` returns on Windows.
fn workspace_key(root: &Path) -> PathBuf {
    dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// The testable core of [`last_environment`], taking the config base directory explicitly.
fn last_environment_at(base: &Path, root: &Path) -> Option<String> {
    read_last_environments(base).remove(&root.to_string_lossy().into_owned())
}

/// The testable core of [`record_environment`], taking the config base directory explicitly.
/// Drops the entries of workspaces no longer in the recent list, so the file never outgrows it.
fn record_environment_at(base: &Path, root: &Path, name: Option<&str>) {
    let mut environments = read_last_environments(base);
    let key = root.to_string_lossy().into_owned();
    match name {
        Some(name) => environments.insert(key, name.to_string()),
        None => environments.remove(&key),
    };
    let recent: HashSet<String> = read_recent_workspaces(base)
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    environments.retain(|workspace, _| recent.contains(workspace));
    write_last_environments(base, &environments);
}

/// Reads `<base>/last-environments.toml`. A missing or unreadable file reads as empty.
fn read_last_environments(base: &Path) -> BTreeMap<String, String> {
    fs::read_to_string(base.join(LAST_ENVIRONMENTS_FILE))
        .ok()
        .and_then(|content| toml::from_str(&content).ok())
        .unwrap_or_default()
}

/// Writes `<base>/last-environments.toml`, creating the folder if needed.
fn write_last_environments(base: &Path, environments: &BTreeMap<String, String>) {
    let path = base.join(LAST_ENVIRONMENTS_FILE);
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = fs::create_dir_all(parent) {
        log::warn!("could not create {}: {error}", parent.display());
        return;
    }
    let content = match toml::to_string(environments) {
        Ok(content) => content,
        Err(error) => {
            log::warn!("could not serialize {}: {error}", path.display());
            return;
        }
    };
    if let Err(error) = fs::write(&path, content) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

/// Reads the recent workspace list from `<base>/recent-workspaces.txt`, migrating from
/// `<base>/last-workspace.txt` if the new file does not exist yet. Split out from
/// [`recent_workspaces`] so tests can point `base` at a temporary directory instead of the real,
/// per-user OS config directory.
fn read_recent_workspaces(base: &Path) -> Vec<PathBuf> {
    match fs::read_to_string(base.join(RECENT_WORKSPACES_FILE)) {
        Ok(content) => parse_recent_workspaces(&content),
        Err(_) => migrate_legacy_last_workspace(base),
    }
}

/// Parses the content of a recent-workspaces file: one absolute path per line, blank lines
/// ignored. Older versions saved Windows paths in their verbatim `\\?\C:\...` form; those are
/// read back as plain `C:\...` paths.
fn parse_recent_workspaces(content: &str) -> Vec<PathBuf> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| dunce::simplified(Path::new(line)).to_path_buf())
        .collect()
}

/// One-time migration from the legacy `last-workspace.txt` file: if it exists, its single entry
/// becomes the whole recent list and is written to the new file, so the migration only happens
/// once. Returns an empty list if there is nothing to migrate.
fn migrate_legacy_last_workspace(base: &Path) -> Vec<PathBuf> {
    let Ok(content) = fs::read_to_string(base.join(LEGACY_LAST_WORKSPACE_FILE)) else {
        return Vec::new();
    };
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let recents = vec![PathBuf::from(trimmed)];
    write_recent_workspaces(base, &recents);
    recents
}

/// Writes the recent workspace list to `<base>/recent-workspaces.txt`, creating the
/// folder if needed, one absolute path per line, most recent first.
fn write_recent_workspaces(base: &Path, recents: &[PathBuf]) {
    let path = base.join(RECENT_WORKSPACES_FILE);
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = fs::create_dir_all(parent) {
        log::warn!("could not create {}: {error}", parent.display());
        return;
    }
    let mut content = recents
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    if let Err(error) = fs::write(&path, content) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn the_last_version_tells_a_first_run_from_an_earlier_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path().join("brasa");
        assert_eq!(last_seen_version_at(&base), LastSeen::FirstRun);
        record_workspace_at(&base, Path::new("/work/api"));
        assert_eq!(last_seen_version_at(&base), LastSeen::Unrecorded);
        record_version_at(&base, "0.6.0");
        assert_eq!(
            last_seen_version_at(&base),
            LastSeen::Version("0.6.0".to_string())
        );
        record_version_at(&base, "0.7.0");
        assert_eq!(
            fs::read_to_string(base.join(LAST_VERSION_FILE)).expect("file"),
            "0.7.0\n"
        );
    }

    #[test]
    fn missing_files_read_as_an_empty_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_recent_workspaces(dir.path()), Vec::<PathBuf>::new());
    }

    #[test]
    fn record_workspace_adds_a_new_entry_at_the_front() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        write_recent_workspaces(base, &[base.join("a")]);

        record_workspace_at(base, &base.join("b"));

        assert_eq!(
            read_recent_workspaces(base),
            vec![base.join("b"), base.join("a")]
        );
    }

    #[test]
    fn record_workspace_moves_an_existing_entry_to_the_front_without_duplicating_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        write_recent_workspaces(base, &[base.join("a"), base.join("b"), base.join("c")]);

        record_workspace_at(base, &base.join("b"));

        assert_eq!(
            read_recent_workspaces(base),
            vec![base.join("b"), base.join("a"), base.join("c")]
        );
    }

    #[test]
    fn forget_workspace_removes_the_entry_and_keeps_the_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let folder = base.join("b");
        fs::create_dir(&folder).expect("create folder");
        write_recent_workspaces(base, &[base.join("a"), folder.clone(), base.join("c")]);

        forget_workspace_at(base, &folder);

        assert_eq!(
            read_recent_workspaces(base),
            vec![base.join("a"), base.join("c")]
        );
        assert!(folder.is_dir());
    }

    #[test]
    fn record_workspace_caps_the_list_at_ten_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let recents: Vec<PathBuf> = (0..10)
            .map(|index| base.join(format!("w{index}")))
            .collect();
        write_recent_workspaces(base, &recents);

        record_workspace_at(base, &base.join("new"));

        let stored = read_recent_workspaces(base);
        assert_eq!(stored.len(), MAX_RECENT_WORKSPACES);
        assert_eq!(stored[0], base.join("new"));
        assert!(!stored.contains(&base.join("w9")));
    }

    #[test]
    fn migrates_from_the_legacy_single_workspace_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let legacy_path = base.join(LEGACY_LAST_WORKSPACE_FILE);
        fs::create_dir_all(legacy_path.parent().expect("parent")).expect("create brasa dir");
        fs::write(
            &legacy_path,
            base.join("legacy-workspace").to_string_lossy().as_bytes(),
        )
        .expect("write legacy file");

        let recents = read_recent_workspaces(base);

        assert_eq!(recents, vec![base.join("legacy-workspace")]);
        // The migration writes the new file, so it only happens once.
        assert!(base.join(RECENT_WORKSPACES_FILE).is_file());
    }

    #[test]
    fn migration_only_happens_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let legacy_path = base.join(LEGACY_LAST_WORKSPACE_FILE);
        fs::create_dir_all(legacy_path.parent().expect("parent")).expect("create brasa dir");
        fs::write(
            &legacy_path,
            base.join("legacy-workspace").to_string_lossy().as_bytes(),
        )
        .expect("write legacy file");
        // First read migrates.
        read_recent_workspaces(base);
        // A later change to the legacy file must not be picked up again.
        fs::write(
            &legacy_path,
            base.join("ignored").to_string_lossy().as_bytes(),
        )
        .expect("rewrite legacy file");

        assert_eq!(
            read_recent_workspaces(base),
            vec![base.join("legacy-workspace")]
        );
    }

    #[test]
    fn first_existing_skips_paths_that_no_longer_exist() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing");
        let real = dir.path().join("real");
        fs::create_dir(&real).expect("create real dir");

        assert_eq!(
            first_existing(vec![missing.clone(), real.clone()]),
            Some(real)
        );
        assert_eq!(first_existing(vec![missing]), None);
    }

    #[test]
    fn last_environment_round_trips_per_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let (a, b) = (base.join("a"), base.join("b"));
        record_workspace_at(base, &a);
        record_workspace_at(base, &b);

        assert_eq!(last_environment_at(base, &a), None);
        record_environment_at(base, &a, Some("local"));
        record_environment_at(base, &b, Some("prod"));
        assert_eq!(last_environment_at(base, &a), Some("local".to_string()));
        assert_eq!(last_environment_at(base, &b), Some("prod".to_string()));

        record_environment_at(base, &a, None);
        assert_eq!(last_environment_at(base, &a), None);
        assert_eq!(last_environment_at(base, &b), Some("prod".to_string()));
    }

    #[test]
    fn last_environment_forgets_workspaces_that_left_the_recent_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let old = base.join("old");
        record_workspace_at(base, &old);
        record_environment_at(base, &old, Some("local"));
        for index in 0..MAX_RECENT_WORKSPACES {
            record_workspace_at(base, &base.join(format!("w{index}")));
        }

        let current = base.join("w0");
        record_environment_at(base, &current, Some("dev"));

        assert_eq!(last_environment_at(base, &old), None);
        assert_eq!(last_environment_at(base, &current), Some("dev".to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_windows_paths_read_as_plain_paths() {
        assert_eq!(
            parse_recent_workspaces("\\\\?\\C:\\dev\\http\n"),
            vec![PathBuf::from("C:\\dev\\http")]
        );
    }
}
