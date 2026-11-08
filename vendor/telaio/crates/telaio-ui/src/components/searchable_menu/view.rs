//! [`SearchableMenu`]: the popover element around a trigger.

use gpui_kit::component::popover::Popover;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::state::SearchableMenuState;
use crate::metrics::SEARCHABLE_MENU_GAP;

/// Keeps the view that renders the menu up to date: the menu can be closed from the app or by
/// the user, and the popover reads its open state while rendering.
struct OpenObserver {
    _subscription: Subscription,
}

/// A popover showing a [`SearchableMenuState`] above (by default) or below a trigger.
///
/// The trigger is any `Selectable` element, such as gpui-kit's `Button`; a click on it opens and
/// closes the menu. The state decides what is inside and tells the app what was chosen.
#[derive(IntoElement)]
pub struct SearchableMenu {
    id: ElementId,
    state: Entity<SearchableMenuState>,
    anchor: Anchor,
    trigger: Option<Box<dyn FnOnce(Popover) -> Popover>>,
}

impl SearchableMenu {
    /// A menu for `state`. `id` must be unique among the elements of the parent view.
    pub fn new(id: impl Into<ElementId>, state: &Entity<SearchableMenuState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            anchor: Anchor::BottomLeft,
            trigger: None,
        }
    }

    /// Where the panel sits relative to the trigger. This names the panel's own corner:
    /// `BottomLeft` (the default) opens upwards aligned to the trigger's left edge, `TopLeft`
    /// opens downwards, `BottomRight` and `TopRight` align to the right edge.
    pub fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Sets the element that opens the menu and that the panel is anchored to.
    pub fn trigger<T>(mut self, trigger: T) -> Self
    where
        T: Selectable + IntoElement + 'static,
    {
        self.trigger = Some(Box::new(move |popover| popover.trigger(trigger)));
        self
    }
}

impl RenderOnce for SearchableMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state;
        let parent = window.current_view();
        let observed = state.clone();
        window.use_keyed_state((self.id.clone(), "observer"), cx, move |_, cx| {
            let mut was_open = observed.read(cx).is_open();
            OpenObserver {
                _subscription: cx.observe(&observed, move |_, state, cx| {
                    let open = state.read(cx).is_open();
                    if open != was_open {
                        was_open = open;
                        let app: &mut App = cx;
                        app.notify(parent);
                    }
                }),
            }
        });

        let (open, focus) = {
            let menu = state.read(cx);
            (menu.is_open(), menu.input.read(cx).focus_handle(cx))
        };
        let changed = state.clone();
        let content = state.clone();
        let popover = Popover::new(self.id)
            .anchor(self.anchor)
            .offset(px(SEARCHABLE_MENU_GAP))
            .appearance(false)
            // The panel closes itself on a click outside, since its submenu lies outside the
            // popover's own bounds.
            .overlay_closable(false)
            .open(open)
            .track_focus(&focus)
            .on_open_change(move |open, window, cx| {
                changed.update(cx, |menu, cx| {
                    if *open {
                        menu.open(window, cx);
                    } else {
                        menu.dismiss(cx);
                    }
                });
            })
            .content(move |_, _, _| content.clone());
        match self.trigger {
            Some(trigger) => trigger(popover),
            None => popover,
        }
    }
}
