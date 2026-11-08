//! The editable model behind the environment editor tab (GitHub #65): the variables of
//! `<name>.env` and `<name>.local.env` as one list of rows, with the filter, the file scope and
//! the secrets mask, dirty detection, validation and the change list `Ctrl+S` hands to
//! [`brasa_workspace::Workspace::save_environment`]. No `gpui` types, so it is unit tested
//! directly. Render never computes anything proportional to the data size beyond the visible
//! rows: the visible list is rebuilt by [`EnvEditTab::reindex`] whenever something it depends on
//! changes, never per frame.

use std::collections::{HashMap, HashSet};

use brasa_core::KeyValue;
use brasa_workspace::{EnvChange, EnvLayer, EnvLayers};
use rust_i18n::t;

/// The file name of the environment `name` with `extension` (`.env`, `.local.env`), at most `max`
/// characters. A longer name is cut in its middle: the extension tells the two files apart, so it
/// always stays.
pub fn env_file_label(name: &str, extension: &str, max: usize) -> String {
    let count = name.chars().count();
    let room = max.saturating_sub(extension.chars().count() + 1);
    if count + extension.chars().count() <= max || room == 0 {
        return format!("{name}{extension}");
    }
    let start: String = name.chars().take(room - room / 2).collect();
    let end: String = name.chars().skip(count - room / 2).collect();
    format!("{start}\u{2026}{end}{extension}")
}

/// The id of the document tab editing the environment `name`. Prefixed so it can never collide
/// with a request id (a workspace path) or a load test id.
pub fn tab_id(name: &str) -> String {
    format!("env:{name}")
}

/// The label of the document tab editing the environment `name`.
pub fn tab_label(name: &str) -> String {
    t!("environments.tab_label", name = name).into_owned()
}

/// The workspace relative path of the file the status bar shows for the environment `name`.
pub fn status_path(name: &str) -> String {
    format!("environments/{name}.env")
}

/// One variable stored in a file. `id` is stable for the life of the tab, so the inputs bound to
/// the row survive edits to the other rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVarRow {
    /// The row id, unique within the tab.
    pub id: u64,
    /// The variable name.
    pub name: String,
    /// The variable value.
    pub value: String,
    /// The file the variable is stored in.
    pub layer: EnvLayer,
}

/// Which rows the table lists: the merged view, or the raw content of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnvScope {
    /// Both files merged, `.local.env` over `.env`, plus the session rows.
    #[default]
    All,
    /// Only the rows of `<name>.env`.
    Base,
    /// Only the rows of `<name>.local.env`.
    Local,
}

/// Why the editor refuses to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvValidationError {
    /// A row has a value but no name.
    EmptyName,
    /// The name contains `=`, a line break or starts with `#`, so it could not be read back.
    InvalidName(String),
    /// The value contains a line break.
    InvalidValue(String),
    /// Two rows of the same file have the same name.
    Duplicate(String),
}

/// A note shown under a row's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowNote {
    /// A `.local.env` row whose name also exists in `.env`.
    OverridesBase,
}

/// The state of one environment editor tab.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvEditTab {
    /// The environment name.
    pub name: String,
    /// Every variable of both files, `.env` rows first in file order, then `.local.env` rows.
    /// Rows added in the editor are appended.
    pub rows: Vec<EnvVarRow>,
    original: Vec<EnvVarRow>,
    /// Whether `<name>.env` exists on disk.
    pub base_exists: bool,
    /// Whether `<name>.local.env` exists on disk.
    pub local_exists: bool,
    next_id: u64,
    /// Ids from here on belong to rows added in this tab (they are listed last).
    first_new_id: u64,
    /// The text of the filter input, matched against names, case-insensitive.
    pub filter: String,
    /// Which rows the table lists.
    pub scope: EnvScope,
    /// Whether values stored in `.local.env` show in clear text.
    pub show_secrets: bool,
    /// The validation error of the last save attempt, cleared by the next edit.
    pub error: Option<EnvValidationError>,
    visible: Vec<usize>,
    count_all: usize,
}

impl EnvEditTab {
    /// Builds the tab for `name` from the two files read apart.
    pub fn load(name: &str, layers: &EnvLayers) -> Self {
        let mut rows = Vec::new();
        let mut next_id = 0;
        for (layer, variables) in [
            (EnvLayer::Base, layers.base.as_deref()),
            (EnvLayer::Local, layers.local.as_deref()),
        ] {
            for variable in variables.unwrap_or_default() {
                rows.push(EnvVarRow {
                    id: next_id,
                    name: variable.key.clone(),
                    value: variable.value.clone(),
                    layer,
                });
                next_id += 1;
            }
        }
        let mut tab = Self {
            name: name.to_string(),
            original: rows.clone(),
            rows,
            base_exists: layers.base.is_some(),
            local_exists: layers.local.is_some(),
            next_id,
            first_new_id: next_id,
            filter: String::new(),
            scope: EnvScope::All,
            show_secrets: false,
            error: None,
            visible: Vec::new(),
            count_all: 0,
        };
        tab.reindex();
        tab
    }

    /// The indexes into [`Self::rows`] the table lists, in order.
    pub fn visible(&self) -> &[usize] {
        &self.visible
    }

    /// How many variables the merged view has, ignoring the filter and without session rows.
    pub fn count_all(&self) -> usize {
        self.count_all
    }

    /// Whether the value of `row` is hidden behind bullets.
    pub fn is_masked(&self, row: &EnvVarRow) -> bool {
        row.layer == EnvLayer::Local && !self.show_secrets
    }

    /// The note to show under `row`'s value, if any.
    pub fn note(&self, row: &EnvVarRow) -> Option<RowNote> {
        let name = row.name.trim();
        let overrides = row.layer == EnvLayer::Local
            && !name.is_empty()
            && self
                .rows
                .iter()
                .any(|other| other.layer == EnvLayer::Base && other.name.trim() == name);
        overrides.then_some(RowNote::OverridesBase)
    }

    /// The session rows to list: only in the merged scope, filtered by name.
    pub fn visible_session<'a>(&self, session: &'a [KeyValue]) -> Vec<&'a KeyValue> {
        if self.scope != EnvScope::All {
            return Vec::new();
        }
        session
            .iter()
            .filter(|variable| self.matches_filter(&variable.key))
            .collect()
    }

    /// Sets the filter text.
    pub fn set_filter(&mut self, filter: &str) {
        if self.filter != filter {
            self.filter = filter.to_string();
            self.reindex();
        }
    }

    /// Sets which rows the table lists.
    pub fn set_scope(&mut self, scope: EnvScope) {
        self.scope = scope;
        self.reindex();
    }

    /// Flips the secrets toggle.
    pub fn toggle_secrets(&mut self) {
        self.show_secrets = !self.show_secrets;
    }

    /// Appends an empty row stored in `.env` and returns its id. The filter and the scope are
    /// reset first so the new row is on screen.
    pub fn add_variable(&mut self) -> u64 {
        self.filter.clear();
        self.scope = EnvScope::All;
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(EnvVarRow {
            id,
            name: String::new(),
            value: String::new(),
            layer: EnvLayer::Base,
        });
        self.error = None;
        self.reindex();
        id
    }

    /// The row the merged view shows for `name` (the `.local.env` one when it overrides `.env`),
    /// with the filter and the scope reset first so it is on screen. `None`, leaving the view as
    /// it was, when no row has that name.
    pub fn reveal(&mut self, name: &str) -> Option<u64> {
        let name = name.trim();
        let found = self.merged_order().into_iter().find_map(|index| {
            let row = &self.rows[index];
            (row.name.trim() == name).then_some(row.id)
        })?;
        self.filter.clear();
        self.scope = EnvScope::All;
        self.reindex();
        Some(found)
    }

    /// Removes the row `id`.
    pub fn remove(&mut self, id: u64) {
        self.rows.retain(|row| row.id != id);
        self.error = None;
        self.reindex();
    }

    /// Sets the name of the row `id`. Returns whether something changed.
    pub fn set_name(&mut self, id: u64, name: &str) -> bool {
        let changed = self.edit(id, |row| {
            let changed = row.name != name;
            row.name = name.to_string();
            changed
        });
        if changed {
            self.reindex();
        }
        changed
    }

    /// Sets the value of the row `id`. Returns whether something changed.
    pub fn set_value(&mut self, id: u64, value: &str) -> bool {
        self.edit(id, |row| {
            let changed = row.value != value;
            row.value = value.to_string();
            changed
        })
    }

    /// Moves the row `id` to the file `layer`.
    pub fn set_layer(&mut self, id: u64, layer: EnvLayer) {
        let changed = self.edit(id, |row| {
            let changed = row.layer != layer;
            row.layer = layer;
            changed
        });
        if changed {
            self.reindex();
        }
    }

    fn edit(&mut self, id: u64, f: impl FnOnce(&mut EnvVarRow) -> bool) -> bool {
        let changed = self.rows.iter_mut().find(|row| row.id == id).is_some_and(f);
        if changed {
            self.error = None;
        }
        changed
    }

    /// Whether the rows differ from what was loaded or last saved.
    pub fn is_dirty(&self) -> bool {
        self.rows != self.original
    }

    /// Checks the rows against what a file can hold. Rows added in the editor and left blank
    /// are ignored.
    pub fn validate(&self) -> Result<(), EnvValidationError> {
        let mut seen: HashSet<(bool, &str)> = HashSet::new();
        for row in self.rows.iter().filter(|row| !self.is_ignored(row)) {
            let name = row.name.trim();
            if name.is_empty() {
                return Err(EnvValidationError::EmptyName);
            }
            if name.contains(['=', '\n', '\r']) || name.starts_with('#') {
                return Err(EnvValidationError::InvalidName(name.to_string()));
            }
            if row.value.contains(['\n', '\r']) {
                return Err(EnvValidationError::InvalidValue(name.to_string()));
            }
            if !seen.insert((row.layer == EnvLayer::Local, name)) {
                return Err(EnvValidationError::Duplicate(name.to_string()));
            }
        }
        Ok(())
    }

    fn is_ignored(&self, row: &EnvVarRow) -> bool {
        row.id >= self.first_new_id && row.name.trim().is_empty() && row.value.is_empty()
    }

    /// The edits that turn the files as loaded into the current rows, see
    /// [`brasa_workspace::Workspace::save_environment`] for how they are applied.
    pub fn changes(&self) -> Vec<EnvChange> {
        let current: HashMap<u64, &EnvVarRow> = self.rows.iter().map(|row| (row.id, row)).collect();
        let mut original_counts: HashMap<(bool, &str), usize> = HashMap::new();
        for row in &self.original {
            *original_counts
                .entry((row.layer == EnvLayer::Local, row.name.as_str()))
                .or_default() += 1;
        }
        let mut changes = Vec::new();
        for old in &self.original {
            let Some(new) = current.get(&old.id) else {
                changes.push(EnvChange::Remove {
                    layer: old.layer,
                    key: old.name.clone(),
                });
                continue;
            };
            let name = new.name.trim();
            if new.layer != old.layer {
                changes.push(EnvChange::Remove {
                    layer: old.layer,
                    key: old.name.clone(),
                });
                changes.push(EnvChange::Set {
                    layer: new.layer,
                    key: name.to_string(),
                    value: new.value.clone(),
                });
                continue;
            }
            if name != old.name {
                changes.push(EnvChange::Rename {
                    layer: old.layer,
                    from: old.name.clone(),
                    to: name.to_string(),
                });
            }
            // A key defined twice in a file keeps both rows in sync with their lines: removing
            // or renaming one acts on the first line with that key, so the survivors are set
            // again to what they show.
            let was_duplicate = original_counts
                .get(&(old.layer == EnvLayer::Local, old.name.as_str()))
                .is_some_and(|count| *count > 1);
            if new.value != old.value || was_duplicate {
                changes.push(EnvChange::Set {
                    layer: old.layer,
                    key: name.to_string(),
                    value: new.value.clone(),
                });
            }
        }
        let original_ids: HashSet<u64> = self.original.iter().map(|row| row.id).collect();
        for row in &self.rows {
            if !original_ids.contains(&row.id) && !self.is_ignored(row) {
                changes.push(EnvChange::Set {
                    layer: row.layer,
                    key: row.name.trim().to_string(),
                    value: row.value.clone(),
                });
            }
        }
        changes
    }

    /// Marks the current rows as what is on disk, after a successful save.
    pub fn mark_saved(&mut self) {
        let first_new_id = self.first_new_id;
        self.rows.retain(|row| {
            row.id < first_new_id || !row.name.trim().is_empty() || !row.value.is_empty()
        });
        for row in &mut self.rows {
            row.name = row.name.trim().to_string();
        }
        self.base_exists |= self.rows.iter().any(|row| row.layer == EnvLayer::Base);
        self.local_exists |= self.rows.iter().any(|row| row.layer == EnvLayer::Local);
        self.original = self.rows.clone();
        self.error = None;
        self.reindex();
    }

    fn matches_filter(&self, name: &str) -> bool {
        let filter = self.filter.trim();
        filter.is_empty() || name.to_lowercase().contains(&filter.to_lowercase())
    }

    /// Rebuilds the visible list and the counts. Called by every edit that can change them.
    fn reindex(&mut self) {
        let merged = self.merged_order();
        self.count_all = merged.len();
        let order: Vec<usize> = match self.scope {
            EnvScope::All => merged,
            EnvScope::Base => self.layer_order(EnvLayer::Base),
            EnvScope::Local => self.layer_order(EnvLayer::Local),
        };
        self.visible = order
            .into_iter()
            .filter(|&index| {
                let name = &self.rows[index].name;
                name.trim().is_empty() || self.matches_filter(name)
            })
            .collect();
    }

    /// The rows of one file, rows added in the editor last.
    fn layer_order(&self, layer: EnvLayer) -> Vec<usize> {
        self.new_rows_last((0..self.rows.len()).filter(|&i| self.rows[i].layer == layer))
    }

    /// The merged view: `.env` order, a `.env` row replaced by the `.local.env` row that
    /// overrides it, then the `.local.env` only rows, then the rows added in the editor.
    fn merged_order(&self) -> Vec<usize> {
        let mut local_by_name: HashMap<&str, usize> = HashMap::new();
        for (index, row) in self.rows.iter().enumerate() {
            let name = row.name.trim();
            if row.layer == EnvLayer::Local && !name.is_empty() {
                local_by_name.entry(name).or_insert(index);
            }
        }
        let mut shown = vec![false; self.rows.len()];
        let mut order = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            if row.layer != EnvLayer::Base {
                continue;
            }
            // A `.env` row overridden by a `.local.env` row stays out of the merged view and the
            // local row takes its place.
            shown[index] = true;
            let chosen = local_by_name.get(row.name.trim()).copied().unwrap_or(index);
            if chosen == index || !shown[chosen] {
                shown[chosen] = true;
                order.push(chosen);
            }
        }
        order.extend((0..self.rows.len()).filter(|&index| !shown[index]));
        self.new_rows_last(order.into_iter())
    }

    /// Stable partition: rows loaded from disk first, rows added in the editor after them.
    fn new_rows_last(&self, order: impl Iterator<Item = usize>) -> Vec<usize> {
        let (old, new): (Vec<usize>, Vec<usize>) =
            order.partition(|&index| self.rows[index].id < self.first_new_id);
        old.into_iter().chain(new).collect()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn long_file_labels_keep_the_extension() {
        assert_eq!(env_file_label("local", ".env", 22), "local.env");
        let long = "Acme Payments Platform production";
        let label = env_file_label(long, ".local.env", 22);
        assert_eq!(label.chars().count(), 22);
        assert_eq!(label, "Acme P\u{2026}ction.local.env");
    }
    use pretty_assertions::assert_eq;

    fn layers(base: &[(&str, &str)], local: Option<&[(&str, &str)]>) -> EnvLayers {
        let convert = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(key, value)| KeyValue::new(*key, *value))
                .collect()
        };
        EnvLayers {
            base: Some(convert(base)),
            local: local.map(convert),
        }
    }

    fn sample() -> EnvEditTab {
        EnvEditTab::load(
            "staging",
            &layers(
                &[
                    ("baseUrl", "https://x"),
                    ("apiKey", "old"),
                    ("timeout", "5"),
                ],
                Some(&[("password", "pw"), ("apiKey", "secret")]),
            ),
        )
    }

    fn visible_names(tab: &EnvEditTab) -> Vec<&str> {
        tab.visible()
            .iter()
            .map(|&index| tab.rows[index].name.as_str())
            .collect()
    }

    #[test]
    fn ids_and_labels_do_not_collide_with_request_ids() {
        assert_eq!(tab_id("local"), "env:local");
        assert_eq!(status_path("local"), "environments/local.env");
        assert!(tab_label("local").contains("local"));
    }

    #[test]
    fn the_merged_view_shows_the_local_row_in_place_of_the_base_one() {
        let tab = sample();
        assert_eq!(
            visible_names(&tab),
            vec!["baseUrl", "apiKey", "timeout", "password"]
        );
        let api_key = &tab.rows[tab.visible()[1]];
        assert_eq!(api_key.layer, EnvLayer::Local);
        assert_eq!(api_key.value, "secret");
        assert_eq!(tab.note(api_key), Some(RowNote::OverridesBase));
        assert_eq!(tab.count_all(), 4);
    }

    #[test]
    fn a_file_scope_lists_the_raw_rows_of_that_file() {
        let mut tab = sample();
        tab.set_scope(EnvScope::Base);
        assert_eq!(visible_names(&tab), vec!["baseUrl", "apiKey", "timeout"]);
        tab.set_scope(EnvScope::Local);
        assert_eq!(visible_names(&tab), vec!["password", "apiKey"]);
    }

    #[test]
    fn the_filter_matches_names_case_insensitively() {
        let mut tab = sample();
        tab.set_filter("URL");
        assert_eq!(visible_names(&tab), vec!["baseUrl"]);
        tab.set_filter("zzz");
        assert!(tab.visible().is_empty());
        tab.set_filter("");
        assert_eq!(tab.visible().len(), 4);
    }

    #[test]
    fn values_of_local_rows_are_masked_until_secrets_are_shown() {
        let mut tab = sample();
        let base = tab.rows[0].clone();
        let local = tab.rows[3].clone();
        assert!(!tab.is_masked(&base));
        assert!(tab.is_masked(&local));
        tab.toggle_secrets();
        assert!(!tab.is_masked(&local));
    }

    #[test]
    fn session_rows_only_show_in_the_merged_scope_and_follow_the_filter() {
        let mut tab = sample();
        let session = vec![KeyValue::new("token", "t"), KeyValue::new("other", "o")];
        assert_eq!(tab.visible_session(&session).len(), 2);
        tab.set_filter("tok");
        assert_eq!(tab.visible_session(&session).len(), 1);
        tab.set_scope(EnvScope::Local);
        assert!(tab.visible_session(&session).is_empty());
    }

    #[test]
    fn a_fresh_tab_is_clean_and_an_edit_makes_it_dirty_until_undone() {
        let mut tab = sample();
        assert!(!tab.is_dirty());
        assert!(tab.set_value(0, "https://y"));
        assert!(tab.is_dirty());
        assert!(tab.set_value(0, "https://x"));
        assert!(!tab.is_dirty());
        assert!(!tab.set_value(0, "https://x"));
    }

    #[test]
    fn adding_a_variable_resets_the_filter_and_lists_it_last() {
        let mut tab = sample();
        tab.set_filter("url");
        tab.set_scope(EnvScope::Local);
        let id = tab.add_variable();
        assert_eq!(tab.filter, "");
        assert_eq!(tab.scope, EnvScope::All);
        let last = *tab.visible().last().expect("visible");
        assert_eq!(tab.rows[last].id, id);
        assert_eq!(tab.rows[last].layer, EnvLayer::Base);
        assert!(tab.is_dirty());
    }

    #[test]
    fn a_blank_new_row_is_ignored_by_validation_and_changes() {
        let mut tab = sample();
        tab.add_variable();
        assert_eq!(tab.validate(), Ok(()));
        assert!(tab.changes().is_empty());
    }

    #[test]
    fn validation_rejects_bad_names_and_duplicates_in_the_same_file() {
        let mut tab = sample();
        let id = tab.add_variable();
        tab.set_value(id, "v");
        assert_eq!(tab.validate(), Err(EnvValidationError::EmptyName));
        tab.set_name(id, "a=b");
        assert_eq!(
            tab.validate(),
            Err(EnvValidationError::InvalidName("a=b".to_string()))
        );
        tab.set_name(id, "# c");
        assert!(matches!(
            tab.validate(),
            Err(EnvValidationError::InvalidName(_))
        ));
        tab.set_name(id, "timeout");
        assert_eq!(
            tab.validate(),
            Err(EnvValidationError::Duplicate("timeout".to_string()))
        );
        tab.set_layer(id, EnvLayer::Local);
        assert_eq!(tab.validate(), Ok(()));
        tab.set_value(id, "a\nb");
        assert!(matches!(
            tab.validate(),
            Err(EnvValidationError::InvalidValue(_))
        ));
    }

    #[test]
    fn an_edit_clears_the_previous_error() {
        let mut tab = sample();
        tab.error = Some(EnvValidationError::EmptyName);
        tab.set_value(0, "z");
        assert_eq!(tab.error, None);
    }

    #[test]
    fn changes_cover_set_rename_move_remove_and_add() {
        let mut tab = sample();
        // rows: 0 baseUrl, 1 apiKey, 2 timeout (base), 3 password, 4 apiKey (local)
        tab.set_value(0, "https://y");
        tab.set_name(2, "timeoutMs");
        tab.set_layer(1, EnvLayer::Local);
        tab.remove(3);
        let id = tab.add_variable();
        tab.set_name(id, " fresh ");
        tab.set_value(id, "1");
        tab.set_layer(id, EnvLayer::Local);

        assert_eq!(
            tab.changes(),
            vec![
                EnvChange::Set {
                    layer: EnvLayer::Base,
                    key: "baseUrl".into(),
                    value: "https://y".into()
                },
                EnvChange::Remove {
                    layer: EnvLayer::Base,
                    key: "apiKey".into()
                },
                EnvChange::Set {
                    layer: EnvLayer::Local,
                    key: "apiKey".into(),
                    value: "old".into()
                },
                EnvChange::Rename {
                    layer: EnvLayer::Base,
                    from: "timeout".into(),
                    to: "timeoutMs".into()
                },
                EnvChange::Remove {
                    layer: EnvLayer::Local,
                    key: "password".into()
                },
                EnvChange::Set {
                    layer: EnvLayer::Local,
                    key: "fresh".into(),
                    value: "1".into()
                },
            ]
        );
    }

    #[test]
    fn rows_of_a_duplicated_key_are_set_again_so_the_right_line_survives() {
        let mut tab = EnvEditTab::load("dev", &layers(&[("a", "1"), ("a", "2")], None));
        tab.remove(0);
        assert_eq!(
            tab.changes(),
            vec![
                EnvChange::Remove {
                    layer: EnvLayer::Base,
                    key: "a".into()
                },
                EnvChange::Set {
                    layer: EnvLayer::Base,
                    key: "a".into(),
                    value: "2".into()
                },
            ]
        );
    }

    #[test]
    fn duplicates_loaded_from_a_file_fail_validation() {
        let tab = EnvEditTab::load("dev", &layers(&[("a", "1"), ("a", "2")], None));
        assert_eq!(
            tab.validate(),
            Err(EnvValidationError::Duplicate("a".to_string()))
        );
    }

    #[test]
    fn mark_saved_makes_the_tab_clean_and_records_the_new_files() {
        let mut tab = EnvEditTab::load("dev", &layers(&[("a", "1")], None));
        assert!(!tab.local_exists);
        let id = tab.add_variable();
        tab.set_name(id, " b ");
        tab.set_layer(id, EnvLayer::Local);
        tab.add_variable();

        tab.mark_saved();

        assert!(!tab.is_dirty());
        assert!(tab.local_exists);
        assert_eq!(tab.rows.len(), 2);
        assert_eq!(tab.rows[1].name, "b");
        assert!(tab.changes().is_empty());
    }

    #[test]
    fn reveal_finds_the_row_the_merged_view_shows_and_resets_the_view() {
        let mut tab = sample();
        tab.set_filter("timeout");
        tab.set_scope(EnvScope::Base);
        // apiKey is in both files: the local row (id 4) overrides the base one.
        assert_eq!(tab.reveal("apiKey"), Some(4));
        assert_eq!(tab.filter, "");
        assert_eq!(tab.scope, EnvScope::All);
        assert_eq!(tab.reveal("baseUrl"), Some(0));
    }

    #[test]
    fn reveal_of_an_unknown_name_leaves_the_view_alone() {
        let mut tab = sample();
        tab.set_filter("url");
        assert_eq!(tab.reveal("missing"), None);
        assert_eq!(tab.filter, "url");
        assert!(!tab.is_dirty());
    }
}
