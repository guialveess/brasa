//! The environment editor tab (GitHub #65): a header with the environment's name and actions, the
//! two files it is stored in, a toolbar (filter, file scope, secrets toggle, add), a table of the
//! variables with inline inputs, and the resolution order. The rows and every decision about
//! them live in `state::env_edit`; this file only draws them and forwards the edits. The input
//! entities are created once per tab (and once per added row), never per frame.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_ui::Tone;
use telaio_ui::components::StatusBadge;

use brasa_core::KeyValue;
use brasa_workspace::EnvLayer;

use crate::state::define_variable;
use crate::state::env_color::EnvColor;
use crate::state::env_edit::{
    EnvEditTab, EnvScope, EnvValidationError, EnvVarRow, RowNote, env_file_label,
};
use crate::state::number::format_integer;
use crate::state::workspace_log::log_workspace_error;
use crate::theme::metrics::{
    EDITABLE_TABLE_ROW_HEIGHT, INPUT_PADDING_X, MODAL_WIDTH_DEFAULT, RADIUS_MD, RADIUS_SM,
};
use crate::theme::{Palette, PaletteExt};
use crate::views::components::NeutralButtonExt;
use crate::views::components::{
    ConfirmDialog, EditableCell, EditableColumn, EditableRow, EditableTable, GhostButton,
    HoverInput, IconButton, InlineMessage, InlineMessageKind, ModalPlacement, PrimaryButton,
    SecondaryButton, SegmentedControl, SegmentedItem, edit_menu, framed_dialog, modal_body,
    modal_field, modal_footer, modal_header, open_confirm_dialog,
};
use crate::views::env_color_picker::color_picker;

use super::root::AppView;

/// Width of the Name column.
const NAME_COLUMN_WIDTH: f32 = 200.0;
/// Width of the Stored in column.
const STORED_COLUMN_WIDTH: f32 = 190.0;
/// Longest file name, in characters, that fits in a "stored in" chip of that column.
const STORED_CHIP_MAX_CHARS: usize = 22;
/// Longest file name, in characters, of the chip's menu.
const STORED_MENU_MAX_CHARS: usize = 64;
/// Width of the trash column.
const TRASH_COLUMN_WIDTH: f32 = 30.0;
/// Height of the toolbar and header controls.
const CONTROL_HEIGHT: f32 = 28.0;

/// A shared dialog submit handler.
type SubmitHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Which input of a row an edit comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvField {
    Name,
    Value,
}

/// The two inputs of one row.
struct EnvRowInputs {
    name: Entity<InputState>,
    value: Entity<InputState>,
}

/// The `gpui` entities of one environment editor tab.
pub(crate) struct EnvEditorEntities {
    filter: Entity<InputState>,
    rows: HashMap<u64, EnvRowInputs>,
    /// The table's scroll position, so a row can be brought into view.
    scroll: ScrollHandle,
    /// A row to scroll into view at the next prepaint of the table, then cleared.
    reveal: Rc<Cell<Option<u64>>>,
}

impl AppView {
    /// Opens the editor tab of the environment `name`, or activates it if it is already open.
    pub(crate) fn open_environment_tab(&mut self, name: String, cx: &mut Context<Self>) {
        self.env_color_picker_open = false;
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        if let Some(index) = self
            .state
            .tabs
            .index_of(&crate::state::env_edit::tab_id(&name))
        {
            self.state.tabs.set_active(index);
            cx.notify();
            return;
        }
        match workspace.load_environment_layers(&name) {
            Ok(layers) => {
                log::debug!("opened environment tab {name}");
                self.state
                    .tabs
                    .open_environment(EnvEditTab::load(&name, &layers));
                self.workspace_error = None;
            }
            Err(error) => {
                log_workspace_error(&format!("open environment {name:?}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Opens the editor tab of the environment `name` on the rows of `keys`, for defining them
    /// from a request: a key the environment already has is shown as it is, a missing one gets a
    /// new row (in `.local.env` when its name looks like a secret). The view scrolls to the first
    /// key and focuses its value. Nothing is saved.
    pub(crate) fn open_env_tab_for_variables(
        &mut self,
        name: String,
        keys: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab_id = crate::state::env_edit::tab_id(&name);
        self.open_environment_tab(name, cx);
        if self.state.tabs.active().is_none_or(|tab| tab.id != tab_id) {
            return;
        }
        self.ensure_env_entities(&tab_id, window, cx);
        // Each key's row id, with what a new row needs for its inputs (its name and mask).
        let mut targets: Vec<(u64, Option<(String, bool)>)> = Vec::new();
        self.edit_env(&tab_id, cx, |edit| {
            for key in keys {
                if let Some(id) = edit.reveal(key) {
                    targets.push((id, None));
                    continue;
                }
                let id = edit.add_variable();
                edit.set_name(id, key);
                if define_variable::looks_sensitive(key) {
                    edit.set_layer(id, EnvLayer::Local);
                }
                let masked = edit
                    .rows
                    .iter()
                    .find(|row| row.id == id)
                    .is_some_and(|row| edit.is_masked(row));
                targets.push((id, Some((key.clone(), masked))));
            }
            true
        });
        let mut new_inputs = Vec::new();
        for (id, new_row) in &targets {
            if let Some((key, masked)) = new_row {
                let inputs = build_row_inputs(&tab_id, *id, key, "", *masked, window, cx);
                new_inputs.push((*id, inputs));
            }
        }
        let Some(entities) = self.env_editors.get_mut(&tab_id) else {
            return;
        };
        entities
            .filter
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        entities.rows.extend(new_inputs);
        let Some(&(first, _)) = targets.first() else {
            return;
        };
        entities.reveal.set(Some(first));
        if let Some(inputs) = entities.rows.get(&first) {
            let value_input = inputs.value.clone();
            value_input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    /// Creates the entities of the tab `tab_id` the first time it is shown.
    fn ensure_env_entities(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.env_editors.contains_key(tab_id) {
            return;
        }
        let Some(edit) = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.environment())
        else {
            return;
        };
        let filter_text = edit.filter.clone();
        let rows: Vec<(u64, String, String, bool)> = edit
            .rows
            .iter()
            .map(|row| {
                (
                    row.id,
                    row.name.clone(),
                    row.value.clone(),
                    edit.is_masked(row),
                )
            })
            .collect();

        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("environments.filter_placeholder"))
        });
        filter.update(cx, |state, cx| state.set_value(filter_text, window, cx));
        {
            let tab_id = tab_id.to_string();
            cx.subscribe(&filter, move |view, entity, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = entity.read(cx).value().to_string();
                    view.set_env_filter(&tab_id, &text, cx);
                }
            })
            .detach();
        }
        let mut entities = EnvEditorEntities {
            filter,
            rows: HashMap::new(),
            scroll: ScrollHandle::new(),
            reveal: Rc::new(Cell::new(None)),
        };
        for (id, name, value, masked) in rows {
            let inputs = build_row_inputs(tab_id, id, &name, &value, masked, window, cx);
            entities.rows.insert(id, inputs);
        }
        self.env_editors.insert(tab_id.to_string(), entities);
    }

    /// Re-applies the translated placeholders of every editor's inputs after a language change.
    pub(crate) fn relocalize_env_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for entities in self.env_editors.values() {
            entities.filter.update(cx, |input, cx| {
                input.set_placeholder(t!("environments.filter_placeholder"), window, cx);
            });
            for inputs in entities.rows.values() {
                inputs.name.update(cx, |input, cx| {
                    input.set_placeholder(t!("environments.name_placeholder"), window, cx);
                });
                inputs.value.update(cx, |input, cx| {
                    input.set_placeholder(t!("common.value"), window, cx);
                });
            }
        }
    }

    /// Applies `f` to the editor state of the tab `tab_id` and, when it reports a change, syncs
    /// the tab's dirty flag and repaints. The one place every edit funnels through.
    fn edit_env(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut EnvEditTab) -> bool,
    ) {
        let Some(index) = self.state.tabs.index_of(tab_id) else {
            return;
        };
        let Some(edit) = self
            .state
            .tabs
            .get_mut(index)
            .and_then(|tab| tab.environment_mut())
        else {
            return;
        };
        if f(edit) {
            let dirty = edit.is_dirty();
            self.state.tabs.set_dirty(index, dirty);
            cx.notify();
        }
    }

    fn edit_env_row(
        &mut self,
        tab_id: &str,
        row_id: u64,
        field: EnvField,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        self.edit_env(tab_id, cx, |edit| match field {
            EnvField::Name => edit.set_name(row_id, text),
            EnvField::Value => edit.set_value(row_id, text),
        });
    }

    fn set_env_filter(&mut self, tab_id: &str, text: &str, cx: &mut Context<Self>) {
        let Some(edit) = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get_mut(index))
            .and_then(|tab| tab.environment_mut())
        else {
            return;
        };
        edit.set_filter(text);
        cx.notify();
    }

    /// Appends an empty row stored in `.env` to the active environment tab and focuses its name.
    fn add_env_variable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self
            .state
            .tabs
            .active()
            .filter(|tab| tab.environment().is_some())
            .map(|tab| tab.id.clone())
        else {
            return;
        };
        self.ensure_env_entities(&tab_id, window, cx);
        let mut new_id = None;
        self.edit_env(&tab_id, cx, |edit| {
            new_id = Some(edit.add_variable());
            true
        });
        let Some(id) = new_id else {
            return;
        };
        let inputs = build_row_inputs(&tab_id, id, "", "", false, window, cx);
        let name_input = inputs.name.clone();
        if let Some(entities) = self.env_editors.get_mut(&tab_id) {
            entities
                .filter
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
            entities.rows.insert(id, inputs);
            entities.reveal.set(Some(id));
        }
        name_input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn remove_env_row(&mut self, tab_id: &str, row_id: u64, cx: &mut Context<Self>) {
        self.edit_env(tab_id, cx, |edit| {
            edit.remove(row_id);
            true
        });
        if let Some(entities) = self.env_editors.get_mut(tab_id) {
            entities.rows.remove(&row_id);
        }
    }

    /// Moves a row to the other file and updates its mask.
    fn move_env_row(
        &mut self,
        tab_id: &str,
        row_id: u64,
        layer: EnvLayer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_env(tab_id, cx, |edit| {
            edit.set_layer(row_id, layer);
            true
        });
        self.apply_env_masks(tab_id, window, cx);
    }

    fn toggle_env_secrets(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get_mut(index))
            .and_then(|tab| tab.environment_mut())
        else {
            return;
        };
        edit.toggle_secrets();
        self.apply_env_masks(tab_id, window, cx);
        cx.notify();
    }

    /// Sets the masked flag of every value input to what the editor state says.
    fn apply_env_masks(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.environment())
        else {
            return;
        };
        let Some(entities) = self.env_editors.get(tab_id) else {
            return;
        };
        for row in &edit.rows {
            if let Some(inputs) = entities.rows.get(&row.id) {
                let masked = edit.is_masked(row);
                inputs
                    .value
                    .update(cx, |input, cx| input.set_masked(masked, window, cx));
            }
        }
    }

    /// Saves the environment tab at `index`: validates, writes both files keeping everything the
    /// edits did not touch, then marks the tab clean and refreshes the panel.
    pub(crate) fn save_env_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(edit) = self
            .state
            .tabs
            .get_mut(index)
            .and_then(|tab| tab.environment_mut())
        else {
            return;
        };
        if let Err(error) = edit.validate() {
            edit.error = Some(error);
            cx.notify();
            return;
        }
        let name = edit.name.clone();
        let changes = edit.changes();
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        match workspace.save_environment(&name, &changes) {
            Ok(()) => {
                log::debug!("saved environment {name}: {} changes", changes.len());
                if let Some(edit) = self
                    .state
                    .tabs
                    .get_mut(index)
                    .and_then(|tab| tab.environment_mut())
                {
                    edit.mark_saved();
                }
                self.state.tabs.mark_saved(index);
                self.workspace_error = None;
                // Also reloads the active environment, so variable hints and the next send see
                // the saved values.
                self.refresh_environments();
                self.show_toast(t!("common.saved_toast"), cx);
            }
            Err(error) => {
                log_workspace_error(&format!("save environment {name:?}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Closes the tab at `index`, asking first when it is an environment tab with unsaved edits.
    pub(crate) fn request_close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.tabs.get(index) else {
            return;
        };
        let Some(edit) = tab.environment().filter(|_| tab.dirty) else {
            self.close_tab(index, cx);
            return;
        };
        let name = edit.name.clone();
        let tab_id = tab.id.clone();
        let weak = cx.weak_entity();
        open_confirm_dialog(
            window,
            cx,
            ConfirmDialog {
                title: t!("environments.discard.title").into(),
                text: t!("environments.discard.text", name = name.as_str()).into(),
                confirm_label: t!("environments.discard.confirm").into(),
                destructive: true,
                show_cancel: true,
                on_confirm: Rc::new(move |_, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        if let Some(index) = view.state.tabs.index_of(&tab_id) {
                            view.close_tab(index, cx);
                        }
                    });
                }),
            },
        );
    }

    /// Renames the environment `old` to `new` on disk and everywhere the app remembers it.
    pub(crate) fn rename_environment(&mut self, old: String, new: String, cx: &mut Context<Self>) {
        if old == new {
            return;
        }
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        match workspace.rename_environment(&old, &new) {
            Ok(()) => {
                log::debug!("renamed environment {old} to {new}");
                let root = workspace.root().to_path_buf();
                self.state.tabs.rename_environment(&old, &new);
                if let Some(entities) = self
                    .env_editors
                    .remove(&crate::state::env_edit::tab_id(&old))
                {
                    self.env_editors
                        .insert(crate::state::env_edit::tab_id(&new), entities);
                }
                // The remembered environment is rewritten even when it is not the active one.
                if self.state.active_environment.as_deref() == Some(old.as_str()) {
                    self.state.set_active_environment(Some(new.clone()));
                    crate::state::config::record_environment(&root, Some(&new));
                } else if crate::state::config::last_environment(&root).as_deref()
                    == Some(old.as_str())
                {
                    crate::state::config::record_environment(&root, Some(&new));
                }
                self.workspace_error = None;
                self.refresh_environments();
            }
            Err(error) => {
                log_workspace_error(&format!("rename environment {old:?}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Deletes both files of the environment `name`, closes its tab and deactivates it.
    pub(crate) fn delete_environment(&mut self, name: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        match workspace.delete_environment(&name) {
            Ok(()) => {
                log::debug!("deleted environment {name}");
                let tab_id = crate::state::env_edit::tab_id(&name);
                if let Some(index) = self.state.tabs.index_of(&tab_id) {
                    self.close_tab(index, cx);
                }
                if self.state.active_environment.as_deref() == Some(name.as_str()) {
                    self.select_environment(None, cx);
                }
                self.workspace_error = None;
                self.refresh_environments();
            }
            Err(error) => {
                log_workspace_error(&format!("delete environment {name:?}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    fn open_env_rename_dialog(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("navigation.env.name_placeholder"))
        });
        input.update(cx, |state, cx| state.set_value(name.clone(), window, cx));
        let weak = cx.weak_entity();
        // Renames and closes the dialog; shared by Enter in the input and the Rename button.
        let submit: SubmitHandler = {
            let input = input.clone();
            let weak = weak.clone();
            let old = name.clone();
            Rc::new(move |window, cx| {
                let new = input.read(cx).value().trim().to_string();
                if new.is_empty() || new == old {
                    return;
                }
                window.close_dialog(cx);
                let old = old.clone();
                let _ = weak.update(cx, |view, cx| view.rename_environment(old, new, cx));
            })
        };
        {
            let submit = submit.clone();
            cx.subscribe_in(
                &input,
                window,
                move |_, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        submit(window, cx);
                    }
                },
            )
            .detach();
        }
        let dialog_input = input.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let input = dialog_input.clone();
            let submit = submit.clone();
            framed_dialog(
                dialog,
                MODAL_WIDTH_DEFAULT,
                None,
                ModalPlacement::DEFAULT,
                window,
                move |_, cx| {
                    let submit = submit.clone();
                    v_flex()
                        .child(modal_header(t!("environments.rename.title"), cx))
                        .child(modal_body().child(modal_field(
                            t!("common.name"),
                            HoverInput::new(
                                "env-rename-input",
                                &input,
                                Input::new(&input)
                                    .context_menu(edit_menu(&input, cx))
                                    .px(px(INPUT_PADDING_X))
                                    .py_0(),
                            ),
                            cx,
                        )))
                        .child(
                            modal_footer(cx)
                                .child(
                                    SecondaryButton::new("env-rename-cancel", t!("common.cancel"))
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    PrimaryButton::new("env-rename-ok", t!("common.rename"))
                                        .on_click(move |_, window, cx| submit(window, cx)),
                                ),
                        )
                        .into_any_element()
                },
            )
        });
        // Focus only after `open_dialog`, which captures the focused handle to restore on close
        // (see `views/command_palette.rs`). Selecting the whole name makes typing replace it.
        input.update(cx, |state, cx| {
            state.focus(window, cx);
            state.select_all(window, cx);
        });
    }

    fn open_env_delete_dialog(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        open_confirm_dialog(
            window,
            cx,
            ConfirmDialog {
                title: t!("environments.delete.title").into(),
                text: t!("environments.delete.text", name = name.as_str()).into(),
                confirm_label: t!("common.delete").into(),
                destructive: true,
                show_cancel: true,
                on_confirm: Rc::new(move |_, cx| {
                    let target = name.clone();
                    let _ = weak.update(cx, |view, cx| view.delete_environment(target, cx));
                }),
            },
        );
    }

    /// Renders the editor of the active environment tab.
    pub(crate) fn render_env_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab_id) = self.state.tabs.active().map(|tab| tab.id.clone()) else {
            return div().into_any_element();
        };
        self.ensure_env_entities(&tab_id, window, cx);
        let (Some(edit), Some(entities)) = (
            self.state.tabs.active().and_then(|tab| tab.environment()),
            self.env_editors.get(&tab_id),
        ) else {
            return div().into_any_element();
        };
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let weak = cx.weak_entity();
        let is_active = self.state.active_environment.as_deref() == Some(edit.name.as_str());
        let reveal_path = self.state.workspace.as_ref().map(|workspace| {
            let layer = if edit.base_exists {
                EnvLayer::Base
            } else {
                EnvLayer::Local
            };
            workspace.environment_path(&edit.name, layer)
        });
        // Where "Move up" and "Move down" can go, from the panel's order.
        let position = self.env_rows.iter().position(|row| row.name == edit.name);
        let moves = EnvMoves {
            up: position.is_some_and(|position| position > 0),
            down: position.is_some_and(|position| position + 1 < self.env_rows.len()),
        };
        let color = EnvColorState {
            chosen: self.state.env_colors.chosen(&edit.name),
            picker_open: self.env_color_picker_open,
        };
        let session: Vec<KeyValue> = if is_active {
            self.state.session_env.as_slice().to_vec()
        } else {
            Vec::new()
        };

        v_flex()
            .size_full()
            .bg(palette.bg)
            .child(render_header(
                edit,
                reveal_path,
                is_active,
                moves,
                color,
                &weak,
                &palette,
                cx,
            ))
            .child(render_toolbar(
                edit,
                entities,
                session.len(),
                &tab_id,
                &weak,
                &palette,
                cx,
            ))
            .children(edit.error.as_ref().map(|error| {
                div().px(px(20.0)).pt(px(10.0)).child(InlineMessage::new(
                    InlineMessageKind::Danger,
                    error_text(error),
                ))
            }))
            .child(
                div()
                    .id("env-table-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&entities.scroll)
                    .px(px(20.0))
                    .py(px(12.0))
                    .child(render_table(
                        edit, entities, &session, &tab_id, &weak, &palette, &mono_font, cx,
                    )),
            )
            .child(render_footer(&palette))
            .into_any_element()
    }
}

/// The translated text of a validation error.
fn error_text(error: &EnvValidationError) -> String {
    match error {
        EnvValidationError::EmptyName => t!("environments.error.empty_name").into_owned(),
        EnvValidationError::InvalidName(name) => {
            t!("environments.error.invalid_name", name = name.as_str()).into_owned()
        }
        EnvValidationError::InvalidValue(name) => {
            t!("environments.error.invalid_value", name = name.as_str()).into_owned()
        }
        EnvValidationError::Duplicate(name) => {
            t!("environments.error.duplicate", name = name.as_str()).into_owned()
        }
    }
}

/// Builds the name and value inputs of one row, prefilled, and subscribes both to write typed
/// changes back into the tab. The value is set before subscribing so it does not count as an
/// edit.
fn build_row_inputs(
    tab_id: &str,
    row_id: u64,
    name: &str,
    value: &str,
    masked: bool,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> EnvRowInputs {
    let name_input =
        cx.new(|cx| InputState::new(window, cx).placeholder(t!("environments.name_placeholder")));
    name_input.update(cx, |state, cx| {
        state.set_value(name.to_string(), window, cx)
    });
    let value_input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("common.value")));
    value_input.update(cx, |state, cx| {
        state.set_value(value.to_string(), window, cx);
        state.set_masked(masked, window, cx);
    });
    for (input, field) in [
        (&name_input, EnvField::Name),
        (&value_input, EnvField::Value),
    ] {
        let tab_id = tab_id.to_string();
        cx.subscribe(input, move |view, entity, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = entity.read(cx).value().to_string();
                view.edit_env_row(&tab_id, row_id, field, &text, cx);
            }
        })
        .detach();
    }
    EnvRowInputs {
        name: name_input,
        value: value_input,
    }
}

/// The color of the environment of a tab (#217): the chosen one (`None` when automatic) and
/// whether its picker is open.
#[derive(Clone, Copy)]
struct EnvColorState {
    chosen: Option<EnvColor>,
    picker_open: bool,
}

/// Whether the environment of a tab can move up or down in the order (#216).
#[derive(Clone, Copy)]
struct EnvMoves {
    up: bool,
    down: bool,
}

/// The header: color dot, name, "Use this environment", the "..." menu and the two file chips.
#[allow(clippy::too_many_arguments)]
fn render_header(
    edit: &EnvEditTab,
    reveal_path: Option<PathBuf>,
    is_active: bool,
    moves: EnvMoves,
    color: EnvColorState,
    weak: &WeakEntity<AppView>,
    palette: &Palette,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let name = edit.name.clone();
    // Active is a status, not a disabled action: a status badge reads as confirmed, where a
    // greyed out button read as broken. Only the inactive state is still a clickable action.
    let use_control: AnyElement = if is_active {
        StatusBadge::new(t!("environments.active_env"))
            .tone(Tone::Success)
            .into_any_element()
    } else {
        SecondaryButton::new("env-use", t!("environments.use"))
            .icon(IconName::Check)
            .height(CONTROL_HEIGHT)
            .on_click({
                let weak = weak.clone();
                let name = name.clone();
                move |_, _, cx| {
                    let name = name.clone();
                    let _ = weak.update(cx, |view, cx| view.select_environment(Some(name), cx));
                }
            })
            .into_any_element()
    };

    let more = Button::new("env-more")
        .neutral_bordered(cx, palette.raised, palette.border_strong, palette.fg_muted)
        .icon(Icon::new(IconName::Ellipsis))
        .w(px(CONTROL_HEIGHT))
        .h(px(CONTROL_HEIGHT))
        .rounded(px(RADIUS_MD))
        .border_1()
        .text_color(palette.fg_muted)
        .tooltip(t!("environments.more_tooltip"))
        .dropdown_menu({
            let weak = weak.clone();
            let name = name.clone();
            move |menu, _, _| {
                let rename = {
                    let weak = weak.clone();
                    let name = name.clone();
                    PopupMenuItem::new(t!("common.rename")).on_click(move |_, window, cx| {
                        let name = name.clone();
                        let _ = weak
                            .update(cx, |view, cx| view.open_env_rename_dialog(name, window, cx));
                    })
                };
                let reveal = {
                    let path = reveal_path.clone();
                    PopupMenuItem::new(t!("environments.menu.reveal")).on_click(move |_, _, cx| {
                        if let Some(path) = &path {
                            cx.reveal_path(path);
                        }
                    })
                };
                let delete = {
                    let weak = weak.clone();
                    let name = name.clone();
                    PopupMenuItem::new(t!("common.delete")).on_click(move |_, window, cx| {
                        let name = name.clone();
                        let _ = weak
                            .update(cx, |view, cx| view.open_env_delete_dialog(name, window, cx));
                    })
                };
                let move_item = |label: &str, up: bool, enabled: bool| {
                    let weak = weak.clone();
                    let name = name.clone();
                    PopupMenuItem::new(label.to_string())
                        .disabled(!enabled)
                        .on_click(move |_, _, cx| {
                            let _ =
                                weak.update(cx, |view, cx| view.move_environment(&name, up, cx));
                        })
                };
                let pick_color = {
                    let weak = weak.clone();
                    PopupMenuItem::new(t!("environments.menu.color")).on_click(move |_, _, cx| {
                        let _ =
                            weak.update(cx, |view, cx| view.set_env_color_picker_open(true, cx));
                    })
                };
                let move_up = move_item(&t!("environments.menu.move_up"), true, moves.up);
                let move_down = move_item(&t!("environments.menu.move_down"), false, moves.down);
                menu.item(pick_color)
                    .item(move_up)
                    .item(move_down)
                    .separator()
                    .item(rename)
                    .item(reveal)
                    .separator()
                    .item(delete)
            }
        });

    v_flex()
        .gap(px(10.0))
        .px(px(20.0))
        .pt(px(16.0))
        .pb(px(12.0))
        .border_b_1()
        .border_color(palette.border)
        .child(
            h_flex()
                .items_center()
                .gap(px(10.0))
                .child(color_picker(
                    &name,
                    color.chosen,
                    color.picker_open,
                    weak,
                    palette,
                    cx,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(17.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.fg)
                        .child(name.clone()),
                )
                .child(use_control)
                .child(more),
        )
        .child(
            h_flex()
                .gap(px(8.0))
                .flex_wrap()
                .child(file_chip(
                    IconName::File,
                    format!("environments/{name}.env"),
                    t!("environments.file.versioned").into_owned(),
                    true,
                    palette,
                    cx,
                ))
                .child(file_chip(
                    IconName::Lock,
                    format!("environments/{name}.local.env"),
                    if edit.local_exists {
                        t!("environments.file.gitignored").into_owned()
                    } else {
                        t!("environments.file.not_created").into_owned()
                    },
                    edit.local_exists,
                    palette,
                    cx,
                )),
        )
        .into_any_element()
}

/// One file chip of the header: icon, mono path and a muted note. Muted when the file does not
/// exist.
fn file_chip(
    icon: IconName,
    path: String,
    note: String,
    exists: bool,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let mono_font = cx.theme().mono_font_family.clone();
    h_flex()
        .max_w_full()
        .min_w_0()
        .items_center()
        .gap(px(6.0))
        .h(px(22.0))
        .px(px(8.0))
        .rounded(px(RADIUS_SM))
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .text_color(if exists {
            palette.fg_muted
        } else {
            palette.fg_subtle
        })
        .child(Icon::new(icon).with_size(px(11.0)).flex_none())
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_family(mono_font)
                .text_size(px(11.5))
                .child(path),
        )
        .child(
            div()
                .flex_none()
                .text_size(px(11.5))
                .text_color(palette.fg_subtle)
                .child(format!("\u{b7} {note}")),
        )
        .into_any_element()
}

/// The toolbar: filter, scope control, secrets toggle and "Add variable".
fn render_toolbar(
    edit: &EnvEditTab,
    entities: &EnvEditorEntities,
    session_count: usize,
    tab_id: &str,
    weak: &WeakEntity<AppView>,
    palette: &Palette,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let scope_item = |label: String, scope: EnvScope| {
        let weak = weak.clone();
        let tab_id = tab_id.to_string();
        SegmentedItem::new(label)
            .selected(edit.scope == scope)
            .on_click(move |_, cx| {
                let tab_id = tab_id.clone();
                let _ = weak.update(cx, |view, cx| {
                    view.edit_env(&tab_id, cx, |edit| {
                        edit.set_scope(scope);
                        true
                    });
                });
            })
    };
    let all_label = t!(
        "environments.scope.all",
        count = format_integer((edit.count_all() + session_count) as u64).as_str()
    )
    .into_owned();
    let secrets_label = if edit.show_secrets {
        t!("environments.hide_secrets")
    } else {
        t!("environments.show_secrets")
    };
    let secrets_icon = if edit.show_secrets {
        IconName::EyeOff
    } else {
        IconName::Eye
    };

    h_flex()
        .items_center()
        .gap(px(8.0))
        .px(px(20.0))
        .pt(px(12.0))
        .child(
            div().w(px(220.0)).child(HoverInput::new(
                "env-filter",
                &entities.filter,
                Input::new(&entities.filter)
                    .context_menu(edit_menu(&entities.filter, cx))
                    .px(px(INPUT_PADDING_X))
                    .h(px(CONTROL_HEIGHT))
                    .py_0()
                    .prefix(
                        Icon::new(IconName::ListFilter)
                            .small()
                            .text_color(palette.fg_subtle),
                    )
                    .cleanable(true),
            )),
        )
        .child(
            SegmentedControl::new("env-scope")
                .item(scope_item(all_label, EnvScope::All))
                .item(scope_item(".env".to_string(), EnvScope::Base))
                .item(scope_item(".local.env".to_string(), EnvScope::Local)),
        )
        .child(div().flex_1())
        .child(
            GhostButton::new("env-secrets", secrets_label)
                .icon(secrets_icon)
                .height(CONTROL_HEIGHT)
                .on_click({
                    let weak = weak.clone();
                    let tab_id = tab_id.to_string();
                    move |_, window, cx| {
                        let tab_id = tab_id.clone();
                        let _ = weak
                            .update(cx, |view, cx| view.toggle_env_secrets(&tab_id, window, cx));
                    }
                }),
        )
        .child(
            PrimaryButton::new("env-add", t!("environments.add"))
                .icon(IconName::Plus)
                .height(CONTROL_HEIGHT)
                .on_click({
                    let weak = weak.clone();
                    move |_, window, cx| {
                        let _ = weak.update(cx, |view, cx| view.add_env_variable(window, cx));
                    }
                }),
        )
        .into_any_element()
}

/// The variables table, telaio's [`EditableTable`]: one row per visible variable, then the
/// session rows.
#[allow(clippy::too_many_arguments)]
fn render_table(
    edit: &EnvEditTab,
    entities: &EnvEditorEntities,
    session: &[KeyValue],
    tab_id: &str,
    weak: &WeakEntity<AppView>,
    palette: &Palette,
    mono_font: &SharedString,
    cx: &mut Context<AppView>,
) -> AnyElement {
    let reveal = entities.reveal.get();
    if reveal.is_some_and(|target| {
        !edit
            .visible()
            .iter()
            .any(|&index| edit.rows[index].id == target)
    }) {
        // Filtered out: nothing to scroll to.
        entities.reveal.set(None);
    }
    let mut rows = Vec::new();
    for &index in edit.visible() {
        let row = &edit.rows[index];
        let Some(inputs) = entities.rows.get(&row.id) else {
            continue;
        };
        let reveal = (reveal == Some(row.id)).then(|| RevealRequest {
            target: row.id,
            scroll: entities.scroll.clone(),
            pending: entities.reveal.clone(),
        });
        rows.push(file_row(
            edit, row, inputs, reveal, tab_id, weak, palette, mono_font, cx,
        ));
    }
    for variable in edit.visible_session(session) {
        rows.push(session_row(variable, palette, mono_font));
    }
    let empty = if edit.filter.trim().is_empty() {
        t!("environments.empty")
    } else {
        t!("environments.empty_filter")
    };
    EditableTable::new([
        EditableColumn::fixed(t!("environments.col.name"), NAME_COLUMN_WIDTH),
        EditableColumn::flex(t!("common.value")),
        EditableColumn::fixed(t!("environments.col.stored_in"), STORED_COLUMN_WIDTH),
        EditableColumn::fixed("", TRASH_COLUMN_WIDTH),
    ])
    .rows(rows)
    .empty_message(empty)
    .into_any_element()
}

/// A pending request to scroll the table so a newly added row is fully visible.
struct RevealRequest {
    target: u64,
    scroll: ScrollHandle,
    pending: Rc<Cell<Option<u64>>>,
}

/// Wraps `content`, the trash column of the row `reveal` asks for, in a box as tall as a row with
/// an empty canvas that reads the row's bounds at prepaint: the scroll offset is adjusted once,
/// the request cleared, and one more frame drawn. The rows are inside telaio's table, not
/// children of the scrolled element, so `ScrollHandle::scroll_to_item` cannot reach them.
fn reveal_on_prepaint(content: impl IntoElement, reveal: Option<RevealRequest>) -> AnyElement {
    let Some(reveal) = reveal else {
        return content.into_any_element();
    };
    div()
        .relative()
        .h(px(EDITABLE_TABLE_ROW_HEIGHT))
        .flex()
        .items_center()
        .child(content)
        .child(
            canvas(
                move |row, window, _cx| {
                    if reveal.pending.get() != Some(reveal.target) {
                        return;
                    }
                    reveal.pending.set(None);
                    let view = reveal.scroll.bounds();
                    let margin = px(12.0);
                    let mut offset = reveal.scroll.offset();
                    if row.top() < view.top() {
                        offset.y += view.top() - row.top() + margin;
                    } else if row.bottom() > view.bottom() {
                        offset.y -= row.bottom() - view.bottom() + margin;
                    } else {
                        return;
                    }
                    reveal.scroll.set_offset(offset);
                    window.refresh();
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
        .into_any_element()
}

/// A small button drawn as a "stored in" chip: icon and mono file name on a tinted background.
/// Opens a menu when given one with `dropdown_menu`.
#[allow(clippy::too_many_arguments)]
fn chip_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: String,
    bg: Hsla,
    fg: Hsla,
    palette: &Palette,
    mono_font: &SharedString,
    cx: &App,
) -> Button {
    let custom = ButtonCustomVariant::new(cx)
        .color(bg)
        .foreground(fg)
        // Over the chip's own tint, which a hover fill replaces instead of layering over.
        .hover(palette.hover_over(bg))
        .active(palette.pressed_over(bg));
    Button::new(id)
        .custom(custom)
        .h(px(20.0))
        .px(px(7.0))
        .rounded(px(5.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(5.0))
                .child(Icon::new(icon).with_size(px(10.0)))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(mono_font.clone())
                        .child(label),
                ),
        )
}

/// The background and text colors of a "stored in" chip.
fn chip_colors(layer: Option<EnvLayer>, palette: &Palette) -> (Hsla, Hsla) {
    match layer {
        Some(EnvLayer::Base) => (palette.raised, palette.fg_muted),
        Some(EnvLayer::Local) => (palette.warning_subtle, palette.warning),
        None => (palette.accent_subtle, palette.accent_text),
    }
}

/// One editable row of a file.
#[allow(clippy::too_many_arguments)]
fn file_row(
    edit: &EnvEditTab,
    row: &EnvVarRow,
    inputs: &EnvRowInputs,
    reveal: Option<RevealRequest>,
    tab_id: &str,
    weak: &WeakEntity<AppView>,
    palette: &Palette,
    mono_font: &SharedString,
    cx: &mut Context<AppView>,
) -> EditableRow {
    let row_id = row.id;
    let note = edit.note(row).map(|note| match note {
        RowNote::OverridesBase => t!(
            "environments.note.overrides",
            file = format!("{}.env", edit.name).as_str()
        )
        .into_owned(),
    });
    let (chip_bg, chip_fg) = chip_colors(Some(row.layer), palette);
    let (chip_icon, chip_file) = match row.layer {
        EnvLayer::Base => (
            IconName::File,
            env_file_label(&edit.name, ".env", STORED_CHIP_MAX_CHARS),
        ),
        EnvLayer::Local => (
            IconName::Lock,
            env_file_label(&edit.name, ".local.env", STORED_CHIP_MAX_CHARS),
        ),
    };
    let current = row.layer;
    let picker = chip_button(
        ("env-stored", row_id as usize),
        chip_icon,
        chip_file,
        chip_bg,
        chip_fg,
        palette,
        mono_font,
        cx,
    )
    .tooltip(t!("environments.stored_in_tooltip"))
    .dropdown_menu({
        let weak = weak.clone();
        let tab_id = tab_id.to_string();
        let name = edit.name.clone();
        move |mut menu, _, _| {
            for (layer, file) in [
                (
                    EnvLayer::Base,
                    env_file_label(&name, ".env", STORED_MENU_MAX_CHARS),
                ),
                (
                    EnvLayer::Local,
                    env_file_label(&name, ".local.env", STORED_MENU_MAX_CHARS),
                ),
            ] {
                let weak = weak.clone();
                let tab_id = tab_id.clone();
                menu = menu.item(PopupMenuItem::new(file).checked(current == layer).on_click(
                    move |_, window, cx| {
                        let tab_id = tab_id.clone();
                        let _ = weak.update(cx, |view, cx| {
                            view.move_env_row(&tab_id, row_id, layer, window, cx)
                        });
                    },
                ));
            }
            menu
        }
    });

    let remove = IconButton::new(("env-remove", row_id as usize), IconName::Trash)
        .tooltip(t!("environments.remove_tooltip"))
        .on_click({
            let weak = weak.clone();
            let tab_id = tab_id.to_string();
            move |_, _, cx| {
                let tab_id = tab_id.clone();
                let _ = weak.update(cx, |view, cx| view.remove_env_row(&tab_id, row_id, cx));
            }
        });
    let value = EditableCell::input(&inputs.value);
    EditableRow::new([
        EditableCell::input(&inputs.name).color(palette.accent_text),
        match note {
            Some(note) => value.note(note),
            None => value,
        },
        EditableCell::element(picker),
        EditableCell::element(reveal_on_prepaint(remove, reveal)).centered(),
    ])
}

/// One read only session row: a variable a script set with `env.set` during this session.
fn session_row(variable: &KeyValue, palette: &Palette, mono_font: &SharedString) -> EditableRow {
    let (chip_bg, chip_fg) = chip_colors(None, palette);
    let chip = h_flex()
        .items_center()
        .gap(px(5.0))
        .h(px(20.0))
        .px(px(7.0))
        .rounded(px(5.0))
        .bg(chip_bg)
        .text_color(chip_fg)
        .text_size(px(11.0))
        .font_family(mono_font.clone())
        .child(Icon::new(IconName::Zap).with_size(px(10.0)))
        .child(t!("environments.source.session"));
    EditableRow::new([
        EditableCell::text(variable.key.clone()).color(palette.accent_text),
        EditableCell::text(variable.value.clone()).note(t!("environments.note.session")),
        EditableCell::element(chip),
    ])
    .background(palette.accent_subtle.opacity(0.5))
}

/// The footer: the resolution order of a variable.
fn render_footer(palette: &Palette) -> AnyElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .px(px(20.0))
        .py(px(10.0))
        .border_t_1()
        .border_color(palette.border)
        .text_size(px(12.0))
        .text_color(palette.fg_subtle)
        .child(Icon::new(IconName::Layers).with_size(px(13.0)))
        .child(t!("environments.resolution.label"))
        .child(
            div()
                .text_color(palette.fg_muted)
                .child(t!("environments.resolution.order")),
        )
        .into_any_element()
}
