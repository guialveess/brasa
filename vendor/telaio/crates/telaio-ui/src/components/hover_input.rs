//! [`HoverInput`]: an `Input` with the hover feedback of the title bar's search trigger. It rests
//! on the `border` token and takes `border_strong` while the pointer is over it. A focused input
//! keeps its own focus border.
//!
//! gpui-component's `Input` has no hover style of its own: it is only `Styled`, and its border
//! belongs to an inner element. So the wrapper tracks the pointer, remembers it in element state
//! between frames, and the next render recolors the border. Only entering and leaving the input
//! renders again. An input whose border is drawn by a container (the URL bar, the sidebar filter)
//! sets the same two colors on that container with `hover` instead.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;

/// An `Input` whose border follows the pointer. Its wrapper takes the styles set on it (a width,
/// a height), the `Input` keeps its own.
#[derive(IntoElement)]
pub struct HoverInput {
    id: ElementId,
    state: Entity<InputState>,
    input: Input,
    style: StyleRefinement,
}

impl HoverInput {
    /// Wraps `input`, built for `state`. `id` must be unique on screen: it keys the remembered
    /// hover state.
    pub fn new(id: impl Into<ElementId>, state: &Entity<InputState>, input: Input) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            input,
            style: StyleRefinement::default(),
        }
    }
}

impl Styled for HoverInput {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for HoverInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hovered = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let palette = cx.palette();
        let border = if *hovered.read(cx) {
            palette.border_strong
        } else {
            palette.border
        };
        let mut wrapper = div().id(self.id).w_full();
        wrapper.style().refine(&self.style);
        wrapper
            .on_hover(move |is_hovered, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    if *hovered != *is_hovered {
                        *hovered = *is_hovered;
                        cx.notify();
                    }
                });
            })
            // A focused input keeps the border its caller or its own focus style gives it.
            .child(
                self.input
                    .when(!focused, |input| input.border_color(border)),
            )
    }
}
