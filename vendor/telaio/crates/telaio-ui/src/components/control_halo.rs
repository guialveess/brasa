//! The hover and pressed halo of checkboxes and radio buttons (telaio #13), and their labelled
//! forms.
//!
//! gpui-component's `Checkbox` and `Radio` have no hover style of their own, and with a label the
//! whole row is one element, so a halo cannot be put around the box from outside. A labelled
//! control is therefore built here from a bare box, the label beside it, and one hover group
//! over both: the halo lights up whether the pointer is over the box or over the label. The
//! halo is the box's own padding, [`CHECKBOX_HALO_PADDING`], kept out of the layout by a
//! negative margin, and is part of the click target.

use std::rc::Rc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::Palette;
use crate::metrics::{CHECKBOX_HALO_PADDING, RADIUS_SM};

/// Handler of a labelled control, called with the requested checked value.
type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// `checkbox` with a halo around its box: `pressed` on hover and `border_strong` while held, one
/// step above the usual `hover` and `pressed`, which barely show on an area this small. The halo
/// is [`CHECKBOX_HALO_PADDING`] around the box, with the same negative margin as `switch_halo`, so
/// the layout is that of a bare checkbox. Give it no label (see [`labelled_checkbox`] for one).
/// A `disabled` checkbox shows no halo.
pub fn checkbox_halo(checkbox: Checkbox, disabled: bool, palette: &Palette) -> Checkbox {
    let (hover, pressed) = (palette.pressed, palette.border_strong);
    checkbox
        .p(px(CHECKBOX_HALO_PADDING))
        .m(px(-CHECKBOX_HALO_PADDING))
        .rounded(px(RADIUS_SM))
        .when(!disabled, |checkbox| {
            checkbox
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed))
        })
}

/// `radio` with a round halo around its box, like [`checkbox_halo`].
pub fn radio_halo(radio: Radio, disabled: bool, palette: &Palette) -> Radio {
    let (hover, pressed) = (palette.pressed, palette.border_strong);
    radio
        .p(px(CHECKBOX_HALO_PADDING))
        .m(px(-CHECKBOX_HALO_PADDING))
        .rounded_full()
        .when(!disabled, |radio| {
            radio
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed))
        })
}

/// A checkbox with a label. The halo around the box reacts to the pointer over the label too, and
/// a click on the label toggles it.
pub fn labelled_checkbox(
    id: impl Into<ElementId>,
    checked: bool,
    label: impl Into<SharedString>,
    palette: &Palette,
) -> LabelledControl {
    LabelledControl::new(
        ControlKind::Checkbox,
        id.into(),
        checked,
        label.into(),
        palette,
    )
}

/// A radio button with a label, like [`labelled_checkbox`]. Clicking the label of the selected
/// radio does nothing.
pub fn labelled_radio(
    id: impl Into<ElementId>,
    checked: bool,
    label: impl Into<SharedString>,
    palette: &Palette,
) -> LabelledControl {
    LabelledControl::new(
        ControlKind::Radio,
        id.into(),
        checked,
        label.into(),
        palette,
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ControlKind {
    Checkbox,
    Radio,
}

/// The element [`labelled_checkbox`] and [`labelled_radio`] return.
#[derive(IntoElement)]
pub struct LabelledControl {
    kind: ControlKind,
    id: ElementId,
    checked: bool,
    label: SharedString,
    palette: Palette,
    disabled: bool,
    on_change: Option<ChangeHandler>,
}

impl LabelledControl {
    fn new(
        kind: ControlKind,
        id: ElementId,
        checked: bool,
        label: SharedString,
        palette: &Palette,
    ) -> Self {
        Self {
            kind,
            id,
            checked,
            label,
            palette: palette.clone(),
            disabled: false,
            on_change: None,
        }
    }

    /// Greys the label out, drops the halo and ignores clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the requested checked value, from the box, the label or the keyboard.
    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

/// The value a click asks for: a checkbox flips, a radio only ever selects.
fn requested_value(kind: ControlKind, checked: bool) -> Option<bool> {
    match kind {
        ControlKind::Checkbox => Some(!checked),
        ControlKind::Radio => (!checked).then_some(true),
    }
}

impl RenderOnce for LabelledControl {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let palette = self.palette;
        let group = SharedString::from(format!("{}-halo-group", self.id));
        let (hover, pressed) = (palette.pressed, palette.border_strong);
        let disabled = self.disabled;
        let handler = self.on_change;

        // The box takes the halo from the group, so it also lights up under the label.
        let control = match self.kind {
            ControlKind::Checkbox => Checkbox::new(self.id.clone())
                .checked(self.checked)
                .disabled(disabled)
                .p(px(CHECKBOX_HALO_PADDING))
                .m(px(-CHECKBOX_HALO_PADDING))
                .rounded(px(RADIUS_SM))
                .when(!disabled, |checkbox| {
                    checkbox
                        .group_hover(group.clone(), move |style| style.bg(hover))
                        .group_active(group.clone(), move |style| style.bg(pressed))
                })
                .when_some(handler.clone(), |checkbox, handler| {
                    checkbox.on_click(move |checked, window, cx| handler(checked, window, cx))
                })
                .into_any_element(),
            ControlKind::Radio => Radio::new(self.id.clone())
                .checked(self.checked)
                .disabled(disabled)
                .p(px(CHECKBOX_HALO_PADDING))
                .m(px(-CHECKBOX_HALO_PADDING))
                .rounded_full()
                .when(!disabled, |radio| {
                    radio
                        .group_hover(group.clone(), move |style| style.bg(hover))
                        .group_active(group.clone(), move |style| style.bg(pressed))
                })
                .when_some(handler.clone(), |radio, handler| {
                    radio.on_click(move |checked, window, cx| handler(checked, window, cx))
                })
                .into_any_element(),
        };

        let next = requested_value(self.kind, self.checked);
        let label = div()
            .id(SharedString::from(format!("{}-label", self.id)))
            .text_color(if disabled {
                palette.fg_muted
            } else {
                palette.fg
            })
            .when(!disabled, |label| label.cursor_pointer())
            .when_some(
                handler.filter(|_| !disabled).zip(next),
                |label, (handler, next)| {
                    label.on_click(move |_, window, cx| handler(&next, window, cx))
                },
            )
            .child(self.label);

        h_flex()
            .id(SharedString::from(format!("{}-row", self.id)))
            .group(group)
            .gap_2()
            .items_center()
            .child(control)
            .child(label)
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlKind, requested_value};

    #[test]
    fn a_checkbox_flips_and_a_radio_only_selects() {
        assert_eq!(requested_value(ControlKind::Checkbox, false), Some(true));
        assert_eq!(requested_value(ControlKind::Checkbox, true), Some(false));
        assert_eq!(requested_value(ControlKind::Radio, false), Some(true));
        assert_eq!(requested_value(ControlKind::Radio, true), None);
    }
}
