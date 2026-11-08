//! What the diff tab paints (design 7a), computed once per patch on the background executor: the
//! whole file's lines from `telaio_git::parse_file_diff`, each one cut into runs that carry a
//! syntax kind (from `brasa_format::highlight`, on each side's full text) and whether the run
//! is a changed word. Render only reads it, so scrolling a big file costs nothing per line.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::SharedString;
use brasa_format::highlight::{Language, Token, TokenKind, highlight, language_for_path};
use telaio_git::{FileDiff, LineKind, parse_file_diff};

/// The longest part of a line that is shown (bytes). The row cuts it with an ellipsis long before
/// that, and a minified file of one huge line would otherwise be copied on every frame.
const MAX_SHOWN_LINE: usize = 2000;

/// One run of a line: its byte range, its syntax kind and whether it is a changed word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Byte range inside the line.
    pub range: Range<usize>,
    /// The syntax kind, `None` for plain text.
    pub kind: Option<TokenKind>,
    /// Whether the run is part of a word that changed.
    pub emphasis: bool,
}

/// A parsed and highlighted file diff.
#[derive(Debug, Default)]
pub struct DiffModel {
    /// The rows, lines and change blocks.
    pub diff: FileDiff,
    /// The shown text of every line of [`FileDiff::lines`], same order, cut at
    /// [`MAX_SHOWN_LINE`].
    pub texts: Vec<SharedString>,
    /// The runs of every line of [`FileDiff::lines`] (of its shown text), same order.
    pub runs: Vec<Vec<Run>>,
}

impl DiffModel {
    /// Parses the full context `patch` of `path` and highlights both of its sides.
    pub fn build(patch: &str, path: &str) -> Arc<DiffModel> {
        let diff = parse_file_diff(patch);
        let texts: Vec<SharedString> = diff
            .lines
            .iter()
            .map(|line| SharedString::from(shown(&line.text).to_string()))
            .collect();
        let runs = line_runs(&diff, &texts, language_for_path(path));
        Arc::new(DiffModel { diff, texts, runs })
    }
}

/// The start of `text` that is shown, cut on a char boundary.
fn shown(text: &str) -> &str {
    if text.len() <= MAX_SHOWN_LINE {
        return text;
    }
    let mut end = MAX_SHOWN_LINE;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Highlights the old side (context and removed lines) and the new side (context and added
/// lines) as whole files, so a line knows the section it is in, and cuts every line into runs.
fn line_runs(diff: &FileDiff, texts: &[SharedString], language: Language) -> Vec<Vec<Run>> {
    let side = |keep: &dyn Fn(LineKind) -> bool| -> Vec<Vec<Token>> {
        if matches!(language, Language::Plain) {
            return Vec::new();
        }
        let text: Vec<&str> = diff
            .lines
            .iter()
            .filter(|line| keep(line.kind))
            .map(|line| line.text.as_str())
            .collect();
        highlight(&text.join("\n"), language)
    };
    let mut old_tokens = side(&|kind| kind != LineKind::Added).into_iter();
    let mut new_tokens = side(&|kind| kind != LineKind::Removed).into_iter();
    diff.lines
        .iter()
        .zip(texts)
        .map(|(line, text)| {
            let tokens = match line.kind {
                LineKind::Removed => old_tokens.next(),
                LineKind::Added => new_tokens.next(),
                LineKind::Context => {
                    old_tokens.next();
                    new_tokens.next()
                }
            }
            .unwrap_or_default();
            runs(text.len(), &tokens, &line.emphasis)
        })
        .collect()
}

/// Cuts a line of `len` bytes into runs where neither the syntax kind nor the emphasis changes.
/// `tokens` and `emphasis` are sorted and do not overlap among themselves; what lies past `len`
/// is ignored.
pub fn runs(len: usize, tokens: &[Token], emphasis: &[Range<usize>]) -> Vec<Run> {
    let mut cuts: Vec<usize> = vec![0, len];
    for token in tokens {
        cuts.extend([token.range.start.min(len), token.range.end.min(len)]);
    }
    for range in emphasis {
        cuts.extend([range.start.min(len), range.end.min(len)]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    // Every token and emphasis edge is a cut, so a run is either inside one or outside all.
    let (mut token, mut changed) = (0, 0);
    let mut out: Vec<Run> = Vec::new();
    for pair in cuts.windows(2) {
        let range = pair[0]..pair[1];
        while token < tokens.len() && tokens[token].range.end <= range.start {
            token += 1;
        }
        while changed < emphasis.len() && emphasis[changed].end <= range.start {
            changed += 1;
        }
        let kind = tokens
            .get(token)
            .filter(|token| token.range.start <= range.start)
            .map(|token| token.kind);
        let emphasis = emphasis
            .get(changed)
            .is_some_and(|changed| changed.start <= range.start);
        match out.last_mut() {
            Some(last) if last.kind == kind && last.emphasis == emphasis => {
                last.range.end = range.end
            }
            _ => out.push(Run {
                range,
                kind,
                emphasis,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn splits_at_token_and_emphasis_edges_and_merges_equal_neighbours() {
        let tokens = vec![
            Token {
                range: 0..4,
                kind: TokenKind::Method,
            },
            Token {
                range: 5..12,
                kind: TokenKind::Url,
            },
        ];
        let runs = runs(12, &tokens, &[Range { start: 8, end: 10 }]);
        let run = |range: Range<usize>, kind, emphasis| Run {
            range,
            kind,
            emphasis,
        };
        assert_eq!(
            runs,
            vec![
                run(0..4, Some(TokenKind::Method), false),
                run(4..5, None, false),
                run(5..8, Some(TokenKind::Url), false),
                run(8..10, Some(TokenKind::Url), true),
                run(10..12, Some(TokenKind::Url), false),
            ]
        );
    }

    #[test]
    fn an_empty_line_has_no_runs() {
        assert!(runs(0, &[], &[]).is_empty());
    }

    #[test]
    fn highlights_each_side_in_its_own_section() {
        // The added line is in the json body on the new side only.
        let patch = "diff --git a/r.brasa b/r.brasa\n--- a/r.brasa\n+++ b/r.brasa\n\
                     @@ -1,3 +1,4 @@\n GET x\n \n-::: docs\n+::: body json\n+{\"a\": 1}\n";
        let model = DiffModel::build(patch, "r.brasa");
        let kinds = |line: usize| -> Vec<Option<TokenKind>> {
            model.runs[line].iter().map(|run| run.kind).collect()
        };
        assert_eq!(model.texts[4], "{\"a\": 1}");
        assert!(kinds(4).contains(&Some(TokenKind::Key)));
        assert!(kinds(4).contains(&Some(TokenKind::Number)));
        // The changed word splits it in two runs, both of the section.
        assert!(
            kinds(2)
                .iter()
                .all(|kind| *kind == Some(TokenKind::Section))
        );
    }

    #[test]
    fn long_lines_are_cut_on_a_char_boundary() {
        let text = "é".repeat(MAX_SHOWN_LINE);
        let cut = shown(&text);
        assert!(cut.len() <= MAX_SHOWN_LINE);
        assert!(text.is_char_boundary(cut.len()));
    }
}
