//! The diff tab of the Git integration (designs 2a, 2b, 7a): which file it shows and what is
//! known about its changes. The patch itself is read by `views/git/diff_tab.rs` on the
//! background executor and turned into rows by `state/diff_view.rs`; this is only plain data.

use telaio_git::{FileDiff, LineKind};

/// The id of the diff tab. There is one: clicking another changed file reuses it, like a
/// preview.
pub const TAB_ID: &str = "git:diff";

/// What the diff tab compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    /// The last commit against the files on disk: everything the next commit could include.
    WorkTree,
    /// What a commit changed, against its first parent (opened from the log tab).
    Commit {
        /// The commit, full id.
        oid: String,
        /// Its short id, for the titles.
        short: String,
    },
}

impl DiffTarget {
    /// The tab's detail after the file name: "Working Tree" or the short commit id.
    pub fn detail(&self) -> String {
        match self {
            DiffTarget::WorkTree => rust_i18n::t!("git.diff.working_tree").into_owned(),
            DiffTarget::Commit { short, .. } => short.clone(),
        }
    }
}

/// What a diff tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffTab {
    /// The file, relative to the repository root, `/` separated.
    pub repo_path: String,
    /// The file, relative to the workspace: what the header shows and, for a request, its id.
    pub path: String,
    /// What is compared.
    pub target: DiffTarget,
    /// The status letter of the file, as in the Source Control panel.
    pub letter: char,
    /// Bumped every time the patch is read again, so the view knows when to reload the viewer.
    pub generation: u64,
    /// What the last read found, `None` while it runs.
    pub summary: Option<DiffSummary>,
}

/// What a read of the patch found, for the header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffSummary {
    /// Added lines.
    pub additions: usize,
    /// Removed lines.
    pub deletions: usize,
    /// How many blocks of changes (runs of removed and added lines) the file has.
    pub blocks: usize,
    /// Why the patch could not be read, shown instead of the diff.
    pub error: Option<String>,
}

impl DiffTab {
    /// A tab for `repo_path`, shown as `path` (relative to the workspace).
    pub fn new(repo_path: String, path: String, target: DiffTarget, letter: char) -> Self {
        DiffTab {
            repo_path,
            path,
            target,
            letter,
            generation: 0,
            summary: None,
        }
    }

    /// The tab's label: the file name.
    pub fn label(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The folder of the file with a trailing `/`, empty at the workspace root.
    pub fn folder(&self) -> &str {
        match self.path.rfind('/') {
            Some(index) => &self.path[..=index],
            None => "",
        }
    }

    /// The request id when the file is a `.brasa` request, for "Open request".
    pub fn request_id(&self) -> Option<&str> {
        self.path
            .ends_with(".brasa")
            .then_some(self.path.as_str())
    }
}

/// Counts the added and removed lines and the change blocks of a parsed file diff.
pub fn summarize(diff: &FileDiff) -> DiffSummary {
    let count = |kind: LineKind| diff.lines.iter().filter(|line| line.kind == kind).count();
    DiffSummary {
        additions: count(LineKind::Added),
        deletions: count(LineKind::Removed),
        blocks: diff.blocks.len(),
        error: None,
    }
}

/// Whether a hunk of the work tree diff is in the index already.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkStage {
    /// Not staged: "Stage" applies it to the index.
    Unstaged,
    /// Staged: "Unstage" takes it back out.
    Staged,
    /// Partly staged: only Revert is offered.
    Mixed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn counts_lines_and_blocks_but_not_the_file_header() {
        // Two hunks of git, but three blocks: a context line splits the first hunk.
        let patch = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1,3 +1,2 @@\n-a\n+A\n b\n-c\n\
                     @@ -9 +9,2 @@\n z\n+y\n";
        assert_eq!(
            summarize(&telaio_git::parse_file_diff(patch)),
            DiffSummary {
                additions: 2,
                deletions: 2,
                blocks: 3,
                error: None,
            }
        );
    }

    #[test]
    fn labels_and_folder() {
        let tab = DiffTab::new(
            "api/auth/login.brasa".into(),
            "auth/login.brasa".into(),
            DiffTarget::WorkTree,
            'M',
        );
        assert_eq!(tab.label(), "login.brasa");
        assert_eq!(tab.folder(), "auth/");
        assert_eq!(tab.request_id(), Some("auth/login.brasa"));
        let root = DiffTab::new(
            ".gitignore".into(),
            ".gitignore".into(),
            DiffTarget::WorkTree,
            'A',
        );
        assert_eq!(root.folder(), "");
        assert_eq!(root.request_id(), None);
    }
}
