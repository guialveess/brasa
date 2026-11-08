//! The request editor panel: method + URL bar, Send/Cancel, and the Params, Headers, Auth, Body,
//! Pre-request, Post-response and Docs tabs.
//!
//! `gpui` entities for the editable fields (the URL input, the key-value table rows, the body
//! script and docs code editors) are expensive to keep in sync by hand on every
//! keystroke, so [`RequestEditorEntities`] rebuilds them only when what they need to show
//! actually changes shape: a different tab becomes active, a row is added or removed, or the
//! body's type (and so its editor language) or the auth type changes. Typing into an existing field never
//! triggers a rebuild, which is what lets it keep focus and cursor position while the user
//! types; see [`RequestEditorEntities::sync`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    EditorState, IndentInline, InputEvent, InputState, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use gpui::Hsla;
use brasa_core::log_safe::is_sensitive_header;
use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, Method, Request, variable_spans};

use crate::actions::{DeleteLine, RedoRequestEdit, UndoRequestEdit};
use crate::state::auth_edit::{self, AuthField, AuthKind, Overridden};
use crate::state::line_edit;
use crate::state::request_edit::{self, BodyKind, SortColumn, SortOrder};
use crate::state::script_completion::ScriptKind;
use crate::state::script_heuristics;
use crate::state::ui_tabs::RequestTab;
use crate::state::variable_hint::{self, VariableContext};
use crate::theme::PaletteExt;
use crate::theme::metrics::{
    AUTH_FORM_MAX_WIDTH, AUTH_LABEL_WIDTH, COMPLETION_MENU_WIDTH, SEND_BUTTON_HEIGHT,
    SEND_BUTTON_MIN_WIDTH,
};
use crate::views::components::NeutralButtonExt;
use crate::views::components::variable_chip::highlight_style;
use crate::views::components::{
    CodeEditor, GhostButton, InlineMessage, InlineMessageKind, KeyValueRow, KeyValueTable,
    PrimaryButton, SegmentedControl, SegmentedItem, UnderlineTabItem, UnderlineTabs, UrlBar,
    VariableField,
};
use crate::views::script_completion::{ScriptCompletionData, ScriptCompletionProvider};
use crate::views::variable_hover::{BodyMarkers, VariableHover};

use super::root::AppView;

/// Which key-value table a row belongs to: routes an edit to the right field of the active tab's
/// [`Request`]. The Params table never rewrites the URL: the two are combined only when sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RowKind {
    /// The header list.
    Headers,
    /// The `::: query` section.
    Query,
    /// A `Body::Form`'s fields.
    Form,
}

/// Which script field a script code editor edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScriptField {
    /// The `::: pre` section.
    Pre,
    /// The `::: post` section.
    Post,
}

/// Everything about the active tab's shape that requires [`RequestEditorEntities`] to be
/// rebuilt. Row and script *content* is deliberately not part of this key: editing content must
/// never rebuild the entities, or the field being typed into would lose focus on every
/// keystroke. Only counts (which change when a row is added or removed), the body's kind
/// (which changes the body editor's language) and the auth's kind (which changes its fields) are
/// tracked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildKey {
    tab_id: String,
    headers_len: usize,
    query_len: usize,
    form_len: usize,
    body_kind: BodyKind,
    auth_kind: AuthKind,
}

impl BuildKey {
    fn of(tab_id: &str, request: &Request) -> Self {
        let form_len = match &request.body {
            Body::Form(rows) | Body::Multipart(rows) => rows.len(),
            _ => 0,
        };
        Self {
            tab_id: tab_id.to_string(),
            headers_len: request.headers.len(),
            query_len: request.query.len(),
            form_len,
            body_kind: BodyKind::of(&request.body),
            auth_kind: AuthKind::of(&request.auth),
        }
    }
}

/// A single-line field of the request editor that offers autocompletion (`views/completion.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionField {
    /// The URL input.
    Url,
    /// The key input of a Params, Headers or Form row, by index. Only a Headers key offers
    /// anything (header names).
    Key(RowKind, usize),
    /// The value input of a Params, Headers or Form row, by index.
    Value(RowKind, usize),
    /// A field of the Auth tab.
    Auth(AuthField),
}

/// One key-value row's live `gpui` input entities.
struct RowEntities {
    key: Entity<InputState>,
    value: Entity<InputState>,
}

/// The live `gpui` entities behind one key-value table (Params, Headers or Form body), aligned
/// index-for-index with the `Request`'s own `Vec<KeyValue>`.
#[derive(Default)]
struct KeyValueTableEntities {
    rows: Vec<RowEntities>,
}

impl KeyValueTableEntities {
    fn build(
        tab_id: &str,
        rows: &[KeyValue],
        kind: RowKind,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let rows = rows
            .iter()
            .enumerate()
            .map(|(index, row)| build_row(tab_id, kind, index, row, window, cx))
            .collect();
        Self { rows }
    }
}

/// Builds one row's key and value inputs, pre-filled with `row`'s current content, and
/// subscribes each to write typed changes back into the active tab's request.
fn build_row(
    tab_id: &str,
    kind: RowKind,
    index: usize,
    row: &KeyValue,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> RowEntities {
    let key = cx.new(|cx| InputState::new(window, cx).placeholder(t!("request.kv.key")));
    key.update(cx, |state, cx| state.set_value(row.key.clone(), window, cx));
    {
        let tab_id = tab_id.to_string();
        let field = CompletionField::Key(kind, index);
        cx.subscribe(
            &key,
            move |view, entity, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let value = entity.read(cx).value().to_string();
                    let typing = format!("{kind:?}:{index}:key");
                    view.edit_row(&tab_id, kind, index, Some(&typing), cx, |row| {
                        row.key = value
                    });
                    view.on_completion_field_changed(field, cx);
                }
                InputEvent::Blur => view.on_completion_field_blurred(field, cx),
                _ => {}
            },
        )
        .detach();
    }

    let value = cx.new(|cx| InputState::new(window, cx).placeholder(t!("common.value")));
    value.update(cx, |state, cx| {
        state.set_value(row.value.clone(), window, cx)
    });
    {
        let tab_id = tab_id.to_string();
        let field = CompletionField::Value(kind, index);
        cx.subscribe(
            &value,
            move |view, entity, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let new_value = entity.read(cx).value().to_string();
                    let typing = format!("{kind:?}:{index}:value");
                    view.edit_row(&tab_id, kind, index, Some(&typing), cx, |row| {
                        row.value = new_value
                    });
                    view.on_completion_field_changed(field, cx);
                }
                InputEvent::Blur => view.on_completion_field_blurred(field, cx),
                _ => {}
            },
        )
        .detach();
    }

    RowEntities { key, value }
}

/// Builds the input of one Auth tab field, pre-filled with `text`, and subscribes it to write
/// typed changes back into the active tab's request. A secret is masked unless `show_secrets`.
fn build_auth_input(
    tab_id: &str,
    field: AuthField,
    text: &str,
    show_secrets: bool,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<InputState> {
    let input = cx.new(|cx| {
        let state = InputState::new(window, cx).masked(field.is_secret() && !show_secrets);
        match field {
            AuthField::KeyName => state.placeholder("X-API-Key"),
            _ => state,
        }
    });
    input.update(cx, |state, cx| {
        state.set_value(text.to_string(), window, cx)
    });
    let tab_id = tab_id.to_string();
    let completion = CompletionField::Auth(field);
    cx.subscribe(
        &input,
        move |view, entity, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                let value = entity.read(cx).value().to_string();
                let typing = format!("auth:{field:?}");
                view.edit_request(&tab_id, Some(&typing), cx, |request| {
                    auth_edit::set_field(&mut request.auth, field, value)
                });
                view.on_completion_field_changed(completion, cx);
            }
            InputEvent::Blur => view.on_completion_field_blurred(completion, cx),
            _ => {}
        },
    )
    .detach();
    input
}

/// The body editor's language for a request's current body, and the raw text to load into it.
/// `None` for a body kind that has no code editor of its own (`None` or `Form`, which uses the
/// key-value table instead).
fn body_language(body: &Body) -> Option<(&str, &'static str)> {
    match body {
        Body::Json(text) => Some((text.as_str(), "json")),
        Body::Text(text) => Some((text.as_str(), "text")),
        // gpui-component 0.6.6 ships no dedicated XML grammar; "html" is the closest available
        // and still highlights tags and attributes reasonably well for a preview (see this
        // crate's Cargo.toml for the exact tree-sitter features enabled).
        Body::Xml(text) => Some((text.as_str(), "html")),
        Body::None | Body::Form(_) | Body::Multipart(_) => None,
    }
}

/// Builds a JavaScript code editor for a `::: pre` or `::: post` section, subscribed to write
/// typed changes back into the active tab's request, with its completion provider reading
/// `completion_data`.
fn make_script_editor(
    tab_id: &str,
    text: &str,
    field: ScriptField,
    completion_data: &Rc<RefCell<ScriptCompletionData>>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> ScriptEditor {
    let kind = match field {
        ScriptField::Pre => ScriptKind::Pre,
        ScriptField::Post => ScriptKind::Post,
    };
    let completion = Rc::new(ScriptCompletionProvider::new(kind, completion_data.clone()));
    // See the body editor's own comment on `auto_close(false)`.
    let editor =
        cx.new(|cx| EditorState::new(window, cx).language("javascript").auto_close(false));
    editor.update(cx, |state, cx| {
        state.set_value(text.to_string(), window, cx);
        let lsp = state.lsp_mut();
        lsp.completion_provider = Some(completion.clone());
        lsp.completion_menu.max_width = px(COMPLETION_MENU_WIDTH);
    });
    let tab_id = tab_id.to_string();
    cx.subscribe(&editor, move |view, entity, event: &InputEvent, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let text = entity.read(cx).value().to_string();
        let typing = match field {
            ScriptField::Pre => "pre_script",
            ScriptField::Post => "post_script",
        };
        view.edit_request(&tab_id, Some(typing), cx, |request| match field {
            ScriptField::Pre => request.pre_script = text,
            ScriptField::Post => request.post_script = text,
        });
    })
    .detach();
    ScriptEditor { editor, completion }
}

/// A `::: pre` or `::: post` code editor and its completion provider, kept for `Ctrl+Space`.
pub(crate) struct ScriptEditor {
    pub(crate) editor: Entity<EditorState>,
    pub(crate) completion: Rc<ScriptCompletionProvider>,
}

/// The `gpui` entities behind the request editor's editable fields, for whichever tab is
/// currently active. See the module docs for when they are rebuilt versus reused.
#[derive(Default)]
pub(crate) struct RequestEditorEntities {
    built_for: Option<BuildKey>,
    url: Option<Entity<InputState>>,
    /// The inline `Input` for typing a custom method token, shown by [`UrlBar`] in place of the
    /// dropdown while [`AppView::editing_method`] is `true`. Its value is seeded once, when editing
    /// begins ([`AppView::begin_editing_custom_method`]), not resynced here on every render: unlike
    /// the URL, `request.method` deliberately does not change on every keystroke while editing (see
    /// that method's doc comment), so an unconditional resync would fight the user's typing.
    method_input: Option<Entity<InputState>>,
    headers: KeyValueTableEntities,
    query: KeyValueTableEntities,
    form: KeyValueTableEntities,
    /// The inputs of the Auth tab, one per field of the request's auth kind, in display order.
    auth: Vec<(AuthField, Entity<InputState>)>,
    body_editor: Option<Entity<EditorState>>,
    /// How the `{{ }}` markers resolve right now, refreshed on every render of the editor. Every
    /// field that shows markers reads it.
    variable_context: Rc<VariableContext>,
    /// The body editor's marker styling and hover data, rebuilt together with `body_editor`.
    body_markers: Option<BodyMarkerHandles>,
    /// What `body_markers` was last styled for, so it is recomputed only when the context or
    /// the colors change (an edit recomputes it directly, see [`AppView::refresh_body_markers`]).
    body_markers_built_for: Option<(VariableContext, Hsla)>,
    pre_editor: Option<ScriptEditor>,
    post_editor: Option<ScriptEditor>,
    /// What the script editors complete besides the API, refreshed on every render of the
    /// editor like `variable_context`.
    script_completion: Rc<RefCell<ScriptCompletionData>>,
    /// The Docs tab's editor: plain text, with line numbers like the script editors.
    docs: Option<Entity<EditorState>>,
    /// The row whose key input takes the focus once the entities are rebuilt for it: the row
    /// that Tab in the last value just added ([`AppView::add_row_from_tab`]).
    focus_after_rebuild: Option<(RowKind, usize)>,
    /// The last header click of each table of this tab ([`AppView::sort_table`]), for its arrow
    /// and the next click's order. Cleared when another tab becomes active.
    last_sort: HashMap<RowKind, (SortColumn, SortOrder)>,
    /// Rebuild on the next sync even with the same [`BuildKey`]: sorting moves rows without
    /// changing their count.
    force_rebuild: bool,
    /// Rewrite every field's text from the request on the next sync, once: an undo or redo
    /// (#109) put back a whole request without typing into the fields, or the Body tab's
    /// "Format" button rewrote the body outside the editor. Cheaper and kinder than a rebuild,
    /// which would take the focus and the cursor away from the field being edited.
    pub(crate) resync_values: bool,
}

/// The handles that style and explain the markers of the body editor.
struct BodyMarkerHandles {
    decorations: TextDecorationCollection,
    shared: Rc<RefCell<BodyMarkers>>,
}

impl RequestEditorEntities {
    /// Rebuilds every entity when `request`'s shape (see [`BuildKey`]) differs from what they
    /// were last built for, then, unconditionally, keeps the URL input's displayed text in sync
    /// with `request.url`, in case something other than typing in it changed the URL without
    /// changing the build key. Returns whether a rebuild
    /// happened, so the caller can reset [`AppView::editing_method`] (a fresh `method_input`
    /// entity would otherwise show empty while still claiming to be "being edited").
    fn sync(
        &mut self,
        tab_id: &str,
        request: &Request,
        show_auth_secrets: bool,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> bool {
        let key = BuildKey::of(tab_id, request);
        let rebuilt =
            std::mem::take(&mut self.force_rebuild) || self.built_for.as_ref() != Some(&key);
        if self.built_for.as_ref().map(|built| built.tab_id.as_str()) != Some(tab_id) {
            self.last_sort.clear();
        }
        if std::mem::take(&mut self.resync_values) && !rebuilt {
            self.resync_values(request, window, cx);
        }
        if rebuilt {
            self.rebuild(tab_id, request, show_auth_secrets, window, cx);
            self.built_for = Some(key);
            self.body_markers_built_for = None;
            if let Some((kind, index)) = self.focus_after_rebuild.take()
                && let Some(row) = self.table(kind).rows.get(index)
            {
                row.key.update(cx, |state, cx| state.focus(window, cx));
            }
        }

        if let Some(url) = &self.url {
            let current = url.read(cx).value();
            if current.as_ref() != request.url.as_str() {
                let new_value = request.url.clone();
                url.update(cx, |state, cx| state.set_value(new_value, window, cx));
            }
        }

        rebuilt
    }

    /// The entities of the table that `kind` names.
    /// Sets the text of every table cell, auth field, script, the docs and the body from
    /// `request` where it differs. The URL is kept in sync on every sync already; the body used
    /// to be too, but comparing its text on every render could race a composing IME keystroke
    /// and wipe it mid-composition, corrupting what was typed. Only called once after an undo,
    /// a redo, or the Body tab's "Format" button, never per render.
    fn resync_values(&mut self, request: &Request, window: &mut Window, cx: &mut Context<AppView>) {
        let form: &[KeyValue] = match &request.body {
            Body::Form(rows) | Body::Multipart(rows) => rows,
            _ => &[],
        };
        for (kind, rows) in [
            (RowKind::Headers, request.headers.as_slice()),
            (RowKind::Query, request.query.as_slice()),
            (RowKind::Form, form),
        ] {
            for (entities, row) in self.table(kind).rows.iter().zip(rows) {
                set_input_if_changed(&entities.key, &row.key, window, cx);
                set_input_if_changed(&entities.value, &row.value, window, cx);
            }
        }
        for (field, input) in &self.auth {
            set_input_if_changed(
                input,
                auth_edit::field_value(&request.auth, *field),
                window,
                cx,
            );
        }
        let editors = [
            (
                self.pre_editor.as_ref().map(|script| &script.editor),
                &request.pre_script,
            ),
            (
                self.post_editor.as_ref().map(|script| &script.editor),
                &request.post_script,
            ),
            (self.docs.as_ref(), &request.docs),
        ];
        for (editor, text) in editors {
            if let Some(editor) = editor {
                set_editor_if_changed(editor, text, window, cx);
            }
        }
        if let Some(editor) = &self.body_editor
            && let Some((text, _language)) = body_language(&request.body)
        {
            set_editor_if_changed(editor, text, window, cx);
            self.body_markers_built_for = None;
        }
    }

    fn table(&self, kind: RowKind) -> &KeyValueTableEntities {
        match kind {
            RowKind::Headers => &self.headers,
            RowKind::Query => &self.query,
            RowKind::Form => &self.form,
        }
    }

    fn rebuild(
        &mut self,
        tab_id: &str,
        request: &Request,
        show_auth_secrets: bool,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://..."));
        url.update(cx, |state, cx| {
            state.set_value(request.url.clone(), window, cx)
        });
        {
            let tab_id = tab_id.to_string();
            cx.subscribe(
                &url,
                move |view, entity, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        let value = entity.read(cx).value().to_string();
                        view.edit_request(&tab_id, Some("url"), cx, |request| request.url = value);
                        view.on_completion_field_changed(CompletionField::Url, cx);
                    }
                    InputEvent::Blur => view.on_completion_field_blurred(CompletionField::Url, cx),
                    _ => {}
                },
            )
            .detach();
        }
        self.url = Some(url);

        let method_input = cx.new(|cx| InputState::new(window, cx).placeholder("METHOD"));
        {
            let tab_id = tab_id.to_string();
            cx.subscribe(
                &method_input,
                move |view, entity, event: &InputEvent, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let text = entity.read(cx).value().to_string();
                        view.commit_custom_method(&tab_id, text, false, cx);
                    }
                    InputEvent::Blur => {
                        let text = entity.read(cx).value().to_string();
                        view.commit_custom_method(&tab_id, text, true, cx);
                    }
                    _ => {}
                },
            )
            .detach();
        }
        self.method_input = Some(method_input);

        self.headers =
            KeyValueTableEntities::build(tab_id, &request.headers, RowKind::Headers, window, cx);
        self.query =
            KeyValueTableEntities::build(tab_id, &request.query, RowKind::Query, window, cx);
        self.form = match &request.body {
            Body::Form(rows) | Body::Multipart(rows) => {
                KeyValueTableEntities::build(tab_id, rows, RowKind::Form, window, cx)
            }
            _ => KeyValueTableEntities::default(),
        };
        self.auth = AuthKind::of(&request.auth)
            .fields()
            .iter()
            .map(|&field| {
                let text = auth_edit::field_value(&request.auth, field);
                let input = build_auth_input(tab_id, field, text, show_auth_secrets, window, cx);
                (field, input)
            })
            .collect();

        self.body_markers = None;
        self.body_editor = body_language(&request.body).map(|(text, language)| {
            // `auto_close(false)`: gpui-component's auto closing quote/bracket pairs can insert
            // at the start of the document instead of the cursor, corrupting what is typed
            // (notably `"` in JSON). Disabled everywhere a user types in this app until that is
            // fixed upstream.
            let editor =
                cx.new(|cx| EditorState::new(window, cx).language(language).auto_close(false));
            editor.update(cx, |state, cx| {
                state.set_value(text.to_string(), window, cx)
            });
            let shared = Rc::new(RefCell::new(BodyMarkers::default()));
            let decorations = editor.update(cx, |state, cx| {
                state.lsp_mut().hover_provider = Some(Rc::new(VariableHover::new(shared.clone())));
                state.refresh(cx);
                state.create_decorations_collection(Vec::new(), cx)
            });
            self.body_markers = Some(BodyMarkerHandles {
                decorations,
                shared,
            });
            let tab_id = tab_id.to_string();
            cx.subscribe(&editor, move |view, entity, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = entity.read(cx).value().to_string();
                view.edit_request(&tab_id, Some("body"), cx, |request| {
                    request_edit::set_body_text(request, text)
                });
                view.refresh_body_markers(cx);
            })
            .detach();
            editor
        });

        self.pre_editor = Some(make_script_editor(
            tab_id,
            &request.pre_script,
            ScriptField::Pre,
            &self.script_completion,
            window,
            cx,
        ));
        self.post_editor = Some(make_script_editor(
            tab_id,
            &request.post_script,
            ScriptField::Post,
            &self.script_completion,
            window,
            cx,
        ));

        let docs = cx.new(|cx| EditorState::new(window, cx).language("text").auto_close(false));
        docs.update(cx, |state, cx| {
            state.set_value(request.docs.clone(), window, cx)
        });
        {
            let tab_id = tab_id.to_string();
            cx.subscribe(&docs, move |view, entity, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                let text = entity.read(cx).value().to_string();
                view.edit_request(&tab_id, Some("docs"), cx, |request| request.docs = text);
            })
            .detach();
        }
        self.docs = Some(docs);
    }
}

/// Returns the `Vec<KeyValue>` of `request` that `kind` edits, or `None` for `RowKind::Form`
/// when the body currently is not `Body::Form` (which should not happen while its table is
/// shown, but is handled instead of panicking).
fn rows_for_mut(request: &mut Request, kind: RowKind) -> Option<&mut Vec<KeyValue>> {
    match kind {
        RowKind::Headers => Some(&mut request.headers),
        RowKind::Query => Some(&mut request.query),
        RowKind::Form => match &mut request.body {
            Body::Form(rows) | Body::Multipart(rows) => Some(rows),
            _ => None,
        },
    }
}

impl AppView {
    /// Applies a discrete edit (a checkbox, a row button, a menu) to the request of the tab
    /// `tab_id`: one undo step. See [`Self::edit_request`].
    pub(crate) fn edit_active_request(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Request),
    ) {
        self.edit_request(tab_id, None, cx, f);
    }

    /// Applies `f` to the request of the tab with id `tab_id`, records the result in its undo
    /// history, marks it dirty, and re-renders. The single place every request edit funnels
    /// through, so typing, a checkbox and a row button all mark the tab dirty and become undo
    /// steps the same way. `typing` names the field when the edit is typing in it, so a burst
    /// of typing in one field is one undo step (`state::edit_history`).
    pub(crate) fn edit_request(
        &mut self,
        tab_id: &str,
        typing: Option<&str>,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Request),
    ) {
        let Some(index) = self.state.tabs.index_of(tab_id) else {
            return;
        };
        let Some(tab) = self.state.tabs.get_mut(index) else {
            return;
        };
        let Some(request) = tab.request_mut() else {
            return;
        };
        f(request);
        tab.record_edit(typing, Instant::now());
        cx.notify();
    }

    /// Switches the body type of the tab `tab_id`, keeping what the other types held for when
    /// the user switches back (`state::request_edit::BodyStash`), and marks the tab dirty.
    fn set_body_kind(&mut self, tab_id: &str, kind: BodyKind, cx: &mut Context<Self>) {
        let Some(index) = self.state.tabs.index_of(tab_id) else {
            return;
        };
        let Some(tab) = self.state.tabs.get_mut(index) else {
            return;
        };
        tab.set_body_kind(kind);
        tab.record_edit(None, Instant::now());
        cx.notify();
    }

    /// Switches the auth type of the tab `tab_id`, keeping what the other types held for when
    /// the user switches back (`state::auth_edit::AuthStash`), and marks the tab dirty.
    fn set_auth_kind(&mut self, tab_id: &str, kind: AuthKind, cx: &mut Context<Self>) {
        let Some(index) = self.state.tabs.index_of(tab_id) else {
            return;
        };
        let Some(tab) = self.state.tabs.get_mut(index) else {
            return;
        };
        tab.set_auth_kind(kind);
        tab.record_edit(None, Instant::now());
        cx.notify();
    }

    /// Shows the secret fields of the Auth tab of `tab_id` in plain text, or masks them again
    /// (the eye button at their end). Per tab and never saved, so the tab is not marked dirty.
    fn toggle_auth_secrets(&mut self, tab_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.state.tabs.index_of(tab_id) else {
            return;
        };
        let Some(tab) = self.state.tabs.get_mut(index) else {
            return;
        };
        tab.show_auth_secrets = !tab.show_auth_secrets;
        let masked = !tab.show_auth_secrets;
        for (field, input) in &self.request_editor.auth {
            if field.is_secret() {
                input.update(cx, |state, cx| state.set_masked(masked, window, cx));
            }
        }
        cx.notify();
    }

    /// Enters custom-method edit mode: seeds `method_input` with `prefill` (the current custom
    /// token, or empty for a fresh "Custom..." pick, see [`UrlBar`]'s own "Custom..." handler) and
    /// shows it in place of the method dropdown. Deliberately does not write `prefill` into
    /// `request.method` yet: only a valid, confirmed edit does that (`commit_custom_method`), so
    /// opening the editor and clicking away without typing anything never leaves an empty custom
    /// method behind.
    pub(crate) fn begin_editing_custom_method(
        &mut self,
        prefill: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(input) = &self.request_editor.method_input {
            input.update(cx, |state, cx| state.set_value(prefill, window, cx));
        }
        self.editing_method = true;
        cx.notify();
    }

    /// Commits a custom-method edit: if `text` (trimmed) is a valid method token
    /// (`state::request_edit::is_valid_custom_method_token`, the same rule `brasa-format` uses
    /// when reading a request line), writes it into `request.method` and exits edit mode.
    /// Otherwise, on `Blur` (`exit_on_invalid`) exits edit mode anyway, discarding the invalid
    /// edit; on `PressEnter`, stays in edit mode so the user can keep fixing it.
    pub(crate) fn commit_custom_method(
        &mut self,
        tab_id: &str,
        text: String,
        exit_on_invalid: bool,
        cx: &mut Context<Self>,
    ) {
        let trimmed = text.trim().to_string();
        if request_edit::is_valid_custom_method_token(&trimmed) {
            self.edit_active_request(tab_id, cx, |request| {
                request.method = Method::Custom(trimmed);
            });
            self.editing_method = false;
        } else if exit_on_invalid {
            self.editing_method = false;
        }
        cx.notify();
    }

    /// Applies `f` to one row of a key-value table.
    fn edit_row(
        &mut self,
        tab_id: &str,
        kind: RowKind,
        index: usize,
        typing: Option<&str>,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut KeyValue),
    ) {
        self.edit_request(tab_id, typing, cx, |request| {
            if let Some(rows) = rows_for_mut(request, kind)
                && let Some(row) = rows.get_mut(index)
            {
                f(row);
            }
        });
    }

    /// Appends a new, empty row to a key-value table.
    fn add_row(&mut self, tab_id: &str, kind: RowKind, cx: &mut Context<Self>) {
        self.edit_active_request(tab_id, cx, |request| {
            if let Some(rows) = rows_for_mut(request, kind) {
                request_edit::add_row(rows);
            }
        });
    }

    /// Tab in the value of a table's last row: appends an empty row and, once the editor has
    /// rebuilt its inputs for it, puts the cursor in its key.
    fn add_row_from_tab(&mut self, tab_id: &str, kind: RowKind, cx: &mut Context<Self>) {
        let Some(len) = self
            .state
            .tabs
            .active()
            .and_then(|tab| tab.request())
            .map(|request| match kind {
                RowKind::Headers => request.headers.len(),
                RowKind::Query => request.query.len(),
                RowKind::Form => match &request.body {
                    Body::Form(rows) | Body::Multipart(rows) => rows.len(),
                    _ => 0,
                },
            })
        else {
            return;
        };
        self.request_editor.focus_after_rebuild = Some((kind, len));
        self.add_row(tab_id, kind, cx);
    }

    /// A click on the Key or Value header of a table: sorts its rows by that column, A to Z,
    /// or Z to A on a second click (`state::request_edit::next_sort_order`). The inputs follow
    /// their rows by index, so they are rebuilt.
    fn sort_table(
        &mut self,
        tab_id: &str,
        kind: RowKind,
        column: SortColumn,
        cx: &mut Context<Self>,
    ) {
        let last = self.request_editor.last_sort.get(&kind).copied();
        let mut order = SortOrder::Ascending;
        self.edit_active_request(tab_id, cx, |request| {
            if let Some(rows) = rows_for_mut(request, kind) {
                order = request_edit::next_sort_order(rows, last, column);
                request_edit::sort_rows(rows, column, order);
            }
        });
        self.request_editor.last_sort.insert(kind, (column, order));
        self.request_editor.force_rebuild = true;
    }

    /// Removes a row from a key-value table.
    fn remove_row(&mut self, tab_id: &str, kind: RowKind, index: usize, cx: &mut Context<Self>) {
        self.edit_active_request(tab_id, cx, |request| {
            if let Some(rows) = rows_for_mut(request, kind) {
                request_edit::remove_row(rows, index);
            }
        });
    }

    /// Applies the current language to every input placeholder the request editor cached when it
    /// built its inputs. Call it after a language change.
    pub(crate) fn relocalize_request_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = &self.request_editor;
        for table in [&editor.headers, &editor.query, &editor.form] {
            for row in &table.rows {
                row.key.update(cx, |input, cx| {
                    input.set_placeholder(t!("request.kv.key"), window, cx);
                });
                row.value.update(cx, |input, cx| {
                    input.set_placeholder(t!("common.value"), window, cx);
                });
            }
        }
    }

    /// Renders the active tab's request editor, or a placeholder when no tab is open.
    pub(crate) fn render_request_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.state.tabs.active() else {
            self.request_editor = RequestEditorEntities::default();
            return placeholder(cx, t!("request.empty.open_request"));
        };
        let tab_id = tab.id.clone();
        let show_auth_secrets = tab.show_auth_secrets;
        let Some(request) = tab.request().cloned() else {
            self.request_editor = RequestEditorEntities::default();
            return placeholder(cx, t!("request.empty.open_request"));
        };
        if self
            .request_editor
            .sync(&tab_id, &request, show_auth_secrets, window, cx)
        {
            // The inputs the menu belonged to are gone.
            self.completion = None;
            // A fresh `method_input` (built empty) would otherwise show as "being edited" with
            // nothing in it: a rebuild happens on a tab switch or a row/body-kind change, none
            // of which should leave a stale custom-method editor open.
            self.editing_method = false;
        }
        // Computed fresh on every render rather than from a separate `InputState` subscription,
        // since every edit that could change it (URL, query, headers, body) already goes through
        // `edit_active_request`, which calls `cx.notify()` and so triggers exactly this render.
        // Names the pre script sets with `vars.set(...)` count as defined too: the script never
        // runs while editing, so it cannot be seen resolving them the way a real send does.
        let environment = self.active_environment();
        let context = VariableContext::new(
            self.state.active_environment.clone(),
            &environment.variables,
            self.state.session_env.as_slice(),
            script_heuristics::vars_set_names(&request.pre_script),
        );
        if *self.request_editor.variable_context != context {
            self.request_editor.variable_context = Rc::new(context);
        }
        {
            let mut data = self.request_editor.script_completion.borrow_mut();
            data.variables = self.request_editor.variable_context.clone();
            data.response = self.response_shapes.get(&tab_id).cloned();
        }
        self.sync_body_markers(cx);

        // A code editor (the Body tab's JSON/Text/XML, Pre, Post, Docs) scrolls its own content and
        // needs a real, determinate height to fill: an ancestor `overflow_y_scroll()` container
        // instead measures its child's intrinsic height, which collapses a `flex_1` editor to a
        // couple of lines, and an editor taller than the pane would be clipped where it cannot
        // scroll the cursor into view (#128). Params/Headers/Form (a `KeyValueTable`) and Auth
        // are plain content with no scrolling of their own, so they still need it here.
        let uses_code_editor = match self.active_request_tab {
            RequestTab::Body => {
                matches!(request.body, Body::Json(_) | Body::Text(_) | Body::Xml(_))
            }
            RequestTab::Pre | RequestTab::Post | RequestTab::Docs => true,
            _ => false,
        };
        let content = div().id("request-editor-content").flex_1().min_h_0();
        let content = if uses_code_editor {
            content
        } else {
            content.overflow_y_scroll()
        };

        v_flex()
            .size_full()
            .child(self.render_method_url_bar(&tab_id, &request, cx))
            .child(self.render_request_tab_bar(&request, cx))
            .child(
                content
                    // Horizontal inset matches the URL bar's and the tab row's own `px(14.0)`,
                    // so the Key/Value table and every tab's content line up with them instead
                    // of sitting a few px further in.
                    .px(px(14.0))
                    .py(px(10.0))
                    .child(self.render_request_tab_content(&tab_id, &request, cx)),
            )
            .into_any_element()
    }

    /// Renders the method selector, URL input (joined in one `UrlBar`) and Send/Cancel controls.
    fn render_method_url_bar(
        &self,
        tab_id: &str,
        request: &Request,
        cx: &Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let context = self.request_editor.variable_context.clone();
        let Some(url_input) = self.request_editor.url.clone() else {
            return div().into_any_element();
        };

        let method_tab_id = tab_id.to_string();
        let method_weak = weak.clone();
        let editing_method_input = if self.editing_method {
            self.request_editor.method_input.clone()
        } else {
            None
        };
        let app_focus_handle = self.focus_handle.clone();
        let chip_click_weak = weak.clone();
        let chip_context = context.clone();
        let url_bar = UrlBar::new(
            request.method.clone(),
            request.url.clone(),
            variable_spans(&request.url),
            context,
            url_input,
        )
        .editing_method(editing_method_input)
        .on_escape(move |window, cx| app_focus_handle.focus(window, cx))
        .on_method_change(move |method, window, cx| {
            let _ = method_weak.update(cx, |view, cx| {
                if let Method::Custom(prefill) = method {
                    // Only ever reached via `UrlBar`'s "Custom..." menu item: opens the inline
                    // editor instead of committing an empty method right away (see
                    // `begin_editing_custom_method`'s doc comment).
                    view.begin_editing_custom_method(prefill, window, cx);
                } else {
                    view.edit_active_request(&method_tab_id, cx, |request| request.method = method);
                }
            });
        })
        // Takes an unknown variable, or one from the active environment, to the environment
        // editor (`views/define_variable.rs`). A chip set by a script or the session does nothing.
        .on_chip_click(move |name, window, cx| {
            if chip_context.opens_environment(&name) {
                cx.stop_propagation();
                let _ = chip_click_weak
                    .update(cx, |view, cx| view.define_variables(vec![name], window, cx));
            }
        });

        let sending = self.is_sending(tab_id);
        let send_weak = weak.clone();
        let cancel_weak = weak;

        let controls = if sending {
            h_flex()
                .gap_1()
                .child(
                    Button::new("send")
                        .primary()
                        .small()
                        .label(t!("request.send"))
                        .loading(true)
                        .disabled(true),
                )
                .child(
                    Button::new("cancel-send")
                        .neutral_ghost(cx, cx.palette().fg_muted)
                        .small()
                        .label(t!("common.cancel"))
                        .on_click(move |_, _, cx| {
                            let _ = cancel_weak.update(cx, |view, cx| view.cancel_send(cx));
                        }),
                )
        } else {
            h_flex().child(
                PrimaryButton::new("send", t!("request.send"))
                    .icon(Icon::new(gpui_kit::assets::IconName::Send))
                    .height(SEND_BUTTON_HEIGHT)
                    .min_width(SEND_BUTTON_MIN_WIDTH)
                    .on_click(move |_, _, cx| {
                        let _ = send_weak.update(cx, |view, cx| view.send_active_tab(cx));
                    }),
            )
        };

        h_flex()
            .gap_2()
            .items_center()
            .pt(px(12.0))
            .pb(px(8.0))
            .px(px(14.0))
            .child(div().flex_1().child(url_bar))
            .child(controls)
            .into_any_element()
    }

    /// Renders the Params/Headers/Body/Pre-request/Post-response/Docs tab bar, with the enabled row
    /// count next to Params and Headers, and a "Code" ghost button at the right that opens the
    /// snippet dialog.
    fn render_request_tab_bar(&self, request: &Request, cx: &Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        let active = self.active_request_tab;
        let enabled_params = request.query.iter().filter(|row| row.enabled).count();
        let enabled_headers = request.headers.iter().filter(|row| row.enabled).count();

        let mut bar = UnderlineTabs::new("request-tabs");
        for tab in RequestTab::ALL {
            let select_weak = weak.clone();
            let mut item = UnderlineTabItem::new(tab.label()).selected(tab == active);
            match tab {
                RequestTab::Params if enabled_params > 0 => {
                    item = item.count(enabled_params.to_string());
                }
                RequestTab::Headers if enabled_headers > 0 => {
                    item = item.count(enabled_headers.to_string());
                }
                _ => {}
            }
            item = item.on_click(move |_, cx| {
                let _ = select_weak.update(cx, |view, cx| {
                    view.active_request_tab = tab;
                    cx.notify();
                });
            });
            bar = bar.item(item);
        }
        let code_weak = weak;
        bar = bar.suffix(
            GhostButton::new("request-code-snippet", t!("common.code"))
                .icon(gpui_kit::assets::IconName::Code)
                .on_click(move |_, window, cx| {
                    let _ = code_weak.update(cx, |view, cx| view.open_snippet_dialog(window, cx));
                }),
        );
        div().px(px(14.0)).child(bar).into_any_element()
    }

    /// Renders the content of whichever request editor tab is active.
    fn render_request_tab_content(
        &self,
        tab_id: &str,
        request: &Request,
        cx: &Context<Self>,
    ) -> AnyElement {
        match self.active_request_tab {
            RequestTab::Params => self.render_key_value_table(
                tab_id,
                RowKind::Query,
                &request.query,
                &self.request_editor.query,
                cx,
            ),
            RequestTab::Headers => self.render_key_value_table(
                tab_id,
                RowKind::Headers,
                &request.headers,
                &self.request_editor.headers,
                cx,
            ),
            RequestTab::Auth => self.render_auth_tab(tab_id, request, cx),
            RequestTab::Body => self.render_body_tab(tab_id, request, cx),
            RequestTab::Pre => self.render_script_tab(
                tab_id,
                request.pre_enabled,
                ScriptField::Pre,
                &self.request_editor.pre_editor,
                cx,
            ),
            RequestTab::Post => self.render_script_tab(
                tab_id,
                request.post_enabled,
                ScriptField::Post,
                &self.request_editor.post_editor,
                cx,
            ),
            RequestTab::Docs => render_editor_or_placeholder(&self.request_editor.docs, cx),
        }
    }

    /// Renders a Pre-request/Post-response tab: a switch the user flips explicitly to run
    /// `field`'s script, above its code editor. Writing in the editor never turns it back on by
    /// itself: the script is kept either way, so a disabled one still round-trips and can be
    /// re-enabled without retyping it.
    fn render_script_tab(
        &self,
        tab_id: &str,
        enabled: bool,
        field: ScriptField,
        editor: &Option<ScriptEditor>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let tab_id = tab_id.to_string();
        let switch_id = match field {
            ScriptField::Pre => "pre-script-enabled",
            ScriptField::Post => "post-script-enabled",
        };

        let toggle = h_flex()
            .items_center()
            .gap_2()
            .pb(px(8.0))
            .child(Switch::new(switch_id).checked(enabled).on_click(
                move |checked, _, cx| {
                    let checked = *checked;
                    let _ = weak.update(cx, |view, cx| {
                        view.edit_active_request(&tab_id, cx, move |request| match field {
                            ScriptField::Pre => request.pre_enabled = checked,
                            ScriptField::Post => request.post_enabled = checked,
                        });
                    });
                },
            ))
            .child(
                div()
                    .text_sm()
                    .text_color(palette.fg_muted)
                    .child(t!("request.script.enabled")),
            );

        v_flex()
            .size_full()
            .child(toggle)
            .child(
                div().flex_1().min_h_0().child(render_editor_or_placeholder(
                    &editor.as_ref().map(|script| script.editor.clone()),
                    cx,
                )),
            )
            .into_any_element()
    }

    /// Renders the Auth tab, centered: the auth type selector, then a labeled field per value of
    /// the selected type (and where an API key goes), and a warning when a header or param
    /// written by hand is sent instead.
    fn render_auth_tab(&self, tab_id: &str, request: &Request, cx: &Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        let palette = cx.palette();
        let current_kind = AuthKind::of(&request.auth);

        let mut type_selector = SegmentedControl::new("auth-type");
        for kind in AuthKind::ALL {
            let select_weak = weak.clone();
            let tab_id_owned = tab_id.to_string();
            type_selector = type_selector.item(
                SegmentedItem::new(kind.label())
                    .selected(kind == current_kind)
                    .on_click(move |_, cx| {
                        let _ = select_weak.update(cx, |view, cx| {
                            view.set_auth_kind(&tab_id_owned, kind, cx);
                        });
                    }),
            );
        }

        let label = |text: String| {
            div()
                .w(px(AUTH_LABEL_WIDTH))
                .flex_shrink_0()
                .text_color(palette.fg_muted)
                .child(text)
        };
        let row = |text: String, field: AnyElement| {
            h_flex()
                .gap_3()
                .items_center()
                .child(label(text))
                .child(div().flex_1().min_w_0().child(field))
        };

        let mut content = v_flex().gap_2();
        if current_kind == AuthKind::None {
            content = content.child(
                div()
                    .py_2()
                    .text_color(palette.fg_muted)
                    .child(t!("request.auth.empty")),
            );
        }
        let context = self.request_editor.variable_context.clone();
        let show_secrets = self
            .state
            .tabs
            .active()
            .is_some_and(|tab| tab.show_auth_secrets);
        for (field, input) in &self.request_editor.auth {
            let text = auth_edit::field_value(&request.auth, *field).to_string();
            let mut value = VariableField::new(
                format!("auth-{field:?}"),
                input.clone(),
                text,
                context.clone(),
            )
            .mask_all(field.is_secret())
            .masked(field.is_secret() && !show_secrets)
            .on_chip_click(define_variable_click(&weak, &context));
            if field.is_secret() {
                let toggle_weak = weak.clone();
                let toggle_tab_id = tab_id.to_string();
                value = value.on_mask_toggle(move |window, cx| {
                    let _ = toggle_weak.update(cx, |view, cx| {
                        view.toggle_auth_secrets(&toggle_tab_id, window, cx);
                    });
                });
            }
            content = content.child(row(field.label(), value.into_any_element()));
        }
        if let Auth::ApiKey { location, .. } = &request.auth {
            let mut selector = SegmentedControl::new("auth-api-key-location");
            for (option, text) in [
                (ApiKeyLocation::Header, t!("request.auth.in_header")),
                (ApiKeyLocation::Query, t!("request.auth.in_query")),
            ] {
                let select_weak = weak.clone();
                let tab_id_owned = tab_id.to_string();
                selector = selector.item(
                    SegmentedItem::new(text.into_owned())
                        .selected(option == *location)
                        .on_click(move |_, cx| {
                            let _ = select_weak.update(cx, |view, cx| {
                                view.edit_active_request(&tab_id_owned, cx, |request| {
                                    auth_edit::set_api_key_location(&mut request.auth, option)
                                });
                            });
                        }),
                );
            }
            content = content.child(row(
                t!("request.auth.add_to").into_owned(),
                h_flex().child(selector).into_any_element(),
            ));
        }
        if let Some(overridden) = auth_edit::overridden_by(request) {
            let text = match overridden {
                Overridden::Header(name) => t!("request.auth.header_wins", name = name),
                Overridden::Param(name) => t!("request.auth.param_wins", name = name),
            };
            content = content.child(div().pt_1().child(InlineMessage::new(
                InlineMessageKind::Warning,
                text.into_owned(),
            )));
        }

        // Left-aligned, like the Body tab's own type selector, instead of a column floating in
        // the middle of the pane: the type picker sits flush with the Params/Headers tables
        // above it, and the form stays at a readable width without being centered.
        v_flex()
            .w_full()
            .gap(px(16.0))
            .child(type_selector)
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(AUTH_FORM_MAX_WIDTH))
                    .gap_3()
                    .child(content),
            )
            .into_any_element()
    }

    /// Renders the Body tab: the body type selector, a "Format" button (JSON bodies only) and
    /// either a code editor (JSON/Text/XML) or the Form key-value table.
    fn render_body_tab(&self, tab_id: &str, request: &Request, cx: &Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        let current_kind = BodyKind::of(&request.body);

        let mut type_selector = SegmentedControl::new("body-type");
        for kind in BodyKind::ALL {
            let select_weak = weak.clone();
            let tab_id_owned = tab_id.to_string();
            type_selector = type_selector.item(
                SegmentedItem::new(body_type_display_label(kind))
                    .selected(kind == current_kind)
                    .on_click(move |_, cx| {
                        let _ = select_weak.update(cx, |view, cx| {
                            view.set_body_kind(&tab_id_owned, kind, cx);
                        });
                    }),
            );
        }

        let format_weak = weak;
        let format_tab_id = tab_id.to_string();
        let is_json = matches!(request.body, Body::Json(_));
        let format_button = GhostButton::new("format-body", t!("request.body.format"))
            .icon(gpui_kit::assets::IconName::WandSparkles)
            .disabled(!is_json)
            .on_click(move |_, _, cx| {
                let _ = format_weak.update(cx, |view, cx| {
                    let indent = view.state.settings.json_indent;
                    view.edit_active_request(&format_tab_id, cx, |request| {
                        if let Body::Json(text) = &request.body
                            && let Some(pretty) = request_edit::format_json_body(text, indent)
                        {
                            request.body = Body::Json(pretty);
                        }
                    });
                    // The body editor is no longer resynced on every render: tell it to pick up
                    // the reformatted text, the same signal undo/redo already sends.
                    view.request_editor.resync_values = true;
                });
            });

        let body_content = match &request.body {
            Body::None => {
                let palette = cx.palette();
                div()
                    .p_2()
                    .text_color(palette.fg_muted)
                    .child(t!("request.body.empty"))
                    .into_any_element()
            }
            Body::Form(rows) => self.render_key_value_table(
                tab_id,
                RowKind::Form,
                rows,
                &self.request_editor.form,
                cx,
            ),
            Body::Multipart(rows) => {
                let palette = cx.palette();
                v_flex()
                    .id("request-body-multipart")
                    .size_full()
                    .overflow_y_scroll()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.0))
                            .text_color(palette.fg_subtle)
                            .child(t!("request.body.multipart_hint")),
                    )
                    .child(div().flex_none().child(self.render_key_value_table(
                        tab_id,
                        RowKind::Form,
                        rows,
                        &self.request_editor.form,
                        cx,
                    )))
                    .into_any_element()
            }
            Body::Json(_) | Body::Text(_) | Body::Xml(_) => {
                render_editor_or_placeholder(&self.request_editor.body_editor, cx)
            }
        };

        v_flex()
            .size_full()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .py(px(8.0))
                    .child(type_selector)
                    .child(format_button),
            )
            .child(div().flex_1().min_h_0().child(body_content))
            .into_any_element()
    }

    /// Renders an editable key-value table (Params, Headers or Form body): one row per entry,
    /// each with an enable checkbox, a key input, a value input and a remove button, plus an
    /// "Add" button at the end.
    fn render_key_value_table(
        &self,
        tab_id: &str,
        kind: RowKind,
        rows: &[KeyValue],
        entities: &KeyValueTableEntities,
        cx: &Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let context = self.request_editor.variable_context.clone();
        let mut table = KeyValueTable::new(("kv-table", kind as u8 as usize));
        for (index, (row, row_entities)) in rows.iter().zip(entities.rows.iter()).enumerate() {
            let toggle_weak = weak.clone();
            let toggle_tab_id = tab_id.to_string();
            let remove_weak = weak.clone();
            let remove_tab_id = tab_id.to_string();
            let value = VariableField::new(
                format!("kv-{}-{index}-value", kind as u8),
                row_entities.value.clone(),
                row.value.clone(),
                context.clone(),
            )
            // The value of a sensitive header never shows in a tooltip, whatever the variable
            // inside it is called.
            .mask_all(kind == RowKind::Headers && is_sensitive_header(&row.key))
            .on_chip_click(define_variable_click(&weak, &context));
            // Tab in the last value adds a row, unless that row is still empty: then Tab moves
            // on as usual. A single-line input lets `IndentInline` (its Tab) bubble up to here.
            let is_last = index + 1 == rows.len();
            let value = if is_last && !(row.key.is_empty() && row.value.is_empty()) {
                let tab_weak = weak.clone();
                let tab_tab_id = tab_id.to_string();
                div()
                    .w_full()
                    .on_action(move |_: &IndentInline, _, cx| {
                        let _ = tab_weak
                            .update(cx, |view, cx| view.add_row_from_tab(&tab_tab_id, kind, cx));
                    })
                    .child(value)
                    .into_any_element()
            } else {
                value.into_any_element()
            };
            table = table.row(
                KeyValueRow::with_elements(
                    VariableField::new(
                        format!("kv-{}-{index}-key", kind as u8),
                        row_entities.key.clone(),
                        row.key.clone(),
                        context.clone(),
                    )
                    .on_chip_click(define_variable_click(&weak, &context)),
                    value,
                )
                .enabled(row.enabled)
                .on_toggle(move |enabled, _, cx| {
                    let _ = toggle_weak.update(cx, |view, cx| {
                        view.edit_row(&toggle_tab_id, kind, index, None, cx, |row| {
                            row.enabled = enabled
                        });
                    });
                })
                .on_delete(move |_, cx| {
                    let _ = remove_weak.update(cx, |view, cx| {
                        view.remove_row(&remove_tab_id, kind, index, cx);
                    });
                }),
            );
        }

        let add_weak = weak.clone();
        let add_tab_id = tab_id.to_string();
        let sort_weak = weak;
        let sort_tab_id = tab_id.to_string();
        let last_sort = self.request_editor.last_sort.get(&kind).copied();
        table
            .on_add(move |_, cx| {
                let _ = add_weak.update(cx, |view, cx| view.add_row(&add_tab_id, kind, cx));
            })
            .sort(request_edit::shown_sort(rows, last_sort))
            .on_sort(move |column, _, cx| {
                let _ = sort_weak.update(cx, |view, cx| {
                    view.sort_table(&sort_tab_id, kind, column, cx)
                });
            })
            .into_any_element()
    }
}

/// The handler of a click on a variable chip: takes an unknown variable, or one from the active
/// environment, to the environment editor (`views/define_variable.rs`).
fn define_variable_click(
    weak: &WeakEntity<AppView>,
    context: &Rc<VariableContext>,
) -> impl Fn(String, &mut Window, &mut App) + 'static {
    let weak = weak.clone();
    let context = context.clone();
    move |name, window, cx| {
        if context.opens_environment(&name) {
            cx.stop_propagation();
            let _ = weak.update(cx, |view, cx| view.define_variables(vec![name], window, cx));
        }
    }
}

impl AppView {
    /// The input entity of `field` in the active tab, if it exists.
    pub(crate) fn completion_input(&self, field: CompletionField) -> Option<Entity<InputState>> {
        let editor = &self.request_editor;
        match field {
            CompletionField::Url => editor.url.clone(),
            CompletionField::Key(kind, index) => editor
                .table(kind)
                .rows
                .get(index)
                .map(|row| row.key.clone()),
            CompletionField::Value(kind, index) => editor
                .table(kind)
                .rows
                .get(index)
                .map(|row| row.value.clone()),
            CompletionField::Auth(field) => editor
                .auth
                .iter()
                .find(|(shown, _)| *shown == field)
                .map(|(_, input)| input.clone()),
        }
    }

    /// The header name of the row of `field`, for a Headers value cell.
    pub(crate) fn completion_header_name(&self, field: CompletionField) -> Option<String> {
        let CompletionField::Value(RowKind::Headers, index) = field else {
            return None;
        };
        self.state
            .tabs
            .active()
            .and_then(|tab| tab.request())
            .and_then(|request| request.headers.get(index))
            .map(|row| row.key.clone())
    }

    /// Whether `field` is the name of an API key sent as a header, which offers header names
    /// like a Headers key. Sent in the query, it offers none.
    pub(crate) fn completes_header_names(&self, field: CompletionField) -> bool {
        field == CompletionField::Auth(AuthField::KeyName)
            && self
                .state
                .tabs
                .active()
                .and_then(|tab| tab.request())
                .is_some_and(|request| {
                    matches!(
                        request.auth,
                        Auth::ApiKey {
                            location: ApiKeyLocation::Header,
                            ..
                        }
                    )
                })
    }

    /// The completion field holding the focus, if any.
    pub(crate) fn focused_completion_field(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<CompletionField> {
        let focused = |input: &Entity<InputState>| input.focus_handle(cx).is_focused(window);
        let editor = &self.request_editor;
        if editor.url.as_ref().is_some_and(focused) {
            return Some(CompletionField::Url);
        }
        if let Some(index) = editor.headers.rows.iter().position(|row| focused(&row.key)) {
            return Some(CompletionField::Key(RowKind::Headers, index));
        }
        if let Some((field, _)) = editor.auth.iter().find(|(_, input)| focused(input)) {
            return Some(CompletionField::Auth(*field));
        }
        [RowKind::Query, RowKind::Headers, RowKind::Form]
            .into_iter()
            .find_map(|kind| {
                let index = editor
                    .table(kind)
                    .rows
                    .iter()
                    .position(|row| focused(&row.value))?;
                Some(CompletionField::Value(kind, index))
            })
    }

    /// The script editor (`::: pre` or `::: post`) that has the focus, if any.
    pub(crate) fn focused_script_editor(&self, window: &Window, cx: &App) -> Option<&ScriptEditor> {
        let editor = &self.request_editor;
        [&editor.pre_editor, &editor.post_editor]
            .into_iter()
            .flatten()
            .find(|script| script.editor.focus_handle(cx).is_focused(window))
    }

    /// Whether the focus is in one of the request editor's single-line fields: the URL, the
    /// custom method, a table cell or an Auth field. Undo and redo there use the tab's history;
    /// the multi-line editors (body, scripts, docs) keep their own.
    fn focused_single_line_field(&self, window: &Window, cx: &App) -> bool {
        let focused = |input: &Entity<InputState>| input.focus_handle(cx).is_focused(window);
        let editor = &self.request_editor;
        editor.url.as_ref().is_some_and(focused)
            || editor.method_input.as_ref().is_some_and(focused)
            || editor.auth.iter().any(|(_, input)| focused(input))
            || [RowKind::Headers, RowKind::Query, RowKind::Form]
                .into_iter()
                .flat_map(|kind| editor.table(kind).rows.iter())
                .any(|row| focused(&row.key) || focused(&row.value))
    }

    /// `Ctrl+Z` (`Cmd+Z`): undoes the last edit of the active request tab (#109), or lets the
    /// key through to the focused input's own undo, see [`UndoRequestEdit`].
    pub(crate) fn on_undo_request_edit(
        &mut self,
        _: &UndoRequestEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_request_history(false, window, cx);
    }

    /// `Ctrl+Shift+Z` or `Ctrl+Y` (`Cmd+Shift+Z`): redoes the last undone edit of the active
    /// request tab, or lets the key through, see [`RedoRequestEdit`].
    pub(crate) fn on_redo_request_edit(
        &mut self,
        _: &RedoRequestEdit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_request_history(true, window, cx);
    }

    /// Steps the active request tab's history back (`redo` false) or forward, then rewrites
    /// the editor's fields from the restored request on the next render. Lets the key through
    /// when the focus is in a text input other than the request editor's single-line fields,
    /// or when the active tab is not a request.
    fn step_request_history(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        let in_input = window
            .context_stack()
            .iter()
            .any(|context| context.contains("Input"));
        if in_input && !self.focused_single_line_field(window, cx) {
            cx.propagate();
            return;
        }
        let Some(tab) = self
            .state
            .tabs
            .active_index()
            .and_then(|index| self.state.tabs.get_mut(index))
            .filter(|tab| tab.request().is_some())
        else {
            cx.propagate();
            return;
        };
        let changed = if redo { tab.redo() } else { tab.undo() };
        if changed {
            log::debug!("request edit {}", if redo { "redone" } else { "undone" });
            self.request_editor.resync_values = true;
            self.completion = None;
            cx.notify();
        }
    }

    /// `Ctrl+D` (`Cmd+Shift+K` on macOS): deletes the lines of the selection in the focused
    /// script editor, as one undo step (`state::line_edit`), or lets the key through elsewhere.
    pub(crate) fn on_delete_line(
        &mut self,
        _: &DeleteLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(script) = self.focused_script_editor(window, cx) else {
            cx.propagate();
            return;
        };
        script.editor.clone().update(cx, |state, cx| {
            let text = state.value();
            let deletion = line_edit::delete_lines(&text, state.selected_range(), state.cursor());
            if deletion.range.is_empty() {
                return;
            }
            state.set_selected_range(deletion.range, cx);
            state.replace("", window, cx);
            state.set_selected_range(deletion.cursor..deletion.cursor, cx);
            // `replace` is silent: emit the change the editor's subscription writes back.
            cx.emit(InputEvent::Change);
        });
    }

    /// Whether the menu of `field` hides every value: the value of a sensitive header or a
    /// secret Auth field, as its tooltips do.
    pub(crate) fn completion_masks_values(&self, field: CompletionField) -> bool {
        if let CompletionField::Auth(field) = field {
            return field.is_secret();
        }
        let CompletionField::Value(RowKind::Headers, index) = field else {
            return false;
        };
        self.state
            .tabs
            .active()
            .and_then(|tab| tab.request())
            .and_then(|request| request.headers.get(index))
            .is_some_and(|row| is_sensitive_header(&row.key))
    }

    /// How the `{{ }}` markers of the active tab resolve, as of its last render.
    pub(crate) fn request_editor_variable_context(&self) -> Rc<VariableContext> {
        self.request_editor.variable_context.clone()
    }

    /// Restyles the body editor's markers when the variable context or the colors changed since
    /// they were last styled.
    fn sync_body_markers(&mut self, cx: &mut Context<Self>) {
        let key = (
            (*self.request_editor.variable_context).clone(),
            cx.palette().accent_text,
        );
        if self.request_editor.body_markers_built_for.as_ref() == Some(&key) {
            return;
        }
        self.request_editor.body_markers_built_for = Some(key);
        self.refresh_body_markers(cx);
    }

    /// Restyles the body editor's `{{ }}` markers (defined in the accent color, undefined in red)
    /// and refreshes what its hover tooltip shows. Called when the body text changes, and from
    /// [`Self::sync_body_markers`] when the variables or the colors do.
    pub(crate) fn refresh_body_markers(&mut self, cx: &mut Context<Self>) {
        let (Some(editor), Some(handles)) = (
            self.request_editor.body_editor.clone(),
            self.request_editor.body_markers.as_ref(),
        ) else {
            return;
        };
        let context = self.request_editor.variable_context.clone();
        let text = editor.read(cx).value();
        let markers = variable_hint::markers(&text, &context);
        let palette = cx.palette();
        let decorations = markers
            .iter()
            .map(|(span, defined)| {
                TextDecoration::new(span.range.clone(), highlight_style(*defined, &palette))
            })
            .collect();
        handles.decorations.set(decorations, cx);
        *handles.shared.borrow_mut() = BodyMarkers {
            spans: markers.into_iter().map(|(span, _)| span).collect(),
            context,
        };
    }
}

/// The body type segmented control's display label: `BodyKind::label()`'s own text, except for
/// `Form`, where the design's segmented control shows the bare word "Form" rather than
/// `BodyKind::label()`'s fuller `"Form (urlencoded)"` (used elsewhere). A display-only override, not a change to `BodyKind::label()`
/// itself.
fn body_type_display_label(kind: BodyKind) -> String {
    match kind {
        BodyKind::Form => t!("request.body.form").into_owned(),
        other => other.label(),
    }
}

/// Renders a code editor entity if present, or an empty placeholder (only possible transiently,
/// before the first [`RequestEditorEntities::sync`] call), as telaio's [`CodeEditor`]: boxed in a
/// bordered, `raised` container with line numbers, as tall as its `flex_1`/`min_h_0` wrapper (see
/// the call sites).
fn render_editor_or_placeholder(
    editor: &Option<Entity<EditorState>>,
    _cx: &Context<AppView>,
) -> AnyElement {
    match editor {
        Some(editor) => CodeEditor::new(editor).into_any_element(),
        None => div().into_any_element(),
    }
}

/// Sets `input`'s text to `text` unless it already shows it.
fn set_input_if_changed(
    input: &Entity<InputState>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    if input.read(cx).value().as_ref() != text {
        let text = text.to_string();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }
}

/// Sets `editor`'s text to `text` unless it already shows it.
fn set_editor_if_changed(
    editor: &Entity<EditorState>,
    text: &str,
    window: &mut Window,
    cx: &mut Context<AppView>,
) {
    if editor.read(cx).value().as_ref() != text {
        let text = text.to_string();
        editor.update(cx, |state, cx| state.set_value(text, window, cx));
    }
}

/// Renders a centered, muted placeholder message filling the panel (13/400 `fg_muted`).
fn placeholder(cx: &Context<AppView>, message: impl Into<SharedString>) -> AnyElement {
    let palette = cx.palette();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(div().text_color(palette.fg_muted).child(message.into()))
        .into_any_element()
}
