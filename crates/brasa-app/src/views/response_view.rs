//! The response viewer panel: status, time and size, the Body/Headers/Tests/Console tabs, and a
//! warnings strip for missing variables and script errors.
//!
//! The response body is the one field with a persistent `gpui` entity of its own (a read-only
//! [`EditorState`], see [`ResponseEditorEntities`]), so it can show line numbers and a working
//! find bar; every other field is read-only, rendered straight from the
//! [`brasa_runner::RunResult`] of the last send.

use std::sync::Arc;

use gpui_kit::base::SelectableText;
use gpui_kit::component::input::EditorState;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::TestResult;
use brasa_runner::{FailedStage, HttpError, RunResult};

use crate::state::locale;
use crate::state::response_render;
use crate::state::settings::max_response_label;
use crate::state::ui_tabs::ResponseTab;
use crate::theme::{Palette, PaletteExt};
use crate::theme::metrics::{
    HEADERS_COL_GAP, HEADERS_HEADER_HEIGHT, HEADERS_LINE_HEIGHT, HEADERS_NAME_WIDTH,
    HEADERS_PADDING_X, HEADERS_ROW_PADDING_Y, RADIUS_LG,
};
use crate::views::components::{
    CodeEditor, EmptyState, IconButton, InlineMessage, InlineMessageKind, PrimaryButton,
    SegmentedControl, SegmentedItem, StatusState, UnderlineTabItem, UnderlineTabs, status_badge,
};

use super::root::AppView;

/// The response body's key hint shown in the "not sent yet" empty state: `Cmd \u{21b5}` on
/// macOS, `Ctrl \u{21b5}` elsewhere (same local-constant pattern as `views/request_editor.rs`'s
/// `SEND_KEY_HINT`, matching `main.rs`'s key bindings).
#[cfg(target_os = "macos")]
const SEND_KEY_HINT: &str = "Cmd+\u{21b5}";
#[cfg(not(target_os = "macos"))]
const SEND_KEY_HINT: &str = "Ctrl+\u{21b5}";

/// The live, read-only `EditorState` behind the response body view: a code editor, not a plain
/// block of text, so it gets the design's line numbers and, wired in
/// [`AppView::render_response_body_tab`], a working find bar (`EditorState::open_search`, which
/// works on a readonly editor same as a writable one). Rebuilt when the active tab, the raw/pretty
/// toggle or the JSON syntax highlighting changes; the shown text is unconditionally resynced
/// afterwards, the same pattern `RequestEditorEntities` uses for its own editors.
///
/// Text past [`HIGHLIGHT_SIZE_LIMIT`] shows unhighlighted: tokenizing and laying out the gutter
/// of a document with hundreds of thousands of lines (a pretty printed body near
/// `SendOptions::max_response_size` routinely is one) visibly stalled the window, the same class
/// of problem `AppView::ensure_pretty` moves pretty printing itself off of, except this part runs
/// inside the editor entity, which only the window's own thread can touch.
const HIGHLIGHT_SIZE_LIMIT: usize = 2_000_000;

/// The most lines [`AppView::render_response_body_tab`] loads into the body editor. Past it,
/// [`render_too_many_lines`] shows instead: a code editor's own per-line bookkeeping (numbers,
/// cursor, folding) stalls the window at this scale even with highlighting and the double
/// raw-then-pretty layout already ruled out, see [`HIGHLIGHT_SIZE_LIMIT`]'s doc comment.
const MAX_EDITOR_LINES: usize = 50_000;

/// How many lines of a body past [`MAX_EDITOR_LINES`] its "View a preview" button loads into the
/// editor: comfortably inside the range this file's own measurements show the editor handles
/// without a stall, so the button can never reintroduce the freeze it exists to avoid.
const PREVIEW_LINE_CAP: usize = 5_000;

/// The most bytes a single line can be before it is treated the same as too many lines
/// (`MAX_EDITOR_LINES`'s own doc comment): a compact (unindented) raw body routinely is one huge
/// line, and laying out a single multi-megabyte run of text stalls the window on its own, line
/// count notwithstanding.
const MAX_EDITOR_LINE_BYTES: usize = 200_000;

#[derive(Default)]
pub(crate) struct ResponseEditorEntities {
    built_for: Option<(String, bool)>,
    language: Option<&'static str>,
    /// The `(tab id, response generation, JSON indentation, raw)` whose text the editor currently
    /// holds, so a resync is decided by comparing this cheap key and never the (possibly huge)
    /// text.
    synced: Option<(String, u64, usize, bool)>,
    editor: Option<Entity<EditorState>>,
    texts: response_render::BodyTextCache,
    /// The `(tab id, response generation, JSON indentation)` a background pretty print is
    /// already running for, so [`AppView::ensure_pretty`] spawns at most one at a time per
    /// response.
    pretty_pending: Option<(String, u64, usize)>,
    /// The `(tab id, response generation)` whose "View a preview" button
    /// (`render_too_many_lines`) was clicked, past [`MAX_EDITOR_LINES`]: the editor then loads
    /// the first [`PREVIEW_LINE_CAP`] lines instead of staying hidden, still never the whole
    /// thing. Reset by picking another tab or resending, the same as everything else keyed by
    /// generation.
    preview_requested: Option<(String, u64)>,
    /// Whether the body is too large for the editor, and the (full or
    /// [`PREVIEW_LINE_CAP`]-truncated) editor text, both last derived from `shown.text`, kept
    /// alongside the exact `Arc` they were derived from. `BodyTextCache::shown`
    /// returns a clone of the same `Arc` (cheap, a refcount bump) while nothing about the
    /// response changed, so comparing with `Arc::ptr_eq` tells whether a body of tens of
    /// megabytes actually needs scanning again, or whether this render (notably one fired by
    /// scrolling the editor, which otherwise redid both scans on every frame) can reuse it.
    derived: Option<(Arc<str>, bool, Arc<str>)>,
}

impl ResponseEditorEntities {
    /// Rebuilds the editor if `tab_id`, `raw` or the pretty language differ from what it was last
    /// built for, then resyncs its text to `text` when the `(tab_id, generation, indent, raw)`
    /// key changed.
    #[allow(clippy::too_many_arguments)]
    fn sync(
        &mut self,
        tab_id: &str,
        generation: u64,
        indent: usize,
        text: &Arc<str>,
        raw: bool,
        pretty_kind: Option<response_render::PrettyKind>,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        // Raw bodies are never highlighted; a pretty one is, as whatever shape it matched, unless
        // it is so large that highlighting it would itself stall the window: unlike pretty
        // printing the text (`AppView::ensure_pretty`), syntax highlighting and laying out the
        // gutter happen inside the editor entity, which only the window's own thread can touch,
        // so there is no backgrounding it away. A few hundred thousand lines of plain text still
        // lay out quickly; the same text tokenized does not.
        let language = if raw || text.len() > HIGHLIGHT_SIZE_LIMIT {
            None
        } else {
            pretty_kind.map(response_render::PrettyKind::language)
        };
        let key = (tab_id.to_string(), raw);
        if self.built_for.as_ref() != Some(&key) || self.language != language {
            let editor = cx.new(|cx| {
                let state = EditorState::new(window, cx);
                match language {
                    Some(language) => state.language(language),
                    None => state,
                }
            });
            self.editor = Some(editor);
            self.built_for = Some(key);
            self.language = language;
            self.synced = None;
        }
        let synced = (tab_id.to_string(), generation, indent, raw);
        if self.synced.as_ref() != Some(&synced)
            && let Some(editor) = &self.editor
        {
            let started = std::time::Instant::now();
            let new_value = SharedString::from(text.clone());
            editor.update(cx, |state, cx| state.set_value(new_value, window, cx));
            log::debug!(
                "response editor of {tab_id} resynced with {} bytes in {} us",
                text.len(),
                started.elapsed().as_micros()
            );
            self.synced = Some(synced);
        }
    }
}

impl AppView {
    /// Renders the active tab's response, a "sending" spinner while it is in flight, or a
    /// placeholder when nothing has been sent yet.
    pub(crate) fn render_response_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab) = self.state.tabs.active() else {
            return EmptyState::new(t!("response.placeholder.not_sent")).into_any_element();
        };
        let tab_id = tab.id.clone();

        if self.is_sending(&tab_id) {
            let palette = cx.palette();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(Spinner::new().large())
                .child(
                    div()
                        .text_color(palette.fg_muted)
                        .child(t!("common.sending")),
                )
                .into_any_element();
        }

        if !self.responses.contains_key(&tab_id) {
            return EmptyState::new(t!("response.placeholder.not_sent"))
                .hint(SEND_KEY_HINT)
                .into_any_element();
        }
        // Each of these reads `self.responses` for as long as it needs to, but no longer: by the
        // time `render_response_tab_content` runs, neither still borrows `self`, so it is free
        // to take `&mut self` for the Body tab's read-only editor entity.
        let warnings = self.render_warnings_strip(&tab_id, cx);
        let tab_bar = self.render_response_tab_bar(&tab_id, cx);
        // The Body tab's read-only `Editor` scrolls its own content and needs a real, determinate
        // height to fill: an ancestor `overflow_y_scroll()` container instead measures its child's
        // intrinsic height, which collapses a `flex_1` editor to a couple of lines.
        // Headers/Tests/Console are plain lists with no scrolling of their own, so they still need
        // it here, with a visible scrollbar laid over the area (the welcome tab's pattern).
        // The Console tab lays out its own fixed filter bar over its own scrolling list.
        let own_scroll = matches!(
            self.active_response_tab,
            ResponseTab::Body | ResponseTab::Console
        );
        let tab_content = self.render_response_tab_content(&tab_id, window, cx);
        let content = if own_scroll {
            div()
                .id("response-content")
                .flex_1()
                .min_h_0()
                .child(tab_content)
                .into_any_element()
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("response-content")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.response_scroll)
                        .child(tab_content),
                )
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::new(&self.response_scroll)),
                )
                .into_any_element()
        };

        v_flex()
            .size_full()
            .child(tab_bar)
            .children(warnings)
            .child(content)
            .into_any_element()
    }

    /// Renders the status badge, time and size, shown as the tab bar's trailing suffix
    /// (on the right of the same 36 px
    /// bordered row as the tabs, not a separate row above them). The badge's label includes the
    /// reason phrase (`state::response_render::reason_phrase`).
    fn render_status_suffix(&self, tab_id: &str, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette();
        // `render_response_view` already checked this tab has a result before calling here.
        let Some(result) = self.responses.get(tab_id) else {
            return div().into_any_element();
        };
        let state = match &result.response {
            Some(response) => StatusState::Code(response.status),
            None => StatusState::NotSent,
        };
        let mut row = h_flex()
            .items_center()
            .gap(px(14.0))
            .child(status_badge(&state));
        if let Some(response) = &result.response {
            row = row
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(response_render::format_duration(response.time)),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(response_render::format_size(response.size)),
                );
        }
        row.into_any_element()
    }

    /// Renders the Body/Headers/Tests/Console tab bar, with the header count, the tests pass/fail
    /// count colored `success`/`danger`, and the status badge/time/size on the right.
    fn render_response_tab_bar(&self, tab_id: &str, cx: &Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        let palette = cx.palette();
        let active = self.active_response_tab;
        let Some(result) = self.responses.get(tab_id) else {
            return div().into_any_element();
        };
        let header_count = result.response.as_ref().map_or(0, |r| r.headers.len());
        let test_count = result.tests.len();
        let passed_count = result.tests.iter().filter(|test| test.passed).count();
        let error_count = self
            .response_consoles
            .get(tab_id)
            .map_or(0, |timeline| timeline.error_count);

        let mut bar = UnderlineTabs::new("response-tabs")
            .height(crate::theme::metrics::RESPONSE_TAB_BAR_HEIGHT);
        for tab in ResponseTab::ALL {
            let select_weak = weak.clone();
            let mut item = UnderlineTabItem::new(tab.label()).selected(tab == active);
            match tab {
                ResponseTab::Headers if header_count > 0 => {
                    item = item.count(header_count.to_string());
                }
                ResponseTab::Tests if test_count > 0 => {
                    item = item
                        .count(format!("{passed_count}/{test_count}"))
                        .count_color(if passed_count == test_count {
                            palette.success
                        } else {
                            palette.danger
                        });
                }
                ResponseTab::Console if error_count > 0 => {
                    item = item
                        .count(error_count.to_string())
                        .count_color(palette.danger);
                }
                _ => {}
            }
            item = item.on_click(move |_, cx| {
                let _ = select_weak.update(cx, |view, cx| {
                    view.active_response_tab = tab;
                    cx.notify();
                });
            });
            bar = bar.item(item);
        }
        bar = bar.suffix(self.render_status_suffix(tab_id, cx));
        div().px(px(14.0)).child(bar).into_any_element()
    }

    /// Renders the content of whichever response tab is active.
    fn render_response_tab_content(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.active_response_tab == ResponseTab::Body {
            return self.render_response_body_tab(tab_id, window, cx);
        }
        if self.active_response_tab == ResponseTab::Console {
            return self.render_console_tab(tab_id, cx);
        }
        let Some(result) = self.responses.get(tab_id) else {
            return div().into_any_element();
        };
        match self.active_response_tab {
            ResponseTab::Headers => render_headers_tab(tab_id, result, cx),
            ResponseTab::Tests => render_tests_tab(result, cx),
            // Handled above, before `result` was even looked up.
            ResponseTab::Body | ResponseTab::Console => unreachable!(),
        }
    }

    /// Renders the response body: a Pretty/Raw segmented control plus `copy` and `search` icon
    /// buttons on the right, then the body in a read-only, line-numbered code editor.
    fn render_response_body_tab(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let generation = self.response_generations.get(tab_id);
        let indent = self.state.settings.json_indent;
        // Scoped so the borrow of `self.responses` ends before `self.response_editor` (below)
        // needs a mutable one: `shown` and `body_for_pretty` are owned by the time this block
        // ends.
        let (shown, body_for_pretty, waiting_on_pretty) = {
            let Some(result) = self.responses.get(tab_id) else {
                return div().into_any_element();
            };
            let Some(response) = &result.response else {
                return div()
                    .p_2()
                    .text_color(palette.fg_muted)
                    .child(t!("response.body.empty"))
                    .into_any_element();
            };
            // `shown` only reads the raw text into a `String` (cheap, logged at a few ms even
            // for a huge body) and syncs the cache's key; it does not touch the editor entity.
            let shown = self.response_editor.texts.shown(
                tab_id,
                generation,
                &response.body,
                self.response_raw,
                indent,
            );
            let pending = self.response_editor.texts.pretty_pending(tab_id, generation, indent);
            // Cloned only when a background pretty print is actually needed below: the common
            // case (already cached, or already pending) never copies the body.
            let needs_pretty = pending
                && self.response_editor.pretty_pending.as_ref()
                    != Some(&(tab_id.to_string(), generation, indent));
            let body_for_pretty = needs_pretty.then(|| response.body.clone());
            // Laying out a huge body in the editor is itself the slow part (measured at ~20s for
            // 900,000 lines, independent of pretty printing or syntax highlighting, both already
            // addressed). Showing raw automatically while pretty is still pending would pay that
            // cost twice: once now for raw, again moments later once pretty lands. Skip the first
            // one and show a lightweight placeholder instead, for a body big enough that the
            // double layout would be felt; the user's own choice of Raw is never skipped, since
            // the editor only ever renders what they picked.
            let waiting_on_pretty =
                !self.response_raw && pending && response.body.len() > HIGHLIGHT_SIZE_LIMIT;
            (shown, body_for_pretty, waiting_on_pretty)
        };
        if let Some(body) = body_for_pretty {
            self.ensure_pretty(tab_id, generation, indent, body, window, cx);
        }
        if waiting_on_pretty {
            return render_formatting_placeholder(&palette);
        }
        let show_raw = shown.raw;
        let text = shown.text;

        // A code editor tracks state per line (numbers, cursor, folding): past some line count
        // that bookkeeping itself stalls the window no matter how small the highlighting or
        // layout work per line is (measured hanging past 10s at ~900,000 lines, even
        // unhighlighted). The same happens the other way around: a raw, unindented body is
        // routinely one single multi-megabyte line, which stalls the window laying out that one
        // enormous run of text, line count notwithstanding. Either way past `MAX_EDITOR_LINES` or
        // `MAX_EDITOR_LINE_BYTES`, skip the editor by default; its own "View a preview" button
        // loads a bounded, always-fast prefix instead (`PREVIEW_LINE_CAP`, itself also capped per
        // line), never the whole thing.
        //
        // `too_large` and `editor_text` are cached by the exact `Arc` they were derived from
        // (`ResponseEditorEntities::derived`'s own doc comment): scanning a body of tens of
        // megabytes, twice over for a preview, on every render, notably one fired by scrolling
        // the editor itself, visibly stuttered that scroll even though nothing about the
        // response had changed since the last render.
        let (too_large, editor_text) = match &self.response_editor.derived {
            Some((source, too_large, editor_text)) if Arc::ptr_eq(source, &text) => {
                (*too_large, editor_text.clone())
            }
            _ => {
                let (line_count, max_line_bytes) = text
                    .lines()
                    .fold((0usize, 0usize), |(count, max_len), line| {
                        (count + 1, max_len.max(line.len()))
                    });
                let too_large = line_count > MAX_EDITOR_LINES || max_line_bytes > MAX_EDITOR_LINE_BYTES;
                let editor_text: Arc<str> = if too_large {
                    Arc::from(preview_prefix(&text, line_count, PREVIEW_LINE_CAP))
                } else {
                    text.clone()
                };
                self.response_editor.derived = Some((text.clone(), too_large, editor_text.clone()));
                (too_large, editor_text)
            }
        };
        if too_large
            && self.response_editor.preview_requested.as_ref() != Some(&(tab_id.to_string(), generation))
        {
            let weak = cx.weak_entity();
            let tab_id_owned = tab_id.to_string();
            return render_too_many_lines(&palette, text.len(), move |_, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.response_editor.preview_requested = Some((tab_id_owned.clone(), generation));
                    cx.notify();
                });
            });
        }
        // Highlighting was disabled here at first, suspecting it of the scroll stutter that
        // turned out to be the redundant line-count/preview rescans above, now cached by `Arc`
        // identity: with those gone, there is no measured reason a bounded, `PREVIEW_LINE_CAP`
        // sized preview can't be highlighted the same as anything else this size.
        let pretty_kind = shown.pretty_kind;

        self.response_editor.sync(
            tab_id,
            generation,
            indent,
            &editor_text,
            show_raw,
            pretty_kind,
            window,
            cx,
        );

        let weak = cx.weak_entity();
        let pretty_available = shown.pretty_kind.is_some();
        let mut toggle = SegmentedControl::new("body-raw-toggle").item(
            SegmentedItem::new(t!("response.body.pretty"))
                .selected(!show_raw)
                .on_click({
                    let weak = weak.clone();
                    move |_, cx| {
                        if pretty_available {
                            let _ = weak.update(cx, |view, cx| {
                                view.response_raw = false;
                                cx.notify();
                            });
                        }
                    }
                }),
        );
        toggle = toggle.item(
            SegmentedItem::new(t!("response.body.raw"))
                .selected(show_raw)
                .on_click({
                    let weak = weak.clone();
                    move |_, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.response_raw = true;
                            cx.notify();
                        });
                    }
                }),
        );

        let copy_text = text.clone();
        let copy_button = IconButton::new("response-copy", IconName::Copy)
            .tooltip(t!("common.copy"))
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.to_string()));
            });

        let search_editor = self.response_editor.editor.clone();
        let search_button = IconButton::new("response-search", IconName::Search)
            .tooltip(t!("common.search"))
            .on_click(move |_, _, cx| {
                if let Some(editor) = &search_editor {
                    editor.update(cx, |state, cx| state.open_search(false, cx));
                }
            });

        let header = h_flex()
            .items_center()
            .justify_between()
            .pt(px(14.0))
            .pb(px(8.0))
            .child(toggle)
            .child(
                h_flex()
                    .gap(px(2.0))
                    .child(copy_button)
                    .child(search_button),
            );

        // Padded and boxed like the request editor's body (`views/request_editor.rs`'s
        // `render_body_tab` and `render_editor_or_placeholder`), so both panels line up. Read
        // only is set on the element: `Input` applies the element's value to the state on every
        // render, so `EditorState::set_readonly` alone was undone and the body could be typed in.
        let body: AnyElement = match &self.response_editor.editor {
            Some(editor) => CodeEditor::new(editor).readonly(true).into_any_element(),
            None => div().into_any_element(),
        };

        v_flex()
            .size_full()
            // Horizontal inset matches the tab bar's own `px(14.0)`, so this lines up with it
            // and with the request editor's content (`views/request_editor.rs`).
            .px(px(14.0))
            .py(px(10.0))
            .gap_2()
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
            .into_any_element()
    }

    /// Spawns a background pretty print of `body` for `(tab_id, generation, indent)`, committing
    /// the result to `BodyTextCache` and notifying once it lands. Pretty printing a body near
    /// `SendOptions::max_response_size` costs real time (that type's own doc comment measures a
    /// 15 MB response), so it never runs on the window's own thread, only here; the caller
    /// (`render_response_body_tab`) makes sure at most one of these runs per response at a time.
    /// A resend or an active-tab change before it finishes makes `BodyTextCache::store_pretty`
    /// drop the result as stale rather than show it against the wrong response.
    fn ensure_pretty(
        &mut self,
        tab_id: &str,
        generation: u64,
        indent: usize,
        body: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = (tab_id.to_string(), generation, indent);
        self.response_editor.pretty_pending = Some(key.clone());
        let tab_id = tab_id.to_string();
        let spawned = std::time::Instant::now();
        cx.spawn_in(window, async move |this, cx| {
            let body_len = body.len();
            let background_started = std::time::Instant::now();
            let pretty = cx
                .background_spawn(async move {
                    let started = std::time::Instant::now();
                    let result = response_render::pretty_print_json(&body, indent)
                        .map(|text| (text, response_render::PrettyKind::Json))
                        .or_else(|| {
                            response_render::pretty_print_html(&body, indent)
                                .map(|text| (text, response_render::PrettyKind::Html))
                        });
                    log::debug!(
                        "ensure_pretty: background_spawn's own closure ran in {} us ({} bytes)",
                        started.elapsed().as_micros(),
                        body_len
                    );
                    result
                })
                .await;
            log::debug!(
                "ensure_pretty: background_spawn().await returned {} us after it was spawned ({} us after the task itself started polling)",
                spawned.elapsed().as_micros(),
                background_started.elapsed().as_micros()
            );
            let _ = this.update(cx, |view, cx| {
                view.response_editor
                    .texts
                    .store_pretty(&tab_id, generation, indent, pretty);
                if view.response_editor.pretty_pending.as_ref() == Some(&key) {
                    view.response_editor.pretty_pending = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Renders the warnings strip above the response body, using [`InlineMessage`]: one warning row
    /// per unknown variable from the live `preview` (before sending and after, since it always
    /// reflects the request's current text, not necessarily what was last sent), one warning row
    /// per function-call error, and one danger row for a failed pipeline stage. `None` when there
    /// is nothing to show.
    ///
    /// Uses the actual [`RunResult::warnings`] of the last send, not the live preview: unlike the
    /// URL chips (which have no real run to look at until the request is sent, and so keep using
    /// the preview plus the `vars.set` heuristic), this method is only ever reached once a result
    /// exists (`render_response_view` returns the "not sent yet" placeholder before it). At that
    /// point the real pipeline already ran the pre script for real, so a variable it set with
    /// `vars.set(...)` is correctly resolved here with no heuristic needed.
    fn render_warnings_strip(&self, tab_id: &str, cx: &Context<Self>) -> Option<AnyElement> {
        let result = self.responses.get(tab_id)?;
        let weak = cx.weak_entity();
        let mut messages: Vec<AnyElement> = response_render::group_warnings(&result.warnings)
            .into_iter()
            .map(|group| match group {
                response_render::WarningGroup::UnknownVariable(name) => {
                    let define_weak = weak.clone();
                    InlineMessage::new(
                        InlineMessageKind::Warning,
                        t!("response.warning.unknown_variable"),
                    )
                    .mono_suffix(name.clone())
                    .action(t!("common.define"), move |window, cx| {
                        let names = vec![name.clone()];
                        let _ = define_weak
                            .update(cx, |view, cx| view.define_variables(names, window, cx));
                    })
                    .into_any_element()
                }
                response_render::WarningGroup::UnknownVariables(names) => {
                    let define_weak = weak.clone();
                    let all_names = names.clone();
                    InlineMessage::new(
                        InlineMessageKind::Warning,
                        t!("response.warning.unknown_variables"),
                    )
                    .mono_suffix(names.join(", "))
                    .action(t!("common.define"), move |window, cx| {
                        let names = all_names.clone();
                        let _ = define_weak
                            .update(cx, |view, cx| view.define_variables(names, window, cx));
                    })
                    .into_any_element()
                }
                response_render::WarningGroup::Function(message) => {
                    InlineMessage::new(InlineMessageKind::Warning, message).into_any_element()
                }
            })
            .collect();

        if let Some(warning) = result
            .response
            .as_ref()
            .and_then(|response| response.tls_warning.clone())
        {
            messages.insert(
                0,
                InlineMessage::new(InlineMessageKind::Warning, warning).into_any_element(),
            );
        }

        if let Some(stage) = &result.failed_stage {
            messages.push(
                InlineMessage::new(InlineMessageKind::Danger, stage_message(stage))
                    .into_any_element(),
            );
        }

        if messages.is_empty() {
            return None;
        }
        // Padded below too: the tab content scrolls under the strip, and without it a scrolled
        // table would touch the last message.
        Some(
            v_flex()
                .px(px(14.0))
                .py(px(8.0))
                .gap_1()
                .children(messages)
                .into_any_element(),
        )
    }
}

/// A short, user-facing message for a failed pipeline stage.
fn stage_message(stage: &FailedStage) -> String {
    match stage {
        FailedStage::Pre(error) => {
            t!("response.stage.pre_failed", error = error.to_string()).into_owned()
        }
        FailedStage::Send(HttpError::BodyTooLarge(limit)) => t!(
            "response.stage.body_too_large",
            limit = max_response_label(u32::try_from(limit / 1_000_000).unwrap_or(u32::MAX))
        )
        .into_owned(),
        FailedStage::Send(error) => {
            t!("response.stage.send_failed", error = error.to_string()).into_owned()
        }
        FailedStage::Post(error) => {
            t!("response.stage.post_failed", error = error.to_string()).into_owned()
        }
    }
}

/// Shown in place of the body editor while a large response's pretty text is still being
/// computed in the background (`AppView::render_response_body_tab`'s `waiting_on_pretty`), so
/// the editor is only ever built once, with the final text.
fn render_formatting_placeholder(palette: &Palette) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(Spinner::new())
        .child(
            div()
                .text_color(palette.fg_muted)
                .child(t!("response.body.formatting")),
        )
        .into_any_element()
}

/// Shown instead of the body editor when `text` has more than [`MAX_EDITOR_LINES`]: a short
/// explanation and a Copy button, the one way left to get the data out without the editor ever
/// having to lay it out. No Copy here: a preview this size is not something anyone copies to the
/// clipboard, only looks at.
fn render_too_many_lines(
    palette: &Palette,
    text_bytes: usize,
    on_view: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .max_w(px(360.0))
                .text_center()
                .text_color(palette.fg_muted)
                .child(t!(
                    "response.body.too_many_lines",
                    size = response_render::format_size(text_bytes),
                    cap = PREVIEW_LINE_CAP
                )),
        )
        .child(
            PrimaryButton::new("response-view-too-large", t!("response.body.view_preview"))
                .icon(Icon::new(IconName::Eye))
                .on_click(move |_, window, cx| on_view(window, cx)),
        )
        .into_any_element()
}

/// The first `max_lines` lines of `text`, each itself cut to [`MAX_EDITOR_LINE_BYTES`], with a
/// note of how many more lines follow, for [`render_too_many_lines`]'s "View a preview" button:
/// always fast to load into the editor regardless of how large `text` actually is, or how long a
/// single line of it is (a raw, unindented body is routinely one huge line).
fn preview_prefix(text: &str, total: usize, max_lines: usize) -> String {
    let mut prefix = String::new();
    let mut shown = 0;
    for line in text.lines().take(max_lines) {
        if line.len() > MAX_EDITOR_LINE_BYTES {
            let cut = line
                .char_indices()
                .map(|(index, _)| index)
                .take_while(|&index| index <= MAX_EDITOR_LINE_BYTES)
                .last()
                .unwrap_or(0);
            prefix.push_str(&line[..cut]);
            prefix.push_str(&t!("response.body.preview_line_truncated"));
        } else {
            prefix.push_str(line);
        }
        prefix.push('\n');
        shown += 1;
    }
    if total > shown {
        prefix.push_str(&t!("response.body.preview_truncated", shown = shown, total = total));
        prefix.push('\n');
    }
    prefix
}

/// Renders the response headers as a two-column card: a header row with the column labels and
/// the "Copy all headers" button, then one row per header, name in `fg_muted` and value in `fg`,
/// both monospace. Values wrap instead of being cut, since cookies and policies run long. Every
/// cell is selectable text (gpui-base's window selection, copied with Ctrl+C or Cmd+C by `Root`),
/// ordered row by row, name before value, so a drag across cells copies them in reading order.
fn render_headers_tab(tab_id: &str, result: &RunResult, cx: &Context<AppView>) -> AnyElement {
    let palette = cx.palette();
    let Some(response) = &result.response else {
        return div()
            .px(px(14.0))
            .py(px(10.0))
            .text_color(palette.fg_muted)
            .child(t!("response.headers.no_response"))
            .into_any_element();
    };
    if response.headers.is_empty() {
        return div()
            .px(px(14.0))
            .py(px(10.0))
            .text_color(palette.fg_muted)
            .child(t!("response.headers.empty"))
            .into_any_element();
    }

    // The text is built on click, from the response shown then, not on every render.
    let weak = cx.weak_entity();
    let copy_tab = tab_id.to_string();
    let copy_button = IconButton::new("response-headers-copy", IconName::Copy)
        .tooltip(t!("response.headers.copy_all"))
        .on_click(move |_, _, cx| {
            let text = weak.upgrade().and_then(|view| {
                let view = view.read(cx);
                let response = view.responses.get(&copy_tab)?.response.as_ref()?;
                Some(response_render::headers_as_text(&response.headers))
            });
            if let Some(text) = text {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        });

    let row = || {
        h_flex()
            .items_start()
            .gap(px(HEADERS_COL_GAP))
            .px(px(HEADERS_PADDING_X))
    };
    let name_cell = |content: AnyElement| {
        div()
            .flex_none()
            .w(px(HEADERS_NAME_WIDTH))
            .min_w_0()
            .child(content)
    };
    let header_row = row()
        .items_center()
        .h(px(HEADERS_HEADER_HEIGHT))
        .border_b_1()
        .border_color(palette.border)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette.fg_subtle)
        .child(name_cell(
            div().child(t!("response.headers.name")).into_any_element(),
        ))
        .child(div().flex_1().child(t!("common.value")))
        // The button keeps the row's height: it overhangs the padding instead of growing it.
        .child(div().flex_none().mr(px(-8.0)).child(copy_button));

    let mono_family = cx.theme().mono_font_family.clone();
    let last = response.headers.len() - 1;
    let rows = response.headers.iter().enumerate().map(|(index, header)| {
        row()
            .py(px(HEADERS_ROW_PADDING_Y))
            .when(index < last, |row| row.border_b_1())
            .border_color(palette.border)
            .font_family(mono_family.clone())
            .child(name_cell(
                div()
                    .text_color(palette.fg_muted)
                    .child(
                        SelectableText::new(("response-header-name", index), header.name.clone())
                            .document_order(index as u64 * 2),
                    )
                    .into_any_element(),
            ))
            .child(
                div().flex_1().min_w_0().text_color(palette.fg).child(
                    SelectableText::new(("response-header-value", index), header.value.clone())
                        .document_order(index as u64 * 2 + 1),
                ),
            )
    });

    div()
        .p_2()
        .child(
            v_flex()
                .border_1()
                .border_color(palette.border)
                .rounded(px(RADIUS_LG))
                .bg(palette.raised)
                .overflow_hidden()
                .text_size(px(12.5))
                .line_height(px(HEADERS_LINE_HEIGHT))
                .child(header_row)
                .children(rows),
        )
        .into_any_element()
}

/// Renders the `test()` results: a pass/fail summary strip, then a pass/fail icon, the test name,
/// and its failure message for each test.
fn render_tests_tab(result: &RunResult, cx: &Context<AppView>) -> AnyElement {
    let palette = cx.palette();
    if result.tests.is_empty() {
        return div()
            .px(px(14.0))
            .py(px(10.0))
            .text_color(palette.fg_muted)
            .child(t!("response.tests.empty"))
            .into_any_element();
    }
    let total = result.tests.len();
    let passed = result.tests.iter().filter(|test| test.passed).count();
    let summary = if passed == total {
        let key = locale::plural_key("response.tests.passed", total);
        InlineMessage::new(
            InlineMessageKind::Success,
            t!(key.as_str(), passed = passed, total = total).into_owned(),
        )
    } else {
        let key = locale::plural_key("response.tests.failed", total);
        InlineMessage::new(
            InlineMessageKind::Danger,
            t!(key.as_str(), failed = total - passed, total = total).into_owned(),
        )
    };

    v_flex()
        .px(px(14.0))
        .py(px(10.0))
        .gap_2()
        .child(summary)
        .children(
            result
                .tests
                .iter()
                .map(|test| render_test_row(test, &palette)),
        )
        .into_any_element()
}

fn render_test_row(test: &TestResult, palette: &crate::theme::Palette) -> AnyElement {
    let (icon, color) = if test.passed {
        (IconName::Check, palette.success)
    } else {
        (IconName::CircleX, palette.danger)
    };
    h_flex()
        .gap_2()
        .items_start()
        .child(Icon::new(icon).small().text_color(color))
        .child(
            v_flex()
                .child(div().text_sm().child(test.name.clone()))
                .children(
                    test.message
                        .clone()
                        .map(|message| div().text_sm().text_color(palette.danger).child(message)),
                ),
        )
        .into_any_element()
}
