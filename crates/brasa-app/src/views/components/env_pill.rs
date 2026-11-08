//! [`EnvPill`] and [`EnvMenuItem`]: the title bar's environment picker.
//!
//! The trigger is gpui-kit's [`Button`], but the dropdown is our own [`Popover`] with hand-built
//! rows, not gpui-kit's `PopupMenu`: that widget paints its selected-row highlight on a wrapper
//! that only leaves a hardcoded few pixels of margin from the panel's edge, which cannot be
//! widened from the outside. `Popover::content` hands back a plain `Div`, so the rows' own
//! padding and the panel's own [`Popover::p`] set the margins directly.

use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use super::NeutralButtonExt;
use crate::state::env_color::{EnvColor, auto_color};
use crate::theme::metrics::{ENV_PILL_HEIGHT, MENU_DOT_COLUMN_WIDTH, MENU_ROW_HEIGHT, RADIUS_MD};
use crate::theme::{Palette, PaletteExt, BrasaColors};

/// Widest the active environment's name gets on the pill, so a long one does not reach the search
/// box. Longer names are cut.
const TRIGGER_LABEL_MAX_WIDTH: f32 = 110.0;
/// Widest an environment's name gets in the dropdown.
const MENU_LABEL_MAX_WIDTH: f32 = 360.0;
/// Width of the environment picker's popover panel.
const ENV_MENU_WIDTH: f32 = 260.0;
/// Padding between the popover panel's border and its rows, on every side.
const ENV_MENU_PADDING: f32 = 6.0;

/// One row of an [`EnvPill`]'s dropdown: `None` is "No environment" (a hollow ring), `Some(name)`
/// is a named environment (a dot in its color, see [`EnvMenuItem::color`]).
pub struct EnvMenuItem {
    name: Option<SharedString>,
    color: Option<EnvColor>,
    active: bool,
    shortcut: Option<SharedString>,
    on_click: Option<MenuItemHandler>,
}

/// A menu item click handler, factored out because clippy's `type_complexity` flags the inline
/// form.
type MenuItemHandler = Rc<dyn Fn(&mut Window, &mut App)>;

impl EnvMenuItem {
    /// The "No environment" row.
    pub fn none() -> Self {
        Self {
            name: None,
            color: None,
            active: false,
            shortcut: None,
            on_click: None,
        }
    }

    /// A row for the named environment `name`.
    pub fn named(name: impl Into<SharedString>) -> Self {
        Self {
            name: Some(name.into()),
            color: None,
            active: false,
            shortcut: None,
            on_click: None,
        }
    }

    /// Sets the dot color of a named environment, the chosen or automatic one
    /// (`state::env_color::EnvColors`). Without it, the automatic color of the name.
    pub fn color(mut self, color: EnvColor) -> Self {
        self.color = Some(color);
        self
    }

    /// The dot color, `None` for "No environment".
    fn dot_color(&self) -> Option<EnvColor> {
        let name = self.name.as_deref()?;
        Some(self.color.unwrap_or_else(|| auto_color(name)))
    }

    /// Marks this row as the active environment (shows a check instead of its dot/ring).
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Sets the trailing shortcut hint, for example `"Ctrl 3"`.
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Sets the click handler.
    #[allow(dead_code)] // The environment picker handles clicks through its own menu.
    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    fn label(&self) -> SharedString {
        self.name
            .clone()
            .unwrap_or_else(|| t!("common.no_environment").into_owned().into())
    }
}

/// The title bar's environment pill: a dot, the active environment's name, a chevron, on
/// `raised` with a border; opens an [`EnvMenuItem`] dropdown.
#[derive(IntoElement)]
pub struct EnvPill {
    id: SharedString,
    items: Vec<EnvMenuItem>,
}

impl EnvPill {
    /// A pill listing `items` in its dropdown. The active item's label (or "No environment")
    /// shows on the trigger itself.
    pub fn new(id: impl Into<SharedString>, items: Vec<EnvMenuItem>) -> Self {
        Self {
            id: id.into(),
            items,
        }
    }
}

impl RenderOnce for EnvPill {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let active = self.items.iter().find(|item| item.active);
        let trigger_label = active
            .map(EnvMenuItem::label)
            .unwrap_or_else(|| t!("common.no_environment").into_owned().into());
        let trigger_dot = active
            .and_then(EnvMenuItem::dot_color)
            .map(|color| palette.env_color(color));

        let mut trigger_dot_or_ring = div().flex_none().size(px(7.0)).rounded_full();
        trigger_dot_or_ring = match trigger_dot {
            Some(color) => trigger_dot_or_ring.bg(color),
            None => trigger_dot_or_ring
                .border_1()
                .border_color(palette.fg_subtle),
        };

        let trigger = Button::new(self.id.clone())
            .neutral_bordered(cx, palette.raised, palette.border, palette.fg)
            .h(px(ENV_PILL_HEIGHT))
            .px_2p5()
            .rounded(px(RADIUS_MD - 1.0))
            .border_1()
            .child(trigger_dot_or_ring)
            .child(
                div()
                    .max_w(px(TRIGGER_LABEL_MAX_WIDTH))
                    .truncate()
                    .child(trigger_label),
            )
            .dropdown_caret(true);

        let popover_id = SharedString::from(format!("{}-popover", self.id));
        let items = self.items;
        Popover::new(popover_id)
            .w(px(ENV_MENU_WIDTH))
            .p(px(ENV_MENU_PADDING))
            .trigger(trigger)
            .content(move |_, _, cx| render_env_menu(&items, &palette, cx))
            .into_any_element()
    }
}

/// The pill's popover content: one row per item, each closing the popover and running its own
/// `on_click` handler.
fn render_env_menu(items: &[EnvMenuItem], palette: &Palette, cx: &mut Context<PopoverState>) -> Div {
    let rows = items.iter().enumerate().map(|(index, item)| {
        let color = item.dot_color().map(|color| palette.env_color(color));
        let label = item.label();
        let active = item.active;
        let shortcut = item.shortcut.clone();
        let handler = item.on_click.clone();
        let accent_text = palette.accent_text;
        let fg_subtle = palette.fg_subtle;

        let mut leading = div()
            .flex_none()
            .w(px(MENU_DOT_COLUMN_WIDTH))
            .flex()
            .justify_center();
        leading = if active {
            leading.child(Icon::new(IconName::Check).small().text_color(accent_text))
        } else {
            match color {
                Some(color) => leading.child(div().size(px(7.0)).rounded_full().bg(color)),
                None => leading.child(
                    div()
                        .size(px(7.0))
                        .rounded_full()
                        .border_1()
                        .border_color(fg_subtle),
                ),
            }
        };

        h_flex()
            .id(("env-menu-item", index))
            .h(px(MENU_ROW_HEIGHT))
            .items_center()
            .gap(px(12.0))
            .px_3()
            .w_full()
            .rounded(px(RADIUS_MD - 2.0))
            .cursor_pointer()
            .when(active, |row| row.bg(palette.list_active))
            .hover(|style| style.bg(palette.hover))
            .active(|style| style.bg(palette.pressed))
            .child(leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .max_w(px(MENU_LABEL_MAX_WIDTH))
                    .truncate()
                    .child(label.clone()),
            )
            .when_some(shortcut.clone(), |row, shortcut| {
                row.child(
                    div()
                        .flex_none()
                        .text_size(px(11.0))
                        .text_color(fg_subtle)
                        .child(shortcut),
                )
            })
            .on_click(cx.listener(move |popover, _, window, cx| {
                popover.dismiss(window, cx);
                if let Some(handler) = &handler {
                    handler(window, cx);
                }
            }))
    });

    v_flex().gap(px(2.0)).children(rows)
}
