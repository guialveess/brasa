//! Action definitions used as `gpui` key bindings and menu handlers.
//!
//! `gpui`'s `actions!` macro (re-exported by `gpui-kit`) turns each name below into a small unit
//! struct that implements `gpui::Action`, so it can be bound to a key combination with
//! `KeyBinding::new` and matched with `on_action`. The first argument is just a namespace label
//! used in debug output, it does not need to refer to a real module.

use gpui_kit::actions;

actions!(
    brasa,
    [
        /// Saves the active tab's request to disk (`Ctrl+S` / `Cmd+S`).
        SaveActiveTab,
        /// Sends the active tab's request (`Ctrl+Enter` / `Cmd+Enter`).
        SendActiveTab,
        /// Runs the default commit action of Settings from the commit message (`Ctrl+Enter` /
        /// `Cmd+Enter` while it has the focus).
        CommitFromMessage,
        /// Selects the workspace's 1st environment (`Ctrl 1` / `Cmd 1`).
        SelectEnvironment1,
        /// Selects the workspace's 2nd environment (`Ctrl 2` / `Cmd 2`).
        SelectEnvironment2,
        /// Selects the workspace's 3rd environment (`Ctrl 3` / `Cmd 3`).
        SelectEnvironment3,
        /// Selects the workspace's 4th environment (`Ctrl 4` / `Cmd 4`).
        SelectEnvironment4,
        /// Selects the workspace's 5th environment (`Ctrl 5` / `Cmd 5`).
        SelectEnvironment5,
        /// Selects the workspace's 6th environment (`Ctrl 6` / `Cmd 6`).
        SelectEnvironment6,
        /// Selects the workspace's 7th environment (`Ctrl 7` / `Cmd 7`).
        SelectEnvironment7,
        /// Selects the workspace's 8th environment (`Ctrl 8` / `Cmd 8`).
        SelectEnvironment8,
        /// Selects the workspace's 9th environment (`Ctrl 9` / `Cmd 9`).
        SelectEnvironment9,
        /// Selects "No environment" (`Ctrl 0` / `Cmd 0`).
        SelectNoEnvironment,
        /// Moves the environment of the active environment tab one place up (`Alt Up`, #216).
        MoveEnvironmentUp,
        /// Moves the environment of the active environment tab one place down (`Alt Down`).
        MoveEnvironmentDown,
        /// Opens or closes the Collections panel, like its rail button (`Alt 1`, `Ctrl 1` on
        /// macOS).
        ToggleCollections,
        /// Opens or closes the Source Control panel, like its rail button (`Alt 2`, `Ctrl 2` on
        /// macOS).
        ToggleSourceControl,
        /// Opens or closes the Environments panel, like its rail button (`Alt 3`, `Ctrl 3` on
        /// macOS).
        ToggleEnvironments,
        /// Opens or closes the Load tests panel, like its rail button (`Alt 4`, `Ctrl 4` on
        /// macOS).
        ToggleLoadTests,
        /// Opens the Settings modal (`Ctrl ,` / `Cmd ,`).
        OpenSettings,
        /// Opens the command palette (`Ctrl K` / `Cmd K`).
        OpenCommandPalette,
        /// Opens the native folder picker to open a workspace (`Ctrl O` / `Cmd O`), like the
        /// workspace switcher's "Open folder..." item.
        OpenWorkspace,
        /// Opens the OpenAPI import wizard (`Ctrl+Shift+O` / `Cmd+Shift+O`), like the Import
        /// menu's "OpenAPI / Swagger..." item.
        ImportOpenApi,
        /// Opens the keyboard shortcuts cheat sheet (`F1`, or `Cmd+Shift+/` on macOS).
        OpenShortcuts,
        /// Closes the active tab (`Ctrl+W` / `Cmd+W`), asking first when it has unsaved
        /// environment edits like the tab's close button.
        CloseActiveTab,
        /// Activates the next open tab, wrapping around (`Ctrl+Tab`).
        NextTab,
        /// Activates the previous open tab, wrapping around (`Ctrl+Shift+Tab`).
        PreviousTab,
        /// Asks to delete the request or folder selected in the sidebar (`Delete`, only while
        /// the sidebar tree has the focus).
        DeleteSelectedEntry,
        /// Opens the rename dialog of the request or folder selected in the sidebar (`F2`, only
        /// while the sidebar tree has the focus).
        RenameSelectedEntry,
        /// Opens the autocompletion menu (variables, header names and values) in the focused URL
        /// or key-value field (`Ctrl+Space`).
        ShowCompletion,
        /// Highlights the next row of the open autocompletion menu (`Down`).
        NextCompletion,
        /// Highlights the previous row of the open autocompletion menu (`Up`).
        PreviousCompletion,
        /// Inserts the highlighted row of the open autocompletion menu (`Enter`, `Tab`).
        ConfirmCompletion,
        /// Closes the open autocompletion menu (`Escape`).
        DismissCompletion,
        /// Deletes the lines of the selection in the focused `::: pre` or `::: post` script
        /// editor (`Ctrl+D`, or `Cmd+Shift+K` on macOS, where `Ctrl+D` deletes forward).
        DeleteLine,
        /// Undoes the last edit of the active request tab (`Ctrl+Z` / `Cmd+Z`): anywhere outside
        /// a text input, and in the request editor's single-line fields. The body, script and
        /// docs editors and every other input keep their own undo.
        UndoRequestEdit,
        /// Redoes the last undone edit of the active request tab (`Ctrl+Shift+Z` or `Ctrl+Y`,
        /// `Cmd+Shift+Z` on macOS), where [`UndoRequestEdit`] applies.
        RedoRequestEdit,
        /// Copies the summary of the active load test tab's results to the clipboard as Markdown
        /// (`Ctrl+Shift+C` / `Cmd+Shift+C`), like the Export menu's "Copy summary as Markdown".
        CopyLoadTestSummary,
    ]
);
