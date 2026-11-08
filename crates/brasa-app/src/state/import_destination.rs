//! Where an import writes (GitHub #208): the workspace is the destination of the import, chosen
//! in the wizard, not something that must exist before it. Plain logic, unit tested without a
//! window. Nothing here writes to the workspace: the wizard creates it when the user confirms.
//!
//! The folder checks touch the disk, so [`Destination::refresh`] runs when the name or the folder
//! changes, never from render.

use std::fs;
use std::path::{Path, PathBuf};

use brasa_workspace::sanitize_file_name;

use super::new_workspace::default_location_for_user;

/// The folder inside the Documents folder that new import workspaces go in.
const DEFAULT_PARENT: &str = "Brasa";

/// Which kind of destination is selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The workspace that is open.
    Current,
    /// Another workspace of the recent list.
    Recent,
    /// A new workspace, a folder named after the workspace name inside the location.
    New,
    /// A folder the user picked, used as it is.
    Existing,
}

/// What the check of the selected folder found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderCheck {
    /// Nothing to tell: the import can go there.
    Ready,
    /// The new workspace's folder exists and has other things in it. `suggested` is a free name.
    NameTaken { suggested: String },
    /// The folder already is a Brasa workspace. Asks whether to add the collection to it.
    IsWorkspace,
    /// The picked folder has `count` files in it and is not a workspace.
    HasFiles { count: usize },
    /// Nothing can be written there.
    NotWritable,
}

/// The resolved destination of an import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The workspace that is open.
    Current,
    /// An existing folder, opened as the workspace after the import.
    Open(PathBuf),
    /// A folder to create as a new, empty workspace.
    Create(PathBuf),
}

impl Target {
    /// The folder of the target, `None` for the open workspace.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Target::Current => None,
            Target::Open(path) | Target::Create(path) => Some(path),
        }
    }
}

/// The state of the destination block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    /// The open workspace.
    pub current: Option<PathBuf>,
    /// The other recent workspaces that still exist.
    pub recent: Vec<PathBuf>,
    /// What is selected.
    pub choice: Choice,
    /// The index in `recent` of the selected recent workspace.
    pub recent_index: usize,
    /// The folder a new workspace is created in.
    pub location: PathBuf,
    /// The folder picked for [`Choice::Existing`].
    pub existing: Option<PathBuf>,
    /// The folder whose notice the user accepted ("Use it anyway", "Add to that workspace").
    pub accepted: Option<PathBuf>,
    /// The check of the selected folder, as of the last [`Self::refresh`].
    pub check: FolderCheck,
}

impl Destination {
    /// The destination of a wizard opened with `current` as the workspace: that workspace, or a
    /// new one when there is none. `recent` is the recent list, which may include `current`.
    pub fn new(current: Option<PathBuf>, recent: &[PathBuf], location: PathBuf) -> Self {
        let recent = recent
            .iter()
            .filter(|path| Some(*path) != current.as_ref() && path.is_dir())
            .cloned()
            .collect();
        Destination {
            choice: if current.is_some() {
                Choice::Current
            } else {
                Choice::New
            },
            current,
            recent,
            recent_index: 0,
            location,
            existing: None,
            accepted: None,
            check: FolderCheck::Ready,
        }
    }

    /// The folder selected with the workspace name `name`. `None` when no folder is selected yet:
    /// a blank name, no picked folder, or a recent list that is empty.
    pub fn folder(&self, name: &str) -> Option<PathBuf> {
        match self.choice {
            Choice::Current => self.current.clone(),
            Choice::Recent => self.recent.get(self.recent_index).cloned(),
            Choice::Existing => self.existing.clone(),
            Choice::New => {
                let name = name.trim();
                (!name.is_empty()).then(|| self.location.join(sanitize_file_name(name)))
            }
        }
    }

    /// Checks the selected folder again. Touches the disk: call it when the selection changes.
    pub fn refresh(&mut self, name: &str) {
        if self.accepted.is_some() && self.accepted == self.folder(name) {
            self.check = FolderCheck::Ready;
            return;
        }
        self.check = match (self.choice, self.folder(name)) {
            (Choice::Current, _) | (_, None) => FolderCheck::Ready,
            (Choice::Recent, Some(folder)) => {
                if is_writable(&folder) {
                    FolderCheck::Ready
                } else {
                    FolderCheck::NotWritable
                }
            }
            (Choice::New, Some(folder)) => check_new(&folder),
            (Choice::Existing, Some(folder)) => check_existing(&folder),
        };
    }

    /// Accepts the notice of the selected folder: "Add to that workspace" or "Use it anyway".
    pub fn accept(&mut self, name: &str) {
        let Some(folder) = self.folder(name) else {
            return;
        };
        self.existing = Some(folder.clone());
        self.accepted = Some(folder);
        self.choice = Choice::Existing;
        self.check = FolderCheck::Ready;
    }

    /// The resolved destination with the workspace name `name`, `None` while the user still has
    /// to fill something in or answer a notice.
    pub fn target(&self, name: &str) -> Option<Target> {
        let folder = self.folder(name)?;
        let accepted = self.accepted.as_deref() == Some(folder.as_path());
        let ready = accepted || self.check == FolderCheck::Ready;
        if !ready {
            return None;
        }
        match self.choice {
            Choice::Current => Some(Target::Current),
            Choice::Recent | Choice::Existing => Some(Target::Open(folder)),
            Choice::New => Some(Target::Create(folder)),
        }
    }
}

/// The folder new import workspaces are suggested in.
pub fn default_location() -> PathBuf {
    default_location_for_user().join(DEFAULT_PARENT)
}

/// A workspace name from the title of the source: lowercase with hyphens
/// (`Acme Payments API` gives `acme-payments-api`). Empty when nothing usable is left.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for character in title.chars() {
        if character.is_alphanumeric() {
            out.extend(character.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Whether `folder` looks like a Brasa workspace: its `environments` or `.brasa` folder, or a
/// request file at its root.
pub fn is_workspace(folder: &Path) -> bool {
    if folder.join("environments").is_dir() || folder.join(".brasa").is_dir() {
        return true;
    }
    fs::read_dir(folder).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().extension().is_some_and(|ext| ext == "brasa"))
    })
}

/// How many entries `folder` has, `0` when it cannot be read.
fn entry_count(folder: &Path) -> usize {
    fs::read_dir(folder).map_or(0, |entries| entries.flatten().count())
}

/// Whether a file can be created in `folder`, or in its closest existing parent when it does not
/// exist yet.
fn is_writable(folder: &Path) -> bool {
    let Some(existing) = folder.ancestors().find(|path| path.is_dir()) else {
        return false;
    };
    tempfile::tempfile_in(existing).is_ok()
}

/// The check of a folder a new workspace would be created as.
fn check_new(folder: &Path) -> FolderCheck {
    if folder.exists() && entry_count(folder) > 0 {
        if is_workspace(folder) {
            return FolderCheck::IsWorkspace;
        }
        return FolderCheck::NameTaken {
            suggested: free_name(folder),
        };
    }
    if folder.exists() && !folder.is_dir() {
        return FolderCheck::NameTaken {
            suggested: free_name(folder),
        };
    }
    if is_writable(folder) {
        FolderCheck::Ready
    } else {
        FolderCheck::NotWritable
    }
}

/// The check of a folder the user picked.
fn check_existing(folder: &Path) -> FolderCheck {
    if !is_writable(folder) {
        return FolderCheck::NotWritable;
    }
    if is_workspace(folder) {
        return FolderCheck::IsWorkspace;
    }
    match entry_count(folder) {
        0 => FolderCheck::Ready,
        count => FolderCheck::HasFiles { count },
    }
}

/// The first free name next to `folder`: `name-2`, `name-3`, ...
fn free_name(folder: &Path) -> String {
    let name = folder
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let parent = folder.parent().unwrap_or(folder);
    (2..)
        .map(|index| format!("{name}-{index}"))
        .find(|candidate| !parent.join(candidate).exists())
        .unwrap_or(name)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn destination(location: &Path) -> Destination {
        Destination::new(None, &[], location.to_path_buf())
    }

    #[test]
    fn a_slug_is_lowercase_with_hyphens() {
        assert_eq!(slug("Acme Payments API"), "acme-payments-api");
        assert_eq!(slug("  Pagos / v2 (beta)! "), "pagos-v2-beta");
        assert_eq!(slug("***"), "");
    }

    #[test]
    fn without_a_workspace_a_new_one_is_proposed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut destination = destination(dir.path());
        assert_eq!(destination.choice, Choice::New);
        assert_eq!(destination.target(""), None);

        destination.refresh("acme");
        assert_eq!(destination.check, FolderCheck::Ready);
        assert_eq!(
            destination.target("acme"),
            Some(Target::Create(dir.path().join("acme")))
        );
    }

    #[test]
    fn with_a_workspace_the_open_one_is_the_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let current = dir.path().join("pagos");
        let other = dir.path().join("catalogo");
        fs::create_dir(&current).expect("mkdir");
        fs::create_dir(&other).expect("mkdir");
        let gone = dir.path().join("gone");
        let mut destination = Destination::new(
            Some(current.clone()),
            &[current, other.clone(), gone],
            dir.path().to_path_buf(),
        );
        assert_eq!(destination.target("x"), Some(Target::Current));
        assert_eq!(destination.recent, vec![other.clone()]);

        destination.choice = Choice::Recent;
        destination.refresh("x");
        assert_eq!(destination.target("x"), Some(Target::Open(other)));
    }

    #[test]
    fn a_taken_name_proposes_a_free_one() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("acme")).expect("mkdir");
        fs::write(dir.path().join("acme/notes.txt"), "x").expect("write");
        fs::create_dir(dir.path().join("acme-2")).expect("mkdir");
        let mut destination = destination(dir.path());

        destination.refresh("acme");
        assert_eq!(
            destination.check,
            FolderCheck::NameTaken {
                suggested: "acme-3".to_string()
            }
        );
        assert_eq!(destination.target("acme"), None);

        destination.refresh("acme-3");
        assert_eq!(
            destination.target("acme-3"),
            Some(Target::Create(dir.path().join("acme-3")))
        );
    }

    #[test]
    fn an_empty_folder_of_that_name_can_be_the_new_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir(dir.path().join("acme")).expect("mkdir");
        let mut destination = destination(dir.path());
        destination.refresh("acme");
        assert_eq!(destination.check, FolderCheck::Ready);
    }

    #[test]
    fn a_workspace_of_that_name_asks_to_add_to_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let folder = dir.path().join("acme");
        fs::create_dir_all(folder.join("environments")).expect("mkdir");
        let mut destination = destination(dir.path());

        destination.refresh("acme");
        assert_eq!(destination.check, FolderCheck::IsWorkspace);
        assert_eq!(destination.target("acme"), None);

        destination.accept("acme");
        assert_eq!(destination.target("acme"), Some(Target::Open(folder)));
    }

    #[test]
    fn a_picked_folder_with_files_needs_an_answer() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("a.txt"), "x").expect("write");
        fs::write(dir.path().join("b.txt"), "x").expect("write");
        let mut destination = destination(dir.path());
        destination.choice = Choice::Existing;
        destination.existing = Some(dir.path().to_path_buf());

        destination.refresh("");
        assert_eq!(destination.check, FolderCheck::HasFiles { count: 2 });
        assert_eq!(destination.target(""), None);

        destination.accept("");
        assert_eq!(
            destination.target(""),
            Some(Target::Open(dir.path().to_path_buf()))
        );
    }

    #[test]
    fn accepting_a_folder_does_not_carry_over_to_another() {
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        for folder in [&first, &second] {
            fs::create_dir(folder).expect("mkdir");
            fs::write(folder.join("a.txt"), "x").expect("write");
        }
        let mut destination = destination(dir.path());
        destination.choice = Choice::Existing;
        destination.existing = Some(first);
        destination.refresh("");
        destination.accept("");

        destination.existing = Some(second);
        destination.refresh("");
        assert_eq!(destination.target(""), None);
    }

    #[test]
    fn an_accepted_folder_stays_accepted_when_checked_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("a.txt"), "x").expect("write");
        let mut destination = destination(dir.path());
        destination.choice = Choice::Existing;
        destination.existing = Some(dir.path().to_path_buf());
        destination.accept("");
        destination.refresh("");
        assert_eq!(destination.check, FolderCheck::Ready);
    }

    #[test]
    fn a_request_at_the_root_makes_a_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!is_workspace(dir.path()));
        fs::write(dir.path().join("hello.brasa"), "GET http://x\n").expect("write");
        assert!(is_workspace(dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_parent_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).expect("mkdir");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).expect("chmod");
        // Root ignores permissions: nothing to check when running as root.
        if tempfile::tempfile_in(&locked).is_err() {
            let mut destination = Destination::new(None, &[], locked.clone());
            destination.refresh("acme");
            assert_eq!(destination.check, FolderCheck::NotWritable);
            assert_eq!(destination.target("acme"), None);
        }
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).expect("chmod");
    }
}
