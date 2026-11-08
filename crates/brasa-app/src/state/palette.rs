//! Plain data behind the command palette (`Ctrl K`/`Cmd K`): the items it can show, and the fuzzy matching used to filter and rank
//! them as the user types. `views/command_palette.rs` renders this; this module only holds the
//! data and the matching logic, so both are unit tested without a window.

use brasa_core::Method;
use brasa_workspace::Node;
use rust_i18n::t;
use telaio_ui::components::open_hint;

/// The modifier key label shortcut hints show, matching `main.rs`'s key bindings: `Cmd` on
/// macOS, `Ctrl` elsewhere (same local-constant convention as `views/env_picker.rs`'s
/// `MODIFIER_KEY`).
#[cfg(target_os = "macos")]
const MODIFIER_KEY: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const MODIFIER_KEY: &str = "Ctrl";

/// How many environments get a `Ctrl 1..9` shortcut hint, matching `views/env_picker.rs`.
const SHORTCUT_COUNT: usize = 9;

/// A single command palette entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    /// What this entry is and does when chosen.
    pub kind: PaletteItemKind,
    /// The main text shown for this entry, matched against the user's query.
    pub label: String,
    /// Secondary text shown next to the label (a path, an environment's variable count, ...).
    pub detail: Option<String>,
    /// A keyboard shortcut hint shown at the right of the row, if this entry has one.
    pub shortcut: Option<String>,
}

/// What a [`PaletteItem`] represents, and what choosing it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteItemKind {
    /// Opens the request with this workspace id.
    Request(String),
    /// Runs a built-in action.
    Action(ActionId),
    /// Switches to this environment, or to "No environment" for `None`.
    Environment(Option<String>),
    /// Opens the OpenAPI import wizard on the source of the collection in this folder, to
    /// update it (GitHub #203).
    OpenApiUpdate {
        /// The collection folder.
        folder: String,
        /// Where its spec came from.
        source: brasa_format::openapi::ManifestSource,
    },
}

/// The "Update from OpenAPI" item of each collection an OpenAPI import created, except those
/// imported from pasted text, which cannot be read again.
pub fn openapi_update_items(
    collections: &[brasa_workspace::OpenApiCollection],
) -> Vec<PaletteItem> {
    collections
        .iter()
        .filter(|collection| {
            super::openapi_import::source_kind_of(&collection.manifest.source.kind).is_some()
        })
        .map(|collection| PaletteItem {
            kind: PaletteItemKind::OpenApiUpdate {
                folder: collection.folder.clone(),
                source: collection.manifest.source.clone(),
            },
            label: t!("openapi.palette.update", name = collection.folder.clone()).into_owned(),
            detail: None,
            shortcut: None,
        })
        .collect()
}

/// A built-in action reachable from the command palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    /// Sends the active tab's request.
    Send,
    /// Saves the active tab.
    Save,
    /// Starts creating a new request.
    NewRequest,
    /// Starts creating a new folder.
    NewFolder,
    /// Reads the workspace from disk again (#219).
    RefreshCollections,
    /// Opens the import wizard for OpenAPI, on its File source.
    ImportOpenApi,
    /// Opens the import wizard for OpenAPI, on its URL source.
    ImportOpenApiUrl,
    /// Opens the import wizard for Postman, whose export of a collection already imported offers
    /// to update it.
    UpdateFromPostman,
    /// Opens the import wizard for Postman.
    ImportCollection,
    /// Opens the import wizard for Postman.
    ImportEnvironment,
    /// Opens the import wizard for Bruno, on its Folder source (#222).
    ImportBruno,
    /// Opens the import wizard for Bruno, whose collection already imported offers to update it
    /// (#223).
    UpdateFromBruno,
    /// Opens the Settings modal.
    OpenSettings,
    /// Opens the keyboard shortcuts cheat sheet.
    OpenShortcuts,
    /// Opens the "New workspace" dialog.
    NewWorkspace,
    /// Opens the "Open folder" picker.
    OpenWorkspace,
    /// Opens the welcome tab, or brings it to the front.
    ShowWelcome,
    /// Opens the What's new tab, or brings it to the front.
    ShowWhatsNew,
    /// Starts creating a new load test tab.
    NewLoadTest,
    /// Toggles between the light and dark theme.
    ToggleTheme,
    /// Checks for a new version now (only listed when the build has an updater).
    CheckForUpdates,
    /// Commits the staged changes with the commit message, or shows the message when empty.
    GitCommit,
    /// Commits, then pushes after listing the commits.
    GitCommitAndPush,
    /// Pushes the current branch.
    GitPush,
    /// Pulls into the current branch.
    GitPull,
    /// Fetches from the remote.
    GitFetch,
    /// Asks for a name and creates a branch from the current one.
    GitNewBranch,
    /// Opens the branch menu of the status bar.
    GitCheckoutBranch,
    /// Opens the Git log tab.
    GitShowLog,
    /// Runs `git init` in the workspace.
    GitInit,
}

/// What the Git actions of the palette can do in the open workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitPalette {
    /// No workspace, or no usable git: no Git action.
    Unavailable,
    /// The workspace is not in a repository: only "Initialize repository".
    NotARepo,
    /// The workspace is in a repository: every action but "Initialize repository".
    Repo,
}

/// The Git actions for `git` (#175), after the other actions.
pub fn git_action_items(git: GitPalette) -> Vec<PaletteItem> {
    match git {
        GitPalette::Unavailable => Vec::new(),
        GitPalette::NotARepo => vec![action_item(ActionId::GitInit, t!("git.palette.init"), None)],
        GitPalette::Repo => vec![
            action_item(ActionId::GitCommit, t!("git.palette.commit"), None),
            action_item(
                ActionId::GitCommitAndPush,
                t!("git.palette.commit_and_push"),
                None,
            ),
            action_item(ActionId::GitPush, t!("git.palette.push"), None),
            action_item(ActionId::GitPull, t!("git.palette.pull"), None),
            action_item(ActionId::GitFetch, t!("git.palette.fetch"), None),
            action_item(ActionId::GitNewBranch, t!("git.palette.new_branch"), None),
            action_item(
                ActionId::GitCheckoutBranch,
                t!("git.palette.checkout"),
                None,
            ),
            action_item(ActionId::GitShowLog, t!("git.palette.log"), None),
        ],
    }
}

/// Builds the "Requests" group's items from a workspace's collection tree, in tree order
/// (folders depth-first, matching the sidebar): one item per request, skipping broken files
/// (there is nothing to open). The label is the request's id with the `.brasa` extension
/// stripped, so it reads as its path (for example `"auth/login"`); the detail is its HTTP
/// method, when the file parsed enough to report one.
pub fn request_items(nodes: &[Node]) -> Vec<PaletteItem> {
    let mut items = Vec::new();
    collect_request_items(nodes, &mut items);
    items
}

fn collect_request_items(nodes: &[Node], items: &mut Vec<PaletteItem>) {
    for node in nodes {
        match node {
            Node::Folder(folder) => collect_request_items(&folder.children, items),
            Node::Request(request) => {
                if request.broken.is_some() {
                    continue;
                }
                let label = request
                    .id
                    .strip_suffix(".brasa")
                    .unwrap_or(&request.id)
                    .to_string();
                items.push(PaletteItem {
                    kind: PaletteItemKind::Request(request.id.clone()),
                    label,
                    detail: request.method.as_ref().map(Method::to_string),
                    shortcut: None,
                });
            }
        }
    }
}

/// Builds the "Environments" group's items: "No environment" first, then each named environment
/// in `environments` (already sorted, as `Workspace::list_environments` returns them), with the
/// same `Ctrl 0`/`Ctrl 1..9` shortcut hints as the title bar's environment picker
/// (`views/env_picker.rs`).
pub fn environment_items(environments: &[String]) -> Vec<PaletteItem> {
    let mut items = vec![PaletteItem {
        kind: PaletteItemKind::Environment(None),
        label: t!("common.no_environment").into_owned(),
        detail: None,
        shortcut: Some(format!("{MODIFIER_KEY}+0")),
    }];
    for (index, name) in environments.iter().enumerate() {
        items.push(PaletteItem {
            kind: PaletteItemKind::Environment(Some(name.clone())),
            label: name.clone(),
            detail: None,
            shortcut: (index < SHORTCUT_COUNT).then(|| format!("{MODIFIER_KEY}+{}", index + 1)),
        });
    }
    items
}

/// Builds the "Actions" group's items: every [`ActionId`], in the order the palette lists them,
/// with the shortcut hint of the ones that have a real key binding (`main.rs`'s `bind_keys`).
pub fn action_items() -> Vec<PaletteItem> {
    let mut items = vec![
        action_item(
            ActionId::Send,
            t!("shell.palette.send"),
            Some(send_shortcut()),
        ),
        action_item(
            ActionId::Save,
            t!("common.save"),
            Some(format!("{MODIFIER_KEY}+S")),
        ),
        action_item(ActionId::NewRequest, t!("common.new_request"), None),
        action_item(ActionId::NewFolder, t!("common.new_folder"), None),
        action_item(
            ActionId::RefreshCollections,
            t!("shell.sidebar.refresh"),
            None,
        ),
        action_item(
            ActionId::ImportOpenApi,
            t!("openapi.palette.import"),
            Some(format!("{MODIFIER_KEY}+Shift+O")),
        ),
        action_item(
            ActionId::ImportOpenApiUrl,
            t!("openapi.palette.import_url"),
            None,
        ),
        action_item(
            ActionId::ImportCollection,
            t!("shell.palette.import_collection"),
            None,
        ),
        action_item(
            ActionId::ImportEnvironment,
            t!("shell.palette.import_environment"),
            None,
        ),
        action_item(ActionId::ImportBruno, t!("import.palette.bruno"), None),
        action_item(
            ActionId::UpdateFromBruno,
            t!("import.palette.update_bruno"),
            None,
        ),
        action_item(
            ActionId::UpdateFromPostman,
            t!("import.palette.update_postman"),
            None,
        ),
        action_item(
            ActionId::OpenSettings,
            t!("shell.palette.open_settings"),
            Some(format!("{MODIFIER_KEY}+,")),
        ),
        action_item(
            ActionId::OpenShortcuts,
            t!("shortcuts.title"),
            Some(open_hint()),
        ),
        action_item(ActionId::NewWorkspace, t!("common.new_workspace"), None),
        action_item(
            ActionId::OpenWorkspace,
            t!("common.open_folder"),
            Some(format!("{MODIFIER_KEY}+O")),
        ),
        action_item(ActionId::ShowWelcome, t!("welcome.palette_action"), None),
        action_item(ActionId::ShowWhatsNew, t!("whats_new.palette_action"), None),
        action_item(
            ActionId::NewLoadTest,
            t!("shell.palette.new_load_test"),
            None,
        ),
        action_item(
            ActionId::ToggleTheme,
            t!("shell.palette.toggle_theme"),
            None,
        ),
    ];
    if super::update::updater_enabled() {
        items.push(action_item(
            ActionId::CheckForUpdates,
            t!("shell.palette.check_for_updates"),
            None,
        ));
    }
    items
}

fn action_item(
    action: ActionId,
    label: impl Into<String>,
    shortcut: Option<String>,
) -> PaletteItem {
    PaletteItem {
        kind: PaletteItemKind::Action(action),
        label: label.into(),
        detail: None,
        shortcut,
    }
}

/// `"Ctrl+\u{21b5}"` (`"Cmd+\u{21b5}"` on macOS), matching `main.rs`'s `SendActiveTab` binding.
/// The branch menu shows it for "Create branch" too.
pub fn send_shortcut_label() -> String {
    send_shortcut()
}

/// See [`send_shortcut_label`].
fn send_shortcut() -> String {
    format!("{MODIFIER_KEY}+\u{21b5}")
}

/// Bonus added once per matched character that starts a word: the first character of a label, or
/// one right after a non-alphanumeric character.
const WORD_START_BONUS: i32 = 10;

/// Penalty subtracted, per skipped character, between two consecutive matched characters.
const GAP_PENALTY: i32 = 8;

/// Filters and ranks `items` by fuzzy-matching `query` against each label, case-insensitive.
///
/// A query matches an item when every one of its characters appears in the label, in order, not
/// necessarily contiguous (a subsequence match), like most command palettes. An empty query
/// matches every item with the same score, keeping their original order.
///
/// Returns one entry per matching item: its index into `items`, a score used to sort matches
/// (highest first: matches at the start of a word and with fewer gaps between matched characters
/// score higher), and the char indices of the matched characters in the label, for the view to
/// highlight them.
pub fn fuzzy_filter(query: &str, items: &[PaletteItem]) -> Vec<(usize, i32, Vec<usize>)> {
    let mut matches: Vec<(usize, i32, Vec<usize>)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            fuzzy_match(query, &item.label).map(|(score, indices)| (index, score, indices))
        })
        .collect();
    matches.sort_by_key(|(_, score, _)| std::cmp::Reverse(*score));
    matches
}

/// The lowercase of `character`, one char for one char so matched indices stay valid. Unlike
/// `to_ascii_lowercase` it also folds accented capitals (`Ó`), which the translated labels have.
fn lower_char(character: char) -> char {
    character.to_lowercase().next().unwrap_or(character)
}

/// Tries to match `query` as a case-insensitive subsequence of `text`, greedily picking the
/// earliest occurrence of each character. Returns the score and the matched char indices, or
/// `None` if some character of `query` does not appear, in order, in `text`.
fn fuzzy_match(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let lower_text: Vec<char> = text.chars().map(lower_char).collect();
    let lower_query: Vec<char> = query.chars().map(lower_char).collect();

    let mut matched_indices = Vec::with_capacity(lower_query.len());
    let mut score = 0i32;
    let mut search_from = 0usize;
    let mut previous_index: Option<usize> = None;

    for query_char in lower_query {
        let found = lower_text[search_from..]
            .iter()
            .position(|&character| character == query_char)
            .map(|offset| offset + search_from)?;

        let is_word_start = found == 0 || !lower_text[found - 1].is_alphanumeric();
        score += 1;
        if is_word_start {
            score += WORD_START_BONUS;
        }
        if let Some(previous) = previous_index {
            let gap = (found - previous - 1) as i32;
            score -= GAP_PENALTY * gap;
        }

        matched_indices.push(found);
        previous_index = Some(found);
        search_from = found + 1;
    }

    Some((score, matched_indices))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_workspace::{Folder, RequestEntry};
    use pretty_assertions::assert_eq;

    fn item(label: &str) -> PaletteItem {
        PaletteItem {
            kind: PaletteItemKind::Request(label.to_string()),
            label: label.to_string(),
            detail: None,
            shortcut: None,
        }
    }

    fn request(id: &str, name: &str, method: Option<Method>, broken: bool) -> Node {
        Node::Request(RequestEntry {
            id: id.to_string(),
            name: name.to_string(),
            broken: broken.then(|| "broken".to_string()),
            method,
        })
    }

    fn folder(id: &str, name: &str, children: Vec<Node>) -> Node {
        Node::Folder(Folder {
            id: id.to_string(),
            name: name.to_string(),
            children,
        })
    }

    #[test]
    fn request_items_strips_the_brasa_extension_and_keeps_the_path() {
        let tree = vec![folder(
            "auth",
            "auth",
            vec![request(
                "auth/login.brasa",
                "login",
                Some(Method::Post),
                false,
            )],
        )];
        let items = request_items(&tree);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "auth/login");
        assert_eq!(items[0].detail.as_deref(), Some("POST"));
        assert_eq!(
            items[0].kind,
            PaletteItemKind::Request("auth/login.brasa".to_string())
        );
    }

    #[test]
    fn request_items_skips_broken_files() {
        let tree = vec![request("broken.brasa", "broken", None, true)];
        assert!(request_items(&tree).is_empty());
    }

    #[test]
    fn environment_items_lists_no_environment_first_with_shortcuts() {
        let items = environment_items(&["local".to_string(), "prod".to_string()]);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].kind, PaletteItemKind::Environment(None));
        assert_eq!(
            items[1].kind,
            PaletteItemKind::Environment(Some("local".to_string()))
        );
        assert!(items[1].shortcut.is_some());
    }

    #[test]
    fn environment_items_stops_giving_shortcuts_after_nine() {
        let names: Vec<String> = (1..=10).map(|number| format!("env{number}")).collect();
        let items = environment_items(&names);
        // Index 0 is "No environment"; the 10th named environment is index 10.
        assert!(items[9].shortcut.is_some());
        assert!(items[10].shortcut.is_none());
    }

    #[test]
    fn git_actions_follow_the_repository() {
        assert!(git_action_items(GitPalette::Unavailable).is_empty());
        let init = git_action_items(GitPalette::NotARepo);
        assert_eq!(init.len(), 1);
        assert_eq!(init[0].kind, PaletteItemKind::Action(ActionId::GitInit));
        let repo = git_action_items(GitPalette::Repo);
        assert_eq!(repo.len(), 8);
        assert!(
            !repo
                .iter()
                .any(|item| item.kind == PaletteItemKind::Action(ActionId::GitInit))
        );
    }

    #[test]
    fn action_items_cover_every_action_id() {
        let items = action_items();
        assert_eq!(items.len(), 20);
        assert!(
            items
                .iter()
                .any(|item| item.kind == PaletteItemKind::Action(ActionId::Send))
        );
        assert!(
            items
                .iter()
                .any(|item| item.kind == PaletteItemKind::Action(ActionId::ToggleTheme))
        );
        let shortcuts = items
            .iter()
            .find(|item| item.kind == PaletteItemKind::Action(ActionId::OpenShortcuts))
            .expect("the shortcuts action");
        assert_eq!(shortcuts.shortcut, Some(open_hint()));
    }

    #[test]
    fn empty_query_matches_every_item_in_order() {
        let items = vec![item("first"), item("second"), item("third")];
        let matches = fuzzy_filter("", &items);
        let indices: Vec<usize> = matches.iter().map(|(index, _, _)| *index).collect();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn excludes_items_with_no_subsequence_match() {
        let items = vec![item("auth/login"), item("users/list")];
        let matches = fuzzy_filter("zzz", &items);
        assert!(matches.is_empty());
    }

    #[test]
    fn matching_is_case_insensitive() {
        let items = vec![item("Auth/Login")];
        let matches = fuzzy_filter("LOGIN", &items);
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn matching_folds_accented_capitals() {
        let items = vec![item("\u{d3}rdenes")];
        assert_eq!(fuzzy_filter("\u{f3}r", &items).len(), 1);
    }

    #[test]
    fn action_labels_follow_the_language() {
        let spanish = t!("shell.palette.send", locale = "es");
        assert_eq!(spanish, "Enviar petici\u{f3}n");
        assert_eq!(t!("shell.palette.send", locale = "en"), "Send request");
    }

    #[test]
    fn ranks_a_tight_word_start_match_above_a_looser_one() {
        // "lgn" should rank "auth/login" (a tight match right after a word boundary) above
        // "legal notice" (a wider match spanning a word gap).
        let items = vec![item("legal notice"), item("auth/login")];
        let matches = fuzzy_filter("lgn", &items);
        let ranked_labels: Vec<&str> = matches
            .iter()
            .map(|(index, _, _)| items[*index].label.as_str())
            .collect();
        assert_eq!(ranked_labels, vec!["auth/login", "legal notice"]);
    }

    #[test]
    fn reports_the_matched_char_indices() {
        let (_, indices) = fuzzy_match("gt", "get").expect("subsequence match");
        assert_eq!(indices, vec![0, 2]);
    }
}
