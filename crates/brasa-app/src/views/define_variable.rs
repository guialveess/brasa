//! Defining unknown variables from a request: a click on a red chip (`views/request_editor.rs`)
//! or the response pane's "Define" warning action (`views/response_view.rs`). There is no
//! dialog: with an active environment the user lands on its editor tab, on the variable's row
//! (added when missing), and without one on the Environments panel to pick or create one. The
//! decision lives in `state::define_variable`.

use gpui_kit::*;

use crate::state::define_variable::{self, DefineTarget};
use crate::state::navigation::NavSection;

use super::root::AppView;

impl AppView {
    /// Takes the user to where `names` can be defined. A no-op without a workspace or names.
    pub(crate) fn define_variables(
        &mut self,
        names: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if names.is_empty() {
            return;
        }
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let environments = workspace.list_environments().unwrap_or_default();
        match define_variable::target(self.state.active_environment.as_deref(), &environments) {
            DefineTarget::ChooseEnvironment => {
                log::debug!("define variable: no active environment, showing the panel");
                if !self.nav.is_active(NavSection::Environments) {
                    self.click_rail(NavSection::Environments, cx);
                }
            }
            DefineTarget::Environment(name) => {
                log::debug!("define variable: {} name(s) in {name}", names.len());
                self.open_env_tab_for_variables(name, &names, window, cx);
            }
        }
    }
}
