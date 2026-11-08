//! The Code snippet dialog: opened from the "Code" ghost button at the right of the request
//! editor's inner tabs row (`views/request_editor.rs`). Shows the active tab's request, resolved
//! against the current environment (`brasa_runner::preview`, never running a script or sending),
//! as a copyable cURL, `fetch` or Python snippet (`brasa_format::snippet::render_snippet`).
//! Unresolved `{{name}}` markers are kept literally, exactly as `render_snippet` already leaves
//! them.

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::ResolvedRequest;
use brasa_format::snippet::{SnippetLanguage, render_snippet};

use crate::theme::PaletteExt;
use crate::theme::metrics::RADIUS_MD;

use super::components::{
    IconButton, ModalPlacement, SegmentedControl, SegmentedItem, framed_dialog, modal_body,
    modal_footer, modal_header,
};
use super::root::AppView;

/// Width of the dialog.
const DIALOG_WIDTH: f32 = 560.0;
/// Max height of the scrollable code view.
const CODE_MAX_HEIGHT: f32 = 340.0;

/// Every language the dialog offers, in the order the `SegmentedControl` shows them, with their
/// display labels.
const LANGUAGES: [(SnippetLanguage, &str); 3] = [
    (SnippetLanguage::Curl, "cURL"),
    (SnippetLanguage::JsFetch, "fetch"),
    (SnippetLanguage::PythonRequests, "Python"),
];

impl AppView {
    /// Opens the Code snippet dialog for the active tab's request. A no-op when a dialog is
    /// already open, or when no tab is open (nothing to show).
    pub(crate) fn open_snippet_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        self.state.reload_environment();
        let Some(preview) = self.current_preview() else {
            return;
        };
        self.snippet_language = SnippetLanguage::Curl;
        cx.notify();

        let resolved = preview.resolved;
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, _cx| {
            let weak = weak.clone();
            let resolved = resolved.clone();
            framed_dialog(
                dialog,
                DIALOG_WIDTH,
                None,
                ModalPlacement::DEFAULT,
                window,
                move |window, cx| {
                    v_flex()
                        .child(modal_header(t!("common.code"), cx))
                        .child(modal_body().child(render_snippet_body(
                            weak.clone(),
                            &resolved,
                            window,
                            cx,
                        )))
                        .child(render_snippet_footer(weak.clone(), resolved.clone(), cx))
                        .into_any_element()
                },
            )
        });
    }
}

/// Renders the language switch and the code view.
fn render_snippet_body(
    weak: WeakEntity<AppView>,
    resolved: &ResolvedRequest,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(view) = weak.upgrade() else {
        return div().into_any_element();
    };
    let language = view.read(cx).snippet_language;
    let palette = cx.palette();
    let mono_font = cx.theme().mono_font_family.clone();
    let mono_size = cx.theme().mono_font_size;
    let text = render_snippet(resolved, language);

    let mut selector = SegmentedControl::new("snippet-language");
    for (candidate, label) in LANGUAGES {
        let select_weak = weak.clone();
        selector = selector.item(
            SegmentedItem::new(label)
                .selected(candidate == language)
                .on_click(move |_, cx| {
                    let _ = select_weak.update(cx, |view, cx| {
                        view.snippet_language = candidate;
                        cx.notify();
                    });
                }),
        );
    }

    v_flex()
        .gap(px(10.0))
        .child(selector)
        .child(
            div()
                .id("snippet-code")
                .max_h(px(CODE_MAX_HEIGHT))
                .overflow_y_scroll()
                .p(px(12.0))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(palette.border)
                .bg(palette.raised)
                .child(
                    v_flex()
                        .font_family(mono_font)
                        .text_size(mono_size)
                        .children(text.lines().map(|line| div().child(line.to_string()))),
                ),
        )
        .into_any_element()
}

/// The footer's "Copy" button: recomputes the snippet for whichever language is currently
/// selected at click time, so it always copies exactly what is on screen.
fn render_snippet_footer(
    weak: WeakEntity<AppView>,
    resolved: ResolvedRequest,
    cx: &App,
) -> AnyElement {
    modal_footer(cx)
        .child(
            IconButton::new("snippet-copy", IconName::Copy)
                .tooltip(t!("common.copy"))
                .on_click(move |_, _, cx| {
                    let Some(view) = weak.upgrade() else {
                        return;
                    };
                    let language = view.read(cx).snippet_language;
                    let text = render_snippet(&resolved, language);
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }),
        )
        .into_any_element()
}
