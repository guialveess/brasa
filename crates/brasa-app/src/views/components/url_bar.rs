//! [`UrlBar`]: the method selector plus URL field joined in one bordered box.
//!
//! Variable chips in a single-line input need a fallback (see `docs/gpui-notes.md`): gpui-component's `Input`/`InputState` cannot style byte ranges (only its
//! multi-line, language-aware `Editor` can), so `UrlBar` renders a row of plain text and
//! [`VariableChip`] spans while the field is not focused, and swaps to a live `Input` bound to
//! the caller's own `Entity<InputState>` once it is. Wiring `UrlBar` into the real request
//! editor (creating that entity, subscribing to its change events, calling
//! `brasa_runner::preview` on every edit) is the request editor's job; this component only renders and
//! forwards clicks.

use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::{Method, VariableSpan};

use super::NeutralButtonExt;
use super::edit_menu;
use super::variable_line::{TextHandler, marker_children};
use crate::state::variable_hint::VariableContext;
use crate::theme::metrics::{MENU_ROW_HEIGHT, RADIUS_MD, RADIUS_SM, URL_BAR_HEIGHT};
use crate::theme::{Palette, PaletteExt, BrasaColors};

/// Width of the method selector's popover panel.
const METHOD_MENU_WIDTH: f32 = 160.0;
/// Padding between the popover panel's border and its rows, on every side.
const METHOD_MENU_PADDING: f32 = 6.0;

/// Line height of the focused URL `Input`, which is also the height of its selection highlight.
const URL_LINE_HEIGHT: f32 = 22.0;

/// The standard methods offered by the selector, before the trailing "Custom..." entry.
const STANDARD_METHODS: [Method; 7] = [
    Method::Get,
    Method::Post,
    Method::Put,
    Method::Patch,
    Method::Delete,
    Method::Head,
    Method::Options,
];

/// The method selector joined to a URL field, with variable chips styled as
/// described in `docs/design-system.md`: `accent` for a defined `{{var}}`, `danger` with a wavy
/// underline for an unknown one.
#[derive(IntoElement)]
pub struct UrlBar {
    method: Method,
    text: SharedString,
    /// Every `{{ }}` marker in `text` (`brasa_core::variable_spans`), for styling.
    spans: Vec<VariableSpan>,
    /// What each marker in `spans` resolves to: it decides the chip color and the tooltip.
    context: Rc<VariableContext>,
    input_state: Entity<InputState>,
    /// When set, replaces the method dropdown with a live `Input` bound to this entity, for typing
    /// a custom method token.
    editing_method: Option<Entity<InputState>>,
    on_method_change: Option<MethodHandler>,
    on_chip_click: Option<TextHandler>,
    /// Fired when the URL text changes. `UrlBar` is a `RenderOnce` (no `Context<Self>`), so it
    /// cannot subscribe to `input_state`'s own change events itself; the owning view would need
    /// to do that subscription and call this callback from there. The request editor (`views/request_editor.rs`)
    /// does not use this: it already subscribes to the URL `InputState` directly (as it
    /// does for its other inputs) to write edits back into the request, and recomputes the live
    /// variable preview from scratch on every render instead, which is simpler and gives the
    /// same result. Kept on the builder in case a future caller needs the narrower callback.
    #[allow(dead_code)] // no caller needs the narrower callback; see the comment above
    on_text_change: Option<TextHandler>,
    /// Called when Escape blurs the field (see [`Self::on_escape`]'s doc comment).
    on_escape: Option<VoidHandler>,
}

/// A method change handler, factored out because clippy's `type_complexity` flags the inline
/// form.
type MethodHandler = Rc<dyn Fn(Method, &mut Window, &mut App)>;
/// A handler with no payload (Escape), see [`MethodHandler`].
type VoidHandler = Rc<dyn Fn(&mut Window, &mut App)>;

impl UrlBar {
    /// A bar showing `method` and `text`, with `spans` (`brasa_core::variable_spans(text)`)
    /// and `context` (how each marker resolves) driving chip colors and tooltips. `input_state`
    /// backs the live `Input` shown once the bar is focused.
    pub fn new(
        method: Method,
        text: impl Into<SharedString>,
        spans: Vec<VariableSpan>,
        context: Rc<VariableContext>,
        input_state: Entity<InputState>,
    ) -> Self {
        Self {
            method,
            text: text.into(),
            spans,
            context,
            input_state,
            editing_method: None,
            on_method_change: None,
            on_chip_click: None,
            on_text_change: None,
            on_escape: None,
        }
    }

    /// Replaces the method dropdown with a live `Input` bound to `input_state`, for typing a
    /// custom method token. `None` (the default) shows the normal dropdown.
    pub fn editing_method(mut self, input_state: Option<Entity<InputState>>) -> Self {
        self.editing_method = input_state;
        self
    }

    /// Sets the handler for picking a different method.
    pub fn on_method_change(
        mut self,
        handler: impl Fn(Method, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_method_change = Some(Rc::new(handler));
        self
    }

    /// Sets the handler for clicking a variable chip, called with the variable's name.
    /// `views/request_editor.rs` only acts on a danger (undefined) chip, opening the Define
    /// dialog.
    pub fn on_chip_click(
        mut self,
        handler: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_chip_click = Some(Rc::new(handler));
        self
    }

    /// Sets the URL text change handler (see the field's own doc comment for how it is
    /// actually invoked).
    #[allow(dead_code)] // no caller needs the narrower callback; see the field's doc comment
    pub fn on_text_change(
        mut self,
        handler: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_text_change = Some(Rc::new(handler));
        self
    }

    /// Sets the handler run when Escape blurs the field (GitHub #19): the caller is expected to
    /// move focus somewhere the app's own keyboard shortcuts still reach, since a plain
    /// `window.blur` leaves nothing focused and `AppView`'s `Ctrl ,`/`Ctrl S`/... `on_action`
    /// handlers only fire while a descendant of its own tracked focus is focused. Falls back to
    /// `window.blur` when unset (the components gallery's demo `UrlBar` has no `AppView` focus
    /// handle to return to).
    pub fn on_escape(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_escape = Some(Rc::new(handler));
        self
    }

    /// Renders the method selector: while [`Self::editing_method`] is set, a small live `Input`
    /// for typing a custom method token; otherwise the method's label in its own color, with a
    /// dropdown of the standard methods plus "Custom..." (which prefills the current custom
    /// token, if the method is already one, instead of always starting empty, so re-opening it
    /// edits rather than resets).
    fn render_method(&self, cx: &mut App) -> AnyElement {
        let palette = cx.palette();

        if let Some(input) = &self.editing_method {
            return h_flex()
                .h(px(URL_BAR_HEIGHT))
                .w(px(80.0))
                .flex_none()
                .items_center()
                .child(
                    // Same sizing as the focused URL field, see `RenderOnce for UrlBar`.
                    Input::new(input)
                        .context_menu(edit_menu(input, cx))
                        .h_full()
                        .px_2()
                        .py_0()
                        .line_height(px(URL_LINE_HEIGHT))
                        .bordered(false),
                )
                .into_any_element();
        }

        let mono_font = cx.theme().mono_font_family.clone();
        let color = palette.method_color(&self.method);
        let badge_bg = palette.method_badge_bg(&self.method);
        let label = self.method.to_string();
        let handler = self.on_method_change.clone();
        let current_custom = match &self.method {
            Method::Custom(text) => text.clone(),
            _ => String::new(),
        };

        // A chip floating inside the bar, like `MethodBadge::pill`, instead of a flush segment
        // with a hard divider: the method reads as a tag on the request, not a separate control
        // glued to the URL field.
        let trigger = Button::new("url-bar-method")
            .neutral_ghost(cx, color)
            .h(px(URL_BAR_HEIGHT - 12.0))
            .ml(px(6.0))
            .px_2p5()
            .rounded(px(RADIUS_SM))
            .bg(badge_bg)
            .dropdown_caret(true)
            .child(
                div()
                    .font_family(mono_font)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(12.0))
                    .text_color(color)
                    .child(label),
            );

        Popover::new("url-bar-method-popover")
            .w(px(METHOD_MENU_WIDTH))
            .p(px(METHOD_MENU_PADDING))
            .trigger(trigger)
            .content(move |_, _, cx| {
                render_method_menu(&palette, handler.clone(), &current_custom, cx)
            })
            .into_any_element()
    }

    /// Renders the not-focused URL line: plain text segments and [`VariableChip`]s, each chip
    /// wrapped in its own clickable `div` (see this module's doc comment for why, instead of a
    /// single [`gpui::StyledText`] with highlight runs, which cannot carry a per-range click
    /// handler on its own). Clicking anywhere on the row focuses [`Self::input_state`] and moves
    /// the caret to the end, which swaps this line for the live `Input` on the next render. A
    /// chip whose handler acts on the click stops it before it reaches the row.
    fn render_line(&self, cx: &mut App) -> AnyElement {
        let mono_font = cx.theme().mono_font_family.clone();
        let input_state = self.input_state.clone();
        let mut row = h_flex()
            .id("url-bar-line")
            .flex_1()
            .h_full()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .cursor_text()
            .font_family(mono_font)
            .text_size(px(12.5))
            .px_2()
            .on_click(move |_, window, cx| {
                // `character: u32::MAX` clamps to the line's actual length
                // (`gpui_kit::base::input::rope_ext`'s `position_to_offset`), so this reaches the
                // end of the URL without having to compute its UTF-16 length here.
                let end = gpui_kit::base::input::Position::new(0, u32::MAX);
                input_state.update(cx, |state, cx| state.set_cursor_position(end, window, cx));
            });
        row = row.children(marker_children(
            &SharedString::from("url-bar-chip"),
            &self.text,
            &self.spans,
            &self.context,
            false,
            self.on_chip_click.as_ref(),
        ));
        row.into_any_element()
    }
}

/// The method selector's popover content: the standard methods, a separator, then "Custom...".
/// Hand-built rows (not gpui-component's `PopupMenu`), for the same reason as the environment
/// pill's dropdown: full control over hover, cursor and edge spacing.
fn render_method_menu(
    palette: &Palette,
    handler: Option<Rc<dyn Fn(Method, &mut Window, &mut App)>>,
    current_custom: &str,
    cx: &mut Context<PopoverState>,
) -> Div {
    let row = |id: ElementId, label: SharedString, palette: &Palette| {
        h_flex()
            .id(id)
            .h(px(MENU_ROW_HEIGHT))
            .items_center()
            .px_3()
            .w_full()
            .rounded(px(RADIUS_MD - 2.0))
            .cursor_pointer()
            .text_sm()
            .hover(|style| style.bg(palette.hover))
            .active(|style| style.bg(palette.pressed))
            .child(label)
    };

    let rows = STANDARD_METHODS.into_iter().enumerate().map(|(index, method)| {
        let handler = handler.clone();
        row(
            ("url-bar-method-item", index).into(),
            method.to_string().into(),
            palette,
        )
        .on_click(cx.listener(move |popover, _, window, cx| {
            popover.dismiss(window, cx);
            if let Some(handler) = &handler {
                handler(method.clone(), window, cx);
            }
        }))
    });

    let custom_handler = handler.clone();
    let current_custom = current_custom.to_string();
    let custom_row = row(
        "url-bar-method-custom".into(),
        t!("request.url_bar.custom_method").into_owned().into(),
        palette,
    )
    .on_click(cx.listener(move |popover, _, window, cx| {
        popover.dismiss(window, cx);
        if let Some(handler) = &custom_handler {
            handler(Method::Custom(current_custom.clone()), window, cx);
        }
    }));

    v_flex()
        .gap(px(2.0))
        .children(rows)
        .child(div().h(px(1.0)).my(px(4.0)).bg(palette.border))
        .child(custom_row)
}

impl RenderOnce for UrlBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let focused = self
            .input_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);

        let on_escape = self.on_escape.clone();
        let field: AnyElement = if focused {
            // Same mono family, size and horizontal padding as `render_line` below, so focusing
            // the field does not move or restyle the URL (GitHub #16 follow-up): `Input` defaults
            // to the UI sans font. `h_full` plus no vertical padding keeps the line inside the
            // bar's border (a fixed `URL_BAR_HEIGHT` overflowed it by the border's 2px and
            // clipped descenders), and the taller line height gives a selection some room above
            // and below the glyphs. Transparent, so the bar's own background shows through.
            Input::new(&self.input_state)
                .context_menu(edit_menu(&self.input_state, cx))
                .h_full()
                .flex_1()
                .px_2()
                .py_0()
                .line_height(px(URL_LINE_HEIGHT))
                .bordered(false)
                .bg(palette.raised.opacity(0.0))
                .font_family(mono_font)
                .text_size(px(12.5))
                .into_any_element()
        } else {
            self.render_line(cx)
        };

        h_flex()
            // An id keeps the hover state, so entering and leaving renders again.
            .id("url-bar")
            .h(px(URL_BAR_HEIGHT))
            .items_center()
            .rounded(px(RADIUS_MD))
            .border_1()
            // The `Input`'s own `Escape` handling only clears an in-progress selection or IME
            // composition and otherwise propagates (`gpui-base-0.6.6/src/input/base/state.rs`'s
            // `escape`); catching the propagated action here is what makes Escape blur the field
            // and swap back to the chip line, matching a plain click elsewhere. `on_escape`
            // (GitHub #19) moves focus back to the app's own tracked focus handle instead of a
            // plain `window.blur`, which left nothing focused and no keyboard shortcut reachable.
            .on_action::<gpui_kit::base::input::Escape>(move |_, window, cx| match &on_escape {
                Some(handler) => handler(window, cx),
                None => window.blur(cx),
            })
            .border_color(if focused {
                palette.accent
            } else {
                palette.border
            })
            // The hover of every text input (telaio's `HoverInput`).
            .when(!focused, |bar| {
                let border_strong = palette.border_strong;
                bar.hover(move |style| style.border_color(border_strong))
            })
            .when(focused, |bar| {
                bar.shadow(vec![
                    BoxShadow::new(px(0.0), px(0.0), palette.accent_subtle).spread_radius(px(3.0)),
                ])
            })
            .bg(palette.raised)
            .overflow_hidden()
            .child(self.render_method(cx))
            .child(field)
    }
}
