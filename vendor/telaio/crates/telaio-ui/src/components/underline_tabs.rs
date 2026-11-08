//! [`UnderlineTabs`] and [`UnderlineTabItem`]: the request editor's Params/Headers/Body/... row and
//! the response viewer's Body/Headers/Tests/Console row.
//!
//! Despite the name (kept so call sites did not need to change), this renders as a segmented
//! pill track, the same visual language as [`crate::components::SegmentedControl`]: a bordered
//! `surface` capsule, the selected item `raised` with a small shadow, instead of an underline.
//! Custom element for the same reason `SegmentedControl` is: the shadow value has no
//! `ThemeConfigColors` field to carry it.

use std::rc::Rc;

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::{RADIUS_SM, UNDERLINE_TABS_HEIGHT};

/// The selected item's shadow color: opaque black at 12% alpha, the same fixed value
/// `SegmentedControl` uses, not a [`crate::Palette`] field (see that component's doc comment).
const SHADOW_COLOR: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.0,
    a: 0.12,
};

/// One item of an [`UnderlineTabs`] row.
pub struct UnderlineTabItem {
    label: SharedString,
    count: Option<SharedString>,
    count_color: Option<Hsla>,
    selected: bool,
    on_click: Option<ItemHandler>,
}

/// A tab click handler, factored out because clippy's `type_complexity` flags the inline form.
type ItemHandler = Rc<dyn Fn(&mut Window, &mut App)>;

impl UnderlineTabItem {
    /// A new item labeled `label`.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            count: None,
            count_color: None,
            selected: false,
            on_click: None,
        }
    }

    /// Shows `count` after the label, in `fg_subtle` unless [`Self::count_color`] overrides it
    /// (for example the Tests tab's pass/fail count).
    pub fn count(mut self, count: impl Into<SharedString>) -> Self {
        self.count = Some(count.into());
        self
    }

    /// Overrides the count's color (for example `success` when every test passed).
    pub fn count_color(mut self, color: Hsla) -> Self {
        self.count_color = Some(color);
        self
    }

    /// Marks this item as the active tab.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Sets the click handler.
    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// A segmented pill row: a bordered `surface` track with 2 px of padding, the selected item
/// `raised` with a small shadow. An optional [`Self::suffix`] (the response tab bar's status
/// badge, time and size) sits outside the track, pushed to the right by a flex spacer.
#[derive(IntoElement)]
pub struct UnderlineTabs {
    id: ElementId,
    items: Vec<UnderlineTabItem>,
    height: f32,
    suffix: Option<AnyElement>,
}

impl UnderlineTabs {
    /// An empty tab row; add items with [`Self::item`]. Defaults to
    /// [`UNDERLINE_TABS_HEIGHT`]; override with [`Self::height`] for the response viewer's
    /// 36 px row.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            items: Vec::new(),
            height: UNDERLINE_TABS_HEIGHT,
            suffix: None,
        }
    }

    /// Appends one item.
    pub fn item(mut self, item: UnderlineTabItem) -> Self {
        self.items.push(item);
        self
    }

    /// Overrides the row's height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Sets a trailing element, pushed to the right of the track by a flex spacer, in the same
    /// row.
    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }
}

impl RenderOnce for UnderlineTabs {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let height = self.height;
        let suffix = self.suffix;
        // A pill track with a little margin above and below it, instead of filling the whole
        // row: it reads as floating inside the bar rather than touching its edges.
        let track_height = (height - 8.0).max(26.0);
        let inner_height = (track_height - 6.0).max(18.0);

        // No border of its own: a bordered editor or table usually sits right below this row,
        // and the two borders read as a doubled, stuck-together line. The `surface` fill against
        // the window background is enough to read as one grouped control.
        let track = h_flex()
            .id(self.id)
            .items_center()
            .gap(px(2.0))
            .p(px(3.0))
            .h(px(track_height))
            .rounded(px(RADIUS_SM + 2.0))
            .bg(palette.surface)
            .children(self.items.into_iter().enumerate().map(|(index, item)| {
                let selected = item.selected;
                let handler = item.on_click;
                let count_color = item.count_color.unwrap_or(palette.fg_subtle);
                let mut tab = h_flex()
                    .id(("underline-tab", index))
                    .h(px(inner_height))
                    .items_center()
                    .justify_center()
                    .gap(px(6.0))
                    .px(px(12.0))
                    .rounded(px(RADIUS_SM))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .font_weight(if selected {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .when(!selected, |tab| {
                        tab.text_color(palette.fg_muted)
                            .hover(|style| style.bg(palette.hover).text_color(palette.fg))
                            .active(|style| style.bg(palette.pressed))
                    })
                    .when(selected, |tab| {
                        tab.text_color(palette.fg)
                            .bg(palette.raised)
                            .shadow(vec![
                                BoxShadow::new(px(0.0), px(1.0), SHADOW_COLOR)
                                    .blur_radius(px(2.0)),
                            ])
                            .hover(|style| style.bg(palette.hover_over(palette.raised)))
                            .active(|style| style.bg(palette.pressed_over(palette.raised)))
                    })
                    .child(item.label);
                if let Some(count) = item.count {
                    tab = tab.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(count_color)
                            .child(count),
                    );
                }
                if let Some(handler) = handler {
                    tab = tab.on_click(move |_, window, cx| handler(window, cx));
                }
                tab
            }));

        h_flex()
            .items_center()
            .h(px(height))
            .child(track)
            .when_some(suffix, |bar, suffix| {
                bar.child(div().flex_1()).child(suffix)
            })
    }
}
