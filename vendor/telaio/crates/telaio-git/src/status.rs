//! The parser of `git status --porcelain=v2 -z --branch` and the types it produces.

use crate::GitError;

/// What `HEAD` points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    /// A branch with at least one commit.
    Branch(String),
    /// A detached `HEAD`, with the 7 character short commit id.
    Detached(String),
    /// A branch that has no commit yet.
    Unborn(String),
}

/// How a file differs in the index (staged) or in the work tree (unstaged).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// No difference.
    Unmodified,
    /// The content changed.
    Modified,
    /// The file is new.
    Added,
    /// The file was removed.
    Deleted,
    /// The file was renamed.
    Renamed,
    /// The file was copied.
    Copied,
    /// The file type changed, for example from file to symlink.
    TypeChanged,
}

impl Change {
    fn from_code(code: u8) -> Result<Change, GitError> {
        Ok(match code {
            b'.' => Change::Unmodified,
            b'M' => Change::Modified,
            b'A' => Change::Added,
            b'D' => Change::Deleted,
            b'R' => Change::Renamed,
            b'C' => Change::Copied,
            b'T' => Change::TypeChanged,
            // Only seen on unmerged entries, where `conflicted` carries the real meaning.
            b'U' => Change::Modified,
            other => {
                return Err(GitError::Parse(format!(
                    "unknown status code {:?}",
                    other as char
                )));
            }
        })
    }
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path relative to the repository root, separated by `/`.
    pub path: String,
    /// For a rename or copy, the path it came from.
    pub orig_path: Option<String>,
    /// The staged change (index against `HEAD`).
    pub index: Change,
    /// The unstaged change (work tree against index). An untracked file is `Added` here.
    pub worktree: Change,
    /// The file has an unresolved merge conflict.
    pub conflicted: bool,
    /// The file is not tracked by git.
    pub untracked: bool,
}

/// The state of a repository, as reported by `git status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// What `HEAD` points at.
    pub head: Head,
    /// The 7 character short id of the `HEAD` commit, `None` on a branch with no commit yet.
    pub head_oid: Option<String>,
    /// The upstream branch, for example `origin/main`, when one is set.
    pub upstream: Option<String>,
    /// Commits the branch has that the upstream does not.
    pub ahead: u32,
    /// Commits the upstream has that the branch does not.
    pub behind: u32,
    /// The changed and untracked files (ignored files are left out).
    pub entries: Vec<Entry>,
}

impl Status {
    /// Splits the entries into the ones inside `prefix` (a folder relative to the repository
    /// root, `/` separated, `""` for the whole repository) and the count of entries outside it
    /// that have a staged change, which a commit made from the workspace would include.
    pub fn split_by_prefix(&self, prefix: &str) -> (Vec<&Entry>, usize) {
        let prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() {
            return (self.entries.iter().collect(), 0);
        }
        let mut inside = Vec::new();
        let mut staged_outside = 0;
        for entry in &self.entries {
            let in_prefix = entry
                .path
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
            if in_prefix {
                inside.push(entry);
            } else if entry.index != Change::Unmodified && !entry.untracked {
                staged_outside += 1;
            }
        }
        (inside, staged_outside)
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn bad(field: &[u8]) -> GitError {
    GitError::Parse(format!(
        "malformed status line {:?}",
        String::from_utf8_lossy(field)
    ))
}

/// Parses the output of `git status --porcelain=v2 -z --branch`.
pub(crate) fn parse_status(output: &[u8]) -> Result<Status, GitError> {
    let mut oid = String::new();
    let mut head_name = String::new();
    let mut upstream = None;
    let (mut ahead, mut behind) = (0, 0);
    let mut entries = Vec::new();

    let mut fields = output.split(|&b| b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        match field[0] {
            b'#' => {
                let line = text(field);
                let rest = line.trim_start_matches("# ");
                if let Some(v) = rest.strip_prefix("branch.oid ") {
                    oid = v.to_string();
                } else if let Some(v) = rest.strip_prefix("branch.head ") {
                    head_name = v.to_string();
                } else if let Some(v) = rest.strip_prefix("branch.upstream ") {
                    upstream = Some(v.to_string());
                } else if let Some(v) = rest.strip_prefix("branch.ab ") {
                    let mut parts = v.split_whitespace();
                    let a = parts.next().and_then(|s| s.strip_prefix('+')?.parse().ok());
                    let b = parts.next().and_then(|s| s.strip_prefix('-')?.parse().ok());
                    (ahead, behind) = (a.ok_or_else(|| bad(field))?, b.ok_or_else(|| bad(field))?);
                }
            }
            b'1' => {
                let parts: Vec<&[u8]> = field.splitn(9, |&b| b == b' ').collect();
                let [_, xy, .., path] = parts[..] else {
                    return Err(bad(field));
                };
                if parts.len() != 9 || xy.len() != 2 {
                    return Err(bad(field));
                }
                entries.push(Entry {
                    path: text(path),
                    orig_path: None,
                    index: Change::from_code(xy[0])?,
                    worktree: Change::from_code(xy[1])?,
                    conflicted: false,
                    untracked: false,
                });
            }
            b'2' => {
                let parts: Vec<&[u8]> = field.splitn(10, |&b| b == b' ').collect();
                let [_, xy, .., path] = parts[..] else {
                    return Err(bad(field));
                };
                if parts.len() != 10 || xy.len() != 2 {
                    return Err(bad(field));
                }
                let orig = fields.next().ok_or_else(|| bad(field))?;
                entries.push(Entry {
                    path: text(path),
                    orig_path: Some(text(orig)),
                    index: Change::from_code(xy[0])?,
                    worktree: Change::from_code(xy[1])?,
                    conflicted: false,
                    untracked: false,
                });
            }
            b'u' => {
                let parts: Vec<&[u8]> = field.splitn(11, |&b| b == b' ').collect();
                let [_, xy, .., path] = parts[..] else {
                    return Err(bad(field));
                };
                if parts.len() != 11 || xy.len() != 2 {
                    return Err(bad(field));
                }
                entries.push(Entry {
                    path: text(path),
                    orig_path: None,
                    index: Change::from_code(xy[0])?,
                    worktree: Change::from_code(xy[1])?,
                    conflicted: true,
                    untracked: false,
                });
            }
            b'?' => {
                let path = field.get(2..).ok_or_else(|| bad(field))?;
                entries.push(Entry {
                    path: text(path),
                    orig_path: None,
                    index: Change::Unmodified,
                    worktree: Change::Added,
                    conflicted: false,
                    untracked: true,
                });
            }
            // Ignored files, and anything a newer git might add.
            _ => {}
        }
    }

    let head = if oid == "(initial)" {
        Head::Unborn(head_name)
    } else if head_name == "(detached)" {
        Head::Detached(oid.chars().take(7).collect())
    } else if head_name.is_empty() {
        return Err(GitError::Parse(
            "the status has no branch header".to_string(),
        ));
    } else {
        Head::Branch(head_name)
    };
    let head_oid = (oid != "(initial)" && !oid.is_empty()).then(|| oid.chars().take(7).collect());
    Ok(Status {
        head,
        head_oid,
        upstream,
        ahead,
        behind,
        entries,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const OID: &str = "1111111111111111111111111111111111111111";
    const H: &str = "2222222222222222222222222222222222222222";

    fn nul(parts: &[String]) -> Vec<u8> {
        let mut out = Vec::new();
        for part in parts {
            out.extend_from_slice(part.as_bytes());
            out.push(0);
        }
        out
    }

    fn header(head: &str) -> Vec<String> {
        vec![
            format!("# branch.oid {OID}"),
            format!("# branch.head {head}"),
        ]
    }

    #[test]
    fn parses_branch_headers_and_changes() {
        let mut parts = header("main");
        parts.push("# branch.upstream origin/main".into());
        parts.push("# branch.ab +3 -1".into());
        parts.push(format!(
            "1 .M N... 100644 100644 100644 {H} {H} a b/ñandú.postino"
        ));
        parts.push(format!("1 A. N... 000000 100644 100644 {H} {H} new.txt"));
        parts.push(format!("1 .D N... 100644 100644 000000 {H} {H} gone.txt"));
        parts.push("? untracked dir/file.txt".into());
        parts.push("! ignored.log".into());
        let status = parse_status(&nul(&parts)).unwrap();
        assert_eq!(status.head, Head::Branch("main".into()));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (3, 1));
        assert_eq!(status.entries.len(), 4);
        assert_eq!(status.entries[0].path, "a b/ñandú.postino");
        assert_eq!(status.entries[0].index, Change::Unmodified);
        assert_eq!(status.entries[0].worktree, Change::Modified);
        assert_eq!(status.entries[1].index, Change::Added);
        assert_eq!(status.entries[2].worktree, Change::Deleted);
        assert!(status.entries[3].untracked);
        assert_eq!(status.entries[3].path, "untracked dir/file.txt");
    }

    #[test]
    fn parses_a_rename_with_its_original_path() {
        let mut parts = header("main");
        parts.push(format!(
            "2 R. N... 100644 100644 100644 {H} {H} R100 new name.txt"
        ));
        parts.push("old name.txt".into());
        parts.push("? after.txt".into());
        let status = parse_status(&nul(&parts)).unwrap();
        assert_eq!(status.entries.len(), 2);
        let rename = &status.entries[0];
        assert_eq!(rename.index, Change::Renamed);
        assert_eq!(rename.path, "new name.txt");
        assert_eq!(rename.orig_path.as_deref(), Some("old name.txt"));
        assert_eq!(status.entries[1].path, "after.txt");
    }

    #[test]
    fn parses_unmerged_entries() {
        let mut parts = header("main");
        parts.push(format!(
            "u UU N... 100644 100644 100644 100644 {H} {H} {H} conflict.txt"
        ));
        let status = parse_status(&nul(&parts)).unwrap();
        assert!(status.entries[0].conflicted);
        assert_eq!(status.entries[0].path, "conflict.txt");
    }

    #[test]
    fn parses_detached_and_unborn_heads() {
        let status = parse_status(&nul(&header("(detached)"))).unwrap();
        assert_eq!(status.head, Head::Detached("1111111".into()));
        assert_eq!((status.upstream, status.ahead, status.behind), (None, 0, 0));

        let parts = vec![
            "# branch.oid (initial)".to_string(),
            "# branch.head main".to_string(),
        ];
        let status = parse_status(&nul(&parts)).unwrap();
        assert_eq!(status.head, Head::Unborn("main".into()));
    }

    #[test]
    fn rejects_garbage() {
        let mut parts = header("main");
        parts.push("1 too short".into());
        assert!(matches!(
            parse_status(&nul(&parts)),
            Err(GitError::Parse(_))
        ));
        assert!(matches!(parse_status(b""), Err(GitError::Parse(_))));
    }

    fn entry(path: &str, index: Change, untracked: bool) -> Entry {
        Entry {
            path: path.into(),
            orig_path: None,
            index,
            worktree: Change::Unmodified,
            conflicted: false,
            untracked,
        }
    }

    #[test]
    fn splits_entries_by_prefix() {
        let status = Status {
            head: Head::Branch("main".into()),
            head_oid: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            entries: vec![
                entry("ws/a.postino", Change::Added, false),
                entry("ws/sub/b.postino", Change::Unmodified, false),
                entry("wsx/c.txt", Change::Modified, false),
                entry("README.md", Change::Added, false),
                entry("other.txt", Change::Unmodified, false),
                entry("new.txt", Change::Unmodified, true),
            ],
        };
        let (inside, outside) = status.split_by_prefix("ws");
        let paths: Vec<_> = inside.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["ws/a.postino", "ws/sub/b.postino"]);
        assert_eq!(outside, 2);
        assert_eq!(status.split_by_prefix("ws/").0.len(), 2);
        let (all, none) = status.split_by_prefix("");
        assert_eq!((all.len(), none), (6, 0));
    }
}
