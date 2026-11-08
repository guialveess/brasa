//! The pieces of a Settings dialog, which Postino and Vano share: [`SettingsLayout`], the two
//! column body (a category navigation on the left, a header, a scrolled body and a footer on the
//! right), [`settings_dialog`] to frame it, and the two pieces the sections are made of,
//! [`LabeledRow`] and [`SectionHeading`].
//!
//! The app owns everything else: the categories, which one is selected, the title texts and the
//! sections, which it passes as elements. The layout reads no setting and no locale of the app.
//!
//! ```ignore
//! window.open_dialog(cx, move |dialog, window, _| {
//!     settings_dialog(dialog, window, move |_window, cx| {
//!         SettingsLayout::new("settings")
//!             .title("Settings")
//!             .categories(items.clone())
//!             .selected(current)
//!             .on_select(on_select.clone())
//!             .heading(current_label.clone())
//!             .section(first_section)
//!             .section(second_section)
//!             .footer("Changes apply immediately.", reset_button)
//!             .into_any_element()
//!     })
//! });
//! ```
//!
//! The dialog's builder runs on every render, so the app re-reads its settings there and a change
//! shows right away.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::{RADIUS_MD, SETTINGS_HEIGHT, SETTINGS_NAV_WIDTH, SETTINGS_WIDTH};

use super::modal::{MODAL_INNER_RADIUS, ModalPlacement, framed_dialog, modal_footer, modal_header};
use super::scrollbar::ScrollbarOverlay;

/// Height of a category in the navigation.
const NAV_ITEM_HEIGHT: f32 = 28.0;
/// Padding of the right column's body, and of its header and footer.
const BODY_PADDING: f32 = 20.0;
/// Gap between the body's sections, which a divider splits.
const BODY_GAP: f32 = 22.0;
/// Gap between a row's label column and its control.
const ROW_GAP: f32 = 16.0;

/// Frames `build`'s content as the Settings modal: [`SETTINGS_WIDTH`] by [`SETTINGS_HEIGHT`],
/// centered, shrinking only on a window too small for it (its body then scrolls). Call it inside
/// `open_dialog`'s builder, like [`framed_dialog`].
pub fn settings_dialog<F>(dialog: Dialog, window: &Window, build: F) -> Dialog
where
    F: Fn(&mut Window, &mut App) -> AnyElement + 'static,
{
    framed_dialog(
        dialog,
        SETTINGS_WIDTH,
        Some(SETTINGS_HEIGHT),
        ModalPlacement::Centered,
        window,
        build,
    )
}

/// One category of the navigation: an icon and a label.
#[derive(Clone)]
pub struct SettingsCategory {
    icon: IconName,
    label: SharedString,
}

impl SettingsCategory {
    /// A category shown as `icon` and `label`.
    pub fn new(icon: IconName, label: impl Into<SharedString>) -> Self {
        Self {
            icon,
            label: label.into(),
        }
    }
}

/// What the layout runs when the user picks a category, with its position in the list.
type SelectHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The Settings modal's content, to give [`settings_dialog`]: the navigation on the left and, on
/// the right, the header with the selected category's title and the close button, the scrolled
/// body with an always shown scrollbar, and an optional footer.
#[derive(IntoElement)]
pub struct SettingsLayout {
    id: ElementId,
    title: SharedString,
    categories: Vec<SettingsCategory>,
    selected: usize,
    on_select: Option<SelectHandler>,
    nav_footer: Option<AnyElement>,
    heading: SharedString,
    sections: Vec<AnyElement>,
    footer: Option<(AnyElement, AnyElement)>,
}

impl SettingsLayout {
    /// An empty layout. `id` must be unique in the window: it keys the body's scroll position.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            title: SharedString::default(),
            categories: Vec::new(),
            selected: 0,
            on_select: None,
            nav_footer: None,
            heading: SharedString::default(),
            sections: Vec::new(),
            footer: None,
        }
    }

    /// The title above the navigation.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }

    /// The categories of the navigation, in order.
    pub fn categories(mut self, categories: impl IntoIterator<Item = SettingsCategory>) -> Self {
        self.categories = categories.into_iter().collect();
        self
    }

    /// The position of the selected category.
    pub fn selected(mut self, selected: usize) -> Self {
        self.selected = selected;
        self
    }

    /// Runs with the position of the category the user clicks.
    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    /// A note at the bottom of the navigation, in the muted small text (where the settings file
    /// lives, say).
    pub fn nav_footer(mut self, footer: impl IntoElement) -> Self {
        self.nav_footer = Some(footer.into_any_element());
        self
    }

    /// The header's title, usually the selected category's label.
    pub fn heading(mut self, heading: impl Into<SharedString>) -> Self {
        self.heading = heading.into();
        self
    }

    /// Adds a section to the body. The layout puts a divider between sections.
    pub fn section(mut self, section: impl IntoElement) -> Self {
        self.sections.push(section.into_any_element());
        self
    }

    /// Adds several sections to the body.
    pub fn sections(mut self, sections: impl IntoIterator<Item = AnyElement>) -> Self {
        self.sections.extend(sections);
        self
    }

    /// The footer strip: `start` at the left (a note) and `end` at the right (a button). Without
    /// it the body reaches the bottom.
    pub fn footer(mut self, start: impl IntoElement, end: impl IntoElement) -> Self {
        self.footer = Some((start.into_any_element(), end.into_any_element()));
        self
    }
}

impl RenderOnce for SettingsLayout {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let scroll_handle = window
            .use_keyed_state((self.id.clone(), "scroll"), cx, |_, _| {
                ScrollHandle::default()
            })
            .read(cx)
            .clone();

        let mut body = v_flex()
            .id((self.id.clone(), "body"))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll_handle)
            .p(px(BODY_PADDING))
            .gap(px(BODY_GAP));
        for (index, section) in self.sections.into_iter().enumerate() {
            if index > 0 {
                body = body.child(divider(&palette));
            }
            body = body.child(section);
        }

        let right = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(modal_header(self.heading, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(body)
                    .child(ScrollbarOverlay::new(&scroll_handle)),
            )
            .when_some(self.footer, |column, (start, end)| {
                column.child(
                    modal_footer(cx)
                        .justify_between()
                        .child(
                            div()
                                .text_color(palette.fg_subtle)
                                .text_size(px(12.0))
                                .child(start),
                        )
                        .child(end),
                )
            });

        let on_select = self.on_select;
        let selected = self.selected;
        let nav = v_flex()
            .flex_none()
            .w(px(SETTINGS_NAV_WIDTH))
            .h_full()
            // The dialog's `overflow_hidden` clips to its bounds but not to rounded corners, so
            // the nav's background would otherwise paint square ones over the card's.
            .rounded_l(px(MODAL_INNER_RADIUS))
            .bg(palette.surface)
            .border_r_1()
            .border_color(palette.border)
            .px(px(10.0))
            .py(px(14.0))
            .gap(px(2.0))
            .child(
                div()
                    .px(px(8.0))
                    .pt(px(2.0))
                    .pb(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.0))
                    .child(self.title),
            )
            .children(
                self.categories
                    .into_iter()
                    .enumerate()
                    .map(|(index, category)| {
                        let on_select = on_select.clone();
                        nav_item(index, category, index == selected, &palette).on_click(
                            move |_, window, cx| {
                                if let Some(on_select) = &on_select {
                                    on_select(index, window, cx);
                                }
                            },
                        )
                    }),
            )
            .child(div().flex_1())
            .when_some(self.nav_footer, |nav, footer| {
                nav.child(
                    div()
                        .p(px(8.0))
                        .text_color(palette.fg_subtle)
                        .text_size(px(11.5))
                        .line_height(relative(1.5))
                        .child(footer),
                )
            });

        h_flex().size_full().child(nav).child(right)
    }
}

/// One navigation item: highlighted when `selected`, hover and pressed backgrounds otherwise.
fn nav_item(
    index: usize,
    category: SettingsCategory,
    selected: bool,
    palette: &crate::Palette,
) -> Stateful<Div> {
    let item = h_flex()
        .id(ElementId::NamedInteger("settings-nav".into(), index as u64))
        .h(px(NAV_ITEM_HEIGHT))
        .items_center()
        .gap_2()
        .px(px(8.0))
        .rounded(px(RADIUS_MD - 1.0))
        .cursor_pointer();
    let item = if selected {
        item.bg(palette.accent_subtle)
            .text_color(palette.accent_text)
            .font_weight(FontWeight::MEDIUM)
    } else {
        let (hover, pressed) = (palette.hover, palette.pressed);
        item.hover(move |style| style.bg(hover))
            .active(move |style| style.bg(pressed))
    };
    item.child(Icon::new(category.icon).small())
        .child(category.label)
}

/// A full width 1 px divider between sections. `flex_none`, or the scrolled body, whose content
/// overflows it, shrinks this contentless line to nothing.
fn divider(palette: &crate::Palette) -> Div {
    div().flex_none().h(px(1.0)).bg(palette.border)
}

/// A setting's row: a label with an optional muted description on the left, the control on the
/// right.
#[derive(IntoElement)]
pub struct LabeledRow {
    label: SharedString,
    description: Option<SharedString>,
    control: AnyElement,
}

impl LabeledRow {
    /// A row with `label` and `control`.
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            description: None,
            control: control.into_any_element(),
        }
    }

    /// A line under the label, in the muted small text.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl RenderOnce for LabeledRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        // The label column shrinks and wraps a long description instead of pushing the control
        // past the right edge, which kept its natural width and overflowed the modal.
        let leading = v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(2.0))
            .child(div().child(self.label))
            .when_some(self.description, |column, text| {
                column.child(
                    div()
                        .text_color(palette.fg_muted)
                        .text_size(px(12.0))
                        .child(text),
                )
            });
        h_flex()
            .items_center()
            .justify_between()
            .gap(px(ROW_GAP))
            .child(leading)
            .child(div().flex_none().child(self.control))
    }
}

/// A section's title in medium weight, with an optional one line description under it in the
/// muted small text.
#[derive(IntoElement)]
pub struct SectionHeading {
    title: SharedString,
    description: Option<SharedString>,
}

impl SectionHeading {
    /// A heading with `title`.
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            description: None,
        }
    }

    /// The description under the title.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl RenderOnce for SectionHeading {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        v_flex()
            .gap(px(2.0))
            .child(div().font_weight(FontWeight::MEDIUM).child(self.title))
            .when_some(self.description, |column, text| {
                column.child(
                    div()
                        .text_color(palette.fg_muted)
                        .text_size(px(12.0))
                        .child(text),
                )
            })
    }
}
