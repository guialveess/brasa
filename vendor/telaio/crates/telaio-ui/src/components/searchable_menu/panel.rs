//! The drawing and the key handling of a [`SearchableMenuState`]: the panel the popover shows.

use gpui_kit::assets::IconName;
use gpui_kit::base::POPUP_PRIORITY;
use gpui_kit::base::actions::{Cancel, Confirm, SelectDown, SelectLeft, SelectRight, SelectUp};
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::logic::{Entry, highlight_range};
use super::state::{LIST_PADDING_Y, MenuAction, MenuRow, SearchableMenuState};
use crate::PaletteExt;
use crate::fonts::MONO_FAMILY;
use crate::metrics::{
    RADIUS_LG, RADIUS_SM, SEARCHABLE_MENU_ACTION_HEIGHT, SEARCHABLE_MENU_BADGE_TEXT_SIZE,
    SEARCHABLE_MENU_FOOTER_HEIGHT, SEARCHABLE_MENU_LABEL_GAP, SEARCHABLE_MENU_LABEL_HEIGHT,
    SEARCHABLE_MENU_ROW_HEIGHT, SEARCHABLE_MENU_SEARCH_HEIGHT, SEARCHABLE_MENU_SUBTITLE_TEXT_SIZE,
    SEARCHABLE_MENU_TITLE_TEXT_SIZE, SEARCHABLE_MENU_WIDTH,
};
use crate::{Palette, components::scrollbar};

/// Radius of a highlighted row.
const ROW_RADIUS: f32 = 7.0;
/// Horizontal padding of the list, around the rows.
const LIST_PADDING_X: f32 = 6.0;
/// Horizontal padding inside a row and a group label.
const ROW_PADDING_X: f32 = 10.0;
/// Space between the parts of a row.
const ROW_GAP: f32 = 10.0;
/// Width of the leading icon column of a row.
const ICON_COLUMN: f32 = 14.0;
/// Size of a row's icons.
const ICON_SIZE: f32 = 13.0;
/// Size of the chevron and of the toggle of a row.
const TRAILING_ICON_SIZE: f32 = 12.0;
/// Size of the search icon.
const SEARCH_ICON_SIZE: f32 = 13.0;
/// Space between the panel and its submenu.
const SUBMENU_GAP: f32 = 4.0;
/// Padding of the panel's border and of the list that separates the submenu from the row's edge.
const PANEL_EDGE: f32 = 1.0;

impl Render for SearchableMenuState {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = cx.palette();
        let submenu = self.render_submenu();
        let actions = (!self.visible_actions.is_empty()).then(|| self.render_actions(&palette, cx));
        let body = self.render_body(&palette, cx);

        div()
            .key_context("List PopupMenu")
            .relative()
            .w(px(SEARCHABLE_MENU_WIDTH))
            .on_action(cx.listener(|this, _: &SelectUp, _, cx| this.move_by(-1, cx)))
            .on_action(cx.listener(|this, _: &SelectDown, _, cx| this.move_by(1, cx)))
            .on_action(cx.listener(|this, action: &Confirm, _, cx| {
                this.confirm(action.secondary, cx);
            }))
            .on_action(cx.listener(|this, _: &Cancel, window, cx| {
                if this.submenu.is_some() {
                    this.close_submenu(true, window, cx);
                } else {
                    this.dismiss(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectRight, window, cx| {
                if let Some(ix) = this.highlight {
                    this.open_submenu(ix, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectLeft, window, cx| {
                if this.submenu.is_some() {
                    this.close_submenu(true, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // Clicking the panel must not move the focus out of the search box.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                if !this.submenu_bounds.get().contains(&event.position) {
                    this.dismiss(cx);
                }
            }))
            .child(
                v_flex()
                    .w_full()
                    .bg(palette.overlay)
                    .border_1()
                    .border_color(palette.border_strong)
                    .rounded(px(RADIUS_LG))
                    .shadow(palette.shadow.clone())
                    .overflow_hidden()
                    .child(self.render_search(&palette))
                    .children(actions)
                    .child(body)
                    .when(!self.hints.is_empty(), |panel| {
                        panel.child(self.render_hints(&palette))
                    }),
            )
            .children(submenu)
    }
}

impl SearchableMenuState {
    fn render_search(&self, palette: &Palette) -> impl IntoElement {
        h_flex()
            .h(px(SEARCHABLE_MENU_SEARCH_HEIGHT))
            .flex_none()
            .items_center()
            .px(px(12.0))
            .border_b_1()
            .border_color(palette.border)
            .child(
                Input::new(&self.input)
                    .prefix(
                        Icon::new(IconName::Search)
                            .with_size(px(SEARCH_ICON_SIZE))
                            .text_color(palette.fg_subtle),
                    )
                    .cleanable(true)
                    .p_0()
                    .appearance(false),
            )
    }

    fn render_actions(&self, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let hover = palette.hover;
        let mut buttons = Vec::with_capacity(self.visible_actions.len());
        for &ix in &self.visible_actions {
            if let Some(action) = self.actions.get(ix) {
                buttons.push(Self::render_action(ix, action, palette, hover, cx));
            }
        }
        h_flex()
            .flex_none()
            .gap(px(4.0))
            .p(px(6.0))
            .border_b_1()
            .border_color(palette.border)
            .children(buttons)
            .into_any_element()
    }

    fn render_action(
        ix: usize,
        action: &MenuAction,
        palette: &Palette,
        hover: Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = action.id.clone();
        let disabled = action.disabled;
        h_flex()
            .id(("telaio-menu-action", ix))
            .flex_1()
            .min_w_0()
            .h(px(SEARCHABLE_MENU_ACTION_HEIGHT))
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .rounded(px(RADIUS_SM))
            .bg(palette.raised)
            .border_1()
            .border_color(palette.border)
            .text_size(px(12.0))
            .text_color(palette.fg)
            .when(disabled, |button| button.opacity(0.45))
            .when(!disabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.finish(super::SearchableMenuEvent::Action(id.clone()), cx);
                    }))
            })
            .child(
                Icon::new(action.icon)
                    .with_size(px(12.0))
                    .text_color(palette.fg_muted),
            )
            .child(div().truncate().child(action.label.clone()))
            .children(action.badge.clone().map(|badge| {
                div()
                    .flex_none()
                    .font_family(MONO_FAMILY)
                    .text_size(px(10.5))
                    .text_color(palette.accent_text)
                    .child(badge)
            }))
            .into_any_element()
    }

    fn render_body(&self, palette: &Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.is_empty() {
            return div()
                .flex_none()
                .h(px(SEARCHABLE_MENU_ROW_HEIGHT + LIST_PADDING_Y))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(palette.fg_subtle)
                .child(self.empty_message.clone())
                .into_any_element();
        }
        let rows = list(
            self.list.clone(),
            cx.processor(|this, ix, _window, cx| this.render_entry(ix, cx)),
        )
        .size_full();
        div()
            .relative()
            .flex_none()
            .h(px(self.list_height))
            .child(
                div()
                    .size_full()
                    .px(px(LIST_PADDING_X))
                    .pt(px(4.0))
                    .pb(px(6.0))
                    .child(rows),
            )
            .child(scrollbar::overlay(
                &self.list,
                Some("telaio-menu-scrollbar".into()),
                palette,
            ))
            .into_any_element()
    }

    fn render_hints(&self, palette: &Palette) -> impl IntoElement {
        h_flex()
            .h(px(SEARCHABLE_MENU_FOOTER_HEIGHT))
            .flex_none()
            .items_center()
            .gap(px(14.0))
            .px(px(12.0))
            .border_t_1()
            .border_color(palette.border)
            .text_size(px(SEARCHABLE_MENU_SUBTITLE_TEXT_SIZE))
            .text_color(palette.fg_subtle)
            .children(self.hints.iter().map(|(key, meaning)| {
                h_flex()
                    .gap(px(5.0))
                    .child(
                        div()
                            .font_family(MONO_FAMILY)
                            .text_color(palette.fg_muted)
                            .child(key.clone()),
                    )
                    .child(meaning.clone())
            }))
    }

    /// One entry of the list, built only while it is on screen.
    fn render_entry(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette();
        match self.entries.get(ix).copied() {
            Some(Entry::Label(group)) => {
                let label = self
                    .groups
                    .get(group)
                    .map(|g| g.label.clone())
                    .unwrap_or_default();
                div()
                    .w_full()
                    .h(px(SEARCHABLE_MENU_LABEL_HEIGHT + SEARCHABLE_MENU_LABEL_GAP))
                    .pt(px(SEARCHABLE_MENU_LABEL_GAP))
                    .child(
                        h_flex()
                            .h(px(SEARCHABLE_MENU_LABEL_HEIGHT))
                            .items_center()
                            .px(px(ROW_PADDING_X))
                            .text_size(px(SEARCHABLE_MENU_BADGE_TEXT_SIZE))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(palette.fg_subtle)
                            .child(label),
                    )
                    .into_any_element()
            }
            Some(entry) => {
                let Some(row) = self.row_at(ix).cloned() else {
                    return div().into_any_element();
                };
                self.render_row(ix, entry, &row, &palette, cx)
            }
            None => div().into_any_element(),
        }
    }

    fn render_row(
        &self,
        ix: usize,
        entry: Entry,
        row: &MenuRow,
        palette: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let highlighted = self.highlight == Some(ix);
        let accent = palette.accent_text;
        let range = highlight_range(&row.title, &self.query);
        let title = StyledText::new(row.title.clone()).with_highlights(range.map(|range| {
            (
                range,
                HighlightStyle {
                    color: Some(accent),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..HighlightStyle::default()
                },
            )
        }));
        let toggle_id = row.id.clone();
        let with_chevron = self.has_submenus() && entry != Entry::Create;

        div()
            .w_full()
            .h(px(SEARCHABLE_MENU_ROW_HEIGHT))
            .child(
                h_flex()
                    .id(("telaio-menu-row", ix))
                    .size_full()
                    .items_center()
                    .gap(px(ROW_GAP))
                    .px(px(ROW_PADDING_X))
                    .rounded(px(ROW_RADIUS))
                    .cursor_pointer()
                    .when(highlighted, |row| row.bg(palette.list_active))
                    .on_mouse_move(
                        cx.listener(move |this, _, window, cx| this.hover(ix, window, cx)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.activate(ix, cx)))
                    .child(
                        div().w(px(ICON_COLUMN)).flex_none().child(
                            Icon::new(row.icon)
                                .with_size(px(ICON_SIZE))
                                .text_color(row.icon_color.unwrap_or(palette.fg_subtle)),
                        ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .truncate()
                                    .font_family(MONO_FAMILY)
                                    .text_size(px(SEARCHABLE_MENU_TITLE_TEXT_SIZE))
                                    .text_color(row.title_color.unwrap_or(palette.fg))
                                    .child(title),
                            )
                            .children(row.subtitle.clone().map(|subtitle| {
                                div()
                                    .truncate()
                                    .text_size(px(SEARCHABLE_MENU_SUBTITLE_TEXT_SIZE))
                                    .text_color(palette.fg_subtle)
                                    .child(subtitle)
                            })),
                    )
                    .children(row.badge.clone().map(|badge| {
                        div()
                            .flex_none()
                            .font_family(MONO_FAMILY)
                            .text_size(px(SEARCHABLE_MENU_BADGE_TEXT_SIZE))
                            .text_color(accent)
                            .child(badge)
                    }))
                    .children(row.toggle.filter(|&on| on || highlighted).map(|on| {
                        div()
                            .id(("telaio-menu-toggle", ix))
                            .flex_none()
                            .child(
                                Icon::new(IconName::Star)
                                    .with_size(px(TRAILING_ICON_SIZE))
                                    .text_color(if on {
                                        palette.warning
                                    } else {
                                        palette.fg_subtle
                                    }),
                            )
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.stop_propagation();
                                cx.emit(super::SearchableMenuEvent::Toggle(toggle_id.clone()));
                            }))
                    }))
                    .when(with_chevron, |row| {
                        row.child(
                            div()
                                .id(("telaio-menu-chevron", ix))
                                .flex_none()
                                .child(
                                    Icon::new(IconName::ChevronRight)
                                        .with_size(px(TRAILING_ICON_SIZE))
                                        .text_color(if highlighted {
                                            accent
                                        } else {
                                            palette.fg_subtle
                                        }),
                                )
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.open_submenu(ix, window, cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The deferred layer with the open submenu, next to the row it belongs to.
    fn render_submenu(&self) -> Option<impl IntoElement> {
        let open = self.submenu.as_ref()?;
        let bounds = self.list.bounds_for_item(open.entry)?;
        let position = point(
            bounds.right() + px(LIST_PADDING_X + PANEL_EDGE + SUBMENU_GAP),
            bounds.top(),
        );
        let recorded = self.submenu_bounds.clone();
        Some(
            deferred(
                anchored()
                    .position(position)
                    .anchor(Anchor::TopLeft)
                    .snap_to_window_with_margin(px(8.0))
                    .child(
                        div()
                            .on_prepaint(move |bounds, _, _| recorded.set(bounds))
                            .child(open.menu.clone()),
                    ),
            )
            .with_priority(POPUP_PRIORITY + 1),
        )
    }
}
