//! A file's diff as rows for a renderer: every line of the file, side by side or unified, with
//! the changes grouped in blocks and the changed words marked.
//!
//! [`parse_file_diff`] reads the output of [`crate::Repo::diff_full`] (any `git diff` output
//! works, only the lines of its hunks are shown). [`block_patch`] cuts one change block out as a
//! patch that [`crate::Repo::apply`] stages, unstages or reverts alone.

use std::ops::Range;

/// What a line of the diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A line present on both sides, unchanged.
    Context,
    /// A line only in the old version.
    Removed,
    /// A line only in the new version.
    Added,
}

/// One line of the diff, in unified order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// Whether the line is unchanged, removed or added.
    pub kind: LineKind,
    /// The 1-based line number in the old file, `None` for an added line.
    pub old_number: Option<u32>,
    /// The 1-based line number in the new file, `None` for a removed line.
    pub new_number: Option<u32>,
    /// The text without the leading `' '`, `'-'` or `'+'` and without the newline.
    pub text: String,
    /// Byte ranges of `text` with the words that changed. Only on a removed line paired with an
    /// added one in the same block (and the other way round), empty when the whole line changed.
    pub emphasis: Vec<Range<usize>>,
    /// The index in [`FileDiff::blocks`] for a removed or added line, `None` for context.
    pub block: Option<usize>,
    /// Whether the patch marks the line with `\ No newline at end of file`.
    pub no_newline: bool,
}

/// One row of the side by side view. Both are indices in [`FileDiff::lines`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitRow {
    /// The old side, `None` is an empty filler cell.
    pub left: Option<usize>,
    /// The new side, `None` is an empty filler cell.
    pub right: Option<usize>,
}

/// A contiguous run of removed and/or added lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeBlock {
    /// The index in [`FileDiff::lines`] of its first line.
    pub first_line: usize,
    /// How many removed lines it has.
    pub removed: usize,
    /// How many added lines it has.
    pub added: usize,
    /// The index in [`FileDiff::split`] of its first row.
    pub first_split_row: usize,
    /// The old line number where it starts: its first removed line, or for a pure addition the
    /// old line that follows the insertion point.
    pub old_start: u32,
    /// The new line number where it starts, like `old_start` for the new file.
    pub new_start: u32,
}

/// The rows of one file's diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileDiff {
    /// Every line, in the order of the patch (inside a block the removed lines come first).
    pub lines: Vec<DiffLine>,
    /// The same lines paired for the side by side view.
    pub split: Vec<SplitRow>,
    /// The change blocks, in order.
    pub blocks: Vec<ChangeBlock>,
    /// Whether git reported a binary file, which has no lines.
    pub binary: bool,
}

/// Context lines around a block in [`block_patch`], like git's default.
const CONTEXT: usize = 3;
/// Longest line (bytes) that gets word emphasis.
const MAX_EMPHASIS_LINE: usize = 1000;
/// Above this share of the line changed, the emphasis would mark everything and is skipped.
const MAX_CHANGED_SHARE: f64 = 0.7;

const NO_EOL: &str = "\\ No newline at end of file\n";

/// Reads the `git diff` output of one file. An empty patch gives no lines, a binary file sets
/// `binary`. Never fails: what is not understood is skipped.
pub fn parse_file_diff(patch: &str) -> FileDiff {
    let mut diff = FileDiff::default();
    let mut in_hunk = false;
    let (mut old_next, mut new_next) = (1u32, 1u32);
    for raw in patch.split_terminator('\n') {
        if raw.starts_with("@@ ") {
            in_hunk = true;
            if let Some((old, new)) = parse_hunk_header(raw) {
                (old_next, new_next) = (old, new);
            }
            continue;
        }
        if !in_hunk {
            if raw.starts_with("Binary files ") || raw.starts_with("GIT binary patch") {
                diff.binary = true;
            }
            continue;
        }
        let (kind, text) = match raw.as_bytes().first() {
            Some(b'-') => (LineKind::Removed, &raw[1..]),
            Some(b'+') => (LineKind::Added, &raw[1..]),
            Some(b' ') => (LineKind::Context, &raw[1..]),
            Some(b'\\') => {
                if let Some(last) = diff.lines.last_mut() {
                    last.no_newline = true;
                }
                continue;
            }
            // Some tools strip the space of an empty context line.
            None => (LineKind::Context, ""),
            Some(_) => continue,
        };
        let old_number = (kind != LineKind::Added).then_some(old_next);
        let new_number = (kind != LineKind::Removed).then_some(new_next);
        if old_number.is_some() {
            old_next += 1;
        }
        if new_number.is_some() {
            new_next += 1;
        }
        diff.lines.push(DiffLine {
            kind,
            old_number,
            new_number,
            text: text.to_string(),
            emphasis: Vec::new(),
            block: None,
            no_newline: false,
        });
    }
    build_blocks(&mut diff);
    diff
}

/// The numbers of the first old and new lines of a hunk, from `@@ -a,b +c,d @@`. A zero count
/// means the number names the line before the hunk.
fn parse_hunk_header(header: &str) -> Option<(u32, u32)> {
    let mut parts = header.split_whitespace().skip(1);
    let old = parse_range(parts.next()?.strip_prefix('-')?)?;
    let new = parse_range(parts.next()?.strip_prefix('+')?)?;
    Some((old, new))
}

fn parse_range(range: &str) -> Option<u32> {
    let (start, count) = match range.split_once(',') {
        Some((start, count)) => (start, count.parse::<u32>().ok()?),
        None => (range, 1),
    };
    let start = start.parse::<u32>().ok()?;
    Some(if count == 0 { start + 1 } else { start })
}

/// Fills `block`, `blocks`, `split` and the emphasis of `diff.lines`.
fn build_blocks(diff: &mut FileDiff) {
    let mut i = 0;
    while i < diff.lines.len() {
        if diff.lines[i].kind == LineKind::Context {
            diff.split.push(SplitRow {
                left: Some(i),
                right: Some(i),
            });
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while end < diff.lines.len() && diff.lines[end].kind != LineKind::Context {
            end += 1;
        }
        let index = diff.blocks.len();
        let removed: Vec<usize> = (start..end)
            .filter(|n| diff.lines[*n].kind == LineKind::Removed)
            .collect();
        let added: Vec<usize> = (start..end)
            .filter(|n| diff.lines[*n].kind == LineKind::Added)
            .collect();
        let first_split_row = diff.split.len();
        for row in 0..removed.len().max(added.len()) {
            diff.split.push(SplitRow {
                left: removed.get(row).copied(),
                right: added.get(row).copied(),
            });
        }
        for (old, new) in removed.iter().zip(&added) {
            emphasize(diff, *old, *new);
        }
        for n in start..end {
            diff.lines[n].block = Some(index);
        }
        let before = start.checked_sub(1).map(|n| &diff.lines[n]);
        let following = |old: bool| {
            // The first number of the side that the block begins at.
            let own = diff.lines[start..end]
                .iter()
                .find_map(|l| if old { l.old_number } else { l.new_number });
            own.unwrap_or_else(|| {
                let after = diff.lines.get(end);
                let after = after.and_then(|l| if old { l.old_number } else { l.new_number });
                let previous = before.and_then(|l| if old { l.old_number } else { l.new_number });
                after.unwrap_or_else(|| previous.map_or(1, |n| n + 1))
            })
        };
        let (old_start, new_start) = (following(true), following(false));
        diff.blocks.push(ChangeBlock {
            first_line: start,
            removed: removed.len(),
            added: added.len(),
            first_split_row,
            old_start,
            new_start,
        });
        i = end;
    }
}

/// Marks the words that differ between a removed and an added line.
fn emphasize(diff: &mut FileDiff, old: usize, new: usize) {
    let (old_text, new_text) = (&diff.lines[old].text, &diff.lines[new].text);
    if old_text.len() > MAX_EMPHASIS_LINE || new_text.len() > MAX_EMPHASIS_LINE {
        return;
    }
    let (old_tokens, new_tokens) = (tokenize(old_text), tokenize(new_text));
    let (old_changed, new_changed) = changed_tokens(old_text, &old_tokens, new_text, &new_tokens);
    let share = |changed: &[Range<usize>], len: usize| {
        let bytes: usize = changed.iter().map(|r| r.len()).sum();
        bytes as f64 / len.max(1) as f64
    };
    if share(&old_changed, old_text.len()) > MAX_CHANGED_SHARE
        || share(&new_changed, new_text.len()) > MAX_CHANGED_SHARE
    {
        return;
    }
    diff.lines[old].emphasis = old_changed;
    diff.lines[new].emphasis = new_changed;
}

/// Splits a line in words (alphanumeric and `_` runs), whitespace runs and single punctuation
/// characters, as byte ranges.
fn tokenize(text: &str) -> Vec<Range<usize>> {
    #[derive(PartialEq, Clone, Copy)]
    enum Class {
        Word,
        Space,
        Other,
    }
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    };
    let mut tokens: Vec<Range<usize>> = Vec::new();
    let mut last: Option<Class> = None;
    for (at, c) in text.char_indices() {
        let current = class(c);
        let joins = last == Some(current) && current != Class::Other;
        match tokens.last_mut() {
            Some(token) if joins => token.end = at + c.len_utf8(),
            _ => tokens.push(at..at + c.len_utf8()),
        }
        last = Some(current);
    }
    tokens
}

/// The merged byte ranges of each side's tokens that are outside the longest common
/// subsequence of the two token lists.
fn changed_tokens(
    old_text: &str,
    old: &[Range<usize>],
    new_text: &str,
    new: &[Range<usize>],
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let (n, m) = (old.len(), new.len());
    // table[i][j]: the LCS length of old[i..] and new[j..].
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if old_text[old[i].clone()] == new_text[new[j].clone()] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut old_changed, mut new_changed) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && old_text[old[i].clone()] == new_text[new[j].clone()] {
            i += 1;
            j += 1;
        } else if j >= m || (i < n && table[i + 1][j] >= table[i][j + 1]) {
            push_merged(&mut old_changed, old[i].clone());
            i += 1;
        } else {
            push_merged(&mut new_changed, new[j].clone());
            j += 1;
        }
    }
    (old_changed, new_changed)
}

fn push_merged(ranges: &mut Vec<Range<usize>>, range: Range<usize>) {
    match ranges.last_mut() {
        Some(last) if last.end == range.start => last.end = range.end,
        _ => ranges.push(range),
    }
}

/// A patch with the file header of `file_patch` (everything before the first `@@`) and one hunk
/// with exactly the change block `block` of `diff` and up to three lines of context on each
/// side, ready for [`crate::Repo::apply`]. `diff` must come from `file_patch`. `None` for a
/// binary file or a block that does not exist.
pub fn block_patch(file_patch: &str, diff: &FileDiff, block: usize) -> Option<String> {
    if diff.binary {
        return None;
    }
    let change = diff.blocks.get(block)?;
    let first = change.first_line;
    let last = first + change.removed + change.added;
    let mut start = first;
    while start > 0
        && first - start < CONTEXT
        && diff.lines[start - 1].kind == LineKind::Context
        && adjacent(&diff.lines[start - 1], &diff.lines[start])
    {
        start -= 1;
    }
    let mut end = last;
    while end < diff.lines.len()
        && end - last < CONTEXT
        && diff.lines[end].kind == LineKind::Context
        && adjacent(&diff.lines[end - 1], &diff.lines[end])
    {
        end += 1;
    }
    let lines = &diff.lines[start..end];
    let old_count = lines.iter().filter(|l| l.kind != LineKind::Added).count();
    let new_count = lines.iter().filter(|l| l.kind != LineKind::Removed).count();
    let leading = first - start;
    // Without lines on a side the number names the line before the hunk.
    let old_first = change.old_start.saturating_sub(leading as u32);
    let new_first = change.new_start.saturating_sub(leading as u32);
    let old_header = if old_count == 0 {
        old_first.saturating_sub(1)
    } else {
        old_first
    };
    let new_header = if new_count == 0 {
        new_first.saturating_sub(1)
    } else {
        new_first
    };

    let mut out = String::new();
    for line in file_patch.split_inclusive('\n') {
        if line.starts_with("@@ ") {
            break;
        }
        out.push_str(line);
    }
    out.push_str(&format!(
        "@@ -{old_header},{old_count} +{new_header},{new_count} @@\n"
    ));
    for line in lines {
        out.push(match line.kind {
            LineKind::Context => ' ',
            LineKind::Removed => '-',
            LineKind::Added => '+',
        });
        out.push_str(&line.text);
        out.push('\n');
        if line.no_newline {
            out.push_str(NO_EOL);
        }
    }
    Some(out)
}

/// Whether `b` directly follows `a` in the file, so both can share a hunk.
fn adjacent(a: &DiffLine, b: &DiffLine) -> bool {
    match (a.old_number, b.old_number, a.new_number, b.new_number) {
        (Some(x), Some(y), _, _) => x + 1 == y,
        (_, _, Some(x), Some(y)) => x + 1 == y,
        _ => true,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const HEADER: &str = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n";

    fn patch(body: &str) -> String {
        format!("{HEADER}{body}")
    }

    fn emphasized(line: &DiffLine) -> Vec<&str> {
        line.emphasis
            .iter()
            .map(|r| &line.text[r.clone()])
            .collect()
    }

    #[test]
    fn parses_a_modified_file_with_numbers_and_blocks() {
        let p = patch("@@ -1,4 +1,5 @@\n a\n-b\n+B\n+B2\n c\n d\n");
        let diff = parse_file_diff(&p);
        assert!(!diff.binary);
        assert_eq!(diff.lines.len(), 6);
        let numbers: Vec<_> = diff
            .lines
            .iter()
            .map(|l| (l.old_number, l.new_number))
            .collect();
        assert_eq!(
            numbers,
            vec![
                (Some(1), Some(1)),
                (Some(2), None),
                (None, Some(2)),
                (None, Some(3)),
                (Some(3), Some(4)),
                (Some(4), Some(5)),
            ]
        );
        assert_eq!(diff.lines[1].text, "b");
        assert_eq!(diff.lines[0].block, None);
        assert_eq!(diff.lines[2].block, Some(0));
        assert_eq!(
            diff.blocks,
            vec![ChangeBlock {
                first_line: 1,
                removed: 1,
                added: 2,
                first_split_row: 1,
                old_start: 2,
                new_start: 2,
            }]
        );
    }

    #[test]
    fn parses_new_and_deleted_files() {
        let new = parse_file_diff(
            "diff --git a/f b/f\nnew file mode 100644\n--- /dev/null\n+++ b/f\n@@ -0,0 +1,2 @@\n+x\n+y\n",
        );
        assert_eq!(new.lines.len(), 2);
        assert_eq!(new.lines[0].new_number, Some(1));
        assert_eq!(new.blocks.len(), 1);
        assert_eq!((new.blocks[0].old_start, new.blocks[0].new_start), (1, 1));
        assert_eq!(new.split.len(), 2);
        assert!(
            new.split
                .iter()
                .all(|r| r.left.is_none() && r.right.is_some())
        );

        let deleted = parse_file_diff(
            "diff --git a/f b/f\ndeleted file mode 100644\n--- a/f\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-x\n-y\n",
        );
        assert_eq!(deleted.lines.len(), 2);
        assert_eq!(deleted.lines[1].old_number, Some(2));
        assert!(
            deleted
                .split
                .iter()
                .all(|r| r.left.is_some() && r.right.is_none())
        );
    }

    #[test]
    fn tolerates_missing_newline_markers_empty_and_binary() {
        let p = patch(
            "@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n",
        );
        let diff = parse_file_diff(&p);
        assert_eq!(diff.lines.len(), 3);
        assert_eq!(diff.lines[2].text, "c");

        let empty = parse_file_diff("");
        assert!(empty.lines.is_empty() && empty.blocks.is_empty() && !empty.binary);

        let binary =
            parse_file_diff("diff --git a/i b/i\nindex 1..2\nBinary files a/i and b/i differ\n");
        assert!(binary.binary && binary.lines.is_empty());
        assert_eq!(block_patch("x", &binary, 0), None);
    }

    #[test]
    fn pairs_rows_and_leaves_fillers() {
        let p = patch("@@ -1,5 +1,5 @@\n a\n-b1\n-b2\n-b3\n+B1\n+B2\n+B3\n+B4\n z\n");
        let diff = parse_file_diff(&p);
        let rows: Vec<_> = diff.split.iter().map(|r| (r.left, r.right)).collect();
        assert_eq!(
            rows,
            vec![
                (Some(0), Some(0)),
                (Some(1), Some(4)),
                (Some(2), Some(5)),
                (Some(3), Some(6)),
                (None, Some(7)),
                (Some(8), Some(8)),
            ]
        );
        assert_eq!(diff.blocks[0].first_split_row, 1);
        assert_eq!((diff.blocks[0].removed, diff.blocks[0].added), (3, 4));
    }

    #[test]
    fn emphasizes_changed_words_only_when_it_helps() {
        let p = patch(
            "@@ -1,3 +1,3 @@\n-let a = foo(1);\n+let a = bar(1);\n k\n-one two\n+one two three\n-abc def\n+xyz uvw\n",
        );
        let diff = parse_file_diff(&p);
        assert_eq!(emphasized(&diff.lines[0]), vec!["foo"]);
        assert_eq!(emphasized(&diff.lines[1]), vec!["bar"]);
        // An appended word: nothing removed, the new text (space + word) on the added side.
        assert!(diff.lines[3].emphasis.is_empty());
        assert_eq!(emphasized(&diff.lines[4]), vec![" three"]);
        // Nothing in common: the line color is enough.
        assert!(diff.lines[5].emphasis.is_empty());
        assert!(diff.lines[6].emphasis.is_empty());
        // Context never has emphasis.
        assert!(diff.lines[2].emphasis.is_empty());
    }

    #[test]
    fn numbers_blocks_in_order() {
        let p = patch("@@ -1,8 +1,8 @@\n-a\n+A\n b\n c\n d\n e\n f\n-g\n+G\n h\n");
        let diff = parse_file_diff(&p);
        assert_eq!(diff.blocks.len(), 2);
        assert_eq!(diff.blocks[0].first_line, 0);
        assert_eq!(diff.blocks[1].first_line, 7);
        assert_eq!(diff.lines[7].block, Some(1));
        assert_eq!((diff.blocks[1].old_start, diff.blocks[1].new_start), (7, 7));
    }

    #[test]
    fn block_patch_computes_headers() {
        let p = patch("@@ -1,8 +1,8 @@\n-a\n+A\n b\n c\n d\n e\n f\n-g\n+G\n h\n");
        let diff = parse_file_diff(&p);
        let first = block_patch(&p, &diff, 0).unwrap();
        assert_eq!(
            first,
            format!("{HEADER}@@ -1,4 +1,4 @@\n-a\n+A\n b\n c\n d\n")
        );
        let second = block_patch(&p, &diff, 1).unwrap();
        assert_eq!(
            second,
            format!("{HEADER}@@ -4,5 +4,5 @@\n d\n e\n f\n-g\n+G\n h\n")
        );
        assert_eq!(block_patch(&p, &diff, 2), None);
    }

    #[test]
    fn block_patch_handles_empty_sides_and_missing_newline() {
        let new = "diff --git a/f b/f\nnew file mode 100644\n--- /dev/null\n+++ b/f\n@@ -0,0 +1,2 @@\n+x\n+y\n";
        let diff = parse_file_diff(new);
        assert_eq!(block_patch(new, &diff, 0).unwrap(), new);

        let gone = "diff --git a/f b/f\ndeleted file mode 100644\n--- a/f\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-x\n-y\n";
        let diff = parse_file_diff(gone);
        assert_eq!(block_patch(gone, &diff, 0).unwrap(), gone);

        let insert = patch("@@ -1,2 +1,3 @@\n a\n+n\n b\n");
        let diff = parse_file_diff(&insert);
        assert_eq!(diff.blocks[0].old_start, 2);
        assert_eq!(block_patch(&insert, &diff, 0).unwrap(), insert);

        let eol = patch(
            "@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n",
        );
        let diff = parse_file_diff(&eol);
        assert_eq!(block_patch(&eol, &diff, 0).unwrap(), eol);
    }
}
