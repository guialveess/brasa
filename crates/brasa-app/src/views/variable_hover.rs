//! The hover tooltip of the body code editor: the value of the `{{ }}` marker under the pointer.
//!
//! The editor asks a [`HoverProvider`] for a popover after the pointer rests on a position. The
//! provider reads the markers `views/request_editor.rs` computed when the body last changed, so
//! hovering never rescans the text.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::input::{HoverProvider, Rope};
use gpui_kit::*;
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};

use brasa_core::VariableSpan;

use crate::state::variable_hint::{VariableContext, hint};
use crate::views::components::variable_line::tooltip_lines;

/// The markers of the body and how they resolve, shared between the request editor (which
/// refreshes them) and the [`VariableHover`] provider (which reads them).
#[derive(Default)]
pub struct BodyMarkers {
    pub spans: Vec<VariableSpan>,
    pub context: Rc<VariableContext>,
}

/// Answers the body editor's hover requests from [`BodyMarkers`].
pub struct VariableHover {
    markers: Rc<RefCell<BodyMarkers>>,
}

impl VariableHover {
    /// A provider reading `markers`.
    pub fn new(markers: Rc<RefCell<BodyMarkers>>) -> Self {
        Self { markers }
    }
}

impl HoverProvider for VariableHover {
    fn hover(
        &self,
        _text: &Rope,
        offset: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Option<Hover>>> {
        let markers = self.markers.borrow();
        let hover = markers
            .spans
            .iter()
            .find(|span| span.range.contains(&offset))
            .and_then(|span| hint(&markers.context, span, false))
            .map(|hint| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: markdown(&tooltip_lines(&hint)),
                }),
                range: None,
            });
        Task::ready(Ok(hover))
    }
}

/// The popover renders Markdown and joins plain lines into one, so the first line (which holds the
/// value, free text) goes in a fenced block and the notes below it in paragraphs of their own.
fn markdown(lines: &[String]) -> String {
    let mut parts = lines.iter();
    let Some(first) = parts.next() else {
        return String::new();
    };
    let mut out = format!("~~~\n{first}\n~~~");
    for line in parts {
        out.push_str("\n\n");
        out.push_str(line);
    }
    out
}
