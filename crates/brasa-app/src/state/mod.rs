//! Plain Rust application state: everything the app remembers besides what `gpui` itself tracks
//! (focus, layout, animation state, ...). Kept free of `gpui` types so it is unit-tested without
//! a window. `views/` wraps this in `gpui`
//! entities and renders it.

pub mod about;
pub mod auth_edit;
pub mod completion;
pub mod config;
pub mod console;
pub mod debug_open;
pub mod define_variable;
pub mod diff_view;
pub mod edit_history;
pub mod env_color;
pub mod env_edit;
pub mod env_panel;
pub mod format;
pub mod git;
pub mod git_branches;
pub mod git_diff;
pub mod git_log;
pub mod git_sync;
pub mod header_completion;
pub mod html_pretty;
pub mod import_destination;
pub mod json_pretty;
pub mod launch;
pub mod line_edit;
pub mod load_panel;
pub mod load_report;
pub mod load_results;
pub mod load_test;
pub mod locale;
pub mod navigation;
pub mod new_workspace;
pub use telaio_app::number;
pub mod open_queue;
pub mod openapi_import;
pub mod palette;
pub mod request_edit;
pub mod response_render;
pub mod run_log;
pub mod script_completion;
pub mod script_heuristics;
pub mod settings;
pub mod shortcuts;
pub mod sidebar_filter;
mod tabs;
pub mod tree_drop;
pub mod tree_expansion;
pub mod tree_selection;
pub mod ui_tabs;
pub mod update;
pub mod variable_completion;
pub mod variable_hint;
pub mod welcome;
pub mod whats_new;
pub mod window;
pub mod workspace_log;

pub use tabs::{TabKind, TabsState};

use std::sync::Arc;

use brasa_core::Environment;
use brasa_runner::SessionEnv;
use brasa_workspace::Workspace;

use settings::Settings;

/// Everything the app remembers about the current session.
#[derive(Default)]
pub struct AppState {
    /// The open workspace, `None` until a folder has been opened.
    pub workspace: Option<Workspace>,
    /// The requests currently open as tabs.
    pub tabs: TabsState,
    /// The name of the active environment, `None` for "No environment". Change it with
    /// [`Self::set_active_environment`], which also loads its variables.
    pub active_environment: Option<String>,
    /// The active environment's variables as last read from disk (GitHub #79), empty for "No
    /// environment". Render reads this, never the files. Only [`Self::set_active_environment`]
    /// and [`Self::reload_environment`] change it. An `Arc`, so a clone is O(1).
    pub(crate) environment: Arc<Environment>,
    /// Environment overrides scripts have set during this session (the second
    /// variable layer in `docs/format.md`). Carried across requests so, for example, a login token a post script sets with
    /// `env.set` is visible to the next request. The MVP never writes this to disk.
    pub session_env: SessionEnv,
    /// The current appearance settings: theme choice and fonts. Loaded once at startup (`main.rs`)
    /// and from then on only changed by the Settings view, which also persists every change to
    /// `settings.toml`.
    pub settings: Settings,
    /// The in-app updater: the downloaded update waiting for a restart, if any.
    pub update: update::UpdateState,
    /// The colors the user chose for the environments (#217), read with the environment list
    /// (`AppView::refresh_environments`), never in render.
    pub env_colors: env_color::EnvColors,
}

impl AppState {
    /// A state with no workspace open yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the current workspace with `workspace`, already opened (scanned) by the caller.
    ///
    /// Open tabs and the active environment belonged to the previous workspace, so they are
    /// discarded; the session environment is reset too, so a new workspace never sees stale
    /// overrides left over from the last one.
    pub fn set_workspace(&mut self, workspace: Workspace) {
        self.workspace = Some(workspace);
        self.tabs = TabsState::default();
        self.active_environment = None;
        self.environment = Arc::default();
        self.session_env = SessionEnv::new();
        self.env_colors = env_color::EnvColors::default();
    }

    /// Makes `name` the active environment (`None` for "No environment") and loads its variables.
    pub fn set_active_environment(&mut self, name: Option<String>) {
        self.active_environment = name;
        self.reload_environment();
    }

    /// Reads the active environment's variables from disk again. Call when its files may have
    /// changed: the environment editor saved, an import wrote one, a request is about to run.
    /// Never from render. A file that cannot be read loads as an empty environment, as it sends.
    pub fn reload_environment(&mut self) {
        let environment = match (self.workspace.as_ref(), self.active_environment.as_deref()) {
            (Some(workspace), Some(name)) => workspace.load_environment(name).unwrap_or_default(),
            _ => Environment::default(),
        };
        self.environment = Arc::new(environment);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_state_has_no_workspace_and_no_environment() {
        let state = AppState::new();
        assert!(state.workspace.is_none());
        assert!(state.tabs.open_tabs().is_empty());
        assert!(state.active_environment.is_none());
        assert!(state.session_env.as_slice().is_empty());
    }

    #[test]
    fn set_workspace_resets_tabs_environment_and_session_overrides() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = AppState::new();
        state.active_environment = Some("stale".to_string());
        state.session_env.set("token", "stale");

        state.set_workspace(
            Workspace::open(dir.path()).expect("opening an empty folder never fails"),
        );

        assert!(state.workspace.is_some());
        assert!(state.tabs.open_tabs().is_empty());
        assert!(state.active_environment.is_none());
        assert!(state.environment.variables.is_empty());
        assert!(state.session_env.as_slice().is_empty());
    }

    /// A state with a workspace in a fresh folder whose `dev` environment has `host`.
    fn state_with_dev_environment(dir: &std::path::Path) -> AppState {
        let mut state = AppState::new();
        state.set_workspace(Workspace::open(dir).expect("open"));
        write_dev_environment(dir, "host=a.test\n");
        state
    }

    fn write_dev_environment(dir: &std::path::Path, text: &str) {
        let environments = dir.join("environments");
        std::fs::create_dir_all(&environments).expect("create environments");
        std::fs::write(environments.join("dev.env"), text).expect("write dev.env");
    }

    fn host(state: &AppState) -> Option<String> {
        state
            .environment
            .variables
            .iter()
            .find(|variable| variable.key == "host")
            .map(|variable| variable.value.clone())
    }

    #[test]
    fn setting_the_active_environment_loads_its_variables() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = state_with_dev_environment(dir.path());

        state.set_active_environment(Some("dev".to_string()));
        assert_eq!(host(&state).as_deref(), Some("a.test"));

        state.set_active_environment(None);
        assert!(state.environment.variables.is_empty());
    }

    #[test]
    fn the_variables_stay_as_loaded_until_reloaded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = state_with_dev_environment(dir.path());
        state.set_active_environment(Some("dev".to_string()));

        write_dev_environment(dir.path(), "host=b.test\n");
        assert_eq!(host(&state).as_deref(), Some("a.test"));

        state.reload_environment();
        assert_eq!(host(&state).as_deref(), Some("b.test"));
    }

    #[test]
    fn a_missing_environment_loads_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = state_with_dev_environment(dir.path());

        state.set_active_environment(Some("gone".to_string()));

        assert_eq!(state.active_environment.as_deref(), Some("gone"));
        assert!(state.environment.variables.is_empty());
    }

    #[test]
    fn set_workspace_drops_the_loaded_variables() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut state = state_with_dev_environment(dir.path());
        state.set_active_environment(Some("dev".to_string()));

        let other = tempfile::tempdir().expect("tempdir");
        state.set_workspace(Workspace::open(other.path()).expect("open"));

        assert!(state.environment.variables.is_empty());
    }
}
