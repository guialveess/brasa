//! The autocompletion of the `::: pre` and `::: post` code editors: the script API, variable
//! and header names, and the keys of the last JSON response.
//!
//! The editor asks a [`CompletionProvider`] for rows while the user types (when
//! [`is_trigger`] says so) and draws its own menu, with the arrow keys, `Enter` and `Escape`.
//! What the rows are comes from `state::script_completion`; the provider only reads the
//! [`ScriptCompletionData`] the request editor refreshes, so typing never rescans the response.
//! `Ctrl+Space` and `Tab` go through the app's completion actions (`views/completion.rs`).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{CompletionProvider, Enter, Rope, RopeExt};
use gpui_kit::*;
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, MarkupContent, MarkupKind, TextEdit,
};
use rust_i18n::t;

use crate::state::header_completion::HEADER_NAMES;
use crate::state::script_completion::{
    ItemKind, JsonShape, ScriptContext, ScriptKind, completions, is_trigger,
};
use crate::state::variable_hint::{self, VariableContext};

use super::root::AppView;

/// What the script editors complete besides the API: the variables and the last response of
/// the active tab. Shared between the request editor (which refreshes it) and the providers.
#[derive(Default)]
pub struct ScriptCompletionData {
    pub variables: Rc<VariableContext>,
    pub response: Option<Arc<JsonShape>>,
}

/// Answers the completion requests of one script editor.
pub struct ScriptCompletionProvider {
    kind: ScriptKind,
    data: Rc<RefCell<ScriptCompletionData>>,
}

impl ScriptCompletionProvider {
    /// A provider for a `kind` script reading `data`.
    pub fn new(kind: ScriptKind, data: Rc<RefCell<ScriptCompletionData>>) -> Self {
        Self { kind, data }
    }

    /// The rows for the cursor at byte `offset` of `text`. `explicit` when asked with
    /// `Ctrl+Space`, which lists the globals even with nothing typed.
    pub fn items(&self, text: &Rope, offset: usize, explicit: bool) -> Vec<CompletionItem> {
        let data = self.data.borrow();
        let variables = variable_hint::completions(&data.variables, false);
        let context = ScriptContext {
            kind: self.kind,
            variables: &variables,
            header_names: HEADER_NAMES,
            response: data.response.as_deref(),
        };
        let source = text.to_string();
        completions(&source, offset, &context, explicit)
            .into_iter()
            .map(|item| CompletionItem {
                // The menu highlights as many leading characters of the label as this has: what
                // was typed when the label starts with it, nothing for a match inside the label.
                filter_text: Some(
                    source
                        .get(item.replace.clone())
                        .filter(|typed| {
                            item.label.to_lowercase().starts_with(&typed.to_lowercase())
                        })
                        .unwrap_or_default()
                        .to_string(),
                ),
                kind: Some(match item.kind {
                    ItemKind::Object => CompletionItemKind::MODULE,
                    ItemKind::Function => CompletionItemKind::FUNCTION,
                    ItemKind::Property => CompletionItemKind::PROPERTY,
                    ItemKind::Variable => CompletionItemKind::VARIABLE,
                    ItemKind::Header => CompletionItemKind::CONSTANT,
                    ItemKind::JsonKey => CompletionItemKind::FIELD,
                }),
                // Every row gets a detail: the menu draws a row without one shorter.
                detail: match (item.kind, item.detail) {
                    (ItemKind::Variable, None) => Some("set by this script".to_string()),
                    (ItemKind::Variable, Some(value)) if value.is_empty() => {
                        Some(t!("request.variable.empty").into_owned())
                    }
                    (_, detail) => detail,
                },
                documentation: item.documentation.map(|value| {
                    Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value,
                    })
                }),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                    range: lsp_types::Range::new(
                        text.offset_to_position(item.replace.start),
                        text.offset_to_position(item.replace.end),
                    ),
                    new_text: item.insert,
                })),
                label: item.label,
                ..Default::default()
            })
            .collect()
    }
}

impl CompletionProvider for ScriptCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<anyhow::Result<CompletionResponse>> {
        Task::ready(Ok(CompletionResponse::Array(
            self.items(text, offset, false),
        )))
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, _cx: &mut App) -> bool {
        is_trigger(new_text)
    }
}

impl AppView {
    /// `Ctrl+Space` in a script editor: opens its menu for the cursor, with every global when
    /// nothing is typed. `false` when no script editor has the focus.
    pub(crate) fn show_script_completion(&mut self, window: &mut Window, cx: &mut App) -> bool {
        let Some(script) = self.focused_script_editor(window, cx) else {
            return false;
        };
        let completion = script.completion.clone();
        script.editor.update(cx, |state, cx| {
            let offset = state.cursor();
            let items = completion.items(state.text(), offset, true);
            // The menu highlights what was typed from where the rows' replaced range starts.
            let start = items
                .iter()
                .find_map(|item| match &item.text_edit {
                    Some(CompletionTextEdit::Edit(edit)) => {
                        Some(state.text().position_to_offset(&edit.range.start))
                    }
                    _ => None,
                })
                .unwrap_or(offset)
                .min(offset);
            let query = state.text().slice(start..offset).to_string();
            state.present_completion_items(start, query, items, cx);
        });
        true
    }

    /// `Tab` (or `Enter`) while a script editor's menu is open: inserts the highlighted row, as
    /// the editor does on `Enter`. `false` when no script editor menu is open.
    pub(crate) fn confirm_script_completion(&mut self, window: &mut Window, cx: &mut App) -> bool {
        let open = self
            .focused_script_editor(window, cx)
            .is_some_and(|script| script.editor.read(cx).completion_menu_state().open);
        if open {
            window.dispatch_action(
                Box::new(Enter {
                    secondary: false,
                    shift: false,
                }),
                cx,
            );
        }
        open
    }
}
