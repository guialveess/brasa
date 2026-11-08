//! [`ScrollbarOverlay`]: an always-shown vertical scrollbar laid over a scrolling area.
//!
//! Wraps gpui-kit's `Scrollbar` in `ScrollbarMode::Always`, unlike the theme's default that only
//! shows it while scrolling: on a short window nothing else tells the user that more content lies
//! below. Its track is transparent with no border, so only the thumb stands out: the theme's
//! track color is the window's `bg`, which would show as a darker strip on a modal, and an opaque
//! one would square off a rounded corner where the content reaches the bottom.

use gpui_kit::component::scroll::{Scrollbar, ScrollbarHandle, ScrollbarMode};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::{Palette, PaletteExt};

/// A vertical scrollbar for the area scrolled with `handle`.
///
/// Put it in a `relative()` wrapper next to the scrolling element, which must call
/// `.track_scroll(&handle)` with the same handle:
///
/// ```ignore
/// div().relative().child(v_flex().id("body").overflow_y_scroll().track_scroll(&handle))
///     .child(ScrollbarOverlay::new(&handle))
/// ```
#[derive(IntoElement)]
pub struct ScrollbarOverlay {
    handle: ScrollHandle,
}

impl ScrollbarOverlay {
    /// A scrollbar following `handle`. Nothing is drawn while the content does not overflow.
    pub fn new(handle: &ScrollHandle) -> Self {
        Self {
            handle: handle.clone(),
        }
    }
}

impl RenderOnce for ScrollbarOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        overlay(&self.handle, None, &cx.palette())
    }
}

/// The overlay of [`ScrollbarOverlay`] for any scroll handle (a `UniformListScrollHandle` for a
/// virtualized list), with an explicit element `id` for the scrollbar's own state when several
/// share a call site.
pub(crate) fn overlay<H: ScrollbarHandle + Clone>(
    handle: &H,
    id: Option<ElementId>,
    palette: &Palette,
) -> Div {
    let track = palette.overlay.opacity(0.0);
    let border = track;
    let mut scrollbar = Scrollbar::vertical(handle)
        .mode(ScrollbarMode::Always)
        .styles(move |styles| {
            styles
                .track(|style| style.bg(track).border_color(border))
                .track_hover(|style| style.bg(track).border_color(border))
                .track_active(|style| style.bg(track).border_color(border))
        });
    if let Some(id) = id {
        scrollbar = scrollbar.id(id);
    }
    div().absolute().inset_0().child(scrollbar)
}
