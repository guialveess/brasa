//! The load test tab: left config panel (`views/load_test/config_panel.rs`), right dashboard
//! (`views/load_test/dashboard.rs`), and starting/stopping/finishing the actual
//! `brasa_load::LoadRun` in the background (`views/load_test/run.rs`). This file owns opening a
//! tab (from the command palette, a sidebar context menu, or the `BRASA_OPEN=loadtest` debug
//! hook), its tab-strip label, and the single `edit_load_test` helper every other file in this
//! module funnels edits through, mirroring `views/request_editor.rs`'s `edit_active_request`.

pub(crate) mod config_panel;
pub(crate) mod dashboard;
pub(crate) mod errors_tab;
pub(crate) mod export;
pub(crate) mod latency_tab;
pub(crate) mod requests_tab;
pub(crate) mod results_data;
pub(crate) mod run;
pub(crate) mod summary;

pub(crate) use config_panel::LoadTestEntities;

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::state::TabKind;
use brasa_load::ThresholdMetric;

use crate::state::load_results::{
    ChartSeriesKey, RequestColumn, RequestFilter, ResultsTab, StatusFilter,
};
use crate::state::load_test::{self, LatencyChoice, LoadTestTab, LoadTestTarget};

use super::root::AppView;

/// The open-tabs bar label for a load test tab: `"Load test"` before a target is picked, `"Load
/// test \u{b7} <name>"` once one is.
#[must_use]
pub(crate) fn tab_label(load_test: &LoadTestTab) -> String {
    if load_test.target_label.is_empty() {
        t!("load_test.tab.title").into_owned()
    } else {
        format!(
            "{} \u{b7} {}",
            t!("load_test.tab.title"),
            load_test.target_label
        )
    }
}

impl AppView {
    /// Opens a fresh load test tab with no preselected target (the command palette's "New load
    /// test" action).
    pub(crate) fn open_new_load_test_tab(&mut self, cx: &mut Context<Self>) {
        self.open_load_test_tab(LoadTestTab::unset(), cx);
    }

    /// Opens a load test tab preselecting the single request `id` (a sidebar request row's "Load
    /// test..." menu item).
    pub(crate) fn open_load_test_for_request(&mut self, id: String, cx: &mut Context<Self>) {
        let label = self
            .state
            .workspace
            .as_ref()
            .and_then(|workspace| load_test::find_request_name(workspace.tree(), &id))
            .unwrap_or_else(|| crate::state::format::tab_label(&id).to_string());
        self.open_load_test_tab(LoadTestTab::for_request(id, label), cx);
    }

    /// Opens a load test tab preselecting every request under the folder `id` (a sidebar folder
    /// row's "Load test..." menu item).
    pub(crate) fn open_load_test_for_collection(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let Some((label, count)) = load_test::find_folder(workspace.tree(), &id) else {
            return;
        };
        self.open_load_test_tab(LoadTestTab::for_collection(id, label, count), cx);
    }

    /// `BRASA_OPEN=loadtest`: opens a load test tab preselecting the first request found in the
    /// workspace tree, for screenshotting the load test config
    /// panel. A no-op when no workspace is open or it has no requests.
    pub(crate) fn open_load_test_tab_for_first_request(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let Some((id, _label)) = load_test::requests_flat(workspace.tree())
            .into_iter()
            .next()
        else {
            return;
        };
        self.open_load_test_for_request(id, cx);
    }

    /// Opens `load_test` as a new tab, makes it active, and loads its target's run history.
    fn open_load_test_tab(&mut self, load_test: LoadTestTab, cx: &mut Context<Self>) {
        let (id, _index) = self.state.tabs.open_load_test(load_test);
        self.refresh_load_test_history(&id, cx);
        cx.notify();
    }

    /// Applies `f` to the load test state of the tab with id `tab_id`, then re-renders. The
    /// single place every load test edit funnels through (`views/request_editor.rs`'s
    /// `edit_active_request` is the same pattern for a request tab).
    pub(crate) fn edit_load_test(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut LoadTestTab),
    ) {
        let Some(index) = self.state.tabs.index_of(tab_id) else {
            return;
        };
        let Some(tab) = self.state.tabs.get_mut(index) else {
            return;
        };
        let Some(load_test) = tab.load_test_mut() else {
            return;
        };
        f(load_test);
        cx.notify();
    }

    /// Whether the load test tab `tab_id` currently has a run in progress. Every config-changing
    /// action (target/field edits, picking a compare run) is a no-op while this is true: the
    /// config panel's fields are disabled, but a stale click still in flight (or a shortcut)
    /// should not be able to sneak an edit past that.
    fn load_test_is_running(&self, tab_id: &str) -> bool {
        self.state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.load_test())
            .is_some_and(LoadTestTab::is_running)
    }

    /// Switches the target segmented control between Request and Collection. Clears the current
    /// target if it does not match the newly picked kind, since a request id is not a valid
    /// collection id and vice versa.
    pub(crate) fn set_load_test_target_kind(
        &mut self,
        tab_id: &str,
        kind: load_test::TargetKind,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            if load_test.target_kind == kind {
                return;
            }
            load_test.target_kind = kind;
            let matches_kind = matches!(
                (&load_test.target, kind),
                (
                    Some(LoadTestTarget::Request(_)),
                    load_test::TargetKind::Request
                ) | (
                    Some(LoadTestTarget::Collection(_)),
                    load_test::TargetKind::Collection
                )
            );
            if !matches_kind {
                load_test.target = None;
                load_test.target_label = String::new();
                load_test.target_request_count = 0;
            }
        });
    }

    /// Picks a specific target from the config panel's target picker dropdown, and refreshes the
    /// run history for it.
    pub(crate) fn pick_load_test_target(
        &mut self,
        tab_id: &str,
        target: LoadTestTarget,
        label: String,
        request_count: usize,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        let kind = match target {
            LoadTestTarget::Request(_) => load_test::TargetKind::Request,
            LoadTestTarget::Collection(_) => load_test::TargetKind::Collection,
        };
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.target_kind = kind;
            load_test.target = Some(target);
            load_test.target_label = label;
            load_test.target_request_count = request_count;
        });
        self.refresh_load_test_history(tab_id, cx);
    }

    /// Sets the "Virtual users" field's raw text.
    pub(crate) fn set_load_test_vus(
        &mut self,
        tab_id: &str,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| load_test.config.vus = value);
    }

    /// Sets the "Duration" field's raw text, in seconds.
    pub(crate) fn set_load_test_duration_secs(
        &mut self,
        tab_id: &str,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.config.duration_secs = value
        });
    }

    /// Sets the "Ramp-up" field's raw text, in seconds.
    pub(crate) fn set_load_test_ramp_up_secs(
        &mut self,
        tab_id: &str,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.config.ramp_up_secs = value
        });
    }

    /// Sets the "Think time" field's raw text, in milliseconds.
    pub(crate) fn set_load_test_think_time_ms(
        &mut self,
        tab_id: &str,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.config.think_time_ms = value
        });
    }

    /// Toggles the "Stop on errors" switch.
    pub(crate) fn set_load_test_stop_on_error(
        &mut self,
        tab_id: &str,
        checked: bool,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.config.stop_on_error = checked
        });
    }

    /// Toggles the "Reuse connections" switch.
    pub(crate) fn set_load_test_reuse_connections(
        &mut self,
        tab_id: &str,
        checked: bool,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.config.reuse_connections = checked
        });
    }

    /// Adds a "Thresholds" row for `metric`, seeded with its default limit. A no-op when the
    /// metric already has one.
    pub(crate) fn add_load_test_threshold(
        &mut self,
        tab_id: &str,
        metric: ThresholdMetric,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            if load_test
                .config
                .thresholds
                .iter()
                .all(|input| input.metric != metric)
            {
                load_test
                    .config
                    .thresholds
                    .push(load_test::ThresholdInput::new(metric));
            }
        });
    }

    /// Removes the "Thresholds" row for `metric`.
    pub(crate) fn remove_load_test_threshold(
        &mut self,
        tab_id: &str,
        metric: ThresholdMetric,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test
                .config
                .thresholds
                .retain(|input| input.metric != metric);
        });
    }

    /// Sets the limit text of the "Thresholds" row for `metric`.
    pub(crate) fn set_load_test_threshold_limit(
        &mut self,
        tab_id: &str,
        metric: ThresholdMetric,
        value: String,
        cx: &mut Context<Self>,
    ) {
        if self.load_test_is_running(tab_id) {
            return;
        }
        self.edit_load_test(tab_id, cx, |load_test| {
            if let Some(input) = load_test
                .config
                .thresholds
                .iter_mut()
                .find(|input| input.metric == metric)
            {
                input.limit = value;
            }
        });
    }

    /// Picks a run from the "Compare with" dropdown and loads its full snapshot.
    pub(crate) fn set_load_test_compare_with(
        &mut self,
        tab_id: &str,
        number: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        self.edit_load_test(tab_id, cx, |load_test| load_test.compare_with = number);
        self.refresh_load_test_compare_snapshot(tab_id, cx);
    }

    /// Reloads [`LoadTestTab::compare_snapshot`] from disk for whatever run
    /// [`LoadTestTab::compare_with`] currently points at, so the "Compare with" panel never reads
    /// a run file from render code (`state/load_test.rs`'s doc comment on that field). `None`
    /// when there is nothing to compare against, or the file fails to load.
    pub(crate) fn refresh_load_test_compare_snapshot(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
    ) {
        let root = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf());
        let compare_with = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.load_test())
            .and_then(|load_test| load_test.compare_with);
        let snapshot = match (root, compare_with) {
            (Some(root), Some(number)) => brasa_load::history::load(&root, number)
                .ok()
                .map(|record| record.snapshot),
            _ => None,
        };
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.compare_snapshot = snapshot;
            load_test.refresh_chart();
        });
        self.load_load_test_compare_samples(tab_id, cx);
        self.refresh_load_results(tab_id, cx);
    }

    /// Loads a past run and shows it in the dashboard, as if it had just finished
    /// (the history list's "click to view"). A no-op while
    /// a run is in progress for this tab, so clicking a history row can never clobber a live run.
    pub(crate) fn view_load_test_history(
        &mut self,
        tab_id: &str,
        number: u32,
        cx: &mut Context<Self>,
    ) {
        if self.load_runs.contains_key(tab_id) {
            return;
        }
        let Some(root) = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf())
        else {
            return;
        };
        let Ok(record) = brasa_load::history::load(&root, number) else {
            return;
        };
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.run_number = Some(record.number);
            load_test.status = if record.stopped_early {
                load_test::LoadTestStatus::Stopped
            } else {
                load_test::LoadTestStatus::Finished
            };
            // `Method::from_str` is infallible (an unrecognized token becomes `Method::Custom`),
            // so `unwrap_or_default` never actually falls back; it just avoids an `unwrap()` for
            // a `Result` that cannot be `Err`. Empty on a run saved before `target_labels`
            // existed (`brasa_load::history::RunRecord`'s own doc comment), which is exactly
            // the "unknown" shape `target_row_label` already falls back to.
            load_test.target_rows = record
                .target_labels
                .iter()
                .map(|(method, name)| (method.parse().unwrap_or_default(), name.clone()))
                .collect();
            load_test.run_reuse_connections = Some(record.config.reuse_connections);
            load_test.run_duration_secs = Some(record.config.duration_secs);
            load_test.run_thresholds = record.thresholds.clone();
            load_test.run_vus = Some(record.config.vus);
            load_test.run_ramp_up_secs = Some(record.config.ramp_up_secs);
            load_test.run_started_at = Some(record.started_at_unix);
            load_test.snapshot = Some(record.snapshot);
            load_test.refresh_chart();
            load_test.log_unavailable = false;
            load_test.replace_log(None);
            load_test.compare_with = load_test
                .history
                .iter()
                .find(|header| header.number != record.number)
                .map(|header| header.number);
        });
        self.refresh_load_test_compare_snapshot(tab_id, cx);
        self.load_load_test_samples(tab_id, number, cx);
    }

    /// Shows the results tab `results_tab`.
    pub(crate) fn set_load_test_results_tab(
        &mut self,
        tab_id: &str,
        results_tab: ResultsTab,
        cx: &mut Context<Self>,
    ) {
        self.edit_load_test(tab_id, cx, |load_test| load_test.results_tab = results_tab);
        self.refresh_load_results(tab_id, cx);
    }

    /// Changes the Latency tab's controls with `edit` and computes its data again.
    pub(crate) fn edit_load_test_latency(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut LatencyChoice),
    ) {
        self.latency_hover = None;
        self.edit_load_test(tab_id, cx, |load_test| edit(&mut load_test.latency_options));
        self.refresh_load_results(tab_id, cx);
    }

    /// Selects the chart window `window` (inclusive second indices), or the whole run.
    pub(crate) fn set_load_test_window(
        &mut self,
        tab_id: &str,
        window: Option<(usize, usize)>,
        cx: &mut Context<Self>,
    ) {
        self.edit_load_test(tab_id, cx, |load_test| load_test.window = window);
        self.refresh_load_results(tab_id, cx);
    }

    /// Shows or hides one chart series from the legend.
    pub(crate) fn toggle_load_test_series(
        &mut self,
        tab_id: &str,
        key: ChartSeriesKey,
        cx: &mut Context<Self>,
    ) {
        self.edit_load_test(tab_id, cx, |load_test| {
            if let Some(index) = load_test
                .hidden_series
                .iter()
                .position(|hidden| *hidden == key)
            {
                load_test.hidden_series.remove(index);
            } else {
                load_test.hidden_series.push(key);
            }
        });
    }

    /// Applies `f` to the Requests tab's filters, then recomputes its rows.
    pub(crate) fn edit_load_test_request_filter(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut RequestFilter),
    ) {
        self.edit_load_test(tab_id, cx, |load_test| f(&mut load_test.request_filter));
        self.refresh_load_results(tab_id, cx);
    }

    /// Sorts the Requests tab by `column`, flipping the direction when it already is.
    pub(crate) fn sort_load_test_requests(
        &mut self,
        tab_id: &str,
        column: RequestColumn,
        cx: &mut Context<Self>,
    ) {
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.request_sort = load_test.request_sort.toggled(column);
        });
        self.refresh_load_results(tab_id, cx);
    }

    /// The Errors tab's "View requests": the Requests tab filtered to `status`'s kind and to
    /// errors only.
    pub(crate) fn view_load_test_error_requests(
        &mut self,
        tab_id: &str,
        status: brasa_load::StatusKey,
        cx: &mut Context<Self>,
    ) {
        self.edit_load_test(tab_id, cx, |load_test| {
            load_test.results_tab = ResultsTab::Requests;
            load_test.request_filter.status = StatusFilter::for_status(status);
            load_test.request_filter.only_errors = true;
        });
        self.refresh_load_results(tab_id, cx);
    }

    /// Renders the active tab's load test view: the config panel on the left, the dashboard on the
    /// right. Only called while the active tab is a [`TabKind::LoadTest`]
    /// (`views/root.rs::render_main_area`).
    pub(crate) fn render_load_test_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.state.tabs.active() else {
            return div().into_any_element();
        };
        if !matches!(tab.kind, TabKind::LoadTest(_)) {
            return div().into_any_element();
        }
        let tab_id = tab.id.clone();

        h_flex()
            .size_full()
            .min_h_0()
            .child(self.render_load_test_config_panel(&tab_id, window, cx))
            .child(self.render_load_test_dashboard(&tab_id, window, cx))
            .into_any_element()
    }
}
