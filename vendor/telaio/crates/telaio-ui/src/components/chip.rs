//! [`Chip`]: a small accent pill with an optional icon, a label and a dismiss button (in Postino
//! load tests, the "34-46 s" range chip that clears the selection).
//!
//! A plain `div`, since no gpui-kit widget has this shape: the 24 px pill is `accent_subtle`
//! with `accent_text` text, and the 18 px close button lights up with the accent on hover.

use std::rc::Rc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::PaletteExt;
use crate::metrics::{
    CHIP_DISMISS_SIZE, CHIP_GAP, CHIP_HEIGHT, CHIP_ICON_SIZE, CHIP_PADDING_LEFT,
    CHIP_PADDING_RIGHT, CHIP_TEXT_SIZE, RADIUS_SM, RADIUS_XS,
};

/// The opacity of the accent behind the dismiss button while it is hovered.
const DISMISS_HOVER_OPACITY: f32 = 0.25;

/// A dismiss click handler.
type DismissHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// A 24 px accent pill: an optional 12 px icon, a 12 px label and, with [`Self::on_dismiss`], a
/// close button. Without a dismiss handler it is a plain label chip, with equal padding on both
/// sides.
#[derive(IntoElement)]
pub struct Chip {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    tooltip: Option<SharedString>,
    on_dismiss: Option<DismissHandler>,
}

impl Chip {
    /// A chip showing `label`. `id` identifies the dismiss button.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            tooltip: None,
            on_dismiss: None,
        }
    }

    /// Shows `icon` (12 px) before the label.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Sets the tooltip of the dismiss button; "Close" by default.
    pub fn dismiss_tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Adds the dismiss button, which calls `handler` when clicked.
    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Chip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let dismissable = self.on_dismiss.is_some();
        let hover = palette.accent.opacity(DISMISS_HOVER_OPACITY);
        let tooltip = self
            .tooltip
            .unwrap_or_else(|| SharedString::from(t!("telaio.close").to_string()));

        h_flex()
            .h(px(CHIP_HEIGHT))
            .flex_none()
            .items_center()
            .gap(px(CHIP_GAP))
            .pl(px(CHIP_PADDING_LEFT))
            .pr(px(if dismissable {
                CHIP_PADDING_RIGHT
            } else {
                CHIP_PADDING_LEFT
            }))
            .rounded(px(RADIUS_SM))
            .bg(palette.accent_subtle)
            .text_color(palette.accent_text)
            .text_size(px(CHIP_TEXT_SIZE))
            .children(
                self.icon
                    .map(|icon| icon.with_size(px(CHIP_ICON_SIZE)).flex_none()),
            )
            .child(div().min_w_0().truncate().child(self.label))
            .children(self.on_dismiss.map(|handler| {
                h_flex()
                    .id(self.id)
                    .size(px(CHIP_DISMISS_SIZE))
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(RADIUS_XS))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .on_click(move |_, window, cx| handler(window, cx))
                    .child(Icon::new(gpui_kit::assets::IconName::X).with_size(px(CHIP_ICON_SIZE)))
            }))
    }
}
