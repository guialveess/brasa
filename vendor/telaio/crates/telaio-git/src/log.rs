//! The commit log, in pages, and the detail of one commit.

use crate::diff::{check_revision, parse_numstat};
use crate::{GitError, Repo};

/// What a [`RefLabel`] on a commit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefLabelKind {
    /// The checked out branch (or `HEAD` itself when it is detached, then named `HEAD`).
    Head,
    /// A local branch.
    Local,
    /// A remote tracking branch.
    Remote,
    /// A tag.
    Tag,
}

/// A branch or tag that points at a commit, shown as a badge next to its subject.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefLabel {
    /// Which badge to draw.
    pub kind: RefLabelKind,
    /// The short name: `main`, `origin/main`, `v1`.
    pub name: String,
}

/// A commit in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogCommit {
    /// The full id.
    pub oid: String,
    /// The abbreviated id.
    pub short: String,
    /// The full ids of the parents, first parent first. Empty for a root commit.
    pub parents: Vec<String>,
    /// The author's name.
    pub author_name: String,
    /// The author's email.
    pub author_email: String,
    /// The author date, in unix seconds.
    pub date: i64,
    /// The first line of the message.
    pub subject: String,
    /// The branches and tags pointing at this commit.
    pub refs: Vec<RefLabel>,
}

/// What [`Repo::log`] shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogQuery {
    /// Only the history of this branch, tag or revision. `None` shows all the refs (every local
    /// branch, remote branch and tag, and `HEAD`).
    pub branch: Option<String>,
    /// Only commits whose author name or email contains this text, ignoring case.
    pub author: Option<String>,
    /// Only commits that touch this path, relative to the repository root. A folder works.
    pub path: Option<String>,
    /// Commits whose message contains this text (ignoring case), or whose id is the hash prefix
    /// typed here. See [`Repo::log`].
    pub text: Option<String>,
    /// How many commits to skip, for paging.
    pub skip: usize,
    /// The most commits to return. 0 returns none.
    pub limit: usize,
}

/// A file changed by a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    /// The path relative to the repository root. For a rename, the new path.
    pub path: String,
    /// The old path of a rename or copy.
    pub orig_path: Option<String>,
    /// The status letter: `A` added, `M` modified, `D` deleted, `R` renamed, `T` type changed,
    /// `C` copied.
    pub letter: char,
    /// Added lines, 0 for a binary file.
    pub additions: u32,
    /// Removed lines, 0 for a binary file.
    pub deletions: u32,
    /// Whether git considers the file binary.
    pub binary: bool,
}

/// The detail of one commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitDetail {
    /// The full id.
    pub oid: String,
    /// The full ids of the parents.
    pub parents: Vec<String>,
    /// The author's name.
    pub author_name: String,
    /// The author's email.
    pub author_email: String,
    /// The author date, in unix seconds.
    pub date: i64,
    /// The whole message, without trailing whitespace.
    pub message: String,
    /// The changed files.
    pub files: Vec<CommitFile>,
}

/// Separates the fields of a record (ASCII unit separator).
const SEP: char = '\u{1f}';

/// The pretty format of [`Repo::log`]; `%D` lists the decorations.
const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%s%x1f%D";

impl Repo {
    /// One page of the log, newest first, in `--topo-order` (a commit never comes before its
    /// children, which is what the graph layout needs). The same query with growing `skip`
    /// continues where the previous page ended.
    ///
    /// `text` matches the message (ignoring case, as plain text, not a regular expression).
    /// When it also looks like a hash prefix (4 to 40 hex digits) and names a commit that
    /// passes the author, path and branch filters, that commit comes first, once, and counts as
    /// one row of the paging.
    pub fn log(&self, query: &LogQuery) -> Result<Vec<LogCommit>, GitError> {
        if query.limit == 0 || !self.has_head_or_refs()? {
            return Ok(Vec::new());
        }
        let text = query
            .text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty());
        let by_hash = match text {
            Some(text) if looks_like_hash(text) => self.hash_match(query, text)?,
            _ => None,
        };
        let (skip, limit) = match &by_hash {
            // The hash match is row 0, the message matches follow.
            Some(_) if query.skip == 0 => (0, query.limit - 1),
            Some(_) => (query.skip - 1, query.limit),
            None => (query.skip, query.limit),
        };
        let mut commits = Vec::new();
        if let Some(hit) = &by_hash
            && query.skip == 0
        {
            commits.push(hit.clone());
        }
        if limit > 0 {
            for commit in self.run_log(query, text, skip, limit)? {
                if by_hash.as_ref().is_some_and(|hit| hit.oid == commit.oid) {
                    continue;
                }
                commits.push(commit);
            }
        }
        Ok(commits)
    }

    /// The detail of the commit `oid` (any revision git accepts). Its files are compared with
    /// the first parent, also for a merge commit, so a merge lists what it brought in; a root
    /// commit is compared with the empty tree. Renames are detected.
    pub fn commit_detail(&self, oid: &str) -> Result<CommitDetail, GitError> {
        check_revision(oid)?;
        let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%B";
        let output = self.run(&["log", "-1", "--no-walk", format, oid, "--"])?;
        let text = String::from_utf8_lossy(&output);
        let mut fields = text.splitn(6, SEP);
        let (Some(full), Some(parents), Some(name), Some(email), Some(date), Some(message)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            return Err(GitError::Parse(format!("bad commit record for {oid}")));
        };
        let parents: Vec<String> = parents.split_whitespace().map(str::to_string).collect();
        let full = full.trim().to_string();
        let base = match parents.first() {
            Some(parent) => parent.clone(),
            None => self.empty_tree()?,
        };
        let files = self.changed_files(&base, &full)?;
        Ok(CommitDetail {
            oid: full,
            parents,
            author_name: name.to_string(),
            author_email: email.to_string(),
            date: date.parse().unwrap_or(0),
            message: message.trim_end().to_string(),
            files,
        })
    }

    /// The files that differ between the trees `base` and `oid`, with their line counts.
    fn changed_files(&self, base: &str, oid: &str) -> Result<Vec<CommitFile>, GitError> {
        let statuses = self.run(&[
            "diff",
            "--name-status",
            "-z",
            "-M",
            "--no-ext-diff",
            base,
            oid,
            "--",
        ])?;
        let counts = self.run(&[
            "diff",
            "--numstat",
            "-z",
            "-M",
            "--no-ext-diff",
            base,
            oid,
            "--",
        ])?;
        let counts = parse_numstat(&counts)?;
        let mut files = Vec::new();
        for (letter, path, orig_path) in parse_name_status(&statuses)? {
            let stat = counts.iter().find(|s| s.path == path);
            files.push(CommitFile {
                path,
                orig_path,
                letter,
                additions: stat.map_or(0, |s| s.additions),
                deletions: stat.map_or(0, |s| s.deletions),
                binary: stat.is_some_and(|s| s.binary),
            });
        }
        Ok(files)
    }

    /// Whether there is anything to log: a commit at `HEAD`, or any ref.
    fn has_head_or_refs(&self) -> Result<bool, GitError> {
        if self.has_head()? {
            return Ok(true);
        }
        let output = self.run(&["for-each-ref", "--count=1", "--format=%(refname)"])?;
        Ok(!output.is_empty())
    }

    /// Runs `git log` for `query` with an explicit window.
    fn run_log(
        &self,
        query: &LogQuery,
        text: Option<&str>,
        skip: usize,
        limit: usize,
    ) -> Result<Vec<LogCommit>, GitError> {
        let skip = format!("--skip={skip}");
        let limit = format!("--max-count={limit}");
        let mut args: Vec<String> = vec![
            "log".into(),
            "--topo-order".into(),
            "-z".into(),
            "--decorate=full".into(),
            LOG_FORMAT.into(),
            skip,
            limit,
        ];
        self.filter_args(query, text, &mut args);
        match &query.branch {
            Some(branch) => {
                check_revision(branch)?;
                args.push("--end-of-options".into());
                args.push(branch.clone());
            }
            None => {
                args.extend(["--branches", "--remotes", "--tags"].map(String::from));
                // An unborn HEAD cannot be named, the other refs can still exist.
                if self.has_head()? {
                    args.push("HEAD".into());
                }
            }
        }
        args.push("--".into());
        if let Some(path) = query.path.as_deref().filter(|p| !p.is_empty()) {
            args.push(path.trim_end_matches('/').to_string());
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        parse_log(&self.run(&args)?)
    }

    /// The options for the author and message filters. `-F` makes both plain text.
    fn filter_args(&self, query: &LogQuery, text: Option<&str>, args: &mut Vec<String>) {
        let author = query.author.as_deref().filter(|a| !a.is_empty());
        if author.is_some() || text.is_some() {
            args.push("--regexp-ignore-case".into());
            args.push("--fixed-strings".into());
        }
        if let Some(author) = author {
            args.push(format!("--author={author}"));
        }
        if let Some(text) = text {
            args.push(format!("--grep={text}"));
        }
    }

    /// The commit whose id starts with `hash`, when it exists and passes the branch, author and
    /// path filters of `query`.
    fn hash_match(&self, query: &LogQuery, hash: &str) -> Result<Option<LogCommit>, GitError> {
        let spec = format!("{hash}^{{commit}}");
        let full = match self.run(&["rev-parse", "--verify", "--quiet", &spec]) {
            Ok(output) => String::from_utf8_lossy(&output).trim().to_string(),
            Err(GitError::Failed { .. }) => return Ok(None),
            Err(err) => return Err(err),
        };
        // `log -1 <commit>` with the filters prints that commit only when it passes them (for a
        // path, git would print the last commit that touched it instead, so compare ids).
        let mut args: Vec<String> = vec![
            "log".into(),
            "-1".into(),
            "-z".into(),
            "--decorate=full".into(),
            LOG_FORMAT.into(),
        ];
        self.filter_args(query, None, &mut args);
        args.push("--end-of-options".into());
        args.push(full.clone());
        args.push("--".into());
        if let Some(path) = query.path.as_deref().filter(|p| !p.is_empty()) {
            args.push(path.trim_end_matches('/').to_string());
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let Some(commit) = parse_log(&self.run(&refs)?)?
            .into_iter()
            .find(|c| c.oid == full)
        else {
            return Ok(None);
        };
        if let Some(branch) = &query.branch {
            check_revision(branch)?;
            let reachable = self.run(&["merge-base", "--is-ancestor", &full, branch]);
            match reachable {
                Ok(_) => {}
                Err(GitError::Failed { code: Some(1), .. }) => return Ok(None),
                Err(err) => return Err(err),
            }
        }
        Ok(Some(commit))
    }
}

/// Whether `text` could be a hash prefix: 4 to 40 hex digits.
fn looks_like_hash(text: &str) -> bool {
    (4..=40).contains(&text.len()) && text.chars().all(|c| c.is_ascii_hexdigit())
}

/// Parses the NUL separated records printed with [`LOG_FORMAT`].
fn parse_log(output: &[u8]) -> Result<Vec<LogCommit>, GitError> {
    let text = String::from_utf8_lossy(output);
    let mut commits = Vec::new();
    for record in text.split('\0') {
        let record = record.trim_start_matches('\n');
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.split(SEP).collect();
        let [oid, short, parents, name, email, date, subject, decorations] = fields[..] else {
            return Err(GitError::Parse(format!("bad log record {record:?}")));
        };
        commits.push(LogCommit {
            oid: oid.to_string(),
            short: short.to_string(),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            author_name: name.to_string(),
            author_email: email.to_string(),
            date: date.parse().unwrap_or(0),
            subject: subject.to_string(),
            refs: parse_decorations(decorations),
        });
    }
    Ok(commits)
}

/// Parses `%D` printed with `--decorate=full`: `HEAD -> refs/heads/x, refs/heads/y,
/// refs/remotes/origin/y, tag: refs/tags/v1`. Remote `HEAD` symrefs and other refs (the stash)
/// are skipped.
fn parse_decorations(decorations: &str) -> Vec<RefLabel> {
    let mut labels = Vec::new();
    for item in decorations.split(", ") {
        let item = item.trim();
        let label = if item == "HEAD" {
            Some((RefLabelKind::Head, "HEAD"))
        } else if let Some(name) = item
            .strip_prefix("HEAD -> refs/heads/")
            .or_else(|| item.strip_prefix("HEAD -> "))
        {
            Some((RefLabelKind::Head, name))
        } else if let Some(name) = item.strip_prefix("tag: refs/tags/") {
            Some((RefLabelKind::Tag, name))
        } else if let Some(name) = item.strip_prefix("refs/heads/") {
            Some((RefLabelKind::Local, name))
        } else if let Some(name) = item.strip_prefix("refs/remotes/") {
            (!name.ends_with("/HEAD")).then_some((RefLabelKind::Remote, name))
        } else {
            None
        };
        if let Some((kind, name)) = label {
            labels.push(RefLabel {
                kind,
                name: name.to_string(),
            });
        }
    }
    labels
}

/// Parses `git diff --name-status -z -M`: `M NUL path NUL`, and `R100 NUL old NUL new NUL`.
/// Returns `(letter, path, orig_path)`.
fn parse_name_status(output: &[u8]) -> Result<Vec<(char, String, Option<String>)>, GitError> {
    let mut fields = output
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        let Some(letter) = status.chars().next() else {
            continue;
        };
        let missing = || GitError::Parse("name-status record without a path".to_string());
        if matches!(letter, 'R' | 'C') {
            let old = fields.next().ok_or_else(missing)?;
            let new = fields.next().ok_or_else(missing)?;
            files.push((letter, new, Some(old)));
        } else {
            files.push((letter, fields.next().ok_or_else(missing)?, None));
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorations_with_full_names() {
        let labels = parse_decorations(
            "HEAD -> refs/heads/main, tag: refs/tags/v1, refs/remotes/origin/main, \
             refs/remotes/origin/HEAD, refs/heads/feature/x, refs/stash",
        );
        let expected = [
            (RefLabelKind::Head, "main"),
            (RefLabelKind::Tag, "v1"),
            (RefLabelKind::Remote, "origin/main"),
            (RefLabelKind::Local, "feature/x"),
        ];
        assert_eq!(labels.len(), expected.len());
        for (label, (kind, name)) in labels.iter().zip(expected) {
            assert_eq!((label.kind, label.name.as_str()), (kind, name));
        }
        assert_eq!(parse_decorations(""), Vec::new());
        assert_eq!(
            parse_decorations("HEAD"),
            vec![RefLabel {
                kind: RefLabelKind::Head,
                name: "HEAD".into()
            }]
        );
    }

    #[test]
    fn parses_log_records() {
        let record = |oid: &str, parents: &str| {
            format!(
                "{oid}\u{1f}abc\u{1f}{parents}\u{1f}Ann\u{1f}a@x\u{1f}12\u{1f}Subject\u{1f}HEAD -> refs/heads/main\0"
            )
        };
        // With -z git separates the records by NUL, and may add a newline before the next one.
        let output = format!("{}\n{}", record("1", "2 3"), record("2", ""));
        let commits = parse_log(output.as_bytes()).unwrap_or_default();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents, vec!["2".to_string(), "3".to_string()]);
        assert!(commits[1].parents.is_empty());
        assert_eq!(commits[0].date, 12);
        assert!(parse_log(b"too\x1ffew\0").is_err());
    }

    #[test]
    fn parses_name_status() {
        let files = parse_name_status(b"M\0a.txt\0R100\0old.txt\0new.txt\0D\0gone.txt\0")
            .unwrap_or_default();
        assert_eq!(
            files,
            vec![
                ('M', "a.txt".to_string(), None),
                ('R', "new.txt".to_string(), Some("old.txt".to_string())),
                ('D', "gone.txt".to_string(), None),
            ]
        );
    }

    #[test]
    fn recognizes_hash_prefixes() {
        assert!(looks_like_hash("abcd"));
        assert!(looks_like_hash("0123456789abcdef0123456789abcdef01234567"));
        assert!(!looks_like_hash("abc"));
        assert!(!looks_like_hash("fix bug"));
        assert!(!looks_like_hash("abcdefg"));
    }
}
