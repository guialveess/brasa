//! Deleting whole lines in the `::: pre` and `::: post` script editors (`Ctrl+D`, `Cmd+Shift+K`
//! on macOS), as a pure operation on the text. `views/request_editor.rs` applies it to the
//! editor; this module is plain Rust, unit tested without a window.

use std::ops::Range;

/// What deleting the lines of a selection does to the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDeletion {
    /// The bytes to remove.
    pub range: Range<usize>,
    /// Where the cursor goes, as a byte offset of the text once `range` is removed.
    pub cursor: usize,
}

/// Deletes every line of `text` that `selection` touches (the line of the cursor when it is
/// empty), together with one line break, so no empty line is left behind: the one after the
/// lines, or the one before them when they are the last lines. The cursor goes to the line that
/// takes their place at the column of `cursor`, or at its end when it is shorter. A selection
/// that ends at the start of a line does not touch that line. Offsets are in bytes.
pub fn delete_lines(text: &str, selection: Range<usize>, cursor: usize) -> LineDeletion {
    let line_start = |offset: usize| text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_end = |offset: usize| {
        text[offset..]
            .find('\n')
            .map_or(text.len(), |index| offset + index)
    };
    let last = if selection.end > selection.start && line_start(selection.end) == selection.end {
        selection.end - 1
    } else {
        selection.end
    };
    let start = line_start(selection.start);
    let end = line_end(last);
    let column = text[line_start(cursor)..cursor].chars().count();

    if end < text.len() {
        // The line after them moves up into their place.
        let next = &text[end + 1..line_end(end + 1)];
        LineDeletion {
            range: start..end + 1,
            cursor: start + byte_at_column(next.trim_end_matches('\r'), column),
        }
    } else if start > 0 {
        // The last lines: the line break before them goes, and the cursor moves up.
        let break_start = if text[..start - 1].ends_with('\r') {
            start - 2
        } else {
            start - 1
        };
        let previous_start = line_start(break_start);
        let previous = &text[previous_start..break_start];
        LineDeletion {
            range: break_start..end,
            cursor: previous_start + byte_at_column(previous, column),
        }
    } else {
        LineDeletion {
            range: 0..text.len(),
            cursor: 0,
        }
    }
}

/// The byte offset of the character at `column` of `line`, or its length when it is shorter.
fn byte_at_column(line: &str, column: usize) -> usize {
    line.char_indices()
        .nth(column)
        .map_or(line.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Applies [`delete_lines`] for the cursor at `cursor` (no selection) and returns the text
    /// left and the cursor.
    fn delete_at(text: &str, cursor: usize) -> (String, usize) {
        apply(text, cursor..cursor, cursor)
    }

    fn apply(text: &str, selection: Range<usize>, cursor: usize) -> (String, usize) {
        let deletion = delete_lines(text, selection, cursor);
        let mut left = text.to_string();
        left.replace_range(deletion.range, "");
        (left, deletion.cursor)
    }

    #[test]
    fn deletes_the_line_and_keeps_the_column_on_the_next_one() {
        let text = "let a = 1;\nlet bb = 2;\nlet ccc = 3;\n";
        // The cursor after `let b`, column 5.
        let (left, cursor) = delete_at(text, 16);
        assert_eq!(left, "let a = 1;\nlet ccc = 3;\n");
        assert_eq!(cursor, 11 + 5);
    }

    #[test]
    fn the_cursor_goes_to_the_end_of_a_shorter_next_line() {
        let (left, cursor) = delete_at("a long first line\nab\n", 12);
        assert_eq!(left, "ab\n");
        assert_eq!(cursor, 2);
    }

    #[test]
    fn the_last_line_takes_the_line_break_before_it() {
        let (left, cursor) = delete_at("first\nsecond\nlast", 15);
        assert_eq!(left, "first\nsecond");
        // Column 2 of "last", now on "second".
        assert_eq!(cursor, 6 + 2);
    }

    #[test]
    fn the_empty_line_after_a_final_line_break_is_the_last_line() {
        let (left, cursor) = delete_at("one\ntwo\n", 8);
        assert_eq!(left, "one\ntwo");
        assert_eq!(cursor, 4);
    }

    #[test]
    fn the_only_line_leaves_an_empty_text() {
        assert_eq!(delete_at("only line", 3), (String::new(), 0));
        assert_eq!(delete_at("", 0), (String::new(), 0));
    }

    #[test]
    fn a_selection_deletes_every_line_it_touches() {
        let text = "one\ntwo\nthree\nfour\n";
        // From the middle of "two" to the middle of "three".
        let (left, cursor) = apply(text, 5..10, 10);
        assert_eq!(left, "one\nfour\n");
        assert_eq!(cursor, 4 + 2);
    }

    #[test]
    fn a_selection_ending_at_the_start_of_a_line_leaves_that_line() {
        let text = "one\ntwo\nthree\n";
        // "two\n" selected whole, from its start to the start of "three".
        let (left, _) = apply(text, 4..8, 8);
        assert_eq!(left, "one\nthree\n");
    }

    #[test]
    fn a_selection_over_the_last_lines_takes_the_break_before_them() {
        let (left, cursor) = apply("one\ntwo\nthree", 5..13, 13);
        assert_eq!(left, "one");
        assert_eq!(cursor, 3);
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let (left, cursor) = delete_at("ñañ = 1\náéíóú\n", 5);
        assert_eq!(left, "áéíóú\n");
        // Column 3 of "áéíóú" is "ó", 6 bytes in.
        assert_eq!(cursor, 6);
    }

    #[test]
    fn crlf_line_breaks_go_whole() {
        let (left, _) = delete_at("one\r\ntwo\r\nthree", 6);
        assert_eq!(left, "one\r\nthree");
        let (left, cursor) = delete_at("one\r\ntwo", 6);
        assert_eq!(left, "one");
        assert_eq!(cursor, 1);
    }
}
