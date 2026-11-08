//! The shortcuts listed in the keyboard shortcuts cheat sheet, grouped by area, with their
//! translated labels and the keys to show as chips. It mirrors `main.rs`'s `bind_keys`, so a new
//! key binding gets a row here, plus the sidebar's Ctrl+click. telaio's
//! `open_shortcuts_dialog` renders it; this module is plain data, unit tested without a window.

use rust_i18n::t;
use telaio_ui::components::{MODIFIER_KEY, Shortcut, ShortcutGroup};

/// The keys that delete the line in a script editor: `Ctrl+D`, or `Cmd+Shift+K` on macOS, where
/// `Ctrl+D` deletes forward in every text field.
fn delete_line_keys() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["Cmd", "Shift", "K"]
    } else {
        &["Ctrl", "D"]
    }
}

/// The modifier of the shortcuts that open and close the sidebar panels, matching `main.rs`:
/// `Ctrl` on macOS, where Option + a digit types characters, `Alt` elsewhere.
/// The modifier that moves an environment up or down, matching `main.rs`'s `alt-up`: `Option`
/// on macOS, `Alt` elsewhere.
#[cfg(target_os = "macos")]
const MOVE_MODIFIER_KEY: &str = "Option";
#[cfg(not(target_os = "macos"))]
const MOVE_MODIFIER_KEY: &str = "Alt";

#[cfg(target_os = "macos")]
const PANEL_MODIFIER_KEY: &str = "Ctrl";
#[cfg(not(target_os = "macos"))]
const PANEL_MODIFIER_KEY: &str = "Alt";

/// Every shortcut, grouped. Built once when the cheat sheet opens.
pub fn groups() -> Vec<ShortcutGroup> {
    let modifier = MODIFIER_KEY;
    vec![
        ShortcutGroup {
            title: t!("shortcuts.group.general").to_string(),
            shortcuts: vec![
                Shortcut::new(t!("shortcuts.action.palette"), &[modifier, "K"]),
                Shortcut::new(t!("shortcuts.action.open_folder"), &[modifier, "O"]),
                Shortcut::new(
                    t!("shortcuts.action.import_openapi"),
                    &[modifier, "Shift", "O"],
                ),
                Shortcut::new(t!("shortcuts.action.settings"), &[modifier, ","]),
                Shortcut::open_sheet(),
                Shortcut::new(t!("shortcuts.action.close_tab"), &[modifier, "W"]),
                Shortcut::new(t!("shortcuts.action.next_tab"), &["Ctrl", "Tab"]),
                Shortcut::new(
                    t!("shortcuts.action.previous_tab"),
                    &["Ctrl", "Shift", "Tab"],
                ),
            ],
        },
        ShortcutGroup {
            title: t!("shortcuts.group.requests").to_string(),
            shortcuts: vec![
                Shortcut::new(t!("shortcuts.action.send"), &[modifier, "\u{21b5}"]),
                Shortcut::new(t!("shortcuts.action.save"), &[modifier, "S"]),
                // Ctrl on macOS too: Cmd+Space is Spotlight.
                Shortcut::new(t!("shortcuts.action.complete_variable"), &["Ctrl", "Space"]),
                Shortcut::new(t!("shortcuts.action.delete_line"), delete_line_keys()),
                Shortcut::new(t!("shortcuts.action.undo"), &[modifier, "Z"]),
                Shortcut::new(t!("shortcuts.action.redo"), &[modifier, "Shift", "Z"]),
            ],
        },
        ShortcutGroup {
            title: t!("shortcuts.group.sidebar").to_string(),
            shortcuts: vec![
                Shortcut::new(
                    t!("shortcuts.action.toggle_collections"),
                    &[PANEL_MODIFIER_KEY, "1"],
                ),
                Shortcut::new(
                    t!("shortcuts.action.toggle_source_control"),
                    &[PANEL_MODIFIER_KEY, "2"],
                ),
                Shortcut::new(
                    t!("shortcuts.action.toggle_environments"),
                    &[PANEL_MODIFIER_KEY, "3"],
                ),
                Shortcut::new(
                    t!("shortcuts.action.toggle_load_tests"),
                    &[PANEL_MODIFIER_KEY, "4"],
                ),
                Shortcut::new(t!("shortcuts.action.rename"), &["F2"]),
                Shortcut::new(t!("shortcuts.action.delete"), &["Delete"]),
                Shortcut::new(
                    t!("shortcuts.action.pick"),
                    &[modifier, &t!("shortcuts.key.click")],
                ),
            ],
        },
        ShortcutGroup {
            title: t!("shortcuts.group.environments").to_string(),
            shortcuts: vec![
                Shortcut::new(
                    t!("shortcuts.action.select_environment"),
                    &[modifier, "1-9"],
                ),
                Shortcut::new(t!("shortcuts.action.no_environment"), &[modifier, "0"]),
                Shortcut::new(
                    t!("shortcuts.action.move_environment"),
                    &[MOVE_MODIFIER_KEY, "\u{2191}/\u{2193}"],
                ),
            ],
        },
        ShortcutGroup {
            title: t!("shortcuts.group.load_tests").to_string(),
            shortcuts: vec![Shortcut::new(
                t!("shortcuts.action.copy_load_summary"),
                &[modifier, "Shift", "C"],
            )],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_group_and_shortcut_has_text_and_keys() {
        let groups = groups();
        assert_eq!(groups.len(), 5);
        for group in &groups {
            assert!(!group.title.is_empty());
            assert!(!group.shortcuts.is_empty());
            for shortcut in &group.shortcuts {
                assert!(!shortcut.label.is_empty());
                assert!(!shortcut.keys.is_empty());
            }
        }
    }

    #[test]
    fn no_two_shortcuts_share_the_same_keys() {
        let mut seen = std::collections::HashSet::new();
        for group in groups() {
            for shortcut in group.shortcuts {
                assert!(
                    seen.insert(shortcut.keys.join("+")),
                    "duplicate keys for {:?}",
                    shortcut.label
                );
            }
        }
    }

    #[test]
    fn the_cheat_sheet_lists_its_own_keys() {
        let general = &groups()[0];
        assert!(
            general
                .shortcuts
                .iter()
                .any(|shortcut| *shortcut == Shortcut::open_sheet())
        );
    }
}
