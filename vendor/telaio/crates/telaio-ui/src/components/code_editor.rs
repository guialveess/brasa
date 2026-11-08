//! [`CodeEditor`]: gpui-component's source code `Editor` in the app's box (radius, background),
//! with the app's edit menu, filling the height of its parent.
//!
//! The editor sets the monospace font and size of the theme (`mono_font` in the theme JSON) and
//! its syntax colors come from the app's [`crate::SyntaxTheme`]. The language, the line numbers
//! and the text live on the [`EditorState`], built by the app:
//!
//! ```ignore
//! let body = cx.new(|cx| EditorState::new(window, cx).language("json"));
//! body.update(cx, |state, cx| state.set_value(text, window, cx));
//! ```
//!
//! Line numbers are on by default; `state.set_line_number(false, window, cx)` hides them. A
//! language only highlights when the app enables its `tree-sitter-*` feature of `gpui-kit`
//! (JSON comes with the base `tree-sitter` feature); without it the text shows unhighlighted.
//! App specific pieces, such as completion or hover providers (`state.lsp_mut()`), stay in the
//! app.

use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::edit_menu::edit_menu;
use crate::PaletteExt as _;
use crate::metrics::RADIUS_MD;

/// A code editor for `state`, as tall as its parent unless [`Self::h`] says otherwise.
#[derive(IntoElement)]
pub struct CodeEditor {
    state: Entity<EditorState>,
    height: DefiniteLength,
    readonly: bool,
}

impl CodeEditor {
    /// The editor for `state`.
    pub fn new(state: &Entity<EditorState>) -> Self {
        Self {
            state: state.clone(),
            height: relative(1.0),
            readonly: false,
        }
    }

    /// The height of the editor, in pixels or relative to its parent; the whole parent by
    /// default. `Editor` needs a definite height to lay out its lines and gutter: with `flex_1`
    /// alone it would size to its content.
    pub fn h(mut self, height: impl Into<DefiniteLength>) -> Self {
        self.height = height.into();
        self
    }

    /// Whether the text can be selected, searched and copied but not changed, as a response
    /// viewer. Set here, not with `EditorState::set_readonly`: the element applies its own value
    /// to the state on every render.
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }
}

impl RenderOnce for CodeEditor {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        // No border of its own: this usually sits right inside a pane a resizable split or
        // another card already outlines, and a second border that close just reads as a doubled
        // line (the same reason `UnderlineTabs` and `SegmentedControl` dropped theirs). The `bg`
        // fill is enough to tell it apart from what is behind it.
        //
        // `Editor` paints its own gutter background directly (not through a styled child), flush
        // with its own square corners. `overflow_hidden` clips to a plain rectangle, never to a
        // rounded shape (gpui has no rounded clip mask), so it cannot hide that square corner
        // poking past the wrapper's curve on its own. The fixed inset below is wide enough to
        // clear `RADIUS_MD`'s curve at any corner (a flat corner inset by at least
        // `radius * (1 - cos 45°)`, about 0.293 of it, never crosses the arc); the editor's own
        // background already matches the wrapper's, so the gap reads as padding, not a seam.
        div()
            .rounded(px(RADIUS_MD))
            .overflow_hidden()
            .bg(palette.bg)
            .h(self.height)
            .child(
                div().size_full().p(px(6.0)).child(
                    Editor::new(&self.state)
                        .context_menu(edit_menu(&self.state, cx))
                        .readonly(self.readonly)
                        .bg(palette.bg)
                        .size_full(),
                ),
            )
    }
}
