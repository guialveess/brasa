//! The pure parts of the searchable menu: filtering, the flat list of what is shown, the
//! highlight ranges of a matched title and the keyboard movement of the highlight. No `gpui`, so
//! all of it is unit tested without a window.

use std::ops::Range;

use crate::metrics::{
    SEARCHABLE_MENU_LABEL_GAP, SEARCHABLE_MENU_LABEL_HEIGHT, SEARCHABLE_MENU_ROW_HEIGHT,
};

/// Height of a group label entry: the label and the space above it.
pub const LABEL_ENTRY_HEIGHT: f32 = SEARCHABLE_MENU_LABEL_HEIGHT + SEARCHABLE_MENU_LABEL_GAP;
/// Height of a row entry.
pub const ROW_ENTRY_HEIGHT: f32 = SEARCHABLE_MENU_ROW_HEIGHT;

/// One entry of the flat list the menu shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// The label of the group at this index.
    Label(usize),
    /// A row: its group and its position in that group.
    Row { group: usize, row: usize },
    /// The row the app creates from the query, always last.
    Create,
}

impl Entry {
    /// Whether the highlight can rest on the entry (everything but a label).
    pub fn is_selectable(self) -> bool {
        !matches!(self, Entry::Label(_))
    }

    /// The height of the entry in pixels.
    pub fn height(self) -> f32 {
        match self {
            Entry::Label(_) => LABEL_ENTRY_HEIGHT,
            Entry::Row { .. } | Entry::Create => ROW_ENTRY_HEIGHT,
        }
    }
}

/// The lowercase form used to compare a query with a text.
pub fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// The entries to show for `query_folded`, over the folded search text of every row of every
/// group. A group with no matching row disappears with its label. An empty query matches
/// everything. The create row is not included, the caller appends it.
pub fn filter_entries(search: &[Vec<String>], query_folded: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for (group, rows) in search.iter().enumerate() {
        let start = entries.len();
        entries.push(Entry::Label(group));
        for (row, text) in rows.iter().enumerate() {
            if query_folded.is_empty() || text.contains(query_folded) {
                entries.push(Entry::Row { group, row });
            }
        }
        if entries.len() == start + 1 {
            entries.pop();
        }
    }
    entries
}

/// The indices of the folded `labels` that match `query_folded`, in order.
pub fn filter_indices(labels: &[String], query_folded: &str) -> Vec<usize> {
    labels
        .iter()
        .enumerate()
        .filter(|(_, label)| query_folded.is_empty() || label.contains(query_folded))
        .map(|(index, _)| index)
        .collect()
}

/// The sum of the heights of `entries`.
pub fn content_height(entries: &[Entry]) -> f32 {
    entries.iter().map(|entry| entry.height()).sum()
}

/// The first entry the highlight can rest on.
pub fn first_selectable(entries: &[Entry]) -> Option<usize> {
    entries.iter().position(|entry| entry.is_selectable())
}

/// The highlight after moving `delta` selectable entries (negative is up) from `current`,
/// skipping labels and wrapping around at both ends. With no current highlight, down goes to the
/// first selectable entry and up to the last.
pub fn move_highlight(entries: &[Entry], current: Option<usize>, delta: isize) -> Option<usize> {
    let selectable: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.is_selectable())
        .map(|(index, _)| index)
        .collect();
    if selectable.is_empty() {
        return None;
    }
    let count = selectable.len() as isize;
    let position = current.and_then(|c| selectable.iter().position(|&index| index == c));
    let target = match position {
        Some(position) => (position as isize + delta).rem_euclid(count),
        None if delta >= 0 => 0,
        None => count - 1,
    };
    Some(selectable[target as usize])
}

/// The byte range of `text` that matches `query`, ignoring case, for the highlighted part of a
/// title. `None` when the query is empty or does not occur. Works on characters, so a letter
/// whose lowercase form has another length (`İ`) still maps back to the right bytes of `text`.
pub fn highlight_range(text: &str, query: &str) -> Option<Range<usize>> {
    if query.is_empty() {
        return None;
    }
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    // Every lowercase char with the byte range of the original char it came from.
    let folded: Vec<(char, usize, usize)> = text
        .char_indices()
        .flat_map(|(start, c)| {
            let end = start + c.len_utf8();
            c.to_lowercase().map(move |l| (l, start, end))
        })
        .collect();
    if wanted.len() > folded.len() {
        return None;
    }
    (0..=folded.len() - wanted.len()).find_map(|at| {
        let window = &folded[at..at + wanted.len()];
        window
            .iter()
            .zip(&wanted)
            .all(|((c, _, _), w)| c == w)
            .then(|| window[0].1..window[window.len() - 1].2)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn search(groups: &[&[&str]]) -> Vec<Vec<String>> {
        groups
            .iter()
            .map(|rows| rows.iter().map(|row| fold(row)).collect())
            .collect()
    }

    #[test]
    fn an_empty_query_lists_every_group_and_row() {
        let data = search(&[&["main", "dev"], &["origin/main"]]);
        assert_eq!(
            filter_entries(&data, ""),
            vec![
                Entry::Label(0),
                Entry::Row { group: 0, row: 0 },
                Entry::Row { group: 0, row: 1 },
                Entry::Label(1),
                Entry::Row { group: 1, row: 0 },
            ]
        );
    }

    #[test]
    fn a_query_keeps_the_matching_rows_and_drops_empty_groups() {
        let data = search(&[&["Main", "dev"], &["origin/MAIN"], &["tags/v1"]]);
        assert_eq!(
            filter_entries(&data, &fold("MAIN")),
            vec![
                Entry::Label(0),
                Entry::Row { group: 0, row: 0 },
                Entry::Label(1),
                Entry::Row { group: 1, row: 0 },
            ]
        );
        assert_eq!(filter_entries(&data, "zzz"), Vec::<Entry>::new());
    }

    #[test]
    fn a_group_without_rows_has_no_label() {
        let data = search(&[&[], &["a"]]);
        assert_eq!(
            filter_entries(&data, ""),
            vec![Entry::Label(1), Entry::Row { group: 1, row: 0 }]
        );
    }

    #[test]
    fn actions_are_filtered_by_their_label() {
        let labels = vec![fold("New branch"), fold("Pull"), fold("Push")];
        assert_eq!(filter_indices(&labels, ""), vec![0, 1, 2]);
        assert_eq!(filter_indices(&labels, "pu"), vec![1, 2]);
        assert_eq!(filter_indices(&labels, "xyz"), Vec::<usize>::new());
    }

    #[test]
    fn heights_add_up() {
        let entries = [
            Entry::Label(0),
            Entry::Row { group: 0, row: 0 },
            Entry::Create,
        ];
        assert_eq!(
            content_height(&entries),
            LABEL_ENTRY_HEIGHT + 2.0 * ROW_ENTRY_HEIGHT
        );
    }

    fn sample() -> Vec<Entry> {
        vec![
            Entry::Label(0),
            Entry::Row { group: 0, row: 0 },
            Entry::Row { group: 0, row: 1 },
            Entry::Label(1),
            Entry::Row { group: 1, row: 0 },
            Entry::Create,
        ]
    }

    #[test]
    fn the_highlight_starts_on_the_first_row_not_the_label() {
        assert_eq!(first_selectable(&sample()), Some(1));
        assert_eq!(first_selectable(&[Entry::Label(0)]), None);
        assert_eq!(first_selectable(&[]), None);
    }

    #[test]
    fn down_and_up_skip_labels() {
        let entries = sample();
        assert_eq!(move_highlight(&entries, Some(1), 1), Some(2));
        assert_eq!(move_highlight(&entries, Some(2), 1), Some(4));
        assert_eq!(move_highlight(&entries, Some(4), 1), Some(5));
        assert_eq!(move_highlight(&entries, Some(4), -1), Some(2));
        assert_eq!(move_highlight(&entries, Some(2), -1), Some(1));
    }

    #[test]
    fn the_highlight_wraps_around() {
        let entries = sample();
        assert_eq!(move_highlight(&entries, Some(5), 1), Some(1));
        assert_eq!(move_highlight(&entries, Some(1), -1), Some(5));
    }

    #[test]
    fn without_a_highlight_down_goes_first_and_up_goes_last() {
        let entries = sample();
        assert_eq!(move_highlight(&entries, None, 1), Some(1));
        assert_eq!(move_highlight(&entries, None, -1), Some(5));
        assert_eq!(move_highlight(&[Entry::Label(0)], None, 1), None);
        assert_eq!(move_highlight(&[], Some(0), 1), None);
    }

    #[test]
    fn a_stale_highlight_on_a_label_recovers() {
        assert_eq!(move_highlight(&sample(), Some(0), 1), Some(1));
    }

    #[test]
    fn the_matched_part_of_a_title_is_found_ignoring_case() {
        assert_eq!(highlight_range("feature/Auth-refresh", "auth"), Some(8..12));
        assert_eq!(highlight_range("main", "MAIN"), Some(0..4));
        assert_eq!(highlight_range("main", ""), None);
        assert_eq!(highlight_range("main", "dev"), None);
        assert_eq!(highlight_range("ma", "main"), None);
    }

    #[test]
    fn the_first_occurrence_is_the_one_highlighted() {
        assert_eq!(highlight_range("ab ab", "ab"), Some(0..2));
    }

    #[test]
    fn highlight_ranges_are_byte_ranges_of_the_original_text() {
        let text = "rama/ñandú";
        assert_eq!(highlight_range(text, "ÑAN").map(|r| &text[r]), Some("ñan"));
        // `İ` lowercases to two chars, and the range still covers the whole original letter.
        let text = "xİy";
        assert_eq!(highlight_range(text, "y").map(|r| &text[r]), Some("y"));
    }
}
