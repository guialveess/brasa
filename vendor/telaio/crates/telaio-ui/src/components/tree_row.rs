//! [`TreeRow`]: one row of a collapsible tree (in Postino, the collections sidebar and the Load
//! tests panel).
//!
//! The row is only the look and the click: the app owns the tree and which folders are open, and
//! renders one `TreeRow` per visible node, flattened with its depth. A folder shows a chevron
//! (down when open, right when closed); a leaf keeps an empty slot of the same width. The
//! content between the chevron and the label (a method badge, a file icon) and the content at the
//! end of the row, shown only while the row is hovered (a play button), are slots.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::{
    RADIUS_SM, TREE_ROW_DETAIL_TEXT_SIZE, TREE_ROW_HEIGHT, TREE_ROW_ICON_WIDTH, TREE_ROW_INDENT,
    TREE_ROW_LEADING_GAP, TREE_ROW_PADDING_LEFT,
};

/// A click handler, factored out because clippy's `type_complexity` flags the inline form.
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The left padding of a row at `depth`.
fn indent(depth: usize) -> f32 {
    TREE_ROW_PADDING_LEFT + depth as f32 * TREE_ROW_INDENT
}

/// One row of a tree: indentation by depth, a chevron on a folder, an optional leading element,
/// the label, an optional detail text after it, an optional trailing element visible on hover, an
/// optional suffix always visible at the very end, and the hover, pressed and selected
/// backgrounds.
///
/// A leaf by default; [`Self::folder`] makes it a folder.
#[derive(IntoElement)]
pub struct TreeRow {
    id: ElementId,
    label: SharedString,
    detail: Option<SharedString>,
    depth: usize,
    folder: Option<bool>,
    selected: bool,
    strikethrough: bool,
    leading: Option<AnyElement>,
    trailing: Option<AnyElement>,
    suffix: Option<AnyElement>,
    on_click: Option<ClickHandler>,
}

impl TreeRow {
    /// A leaf row at depth 0 showing `label`. `id` must be unique among the rows on screen.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail: None,
            depth: 0,
            folder: None,
            selected: false,
            strikethrough: false,
            leading: None,
            trailing: None,
            suffix: None,
            on_click: None,
        }
    }

    /// Shows `detail` right after the label in smaller, subtle text, for example the folder of a
    /// file in a flat list. It gives up its room before the label does.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Indents the row by `depth` levels (the root level is 0).
    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// Makes the row a folder, with its chevron pointing down when `expanded` and right when not.
    pub fn folder(mut self, expanded: bool) -> Self {
        self.folder = Some(expanded);
        self
    }

    /// Marks the row selected: `list_active` background and `accent_text` label.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Strikes the label through, in `fg_muted`: an entry that is gone (a deleted file in a
    /// list of changes).
    pub fn strikethrough(mut self, strikethrough: bool) -> Self {
        self.strikethrough = strikethrough;
        self
    }

    /// Puts `element` between the chevron slot and the label (a method badge, an icon).
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    /// Puts `element` at the end of the row, visible only while the row is hovered. It takes its
    /// own clicks: call `cx.stop_propagation()` in its handler so the row's click does not fire.
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }

    /// Puts `element` at the very end of the row, after the hover-only [`Self::trailing`] slot,
    /// and always visible (a count, a status letter). It does not take clicks of its own.
    pub fn suffix(mut self, element: impl IntoElement) -> Self {
        self.suffix = Some(element.into_any_element());
        self
    }

    /// Sets the click handler, for the whole row (a folder toggles, a leaf opens).
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for TreeRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let group = SharedString::from(format!("telaio-tree-row-{}", self.id));

        let mut chevron_slot = div()
            .flex_none()
            .w(px(TREE_ROW_ICON_WIDTH))
            .flex()
            .items_center()
            .justify_center();
        if let Some(expanded) = self.folder {
            let chevron = if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            };
            chevron_slot =
                chevron_slot.child(Icon::new(chevron).small().text_color(palette.fg_subtle));
        }

        let label_color = if self.selected {
            palette.accent_text
        } else if self.strikethrough {
            palette.fg_muted
        } else {
            palette.fg
        };

        h_flex()
            .id(self.id)
            .group(group.clone())
            .h(px(TREE_ROW_HEIGHT))
            .items_center()
            .gap_1()
            .pl(px(indent(self.depth)))
            .pr(px(4.0))
            .rounded(px(RADIUS_SM))
            .cursor_pointer()
            .when(self.selected, |this| this.bg(palette.list_active))
            .hover(|style| style.bg(palette.hover))
            .active(|style| style.bg(palette.pressed))
            .child(chevron_slot)
            .children(self.leading.map(|leading| {
                div()
                    .flex_none()
                    .mr(px(TREE_ROW_LEADING_GAP))
                    .child(leading)
            }))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(label_color)
                    .when(self.strikethrough, |label| label.line_through())
                    // With a detail the label keeps its width and the detail takes what is left.
                    .map(|label| match self.detail {
                        Some(_) => label.flex_initial(),
                        None => label.flex_1(),
                    })
                    .child(self.label),
            )
            .children(self.detail.map(|detail| {
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .ml(px(2.0))
                    .text_size(px(TREE_ROW_DETAIL_TEXT_SIZE))
                    .text_color(palette.fg_subtle)
                    .child(detail)
            }))
            .children(self.trailing.map(|trailing| {
                div()
                    .flex_none()
                    .opacity(0.0)
                    .group_hover(group, |style| style.opacity(1.0))
                    .child(trailing)
            }))
            .children(self.suffix.map(|suffix| div().flex_none().child(suffix)))
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{TREE_ROW_INDENT, TREE_ROW_PADDING_LEFT, indent};

    #[test]
    fn the_root_level_has_only_the_base_padding() {
        assert_eq!(indent(0), TREE_ROW_PADDING_LEFT);
    }

    #[test]
    fn each_level_indents_by_the_same_step() {
        assert_eq!(indent(3), TREE_ROW_PADDING_LEFT + 3.0 * TREE_ROW_INDENT);
    }
}
