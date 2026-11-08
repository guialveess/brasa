//! Sending the active tab's request through `brasa-runner` on `gpui`'s background executor.
//!
//! The pipeline itself (pre script, interpolation, HTTP, post script) is exactly
//! [`brasa_runner::Runner::run`]; this module's only job is to run it off the main thread so
//! the UI stays responsive, show a spinner while it is in flight, and allow cancelling it.

use std::sync::Arc;
use std::time::Instant;

use gpui_kit::*;
use brasa_core::Environment;
use brasa_runner::{Preview, Runner, preview};

use crate::state::console::ConsoleTimeline;
use crate::state::run_log;
use crate::state::script_completion::JsonShape;

use super::root::AppView;

/// A request currently being sent: which tab it belongs to, and the background [`Task`] driving
/// it.
///
/// Dropping the `Task` cancels the pipeline immediately: `gpui-pre`'s `Task` is documented as
/// "If you drop a task it will be cancelled immediately", unlike `.detach()`, which lets it run
/// to completion. [`AppView::cancel_send`] relies on exactly this by replacing this struct (and
/// therefore its `Task`) with `None`: cancelling drops the result.
pub(crate) struct SendingTask {
    /// The id of the tab whose request is being sent.
    pub(crate) tab_id: String,
    /// Kept alive only so it is not dropped (and cancelled) while still in flight; never polled
    /// directly, its result is delivered through the `view.update` call inside
    /// [`AppView::send_active_tab`]'s spawned future.
    _task: Task<()>,
}

impl AppView {
    /// The environment to send with: the active environment's variables as last loaded
    /// (`AppState::environment`, an O(1) clone), or an empty [`Environment`] for "No environment" or when no
    /// workspace is open. Never reads the files, so render can call it; whatever runs a request
    /// calls `AppState::reload_environment` first, to see edits made outside Brasa.
    pub(crate) fn active_environment(&self) -> Arc<Environment> {
        self.state.environment.clone()
    }

    /// The active tab's request resolved against the active environment and the session
    /// environment, without running its scripts or sending it. `None` when no tab is open. Cheap
    /// enough to call on every render: it only interpolates `{{ }}` markers, it never runs a script
    /// or opens a socket.
    pub(crate) fn current_preview(&self) -> Option<Preview> {
        let tab = self.state.tabs.active()?;
        let request = tab.request()?;
        let environment = self.active_environment();
        Some(preview(request, &environment, &self.state.session_env))
    }

    /// Whether `tab_id` currently has a request in flight.
    pub(crate) fn is_sending(&self, tab_id: &str) -> bool {
        self.sending
            .as_ref()
            .is_some_and(|task| task.tab_id == tab_id)
    }

    /// Sends the active tab's request on the background executor. Replacing
    /// [`AppView::sending`] (done here unconditionally) drops any request already in flight,
    /// cancelling it, so starting a new send also acts as "cancel the previous one".
    pub(crate) fn send_active_tab(&mut self, cx: &mut Context<Self>) {
        // The files may have changed outside Brasa since they were last read.
        self.state.reload_environment();
        let Some(tab) = self.state.tabs.active() else {
            return;
        };
        let Some(request) = tab.request().cloned() else {
            return;
        };
        let tab_id = tab.id.clone();
        let environment = self.active_environment();
        let engine = self.script_engine.clone();
        let options = self.send_options.clone();
        let base_dir = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf());
        let mut session_env = self.state.session_env.clone();

        let task = cx.spawn(async move |this, cx| {
            // The actual pipeline (pre script, interpolation, HTTP, post script) runs on a
            // background thread: `Runner::run` blocks on the HTTP call and can run a script for
            // up to 5 seconds, neither of which should stall the UI thread.
            let (result, session_env, shape, console) = cx
                .background_spawn(async move {
                    let started = Instant::now();
                    let runner = Runner::new(engine, options).with_base_dir(base_dir);
                    let result = runner.run(&request, &environment, &mut session_env);
                    let environment_name =
                        Some(environment.name.as_str()).filter(|name| !name.is_empty());
                    let console = Arc::new(ConsoleTimeline::build(&result, environment_name));
                    log::info!("{}", run_log::summary(&result, started.elapsed()));
                    log::debug!("{}", run_log::details(&result));
                    // Parsed here, once, so completing `res.json().` never touches the body.
                    let shape = result
                        .response
                        .as_ref()
                        .and_then(|response| std::str::from_utf8(&response.body).ok())
                        .and_then(JsonShape::from_body)
                        .map(Arc::new);
                    (result, session_env, shape, console)
                })
                .await;
            // Back on the main thread: fold the script-updated session environment back into
            // the single, per-session `SessionEnv` (one `SessionEnv` per app
            // session), record the result, and clear `sending` unless a newer send has already
            // replaced it (which would mean this one was superseded, not cancelled, since a
            // dropped task never reaches this point at all).
            let _ = this.update(cx, |view, cx| {
                view.state.session_env = session_env;
                view.responses.insert(tab_id.clone(), result);
                view.response_consoles.insert(tab_id.clone(), console);
                match shape {
                    Some(shape) => view.response_shapes.insert(tab_id.clone(), shape),
                    None => view.response_shapes.remove(&tab_id),
                };
                view.response_generations.bump(&tab_id);
                if view.is_sending(&tab_id) {
                    view.sending = None;
                }
                cx.notify();
            });
        });

        self.sending = Some(SendingTask {
            tab_id: tab.id.clone(),
            _task: task,
        });
        cx.notify();
    }

    /// Cancels the request currently in flight, if any, by dropping its `Task`.
    pub(crate) fn cancel_send(&mut self, cx: &mut Context<Self>) {
        if self.sending.take().is_some() {
            log::info!("request cancelled");
        }
        cx.notify();
    }
}
