//! [`EditableTable`], [`EditableColumn`], [`EditableRow`] and [`EditableCell`]: a bordered table
//! of editable rows (in Postino, the environment variables table), telaio #12.
//!
//! The table owns the look: a header of small labels, rows with a line between them (none
//! after the last), borderless monospace inputs, an optional note line under a cell, read only
//! rows and an empty message. The caller owns the data: a cell holds the caller's
//! `Entity<InputState>`, a plain text, or any element (a chip, an `IconButton`).
//!
//! # Text that must not move when a cell is clicked
//!
//! gpui-component's single line `Input` draws its text in an element exactly one line tall
//! (`window.line_height()`), and every time the selection changes (a click, a key) it scrolls
//! that element so the line is inside it: when the element is even a fraction of a pixel shorter
//! than the line, the text moves up by that fraction for a frame, until the input resets the
//! offset. gpui lays out in device pixels and rounds each box, so a line of 22 logical pixels at
//! a scale of 1.2 (26.4 device pixels) gets a box of 26 or 27 device pixels depending on where
//! the row happens to sit. That was the text jumping up and down. The table avoids it by
//! giving its inputs a line height that is a whole number of device pixels (see
//! [`input_line_height`](super::text_field::input_line_height)), where rounding cannot change the size, and by making the input
//! exactly that tall, so nothing is centered by a fractional amount or shrunk by a flex parent.
//! A single line row has a fixed height, so a cell's height never depends on focus, on the text
//! or on its placeholder.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::edit_menu::edit_menu;
use super::text_field::input_line_height;
use crate::PaletteExt;
use crate::metrics::{
    EDITABLE_TABLE_HEADER_HEIGHT, EDITABLE_TABLE_INPUT_PADDING_X, EDITABLE_TABLE_LINE_HEIGHT,
    EDITABLE_TABLE_PADDING_X, EDITABLE_TABLE_ROW_HEIGHT, EDITABLE_TABLE_SMALL_TEXT_SIZE,
    EDITABLE_TABLE_TEXT_SIZE, RADIUS_MD,
};

/// How wide a column is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Width {
    /// Takes the space left over.
    Flex,
    /// A fixed number of logical pixels.
    Fixed(f32),
}

/// A column of an [`EditableTable`]: its header label and its width. An empty label draws an
/// empty header cell, for a column of buttons.
#[derive(Debug, Clone)]
pub struct EditableColumn {
    label: SharedString,
    width: Width,
}

impl EditableColumn {
    /// A column headed `label` that takes the space left over.
    pub fn flex(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            width: Width::Flex,
        }
    }

    /// A column headed `label`, `width` logical pixels wide.
    pub fn fixed(label: impl Into<SharedString>, width: f32) -> Self {
        Self {
            label: label.into(),
            width: Width::Fixed(width),
        }
    }
}

/// What a cell shows.
enum Content {
    /// An editable single line input, in the mono font.
    Input(Entity<InputState>),
    /// Read only text, in the mono font, cut with an ellipsis when it does not fit.
    Text(SharedString),
    /// Any element.
    Element(AnyElement),
}

/// One cell of an [`EditableRow`].
pub struct EditableCell {
    content: Content,
    note: Option<SharedString>,
    color: Option<Hsla>,
    padding_x: Option<f32>,
    centered: bool,
}

impl EditableCell {
    fn new(content: Content) -> Self {
        Self {
            content,
            note: None,
            color: None,
            padding_x: None,
            centered: false,
        }
    }

    /// An editable cell over `state`: a borderless monospace input with the edit context menu.
    /// The table reads no text from it, the caller keeps the state and its events.
    pub fn input(state: &Entity<InputState>) -> Self {
        Self::new(Content::Input(state.clone()))
    }

    /// A read only monospace text.
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self::new(Content::Text(text.into()))
    }

    /// Any element (a chip, a button), vertically centered in the row.
    pub fn element(element: impl IntoElement) -> Self {
        Self::new(Content::Element(element.into_any_element()))
    }

    /// A line of small text under the cell's content.
    pub fn note(mut self, note: impl Into<SharedString>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// The text color of an input or text cell, `fg` by default.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Replaces the horizontal padding of the cell, in logical pixels.
    pub fn padding_x(mut self, padding: f32) -> Self {
        self.padding_x = Some(padding);
        self
    }

    /// Centers an element horizontally, with no padding: for a column holding a small button.
    pub fn centered(mut self) -> Self {
        self.centered = true;
        self
    }

    fn has_note(&self) -> bool {
        self.note.is_some()
    }
}

/// One row of an [`EditableTable`]: a cell per column, left to right.
pub struct EditableRow {
    cells: Vec<EditableCell>,
    background: Option<Hsla>,
}

impl EditableRow {
    /// A row of `cells`. Missing trailing cells leave their columns empty.
    pub fn new(cells: impl IntoIterator<Item = EditableCell>) -> Self {
        Self {
            cells: cells.into_iter().collect(),
            background: None,
        }
    }

    /// Tints the row, to tell a read only row from the editable ones.
    pub fn background(mut self, background: Hsla) -> Self {
        self.background = Some(background);
        self
    }
}

/// A bordered table with a header, editable rows and an empty message.
#[derive(IntoElement)]
pub struct EditableTable {
    columns: Vec<EditableColumn>,
    rows: Vec<EditableRow>,
    empty: Option<SharedString>,
}

impl EditableTable {
    /// A table with `columns` and no rows.
    pub fn new(columns: impl IntoIterator<Item = EditableColumn>) -> Self {
        Self {
            columns: columns.into_iter().collect(),
            rows: Vec::new(),
            empty: None,
        }
    }

    /// Adds a row.
    pub fn row(mut self, row: EditableRow) -> Self {
        self.rows.push(row);
        self
    }

    /// Adds several rows.
    pub fn rows(mut self, rows: impl IntoIterator<Item = EditableRow>) -> Self {
        self.rows.extend(rows);
        self
    }

    /// What the table says when it has no row.
    pub fn empty_message(mut self, message: impl Into<SharedString>) -> Self {
        self.empty = Some(message.into());
        self
    }
}

impl RenderOnce for EditableTable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let line_height = px(input_line_height(
            EDITABLE_TABLE_LINE_HEIGHT,
            window.scale_factor(),
        ));
        let widths: Vec<Width> = self.columns.iter().map(|column| column.width).collect();

        let header = self.columns.iter().fold(
            h_flex()
                .h(px(EDITABLE_TABLE_HEADER_HEIGHT))
                .items_center()
                .bg(palette.surface)
                // `overflow_hidden` clips to a plain rectangle, never to the card's own rounded
                // corners (gpui has no rounded clip mask): the header's own opaque fill needs its
                // top corners rounded to match, or they poke out past the curve.
                .rounded_tl(px(RADIUS_MD))
                .rounded_tr(px(RADIUS_MD))
                .border_b_1()
                .border_color(palette.border),
            |header, column| {
                header.child(
                    sized(div(), column.width)
                        .px(px(EDITABLE_TABLE_PADDING_X))
                        .truncate()
                        .text_size(px(EDITABLE_TABLE_SMALL_TEXT_SIZE))
                        .text_color(palette.fg_subtle)
                        .child(column.label.clone()),
                )
            },
        );

        let mut table = v_flex()
            .border_1()
            .border_color(palette.border)
            .rounded(px(RADIUS_MD))
            .overflow_hidden()
            .child(header);

        let total = self.rows.len();
        for (position, row) in self.rows.into_iter().enumerate() {
            let last = position + 1 == total;
            // A single line row has a fixed height. Only a note under a cell makes it taller.
            let tall = row.cells.iter().any(EditableCell::has_note);
            let row_height = px(EDITABLE_TABLE_ROW_HEIGHT);
            let mut line = h_flex()
                .items_center()
                .when(tall, |line| line.min_h(row_height))
                .when(!tall, |line| line.h(row_height))
                .when_some(row.background, |line, background| line.bg(background))
                // Same reason as the header: a tinted last row's own fill needs its bottom
                // corners rounded to match the card's, since `overflow_hidden` will not do it.
                .when(last, |line| {
                    line.rounded_bl(px(RADIUS_MD)).rounded_br(px(RADIUS_MD))
                })
                .when(!last, |line| line.border_b_1().border_color(palette.border));
            let mut cells = row.cells.into_iter();
            for width in widths.iter().copied() {
                // A missing trailing cell still takes its column, so the others stay aligned.
                line = line.child(match cells.next() {
                    Some(cell) => render_cell(cell, width, line_height, &mono_font, &palette, cx),
                    None => sized(div(), width),
                });
            }
            table = table.child(line);
        }

        if let Some(message) = self.empty.filter(|_| total == 0) {
            table = table.child(
                div()
                    .px(px(EDITABLE_TABLE_PADDING_X))
                    .py(px(14.0))
                    .text_size(px(EDITABLE_TABLE_TEXT_SIZE))
                    .text_color(palette.fg_subtle)
                    .child(message),
            );
        }
        table
    }
}

/// Horizontal padding inside an input cell's `Input`: with the cell's own
/// [`EDITABLE_TABLE_INPUT_PADDING_X`] it adds up to [`EDITABLE_TABLE_PADDING_X`], the inset of
/// every other cell and of the header.
const INPUT_TEXT_INSET: f32 = EDITABLE_TABLE_PADDING_X - EDITABLE_TABLE_INPUT_PADDING_X;

/// `cell` sized for its column.
fn sized(cell: Div, width: Width) -> Div {
    match width {
        Width::Flex => cell.flex_1().min_w_0(),
        Width::Fixed(width) => cell.flex_none().w(px(width)),
    }
}

/// One cell: its content, then its note.
fn render_cell(
    cell: EditableCell,
    width: Width,
    line_height: Pixels,
    mono_font: &SharedString,
    palette: &crate::Palette,
    cx: &mut App,
) -> Div {
    let is_input = matches!(cell.content, Content::Input(_));
    let is_text = matches!(cell.content, Content::Text(_));
    let default_padding = if is_input {
        EDITABLE_TABLE_INPUT_PADDING_X
    } else {
        EDITABLE_TABLE_PADDING_X
    };
    let color = cell.color.unwrap_or(palette.fg);
    let content = match cell.content {
        Content::Input(state) => div()
            .font_family(mono_font.clone())
            .text_size(px(EDITABLE_TABLE_TEXT_SIZE))
            .text_color(color)
            .child(
                // Exactly one line tall, see the module documentation.
                Input::new(&state)
                    .context_menu(edit_menu(&state, cx))
                    // Its own padding is pinned (the default is 10 px) so its text starts where a
                    // text cell's, a note's and the header's do.
                    .px(px(INPUT_TEXT_INSET))
                    .py_0()
                    .text_size(px(EDITABLE_TABLE_TEXT_SIZE))
                    .h(line_height)
                    .line_height(line_height)
                    .appearance(false)
                    .bordered(false),
            )
            .into_any_element(),
        Content::Text(text) => div()
            .truncate()
            .font_family(mono_font.clone())
            .text_size(px(EDITABLE_TABLE_TEXT_SIZE))
            .text_color(color)
            .child(text)
            .into_any_element(),
        Content::Element(element) => element,
    };
    let note = cell.note.map(|note| {
        div()
            // Under an input the note lines up with the input's own padding.
            .when(is_input, |note| note.px(px(INPUT_TEXT_INSET)))
            .text_size(px(EDITABLE_TABLE_SMALL_TEXT_SIZE))
            .text_color(palette.fg_subtle)
            .child(note)
    });
    let has_note = note.is_some();
    let mut column = sized(v_flex(), width);
    column = column.px(px(if cell.centered {
        0.0
    } else {
        cell.padding_x.unwrap_or(default_padding)
    }));
    // Inputs and texts fill the column, anything else keeps its own width.
    column = if cell.centered {
        column.items_center()
    } else if is_input || is_text {
        column
    } else {
        column.items_start()
    };
    if has_note {
        // Vertical room for the note, and the gap between it and the content.
        column = if is_input {
            column.py(px(3.0))
        } else {
            column.py(px(6.0)).gap(px(2.0))
        };
    }
    column.child(content).children(note)
}
