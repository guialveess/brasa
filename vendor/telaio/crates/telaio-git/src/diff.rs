//! Diffs as patch text, single file contents and applying patches.
//!
//! The diffs are standard `git diff` output, so any unified diff parser reads them, and
//! [`hunk_patch`] cuts one hunk out of a file's patch so it can be staged, unstaged or reverted
//! alone with [`Repo::apply`].

use crate::{GitError, Repo};

/// What a diff compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    /// `HEAD` against the work tree: everything the next commit could include.
    HeadToWorkTree,
    /// `HEAD` against the index: what is staged.
    HeadToIndex,
    /// The index against the work tree: what is not staged.
    IndexToWorkTree,
    /// A commit against its first parent, or against the empty tree for a root commit.
    Commit(String),
}

/// Where a file's content is read from. See [`Repo::file_at`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileVersion {
    /// The file in `HEAD`.
    Head,
    /// The file in the index.
    Index,
    /// The file in a commit (any revision git understands).
    Commit(String),
}

/// What [`Repo::apply`] changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyTarget {
    /// Only the index (`--cached`), to stage a patch or, reversed, to unstage it.
    Index,
    /// Only the work tree, for example to revert a hunk with `reverse`.
    WorkTree,
    /// Both the index and the work tree (`--index`).
    Both,
}

/// The lines added and removed in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumStat {
    /// The path relative to the repository root, separated by `/`. For a rename, the new path.
    pub path: String,
    /// Added lines, 0 for a binary file.
    pub additions: u32,
    /// Removed lines, 0 for a binary file.
    pub deletions: u32,
    /// Whether git considers the file binary (it has no line counts).
    pub binary: bool,
}

impl Repo {
    /// The patch for one file, in the format of `git diff`. `path` is relative to the repository
    /// root. An empty string means no difference.
    ///
    /// A file git does not know yet (not in the index) gets a "new file" patch against
    /// `/dev/null` for [`DiffSource::HeadToWorkTree`] and [`DiffSource::IndexToWorkTree`]. On a
    /// repository without commits `HEAD` is the empty tree.
    pub fn diff(&self, path: &str, source: &DiffSource) -> Result<String, GitError> {
        self.diff_with_context(path, source, "-U3")
    }

    /// Like [`Repo::diff`] but with every line of the file as context (`--unified=1000000`), so
    /// the patch holds the whole file. Its hunks are what [`crate::parse_file_diff`] turns into
    /// rows.
    pub fn diff_full(&self, path: &str, source: &DiffSource) -> Result<String, GitError> {
        self.diff_with_context(path, source, "--unified=1000000")
    }

    fn diff_with_context(
        &self,
        path: &str,
        source: &DiffSource,
        context: &str,
    ) -> Result<String, GitError> {
        if self.is_untracked(path, source)? {
            let args = [
                "diff",
                "--no-index",
                "--no-color",
                "--no-ext-diff",
                context,
                "--src-prefix=a/",
                "--dst-prefix=b/",
                "--",
                "/dev/null",
                path,
            ];
            // Exit code 1 only says that the files differ.
            return text(self.run_with(&args, None, &[1])?);
        }
        let revs = self.revisions(source)?;
        let mut args = vec![
            "diff",
            "--no-color",
            "--no-ext-diff",
            context,
            "--histogram",
        ];
        args.extend(revs.iter().map(String::as_str));
        args.push("--");
        args.push(path);
        text(self.run(&args)?)
    }

    /// The content of `path` in `version`, `None` when the file does not exist there (including
    /// `HEAD` on a repository without commits). Invalid UTF-8 is replaced.
    pub fn file_at(&self, path: &str, version: &FileVersion) -> Result<Option<String>, GitError> {
        let (exists, spec) = match version {
            FileVersion::Index => {
                let listed = self.run(&["ls-files", "--stage", "-z", "--", path])?;
                (!listed.is_empty(), format!(":0:{path}"))
            }
            FileVersion::Head | FileVersion::Commit(_) => {
                let rev = match version {
                    FileVersion::Commit(rev) => {
                        check_revision(rev)?;
                        rev.as_str()
                    }
                    _ => {
                        if !self.has_head()? {
                            return Ok(None);
                        }
                        "HEAD"
                    }
                };
                let listed = self.run(&["ls-tree", "-z", rev, "--", path])?;
                (!listed.is_empty(), format!("{rev}:{path}"))
            }
        };
        if !exists {
            return Ok(None);
        }
        text(self.run(&["show", &spec])?).map(Some)
    }

    /// Lines added and removed per file, for the summary line of a diff. Limited to the
    /// workspace folder. Untracked files are not included, git has no counts for them.
    pub fn numstat(&self, source: &DiffSource) -> Result<Vec<NumStat>, GitError> {
        let revs = self.revisions(source)?;
        let mut args = vec!["diff", "--numstat", "-z", "--no-ext-diff"];
        args.extend(revs.iter().map(String::as_str));
        if !self.prefix().is_empty() {
            args.push("--");
            args.push(self.prefix());
        }
        parse_numstat(&self.run(&args)?)
    }

    /// Applies `patch` (as printed by [`Repo::diff`] or cut by [`hunk_patch`]). With `reverse`
    /// the patch is undone instead.
    pub fn apply(&self, patch: &str, target: ApplyTarget, reverse: bool) -> Result<(), GitError> {
        self.run_apply(patch, target, reverse, false).map(|_| ())
    }

    /// Whether [`Repo::apply`] would succeed with the same arguments, without changing anything
    /// (`git apply --check`).
    pub fn can_apply(
        &self,
        patch: &str,
        target: ApplyTarget,
        reverse: bool,
    ) -> Result<bool, GitError> {
        match self.run_apply(patch, target, reverse, true) {
            Ok(_) => Ok(true),
            Err(GitError::Failed { .. }) => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn run_apply(
        &self,
        patch: &str,
        target: ApplyTarget,
        reverse: bool,
        check: bool,
    ) -> Result<Vec<u8>, GitError> {
        let mut args = vec!["apply", "--whitespace=nowarn"];
        match target {
            ApplyTarget::Index => args.push("--cached"),
            ApplyTarget::WorkTree => {}
            ApplyTarget::Both => args.push("--index"),
        }
        if reverse {
            args.push("-R");
        }
        if check {
            args.push("--check");
        }
        args.push("-");
        self.run_with(&args, Some(patch.as_bytes()), &[])
    }

    /// Whether `path` has to be diffed against `/dev/null`: the source ends in the work tree
    /// and the index does not have the file.
    fn is_untracked(&self, path: &str, source: &DiffSource) -> Result<bool, GitError> {
        if !matches!(
            source,
            DiffSource::HeadToWorkTree | DiffSource::IndexToWorkTree
        ) {
            return Ok(false);
        }
        let listed = self.run(&["ls-files", "--cached", "-z", "--", path])?;
        Ok(listed.is_empty() && self.root().join(path).is_file())
    }

    /// The revision arguments of `git diff` for `source`.
    fn revisions(&self, source: &DiffSource) -> Result<Vec<String>, GitError> {
        Ok(match source {
            DiffSource::HeadToWorkTree => vec![self.head_or_empty_tree()?],
            DiffSource::HeadToIndex => vec!["--cached".to_string(), self.head_or_empty_tree()?],
            DiffSource::IndexToWorkTree => Vec::new(),
            DiffSource::Commit(rev) => {
                check_revision(rev)?;
                let parent = format!("{rev}^");
                let base = match self.run(&["rev-parse", "--verify", "--quiet", &parent]) {
                    Ok(_) => parent,
                    Err(GitError::Failed { .. }) => self.empty_tree()?,
                    Err(err) => return Err(err),
                };
                vec![base, rev.clone()]
            }
        })
    }

    fn head_or_empty_tree(&self) -> Result<String, GitError> {
        if self.has_head()? {
            Ok("HEAD".to_string())
        } else {
            self.empty_tree()
        }
    }

    /// The id of the empty tree, which depends on the repository's hash algorithm.
    pub(crate) fn empty_tree(&self) -> Result<String, GitError> {
        let output = self.run_with(&["hash-object", "-t", "tree", "--stdin"], Some(b""), &[])?;
        Ok(String::from_utf8_lossy(&output).trim().to_string())
    }
}

/// Rejects a revision that git would read as an option.
pub(crate) fn check_revision(rev: &str) -> Result<(), GitError> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(GitError::Parse(format!("invalid revision {rev:?}")));
    }
    Ok(())
}

fn text(bytes: Vec<u8>) -> Result<String, GitError> {
    Ok(match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    })
}

/// Parses `git diff --numstat -z`: `added TAB removed TAB path NUL`, and for a rename
/// `added TAB removed TAB NUL old NUL new NUL`.
pub(crate) fn parse_numstat(output: &[u8]) -> Result<Vec<NumStat>, GitError> {
    let mut fields = output
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut stats = Vec::new();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let mut parts = record.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(GitError::Parse(format!("bad numstat record {record:?}")));
        };
        let path = if path.is_empty() {
            let _old = fields.next();
            fields
                .next()
                .ok_or_else(|| GitError::Parse("numstat rename without paths".to_string()))?
        } else {
            path.to_string()
        };
        let binary = added == "-" && removed == "-";
        let count = |value: &str| {
            if binary {
                Ok(0)
            } else {
                value
                    .parse::<u32>()
                    .map_err(|_| GitError::Parse(format!("bad numstat count {value:?}")))
            }
        };
        stats.push(NumStat {
            path,
            additions: count(added)?,
            deletions: count(removed)?,
            binary,
        });
    }
    Ok(stats)
}

/// A patch with the header of `file_patch` (everything before the first `@@`) and only the hunk
/// number `hunk_index` (from 0), or `None` when the patch has no such hunk, for example for a
/// binary file or a pure rename.
pub fn hunk_patch(file_patch: &str, hunk_index: usize) -> Option<String> {
    let mut header = String::new();
    let mut hunks: Vec<String> = Vec::new();
    for line in file_patch.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            hunks.push(String::new());
        }
        match hunks.last_mut() {
            Some(hunk) => hunk.push_str(line),
            None => header.push_str(line),
        }
    }
    let hunk = hunks.into_iter().nth(hunk_index)?;
    Some(header + &hunk)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const PATCH: &str = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n\
@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -10 +10 @@\n-x\n+y\n\\ No newline at end of file\n";

    #[test]
    fn hunk_patch_keeps_the_header_and_one_hunk() {
        let first = hunk_patch(PATCH, 0).unwrap();
        assert_eq!(
            first,
            "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n-a\n+A\n b\n"
        );
        let second = hunk_patch(PATCH, 1).unwrap();
        assert!(second.starts_with("diff --git a/f b/f\n"));
        assert!(second.ends_with("@@ -10 +10 @@\n-x\n+y\n\\ No newline at end of file\n"));
        assert!(!second.contains("-a\n"));
        assert_eq!(hunk_patch(PATCH, 2), None);
        assert_eq!(
            hunk_patch("diff --git a/b b/b\nBinary files differ\n", 0),
            None
        );
    }

    #[test]
    fn parses_numstat_with_binary_and_rename() {
        let out = b"3\t1\ta b.txt\0-\t-\timg.png\x000\t2\t\0old.txt\0new.txt\0";
        assert_eq!(
            parse_numstat(out).unwrap(),
            vec![
                NumStat {
                    path: "a b.txt".into(),
                    additions: 3,
                    deletions: 1,
                    binary: false
                },
                NumStat {
                    path: "img.png".into(),
                    additions: 0,
                    deletions: 0,
                    binary: true
                },
                NumStat {
                    path: "new.txt".into(),
                    additions: 0,
                    deletions: 2,
                    binary: false
                },
            ]
        );
        assert!(parse_numstat(b"garbage\0").is_err());
    }

    #[test]
    fn rejects_option_like_revisions() {
        assert!(check_revision("--output=x").is_err());
        assert!(check_revision("").is_err());
        assert!(check_revision("HEAD~1").is_ok());
    }
}
