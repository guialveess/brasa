//! [`switch_halo`]: the hover halo of a `Switch`, the same one the checkbox shows (Postino #138).
//!
//! gpui-component's `Switch` is only `Styled`, with no hover style of its own, so the halo is a
//! wrapper around it. Its negative margin keeps the layout of a bare switch.

use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::Palette;
use crate::metrics::CHECKBOX_HALO_PADDING;

/// `switch` inside a pill shaped halo, [`CHECKBOX_HALO_PADDING`] around its track: `pressed` on
/// hover and `border_strong` while held, one step above the usual `hover` and `pressed`, like
/// the checkbox's. `id` must be unique on screen.
pub fn switch_halo(id: impl Into<ElementId>, switch: Switch, palette: &Palette) -> Stateful<Div> {
    let (hover, pressed) = (palette.pressed, palette.border_strong);
    div()
        .id(id)
        .flex_none()
        .p(px(CHECKBOX_HALO_PADDING))
        .m(px(-CHECKBOX_HALO_PADDING))
        .rounded_full()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .child(switch)
}
