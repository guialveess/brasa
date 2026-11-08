//! The autocompletion menu of the request editor's single-line fields (see [`CompletionField`]):
//! `{{ }}` variables in the URL, the values of Params, Headers and Form and the Auth fields, common
//! header names and values in the Headers table, and header names in the name of an API key sent
//! as a header.
//!
//! It opens with `Ctrl+Space`, or by itself while a `{{` marker or a header is being typed. The
//! input keeps the focus the whole time: the arrow keys, `Enter`, `Tab` and `Escape` are bound in
//! the `Input` context to the actions below, which act only while the menu is open for the
//! focused input and otherwise let the input's own binding run (`cx.propagate()`). What the menu
//! lists comes from `state::variable_completion` and `state::header_completion`; its rows and
//! the highlighted one are a `state::completion::CompletionMenu`.
//!
//! The script code editors draw their own menu (`views/script_completion.rs`); `Ctrl+Space` and
//! `Tab` reach it through the same actions.

use std::ops::Range;

use gpui::{anchored, deferred};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::actions::{
    ConfirmCompletion, DismissCompletion, NextCompletion, PreviousCompletion, ShowCompletion,
};
use crate::state::completion::{CompletionItem, CompletionMenu};
use crate::state::header_completion::{self, HEADER_NAMES, header_values};
use crate::state::variable_completion::{self, trigger_at};
use crate::state::variable_hint::{Hint, completions};
use crate::theme::PaletteExt;
use crate::theme::metrics::{
    COMPLETION_MENU_MAX_ROWS, COMPLETION_MENU_WIDTH, MENU_ROW_HEIGHT, RADIUS_LG, RADIUS_SM,
};

use super::request_editor::{CompletionField, RowKind};
use super::root::AppView;

/// Gap between the field's text line and the menu below it.
const MENU_OFFSET_Y: f32 = 4.0;

/// The open menu.
pub(crate) struct ActiveCompletion {
    field: CompletionField,
    input: Entity<InputState>,
    menu: CompletionMenu,
    /// Shown below the rows: variables are listed but no environment is selected, so its
    /// variables are missing.
    no_environment: bool,
    /// Where the menu's top left corner goes, in window coordinates.
    anchor: Point<Pixels>,
    scroll: ScrollHandle,
}

/// What the menu of a field lists for its current text and cursor.
struct Listing {
    items: Vec<CompletionItem>,
    /// Whether the variables are part of it, so the "no environment" note applies.
    variables: bool,
    /// The byte offset of the field's text the menu is drawn under.
    anchor_at: usize,
}

impl AppView {
    /// `Ctrl+Space`: opens the menu in the focused field, or lets the key through elsewhere.
    pub(crate) fn on_show_completion(
        &mut self,
        _: &ShowCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.focused_completion_field(window, cx) {
            Some(field) => self.update_completion(field, true, cx),
            None if self.show_script_completion(window, cx) => {}
            None => cx.propagate(),
        }
    }

    /// The text of `field` changed: opens, filters or closes the menu after what was typed.
    pub(crate) fn on_completion_field_changed(
        &mut self,
        field: CompletionField,
        cx: &mut Context<Self>,
    ) {
        self.update_completion(field, false, cx);
    }

    /// `field` lost the focus: its menu closes.
    pub(crate) fn on_completion_field_blurred(
        &mut self,
        field: CompletionField,
        cx: &mut Context<Self>,
    ) {
        if self
            .completion
            .as_ref()
            .is_some_and(|open| open.field == field)
        {
            self.close_completion(cx);
        }
    }

    /// Closes the menu, if it is open.
    pub(crate) fn close_completion(&mut self, cx: &mut Context<Self>) {
        if self.completion.take().is_some() {
            cx.notify();
        }
    }

    /// What `field` offers for `text` with the cursor at `cursor` and `selection` selected.
    ///
    /// Inside a `{{` marker, the variables. Otherwise a Headers key offers header names, a
    /// Headers value the usual values of its header, and the name of an API key sent as a header
    /// the header names too; on demand (`explicit`, `Ctrl+Space`) the value fields add the
    /// variables after them, inserted at the cursor.
    fn completion_listing(
        &self,
        field: CompletionField,
        text: &str,
        cursor: usize,
        selection: Range<usize>,
        explicit: bool,
    ) -> Listing {
        let variables = || {
            completions(
                &self.request_editor_variable_context(),
                self.completion_masks_values(field),
            )
        };

        if let CompletionField::Key(kind, _) = field {
            let items = if kind == RowKind::Headers {
                header_completion::items(HEADER_NAMES, text, explicit)
            } else {
                Vec::new()
            };
            return Listing {
                items,
                variables: false,
                anchor_at: 0,
            };
        }

        if let Some(trigger) = trigger_at(text, cursor, selection.clone(), false) {
            let hints = variable_completion::filter(&variables(), &trigger.query);
            return Listing {
                items: variable_completion::items(hints, &trigger.replace),
                variables: true,
                anchor_at: trigger.replace.start,
            };
        }

        let header_items = match self.completion_header_name(field) {
            Some(name) => header_completion::items(&header_values(&name), text, explicit),
            None if self.completes_header_names(field) => {
                header_completion::items(HEADER_NAMES, text, explicit)
            }
            None => Vec::new(),
        };
        let mut listing = Listing {
            anchor_at: if header_items.is_empty() {
                selection.start
            } else {
                0
            },
            items: header_items,
            variables: false,
        };
        if explicit {
            listing
                .items
                .extend(variable_completion::items(variables(), &selection));
            listing.variables = true;
        }
        listing
    }

    /// Opens or refreshes the menu of `field` for its current text and cursor, or closes it when
    /// there is nothing to offer (and no "no environment" note to show).
    fn update_completion(
        &mut self,
        field: CompletionField,
        explicit: bool,
        cx: &mut Context<Self>,
    ) {
        let was_open = self
            .completion
            .as_ref()
            .is_some_and(|open| open.field == field);
        let Some(input) = self.completion_input(field) else {
            self.close_completion(cx);
            return;
        };
        let (text, cursor, selection) = {
            let state = input.read(cx);
            (state.value(), state.cursor(), state.selected_range())
        };
        let listing = self.completion_listing(field, &text, cursor, selection, explicit);
        let no_environment =
            listing.variables && !self.request_editor_variable_context().has_environment();
        if listing.items.is_empty() && !no_environment {
            self.close_completion(cx);
            return;
        }
        let at = listing.anchor_at..listing.anchor_at;
        let state = input.read(cx);
        let anchor = state
            .range_to_bounds(&at)
            .or_else(|| state.cursor_layout().map(|(bounds, _)| bounds))
            .map_or_else(
                || state.input_bounds().bottom_left(),
                |bounds| bounds.bottom_left(),
            )
            + point(px(0.0), px(MENU_OFFSET_Y));

        // What was typed changed the list: its best match is the highlighted row again.
        let menu = CompletionMenu::new(listing.items);
        let scroll = self
            .completion
            .take()
            .filter(|_| was_open)
            .map_or_else(ScrollHandle::new, |open| open.scroll);
        scroll.scroll_to_item(0);
        self.completion = Some(ActiveCompletion {
            field,
            input,
            menu,
            no_environment,
            anchor,
            scroll,
        });
        cx.notify();
    }

    /// The open menu, when its input has the focus.
    fn focused_completion(&mut self, window: &Window, cx: &App) -> Option<&mut ActiveCompletion> {
        self.completion
            .as_mut()
            .filter(|open| open.input.focus_handle(cx).is_focused(window))
    }

    /// `Down` while the menu is open: highlights the next row.
    pub(crate) fn on_next_completion(
        &mut self,
        _: &NextCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.focused_completion(window, cx) {
            Some(open) => {
                open.menu.select_next();
                open.scroll.scroll_to_item(open.menu.selected);
                cx.notify();
            }
            None => cx.propagate(),
        }
    }

    /// `Up` while the menu is open: highlights the previous row.
    pub(crate) fn on_previous_completion(
        &mut self,
        _: &PreviousCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.focused_completion(window, cx) {
            Some(open) => {
                open.menu.select_previous();
                open.scroll.scroll_to_item(open.menu.selected);
                cx.notify();
            }
            None => cx.propagate(),
        }
    }

    /// `Enter` or `Tab` while the menu is open: inserts the highlighted row. With nothing to
    /// insert (only the "no environment" note), closes the menu and lets the key through.
    pub(crate) fn on_confirm_completion(
        &mut self,
        _: &ConfirmCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.focused_completion(window, cx) else {
            if !self.confirm_script_completion(window, cx) {
                cx.propagate();
            }
            return;
        };
        match open.menu.selected().cloned() {
            Some(item) => self.insert_completion(item, window, cx),
            None => {
                self.close_completion(cx);
                cx.propagate();
            }
        }
    }

    /// `Escape` while the menu is open: closes it.
    pub(crate) fn on_dismiss_completion(
        &mut self,
        _: &DismissCompletion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.focused_completion(window, cx).is_some() {
            self.close_completion(cx);
        } else {
            cx.propagate();
        }
    }

    /// Replaces the row's range of the field with its text, closes the menu and reports the edit
    /// like typing does, so the request follows it.
    fn insert_completion(
        &mut self,
        item: CompletionItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.completion.take() else {
            return;
        };
        open.input.update(cx, |state, cx| {
            state.set_selected_range(item.replace.clone(), cx);
            state.replace(item.insert.clone(), window, cx);
            // `replace` is silent: emit the change the field's subscription writes back.
            cx.emit(InputEvent::Change);
        });
        cx.notify();
    }

    /// The open menu, drawn above everything else at its anchor.
    pub(crate) fn render_completion(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let open = self.completion.as_ref()?;
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let weak = cx.weak_entity();

        let rows = open.menu.items.iter().enumerate().map(|(ix, item)| {
            let selected = ix == open.menu.selected;
            let chosen = item.clone();
            let weak = weak.clone();
            h_flex()
                .id(("completion", ix))
                .h(px(MENU_ROW_HEIGHT))
                .flex_none()
                .items_center()
                .gap_3()
                .px_2()
                .rounded(px(RADIUS_SM))
                .cursor_pointer()
                .when(selected, |this| this.bg(palette.accent_subtle))
                .when(!selected, |this| this.hover(|this| this.bg(palette.hover)))
                // On the press, before the click could take the focus from the input.
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    let chosen = chosen.clone();
                    let _ = weak.update(cx, |view, cx| view.insert_completion(chosen, window, cx));
                })
                .child(
                    div()
                        .flex_none()
                        .font_family(mono_font.clone())
                        .text_sm()
                        .text_color(if selected {
                            palette.accent_text
                        } else {
                            palette.fg
                        })
                        .child(item.label.clone()),
                )
                .children(item.variable.as_ref().map(|hint| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_right()
                        .text_xs()
                        .text_color(palette.fg_subtle)
                        .child(value_label(hint))
                }))
        });

        let menu = v_flex()
            .id("completion-menu")
            .w(px(COMPLETION_MENU_WIDTH))
            .p_1()
            .bg(palette.overlay)
            .border_1()
            .border_color(palette.border)
            .rounded(px(RADIUS_LG))
            .shadow(palette.shadow.clone())
            .on_mouse_down_out(cx.listener(|view, _, _, cx| view.close_completion(cx)))
            .when(!open.menu.items.is_empty(), |this| {
                this.child(
                    v_flex()
                        .id("completion-rows")
                        .max_h(px(MENU_ROW_HEIGHT * COMPLETION_MENU_MAX_ROWS))
                        .overflow_y_scroll()
                        .track_scroll(&open.scroll)
                        .children(rows),
                )
            })
            .when(open.no_environment, |this| {
                this.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(palette.fg_subtle)
                        .child(t!("request.variable.completion_no_environment")),
                )
            });

        Some(
            deferred(
                anchored()
                    .position(open.anchor)
                    .snap_to_window_with_margin(px(8.0))
                    .child(menu),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// What a variable row shows on the right: the value as in the tooltips (masked or shortened), or
/// where it comes from when the value is not known yet.
fn value_label(hint: &Hint) -> String {
    match hint.value.as_deref() {
        Some("") => t!("request.variable.empty").into_owned(),
        Some(value) => value.to_string(),
        None => t!("request.variable.completion_script").into_owned(),
    }
}
