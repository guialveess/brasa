//! The not-focused rendering of a text field that holds `{{ }}` markers: plain text segments and
//! [`VariableChip`]s, each chip with a tooltip showing the variable's current value.
//!
//! A single-line `Input` cannot style byte ranges (see `docs/gpui-notes.md`, "Variable chips"), so
//! [`UrlBar`](super::url_bar::UrlBar) and [`VariableField`] show this row while the field is not
//! focused and swap to the live `Input` when it is. The body code editor does not need it: it
//! styles the markers in place with text decorations (`views/request_editor.rs`).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::{VariableKind, VariableSpan, variable_spans};

use super::HoverInput;
use super::IconButton;
use super::edit_menu;
use super::variable_chip::VariableChip;
use crate::state::variable_hint::{Hint, Origin, VariableContext, hint};
use crate::theme::PaletteExt;
use crate::theme::metrics::{INPUT_PADDING_X, RADIUS_MD, VARIABLE_FIELD_HEIGHT};

/// A text-carrying handler (chip click), factored out because clippy's `type_complexity` flags
/// the inline form.
pub type TextHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// A handler without arguments (the eye button of a masked field), factored out for the same
/// reason as [`TextHandler`].
pub type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// The side of the eye button of a masked field.
const MASK_TOGGLE_SIZE: f32 = 20.0;

/// The lines of the tooltip for `hint`: `name = value`, where the value comes from, and a note
/// when it is hidden.
pub fn tooltip_lines(hint: &Hint) -> Vec<String> {
    let mut lines = Vec::new();
    match hint.value.as_deref() {
        Some("") => lines.push(format!("{} = {}", hint.name, t!("request.variable.empty"))),
        Some(value) => lines.push(format!("{} = {value}", hint.name)),
        None => lines.push(hint.name.clone()),
    }
    lines.push(
        match &hint.origin {
            Some(Origin::Environment(name)) => {
                t!("request.variable.origin_environment", name = name)
            }
            Some(Origin::Session) => t!("request.variable.origin_session"),
            None => t!("request.variable.script_defined"),
        }
        .into_owned(),
    );
    if hint.masked {
        lines.push(t!("request.variable.masked").into_owned());
    }
    lines
}

/// The tooltip text for `hint`, one line per entry of [`tooltip_lines`].
pub fn tooltip_text(hint: &Hint) -> String {
    tooltip_lines(hint).join("\n")
}

/// The children of a line showing `text`: the plain segments between `spans` and one chip per
/// span. A chip is defined or in red according to `context`, shows a tooltip when defined, and
/// calls `on_chip_click` with the variable's name when clicked. `mask_all` hides every value in
/// the tooltips (a variable inside a sensitive header). `id` keeps the chips' element ids apart
/// from other lines on screen.
pub fn marker_children(
    id: &SharedString,
    text: &str,
    spans: &[VariableSpan],
    context: &VariableContext,
    mask_all: bool,
    on_chip_click: Option<&TextHandler>,
) -> Vec<AnyElement> {
    let mut children = Vec::new();
    let mut cursor = 0;
    for span in spans {
        if span.range.start > cursor {
            children.push(
                text[cursor..span.range.start]
                    .to_string()
                    .into_any_element(),
            );
        }
        let defined = context.is_defined(span);
        let marker_text = text[span.range.clone()].to_string();
        let name = span.name.clone();
        let mut chip = div()
            .id((id.clone(), span.range.start))
            .cursor_pointer()
            .child(VariableChip::new(marker_text, defined));
        if let Some(hint) = hint(context, span, mask_all) {
            let tooltip = SharedString::from(tooltip_text(&hint));
            chip = chip.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));
        }
        // Only a variable chip calls the handler, not a template function call. A handler that
        // acts on the click stops its propagation, or the line around the chip also takes it and
        // focuses its input.
        if let Some(handler) = on_chip_click
            && span.kind == VariableKind::Variable
        {
            let handler = handler.clone();
            chip = chip.on_click(move |_, window, cx| handler(name.clone(), window, cx));
        }
        children.push(chip.into_any_element());
        cursor = span.range.end;
    }
    if cursor < text.len() {
        children.push(text[cursor..].to_string().into_any_element());
    }
    children
}

/// `text` with every character outside its `{{ }}` markers replaced by a bullet.
fn mask_literals(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in variable_spans(text) {
        masked.extend(text[cursor..span.range.start].chars().map(|_| '\u{2022}'));
        masked.push_str(&text[span.range.clone()]);
        cursor = span.range.end;
    }
    masked.extend(text[cursor..].chars().map(|_| '\u{2022}'));
    masked
}

/// A single-line field of the request editor (a Params, Headers or Form cell, an Auth field): the
/// live `Input` while focused or while its text has no marker, a row of chips otherwise.
#[derive(IntoElement)]
pub struct VariableField {
    id: SharedString,
    input_state: Entity<InputState>,
    text: SharedString,
    context: Rc<VariableContext>,
    mask_all: bool,
    masked: bool,
    on_chip_click: Option<TextHandler>,
    on_mask_toggle: Option<ActionHandler>,
}

impl VariableField {
    /// A field for `input_state`, whose current content is `text`. `id` must be unique on screen.
    pub fn new(
        id: impl Into<SharedString>,
        input_state: Entity<InputState>,
        text: impl Into<SharedString>,
        context: Rc<VariableContext>,
    ) -> Self {
        Self {
            id: id.into(),
            input_state,
            text: text.into(),
            context,
            mask_all: false,
            masked: false,
            on_chip_click: None,
            on_mask_toggle: None,
        }
    }

    /// Hides the text around the markers behind bullets while not focused, for a secret. The
    /// input itself must be masked too (`InputState::masked`).
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    /// Hides the value of every variable in the tooltips, for the value of a sensitive header.
    pub fn mask_all(mut self, mask_all: bool) -> Self {
        self.mask_all = mask_all;
        self
    }

    /// Adds an eye button at the end of the field, which calls `handler` to switch between the
    /// masked and the plain text. The handler changes both [`Self::masked`] and the input's own
    /// masking.
    pub fn on_mask_toggle(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_mask_toggle = Some(Rc::new(handler));
        self
    }

    /// Sets the handler for clicking a chip, called with the variable's name.
    pub fn on_chip_click(
        mut self,
        handler: impl Fn(String, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_chip_click = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for VariableField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self
            .input_state
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        let text = if self.masked {
            SharedString::from(mask_literals(&self.text))
        } else {
            self.text.clone()
        };
        let spans = if focused || !text.contains("{{") {
            Vec::new()
        } else {
            variable_spans(&text)
        };
        let mask_toggle = self.on_mask_toggle.map(|handler| {
            let (icon, tooltip) = if self.masked {
                (IconName::Eye, t!("request.auth.show_secret"))
            } else {
                (IconName::EyeOff, t!("request.auth.hide_secret"))
            };
            IconButton::new(SharedString::from(format!("{}-mask", self.id)), icon)
                .box_size(MASK_TOGGLE_SIZE)
                .tooltip(tooltip.into_owned())
                .on_click(move |_, window, cx| {
                    // The chips row around the button focuses the input on a click.
                    cx.stop_propagation();
                    handler(window, cx);
                })
        });
        if spans.is_empty() {
            let accent = cx.palette().accent;
            let input = Input::new(&self.input_state)
                .context_menu(edit_menu(&self.input_state, cx))
                .px(px(INPUT_PADDING_X))
                .py_0()
                // The same height as the row of chips, smaller than the default 32 px so it stays
                // clear of the table's row lines.
                .h(px(VARIABLE_FIELD_HEIGHT))
                .w_full()
                // The default 3 px focus ring spills over the table rows around the cell,
                // so a focused cell only gets the accent border.
                .focus_ring(false)
                .when(focused, |input| input.border_color(accent))
                .when_some(mask_toggle, |input, toggle| input.suffix(toggle));
            return HoverInput::new(format!("{}-input", self.id), &self.input_state, input)
                .into_any_element();
        }

        let input_state = self.input_state.clone();
        let palette = cx.palette();
        let border_strong = palette.border_strong;
        let children = marker_children(
            &self.id,
            &text,
            &spans,
            &self.context,
            self.mask_all,
            self.on_chip_click.as_ref(),
        );
        h_flex()
            .id(self.id.clone())
            .w_full()
            .h(px(VARIABLE_FIELD_HEIGHT))
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .cursor_text()
            .px(px(INPUT_PADDING_X))
            .border_1()
            .border_color(palette.border)
            .hover(move |style| style.border_color(border_strong))
            .rounded(px(RADIUS_MD))
            .on_click(move |_, window, cx| {
                // `character: u32::MAX` clamps to the end of the line, which also focuses the
                // input and so swaps this row for it on the next render.
                let end = gpui_kit::base::input::Position::new(0, u32::MAX);
                input_state.update(cx, |state, cx| state.set_cursor_position(end, window, cx));
            })
            .map(|row| match mask_toggle {
                // The button stays at the end while the chips are clipped before it.
                Some(toggle) => row
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .items_center()
                            .children(children),
                    )
                    .child(div().flex_shrink_0().child(toggle)),
                None => row.children(children),
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::mask_literals;
    use pretty_assertions::assert_eq;

    #[test]
    fn mask_literals_keeps_only_the_markers() {
        assert_eq!(mask_literals("pässw"), "\u{2022}".repeat(5));
        assert_eq!(
            mask_literals("a{{ secret }}bc"),
            "\u{2022}{{ secret }}\u{2022}\u{2022}"
        );
        assert_eq!(mask_literals("{{x}}"), "{{x}}");
    }
}
