//! [`TriCheckbox`]: a checkbox with a third, mixed state (telaio #32).
//!
//! gpui-component's `Checkbox` has no indeterminate state, but the `gpui-base` checkbox under it
//! does. This component is built on that one and copies the look of gpui-component's
//! `Checkbox` as [`checkbox_halo`](super::checkbox_halo) styles it: same box size, radius,
//! border, accent fill and check icon, and the same halo on hover and press. The mixed state
//! shows a minus on the accent fill, as a checked box shows its check.
//!
//! The box is controlled: a click calls the handler, and the app decides the next state, usually
//! [`CheckState::toggled`] (a mixed box becomes checked, like a file with some changes staged
//! that gets staged entirely). A click is consumed by the box, it does not reach a parent row.
//! The colors come from the gpui-kit theme, which is built from the palette, so the box matches
//! the stock checkbox in every palette.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Checkbox as BaseCheckbox, CheckboxIndicator, CheckboxState, spring};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::{CHECKBOX_HALO_PADDING, RADIUS_SM};

/// A click handler, factored out because clippy's `type_complexity` flags the inline form.
type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// The state of a [`TriCheckbox`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CheckState {
    /// Not checked.
    #[default]
    Unchecked,
    /// Checked.
    Checked,
    /// Partly checked: some of what the box stands for is, some is not.
    Mixed,
}

impl CheckState {
    /// The state a click usually asks for: checked becomes unchecked, and both unchecked and
    /// mixed become checked.
    pub fn toggled(self) -> Self {
        match self {
            Self::Checked => Self::Unchecked,
            Self::Unchecked | Self::Mixed => Self::Checked,
        }
    }

    fn base(self) -> CheckboxState {
        match self {
            Self::Unchecked => CheckboxState::Unchecked,
            Self::Checked => CheckboxState::Checked,
            Self::Mixed => CheckboxState::Indeterminate,
        }
    }
}

/// A checkbox with checked, unchecked and mixed states, bare or with a label.
#[derive(IntoElement)]
pub struct TriCheckbox {
    id: ElementId,
    state: CheckState,
    label: Option<SharedString>,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

impl TriCheckbox {
    /// A bare box in `state`. `id` must be unique among the elements on screen.
    pub fn new(id: impl Into<ElementId>, state: CheckState) -> Self {
        Self {
            id: id.into(),
            state,
            label: None,
            disabled: false,
            on_click: None,
        }
    }

    /// Puts `label` beside the box. The halo reacts to the pointer over the label too, and a
    /// click on the label counts as a click on the box.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Greys the box out, drops the halo and ignores clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called on a click of the box (or its label) and on the keyboard activation. The box does
    /// not change by itself: set the state you want, see [`CheckState::toggled`].
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

/// The mark inside the box: a check or a minus, faded in and out like the stock checkbox's.
fn mark(
    id: ElementId,
    state: CheckState,
    disabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let opacity = spring(
        (id, "mark"),
        if state == CheckState::Unchecked {
            0.
        } else {
            1.
        },
        cx.theme().motion_tokens().spring_control,
        window,
        cx,
    );
    let color = if disabled {
        cx.theme().primary_foreground.opacity(0.5)
    } else {
        cx.theme().primary_foreground
    };
    let icon = match state {
        CheckState::Mixed => IconName::Minus,
        _ => IconName::Check,
    };
    svg()
        .absolute()
        .top_px()
        .left_px()
        .size_3()
        .text_color(color)
        .when(opacity > 0., |this| this.path(icon.path()).opacity(opacity))
}

impl RenderOnce for TriCheckbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let (hover, pressed) = (palette.pressed, palette.border_strong);
        let disabled = self.disabled;
        let state = self.state;
        let group = SharedString::from(format!("{}-halo-group", self.id));
        let handler = self.on_click.filter(|_| !disabled);

        let unchecked_border = cx.theme().input;
        let accent = cx.theme().primary;
        let disabled_color = if state == CheckState::Unchecked {
            unchecked_border.opacity(0.5)
        } else {
            accent.opacity(0.5)
        };
        let radius = cx.theme().radius.min(px(4.));
        let input_background = cx.theme().input_background();
        let accent_fill = cx.theme().tokens.primary;

        let indicator = CheckboxIndicator::new()
            .state(state.base())
            .disabled(disabled)
            .relative()
            .size(rems(1.))
            .flex_shrink_0()
            .border_1()
            .rounded(radius)
            .when(state == CheckState::Unchecked, |this| {
                this.bg(input_background)
                    .when(!disabled, |this| this.border_color(unchecked_border))
            })
            .styles(|styles| {
                let filled =
                    |style: gpui_kit::base::StateStyle| style.border_color(accent).bg(accent_fill);
                styles
                    .checked(filled)
                    .indeterminate(filled)
                    .disabled(|style| {
                        style
                            .border_color(disabled_color)
                            .when(state != CheckState::Unchecked, |style| {
                                style.bg(disabled_color)
                            })
                    })
            })
            .child(mark(self.id.clone(), state, disabled, window, cx));

        let on_box = handler.clone();
        let boxed = BaseCheckbox::new(self.id.clone())
            .state(state.base())
            .disabled(disabled)
            .when_some(self.label.clone(), |this, label| {
                this.accessibility_label(label)
            })
            .when_some(on_box, |this, handler| {
                this.on_change(move |_, event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx);
                })
            })
            .p(px(CHECKBOX_HALO_PADDING))
            .m(px(-CHECKBOX_HALO_PADDING))
            .rounded(px(RADIUS_SM))
            .when(!disabled, |this| {
                this.cursor_pointer()
                    .when(self.label.is_some(), |this| {
                        this.group_hover(group.clone(), move |style| style.bg(hover))
                            .group_active(group.clone(), move |style| style.bg(pressed))
                    })
                    .when(self.label.is_none(), |this| {
                        this.hover(move |style| style.bg(hover))
                            .active(move |style| style.bg(pressed))
                    })
            })
            .child(indicator);

        let Some(text) = self.label else {
            return boxed.into_any_element();
        };
        let label = div()
            .id(SharedString::from(format!("{}-label", self.id)))
            .text_color(if disabled {
                palette.fg_muted
            } else {
                palette.fg
            })
            .when(!disabled, |label| label.cursor_pointer())
            .when_some(handler, |label, handler| {
                label.on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx);
                })
            })
            .child(text);
        h_flex()
            .id(SharedString::from(format!("{}-row", self.id)))
            .group(group)
            .gap_2()
            .items_center()
            .child(boxed)
            .child(label)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::CheckState;

    #[test]
    fn a_click_checks_unless_already_checked() {
        assert_eq!(CheckState::Unchecked.toggled(), CheckState::Checked);
        assert_eq!(CheckState::Mixed.toggled(), CheckState::Checked);
        assert_eq!(CheckState::Checked.toggled(), CheckState::Unchecked);
    }
}
