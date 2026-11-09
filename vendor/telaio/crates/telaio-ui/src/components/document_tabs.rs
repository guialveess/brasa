//! [`DocumentTab`] and [`DocumentTabs`]: the strip of open documents above an editor.
//!
//! Custom element, for the same reason as [`super::underline_tabs::UnderlineTabs`]: the active
//! tab's inset accent line needs an explicit `BoxShadow`, and the design's colors (transparent
//! or `bg` background, `fg`/`fg_muted` text) are simplest to pin directly from the palette on a
//! plain `div` row. The dirty dot / hover-reveals-close-`x` behavior uses gpui's `group_hover`,
//! toggling opacity between the two rather than swapping elements.
//!
//! Tabs reorder by dragging, like any editor: hovering a tab over another live-swaps them in the
//! strip, each one sliding the one slot it actually moved ([`ReorderPreview`]) instead of a static
//! color marking where it would land — the slide itself is the indicator. Dropping it on the
//! bar's empty space moves it to the end. The bar only reports the final move
//! ([`DocumentTabs::on_reorder`]); the live swap while dragging is a purely visual preview, reset
//! the moment nothing is being dragged.
//!
//! When the tabs do not fit, the row scrolls horizontally (the mouse wheel too: gpui turns the
//! vertical wheel into a horizontal scroll on a row that only scrolls on x) and, on the right,
//! outside the scrolled area, the bar shows a left arrow, a right arrow and a button that lists
//! every tab. The suffix stays right after the tabs, outside the scrolled area, so it is always
//! visible: next to the last tab while they fit, at the edge of the visible tabs once they
//! overflow.
//!
//! The strip keeps its scroll state itself, in a keyed entity of the window
//! ([`Window::use_keyed_state`]), so the caller passes nothing but the tabs. Whether the tabs
//! overflow is read from the scroll handle after layout; the bar draws one frame behind a layout
//! change, so a zero sized canvas checks at paint time whether what was drawn still matches and
//! asks for one more frame only when it does not (no timers, and an idle window stays idle).

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use rust_i18n::t;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::IconButton;
use crate::PaletteExt;
use crate::metrics::{
    DOCUMENT_TAB_CLOSE_SHIFT, DOCUMENT_TAB_DETAIL_TEXT_SIZE, DOCUMENT_TABS_CONTROLS_WIDTH,
    DOCUMENT_TABS_HEIGHT, DOCUMENT_TABS_SUFFIX_PADDING, RADIUS_SM,
};

/// The selected tab's shadow color: opaque black at 12% alpha, the same fixed value
/// `SegmentedControl` and `UnderlineTabs` use, not a [`crate::Palette`] field (see
/// `SegmentedControl`'s doc comment for why).
const TAB_SHADOW_COLOR: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.0,
    a: 0.12,
};

/// One open tab.
pub struct DocumentTab {
    icon: Option<Icon>,
    icon_color: Option<Hsla>,
    prefix: Option<Prefix>,
    label: SharedString,
    detail: Option<SharedString>,
    dirty: bool,
    selected: bool,
    tooltip: Option<SharedString>,
    on_click: Option<TabHandler>,
    on_close: Option<TabHandler>,
    context_menu: Option<MenuBuilder>,
}

/// Builds a tab's right-click menu, when it opens.
type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// A tab click handler, factored out because clippy's `type_complexity` flags the inline form.
type TabHandler = Rc<dyn Fn(&mut Window, &mut App)>;
/// Builds the element before a tab's label, once per render and once for the drag preview.
type Prefix = Rc<dyn Fn() -> AnyElement>;
/// A reorder handler, called with the dragged tab's index and the index it moves to.
type ReorderHandler = Rc<dyn Fn(usize, usize, &mut Window, &mut App)>;

/// The value carried while a tab is dragged, also rendered as the preview under the pointer.
#[derive(Clone)]
struct DraggedTab {
    index: usize,
    prefix: Option<Prefix>,
    label: SharedString,
    detail: Option<SharedString>,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette();
        h_flex()
            .h(px(DOCUMENT_TABS_HEIGHT))
            .items_center()
            .gap(px(6.0))
            .px_3()
            .rounded(px(RADIUS_SM))
            .border_1()
            .border_color(palette.border_strong)
            .bg(palette.bg)
            .shadow(palette.shadow.clone())
            .opacity(0.9)
            .text_size(px(13.0))
            .text_color(palette.fg)
            .children(self.prefix.as_ref().map(|prefix| prefix()))
            .child(self.label.clone())
            .children(
                self.detail
                    .clone()
                    .map(|detail| tab_detail(detail, palette.fg_subtle)),
            )
    }
}

/// The detail text after a tab's label.
fn tab_detail(detail: SharedString, color: Hsla) -> Div {
    div()
        .text_size(px(DOCUMENT_TAB_DETAIL_TEXT_SIZE))
        .text_color(color)
        .child(detail)
}

impl DocumentTab {
    /// A new tab labeled `label` (a file name, for example).
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            icon: None,
            icon_color: None,
            prefix: None,
            label: label.into(),
            detail: None,
            dirty: false,
            selected: false,
            tooltip: None,
            on_click: None,
            on_close: None,
            context_menu: None,
        }
    }

    /// Sets a leading icon (for example `gauge` for a load test tab).
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Colors the leading icon, `fg_subtle` by default (the welcome tab's house is `accent_text`).
    pub fn icon_color(mut self, color: Hsla) -> Self {
        self.icon_color = Some(color);
        self
    }

    /// Shows the element `build` returns before the tab's label, after the icon (an app's badge
    /// for the kind of document, for example). It is a builder because the drag preview needs
    /// its own copy of the element.
    pub fn prefix<E: IntoElement>(mut self, build: impl Fn() -> E + 'static) -> Self {
        self.prefix = Some(Rc::new(move || build().into_any_element()));
        self
    }

    /// Shows `detail` after the label in smaller, subtle text, for example what a diff compares
    /// ("Working Tree").
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Marks the tab as having unsaved changes: a filled dot instead of a close `x`, until
    /// hovered.
    pub fn dirty(mut self, dirty: bool) -> Self {
        self.dirty = dirty;
        self
    }

    /// Marks this tab as the active one.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets a hover tooltip, for example the full path of the document (the tab's own
    /// label is only its file name).
    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Sets the handler for clicking the tab itself (activates it).
    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// The tab's right-click menu, built by `builder` when it opens (Close, Copy path, ...). The
    /// tab is not selected by the right click; the caller's items say which tab they act on.
    pub fn context_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Rc::new(builder));
        self
    }

    /// Sets the handler for the close button (or the dirty dot, once hovered).
    pub fn on_close(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }
}

/// How much of the visible width an arrow scrolls, so the last tab seen stays in view.
const PAGE_FRACTION: f32 = 0.9;
/// Layout values closer than this (in pixels) are the same, to ignore rounding noise.
const EPSILON: f32 = 0.5;
/// The most the tab list menu grows, in pixels, before it scrolls.
const LIST_MENU_MAX_HEIGHT: f32 = 320.0;

/// What the strip shows besides the tabs, derived from the scroll handle after layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StripFlags {
    /// The arrows and the list button are visible: the tabs overflow.
    show_controls: bool,
    can_scroll_left: bool,
    can_scroll_right: bool,
}

impl StripFlags {
    /// No controls, as before the first layout.
    const HIDDEN: Self = Self {
        show_controls: false,
        can_scroll_left: false,
        can_scroll_right: false,
    };

    /// The flags for a scrolled area `viewport` wide, scrolled by `offset` (zero or negative),
    /// that can scroll `max` pixels, while the controls (`controls` wide) `were_shown`.
    ///
    /// The controls take room from the scrolled area, so the test is against the room the tabs
    /// have with the controls hidden: `viewport` plus `controls` when they are shown. Testing
    /// the visible viewport instead would flip the controls on and off for tabs that fit only
    /// without them.
    fn new(offset: f32, max: f32, viewport: f32, controls: f32, were_shown: bool) -> Self {
        let content = viewport + max;
        let room = if were_shown {
            viewport + controls
        } else {
            viewport
        };
        let show_controls = content > room + EPSILON;
        Self {
            show_controls,
            can_scroll_left: show_controls && offset < -EPSILON,
            can_scroll_right: show_controls && -offset < max - EPSILON,
        }
    }

    /// The flags `handle` shows now.
    fn measure(handle: &ScrollHandle, were_shown: bool) -> Self {
        Self::new(
            handle.offset().x.into(),
            handle.max_offset().x.into(),
            handle.bounds().size.width.into(),
            DOCUMENT_TABS_CONTROLS_WIDTH,
            were_shown,
        )
    }
}

/// The scroll offset after an arrow click: one page (about the visible width) towards `forward`
/// or back, clamped to the scrollable range. Offsets are zero or negative.
fn paged_offset(offset: f32, max: f32, viewport: f32, forward: bool) -> f32 {
    let page = viewport * PAGE_FRACTION;
    let target = if forward {
        offset - page
    } else {
        offset + page
    };
    target.clamp(-max.max(0.0), 0.0)
}

/// The state the strip keeps between frames.
struct StripState {
    handle: ScrollHandle,
    /// The selected tab at the last render, to scroll to it only when it changes.
    last_selected: Option<usize>,
    /// What the last render drew.
    flags: StripFlags,
    /// The live drag-reorder preview.
    reorder: ReorderPreview,
}

/// The live preview of a drag reorder: which original tab index renders at which position while
/// one is being dragged over the strip, reset to identity the moment nothing is being dragged
/// ([`cx.has_active_drag`](gpui::App::has_active_drag)).
#[derive(Clone)]
struct ReorderPreview {
    /// A permutation of `0..len`: `order[display_position] = original_index`.
    order: Vec<usize>,
    /// `order` just before the last change, so only the tabs that actually moved animate.
    previous: Vec<usize>,
    /// Bumped on every change, so a moved tab's slide-in restarts exactly when it moves again.
    generation: usize,
}

impl ReorderPreview {
    /// No tab has moved: `order` is `0..len` in order.
    fn identity(len: usize) -> Self {
        let order: Vec<usize> = (0..len).collect();
        Self {
            previous: order.clone(),
            order,
            generation: 0,
        }
    }

    /// Moves `from_original` into `to_original`'s current slot, as if pulled out and dropped
    /// there: the tabs between the two slide over by one to close or open the gap, the same way
    /// dragging a tab past its neighbors works in any editor. A no-op, returning `self` unchanged
    /// (not bumping `generation`), when they are already adjacent in that order.
    fn move_to(&self, from_original: usize, to_original: usize) -> Self {
        let (Some(from_pos), Some(to_pos)) = (
            self.order.iter().position(|&i| i == from_original),
            self.order.iter().position(|&i| i == to_original),
        ) else {
            return self.clone();
        };
        if from_pos == to_pos {
            return self.clone();
        }
        let mut order = self.order.clone();
        let moved = order.remove(from_pos);
        order.insert(to_pos, moved);
        Self {
            previous: self.order.clone(),
            order,
            generation: self.generation + 1,
        }
    }

    /// How many slots `original_index` moved between `previous` and `order`: positive to the
    /// right, negative to the left, `0` if it did not move (or isn't in either, which does not
    /// happen in practice since both are always permutations of the same `0..len`).
    fn slots_moved(&self, original_index: usize) -> isize {
        let was = self.previous.iter().position(|&i| i == original_index);
        let now = self.order.iter().position(|&i| i == original_index);
        match (was, now) {
            (Some(was), Some(now)) => now as isize - was as isize,
            _ => 0,
        }
    }
}

/// A rough, fixed stand-in for a tab's actual (variable) width, just to give the slide-in
/// animation a plausible starting offset. Not pixel accurate — the tabs this session's own
/// labels run, and measuring every tab's real width before animating is not worth it for a cue
/// that only plays for a couple hundred milliseconds.
const REORDER_SLOT_PX: f32 = 72.0;

/// The open-tabs bar: one [`DocumentTab`] per open document, plus the caller's own
/// trailing "+" button (added with [`Self::suffix`]).
#[derive(IntoElement)]
pub struct DocumentTabs {
    id: ElementId,
    items: Vec<DocumentTab>,
    suffix: Option<AnyElement>,
    on_reorder: Option<ReorderHandler>,
}

impl DocumentTabs {
    /// An empty tab bar; add tabs with [`Self::item`].
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            suffix: None,
            on_reorder: None,
        }
    }

    /// Appends one tab.
    pub fn item(mut self, item: DocumentTab) -> Self {
        self.items.push(item);
        self
    }

    /// Makes the tabs draggable: `handler` gets the dragged tab's index and the index it moves to
    /// (`usize::MAX` for "the end", when dropped on the bar's empty space).
    pub fn on_reorder(
        mut self,
        handler: impl Fn(usize, usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_reorder = Some(Rc::new(handler));
        self
    }

    /// Sets the trailing element after the last tab (a "+" new document button).
    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }
}

impl RenderOnce for DocumentTabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let on_reorder = self.on_reorder;
        let bar_reorder = on_reorder.clone();

        let state =
            window.use_keyed_state(ElementId::from((self.id.clone(), "state")), cx, |_, _| {
                StripState {
                    handle: ScrollHandle::new(),
                    last_selected: None,
                    flags: StripFlags::HIDDEN,
                    reorder: ReorderPreview::identity(0),
                }
            });
        let selected_index = self.items.iter().position(|item| item.selected);
        let item_count = self.items.len();
        let has_active_drag = cx.has_active_drag();
        let (handle, flags, reorder) = state.update(cx, |state, _| {
            // Applied by gpui when the row is prepainted, with that frame's tab bounds.
            if selected_index != state.last_selected {
                if let Some(index) = selected_index {
                    state.handle.scroll_to_item(index);
                }
                state.last_selected = selected_index;
            }
            state.flags = StripFlags::measure(&state.handle, state.flags.show_controls);
            // Nothing is being dragged (a drop, a cancel, or the first render ever), or the tab
            // count changed under an in-progress drag (closing one from its own menu, say): the
            // preview no longer means anything, so it resets to "nothing has moved".
            if !has_active_drag || state.reorder.order.len() != item_count {
                state.reorder = ReorderPreview::identity(item_count);
            }
            (state.handle.clone(), state.flags, state.reorder.clone())
        });

        // The tab list for the menu: label, whether it is the active one, and its click handler.
        let entries: Vec<_> = self
            .items
            .iter()
            .map(|item| (item.label.clone(), item.selected, item.on_click.clone()))
            .collect();

        let mut built: Vec<Option<AnyElement>> = self.items.into_iter().enumerate().map(|(index, item)| {
            let group = SharedString::from(format!("doc-tab-{index}"));
            let selected = item.selected;
            let dirty = item.dirty;
            let click_handler = item.on_click;
            let close_handler = item.on_close;
            let context_menu = item.context_menu;
            let middle_click_close = close_handler.clone();

            // A floating chip, like `SegmentedControl`'s selected item, instead of a flat panel
            // with a divider on each side and a top accent line: the selected tab's `raised`
            // fill is opaque, and a hover style replaces it.
            let (hover, pressed) = if selected {
                (
                    palette.hover_over(palette.raised),
                    palette.pressed_over(palette.raised),
                )
            } else {
                (palette.hover, palette.pressed)
            };
            let mut tab = h_flex()
                .id(("doc-tab", index))
                .group(group.clone())
                .flex_none()
                .h(px(DOCUMENT_TABS_HEIGHT - 10.0))
                .items_center()
                .gap(px(6.0))
                .pl_3()
                .pr(px(12.0 - DOCUMENT_TAB_CLOSE_SHIFT))
                .rounded(px(RADIUS_SM))
                .cursor_pointer()
                .whitespace_nowrap()
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed))
                .text_size(px(13.0))
                .text_color(if selected {
                    palette.fg
                } else {
                    palette.fg_muted
                })
                .when(selected, |tab| {
                    tab.bg(palette.raised).shadow(vec![BoxShadow::new(
                        px(0.0),
                        px(1.0),
                        TAB_SHADOW_COLOR,
                    )
                    .blur_radius(px(2.0))])
                });
            if let Some(icon) = item.icon {
                let color = item.icon_color.unwrap_or(palette.fg_subtle);
                tab = tab.child(Icon::new(icon).small().text_color(color));
            }
            if let Some(handler) = on_reorder.clone() {
                let dragged = DraggedTab {
                    index,
                    prefix: item.prefix.clone(),
                    label: item.label.clone(),
                    detail: item.detail.clone(),
                };
                // No color marking the drop target: the tabs between it and the dragged one
                // live-swap into their new slots instead (`ReorderPreview::move_to`, applied
                // below), so the movement itself is the indicator (reported: a static accent
                // line, however drawn, always read as either too weak or too strong).
                let reorder_state = state.clone();
                tab = tab
                    .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                    .drag_over::<DraggedTab>(move |style, dragged, window, cx| {
                        if dragged.index != index {
                            reorder_state.update(cx, |state, _| {
                                let next = state.reorder.move_to(dragged.index, index);
                                if next.generation != state.reorder.generation {
                                    state.reorder = next;
                                    window.refresh();
                                }
                            });
                        }
                        style
                    })
                    .on_drop(move |dragged: &DraggedTab, window, cx| {
                        handler(dragged.index, index, window, cx)
                    });
            }
            if let Some(prefix) = item.prefix {
                tab = tab.child(prefix());
            }
            tab = tab.child(item.label);
            if let Some(detail) = item.detail {
                tab = tab.child(tab_detail(detail, palette.fg_subtle));
            }
            if let Some(tooltip) = item.tooltip {
                tab =
                    tab.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));
            }
            if let Some(handler) = click_handler {
                tab = tab.on_click(move |_, window, cx| handler(window, cx));
            }
            // A middle click closes the tab, the same as its close button.
            if let Some(handler) = middle_click_close {
                tab = tab.on_mouse_up(MouseButton::Middle, move |_, window, cx| {
                    handler(window, cx)
                });
            }

            let mut end = div()
                .id(("doc-tab-end", index))
                .relative()
                .ml(px(DOCUMENT_TAB_CLOSE_SHIFT))
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .rounded(px(4.0))
                .hover(|style| style.bg(palette.hover))
                .active(|style| style.bg(palette.pressed));
            if dirty {
                end = end
                    .child(
                        div()
                            .size(px(8.0))
                            .rounded_full()
                            .bg(palette.fg_subtle)
                            .group_hover(group.clone(), |style| style.opacity(0.0)),
                    )
                    .child(
                        div()
                            .absolute()
                            .opacity(0.0)
                            .group_hover(group.clone(), |style| style.opacity(1.0))
                            .child(
                                Icon::new(IconName::Close)
                                    .small()
                                    .text_color(palette.fg_subtle),
                            ),
                    );
            } else {
                end = end.child(
                    Icon::new(IconName::Close)
                        .small()
                        .text_color(palette.fg_subtle),
                );
            }
            if let Some(handler) = close_handler {
                end = end.on_click(move |_, window, cx| handler(window, cx));
            }
            let tab = tab.child(end);
            let tab = match context_menu {
                Some(builder) => tab
                    .context_menu(move |menu, window, cx| builder(menu, window, cx))
                    .into_any_element(),
                None => tab.into_any_element(),
            };
            // Slides in from where it moved from, settling into its natural flex position: the
            // reorder's own visible feedback, in place of the static marker tried before.
            let slots_moved = reorder.slots_moved(index);
            let tab = if slots_moved == 0 {
                tab
            } else {
                let start_left = -(slots_moved as f32) * REORDER_SLOT_PX;
                div()
                    .child(tab)
                    .with_animation(
                        SharedString::from(format!("doc-tab-slide-{index}-{}", reorder.generation)),
                        Animation::new(Duration::from_millis(180)).with_easing(ease_out_quint()),
                        move |this, delta| this.left(px(start_left * (1.0 - delta))),
                    )
                    .into_any_element()
            };
            Some(tab)
        }).collect();

        // Reordered to the live preview: each slot now shows the original-index tab the preview
        // says belongs there, so the strip visually reflects the drag in progress.
        let tabs: Vec<AnyElement> = reorder
            .order
            .iter()
            .map(|&original| {
                built[original]
                    .take()
                    .expect("ReorderPreview::order holds each original index exactly once")
            })
            .collect();

        let controls = flags.show_controls.then(|| {
            let page = |forward: bool| {
                let handle = handle.clone();
                move |_: &ClickEvent, window: &mut Window, _: &mut App| {
                    let offset = handle.offset();
                    let x = paged_offset(
                        offset.x.into(),
                        handle.max_offset().x.into(),
                        handle.bounds().size.width.into(),
                        forward,
                    );
                    handle.set_offset(point(px(x), offset.y));
                    // The arrows' enabled state is drawn from the new offset.
                    window.refresh();
                }
            };
            let list = IconButton::new("doc-tabs-list", IconName::ChevronDown)
                .tooltip(t!("telaio.tabs.list"))
                .into_button(cx)
                .dropdown_menu(move |menu, _, _| {
                    let mut menu = menu.scrollable(true).max_h(px(LIST_MENU_MAX_HEIGHT));
                    for (label, selected, handler) in entries.clone() {
                        menu = menu.item(PopupMenuItem::new(label).checked(selected).on_click(
                            move |_, window, cx| {
                                if let Some(handler) = &handler {
                                    handler(window, cx);
                                }
                            },
                        ));
                    }
                    menu
                });
            h_flex()
                .flex_none()
                .w(px(DOCUMENT_TABS_CONTROLS_WIDTH))
                .h_full()
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .border_l_1()
                .border_color(palette.border)
                .child(
                    IconButton::new("doc-tabs-previous", IconName::ChevronLeft)
                        .tooltip(t!("telaio.tabs.previous"))
                        .disabled(!flags.can_scroll_left)
                        .on_click(page(false)),
                )
                .child(
                    IconButton::new("doc-tabs-next", IconName::ChevronRight)
                        .tooltip(t!("telaio.tabs.next"))
                        .disabled(!flags.can_scroll_right)
                        .on_click(page(true)),
                )
                .child(list)
        });

        // Checks after paint that the layout still matches what this render drew.
        let check = canvas(|_, _, _| {}, {
            let state = state.clone();
            let handle = handle.clone();
            move |_, _, window, cx| {
                let drawn = state.read(cx).flags;
                if StripFlags::measure(&handle, drawn.show_controls) != drawn {
                    window.defer(cx, |window, _| window.refresh());
                }
            }
        })
        .absolute()
        .size_0();

        h_flex()
            .id(self.id)
            .relative()
            .h(px(DOCUMENT_TABS_HEIGHT))
            .bg(palette.surface)
            .border_b_1()
            .border_color(palette.border)
            .when_some(bar_reorder, |bar, handler| {
                bar.on_drop(move |dragged: &DraggedTab, window, cx| {
                    handler(dragged.index, usize::MAX, window, cx)
                })
            })
            // gpui hands a wheel event to every scrolled element under the pointer. Once the
            // tabs have scrolled it (their listener is inner, so it runs first), stop it here so
            // a scrolled page around the strip does not move as well.
            .when(flags.show_controls, |bar| {
                bar.on_scroll_wheel(|_, _, cx| cx.stop_propagation())
            })
            .child(
                h_flex()
                    .id("doc-tabs-scroll")
                    .flex_shrink(1.0)
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(6.0))
                    .overflow_x_scroll()
                    .track_scroll(&handle)
                    .children(tabs),
            )
            // While the tabs overflow, the last visible tab is cut and its own border is gone, so
            // the suffix draws one to stay apart from it.
            .children(self.suffix.map(|suffix| {
                h_flex()
                    .flex_none()
                    .h_full()
                    .items_center()
                    .px(px(DOCUMENT_TABS_SUFFIX_PADDING))
                    .when(flags.show_controls, |this| {
                        this.border_l_1().border_color(palette.border)
                    })
                    .child(suffix)
            }))
            .child(div().flex_1())
            .children(controls)
            .child(check)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{StripFlags, paged_offset};

    const CONTROLS: f32 = 88.0;

    #[test]
    fn tabs_that_fit_show_no_controls() {
        assert_eq!(
            StripFlags::new(0.0, 0.0, 400.0, CONTROLS, false),
            StripFlags::HIDDEN
        );
    }

    #[test]
    fn overflowing_tabs_show_the_controls_at_their_start() {
        let flags = StripFlags::new(0.0, 120.0, 400.0, CONTROLS, false);
        assert!(flags.show_controls);
        assert!(!flags.can_scroll_left);
        assert!(flags.can_scroll_right);
    }

    #[test]
    fn the_arrows_follow_the_offset() {
        let middle = StripFlags::new(-60.0, 120.0, 312.0, CONTROLS, true);
        assert!(middle.can_scroll_left && middle.can_scroll_right);
        let end = StripFlags::new(-120.0, 120.0, 312.0, CONTROLS, true);
        assert!(end.can_scroll_left && !end.can_scroll_right);
    }

    #[test]
    fn tabs_that_fit_only_without_the_controls_do_not_flip_them() {
        // 450 px of tabs in a 480 px strip: they fit, and with the controls shown the viewport
        // is 392 and the content 450, which must not keep the controls on.
        assert!(!StripFlags::new(0.0, 58.0, 392.0, CONTROLS, true).show_controls);
        // And 450 px of tabs in a 420 px strip overflow, hidden or shown.
        assert!(StripFlags::new(0.0, 30.0, 420.0, CONTROLS, false).show_controls);
        assert!(StripFlags::new(0.0, 118.0, 332.0, CONTROLS, true).show_controls);
    }

    #[test]
    fn rounding_noise_is_not_an_overflow() {
        assert!(!StripFlags::new(0.0, 0.2, 400.0, CONTROLS, false).show_controls);
    }

    #[test]
    fn a_page_is_about_the_visible_width() {
        assert_eq!(paged_offset(0.0, 1000.0, 400.0, true), -360.0);
        assert_eq!(paged_offset(-360.0, 1000.0, 400.0, false), 0.0);
    }

    #[test]
    fn a_page_stops_at_both_ends() {
        assert_eq!(paged_offset(-300.0, 400.0, 400.0, true), -400.0);
        assert_eq!(paged_offset(-100.0, 400.0, 400.0, false), 0.0);
        assert_eq!(paged_offset(0.0, 0.0, 400.0, true), 0.0);
    }
}
