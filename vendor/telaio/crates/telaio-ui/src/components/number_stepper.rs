//! [`NumberStepper`]: a minus button, a monospace readout and a plus button, as the font size
//! stepper of Postino's and Vano's Settings.
//!
//! The caller owns the value: the stepper renders it and calls `on_change` with the next one.
//! The stepping logic is in plain functions ([`step_value`], [`step_decimals`],
//! [`format_value`]) so it is unit tested without a window.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::buttons::DISABLED_OPACITY;
use crate::PaletteExt;
use crate::metrics::{CONTROL_HEIGHT, RADIUS_MD, STEPPER_GAP, STEPPER_READOUT_WIDTH};

/// A boxed change handler, factored out because clippy's `type_complexity` flags the inline form.
type ChangeHandler = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// A boxed readout formatter, for the same reason.
type Formatter = Rc<dyn Fn(f32) -> String>;

/// `-`, the current value and `+`. A button is disabled (dimmed, no hover, no click) when the
/// value is at its bound, and a click calls [`Self::on_change`] with the clamped next value.
///
/// The readout is `value` with as many decimals as it needs (up to those of the step) followed
/// by [`Self::unit`], or exactly [`Self::decimals`] of them, or whatever [`Self::format`]
/// returns, which is how an app writes the number in the user's language.
#[derive(IntoElement)]
pub struct NumberStepper {
    id: ElementId,
    value: f32,
    step: f32,
    min: f32,
    max: f32,
    unit: Option<SharedString>,
    decimals: Option<usize>,
    format: Option<Formatter>,
    on_change: Option<ChangeHandler>,
}

impl NumberStepper {
    /// A stepper showing `value`, with a step of 1 and no bounds. `id` prefixes the ids of its
    /// two buttons.
    pub fn new(id: impl Into<ElementId>, value: f32) -> Self {
        Self {
            id: id.into(),
            value,
            step: 1.0,
            min: f32::NEG_INFINITY,
            max: f32::INFINITY,
            unit: None,
            decimals: None,
            format: None,
            on_change: None,
        }
    }

    /// How much one click adds or removes.
    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// The lowest and highest value, both included.
    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// A suffix for the readout, for example `px` (shown as `13 px`). Ignored with
    /// [`Self::format`].
    pub fn unit(mut self, unit: impl Into<SharedString>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Always shows exactly `decimals` decimals instead of only the ones the value needs.
    /// Ignored with [`Self::format`].
    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = Some(decimals);
        self
    }

    /// Writes the whole readout itself, unit included, for example with the active language's
    /// separators.
    pub fn format(mut self, format: impl Fn(f32) -> String + 'static) -> Self {
        self.format = Some(Rc::new(format));
        self
    }

    /// Called with the new value when a button is clicked.
    pub fn on_change(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }

    /// The text of the readout.
    fn readout(&self) -> String {
        if let Some(format) = &self.format {
            return format(self.value);
        }
        let decimals = self
            .decimals
            .unwrap_or_else(|| shown_decimals(self.value, self.step));
        let number = format!("{:.*}", decimals, self.value);
        match &self.unit {
            Some(unit) => format!("{number} {unit}"),
            None => number,
        }
    }
}

/// `value` moved by `delta` and kept between `min` and `max`, rounded to the decimals of
/// `step` so that float noise never shows (`12.5 + 0.5` is exactly `13`, `0.1 + 0.2` is `0.3`).
pub fn step_value(value: f32, delta: f32, step: f32, min: f32, max: f32) -> f32 {
    let scale = 10f32.powi(step_decimals(step) as i32);
    let moved = ((value + delta) * scale).round() / scale;
    moved.clamp(min, max)
}

/// How many decimals `step` has, at most 6: 1 for 1, 1 for 0.5, 2 for 0.25.
pub fn step_decimals(step: f32) -> usize {
    (0..=6)
        .find(|decimals| {
            let scale = 10f32.powi(*decimals);
            ((step * scale).round() / scale - step).abs() < 1e-6 * scale.max(1.0)
        })
        .unwrap_or(6) as usize
}

/// How many decimals the readout of `value` shows by default: none for a whole number, otherwise
/// the least that write it, up to the decimals of `step`.
pub fn shown_decimals(value: f32, step: f32) -> usize {
    (0..=step_decimals(step))
        .find(|decimals| {
            let scale = 10f32.powi(*decimals as i32);
            ((value * scale).round() / scale - value).abs() < 1e-6 * scale.max(1.0)
        })
        .unwrap_or_else(|| step_decimals(step))
}

/// One `-` or `+` square button, dimmed and inert when `disabled`.
fn stepper_button(
    id: impl Into<ElementId>,
    icon: IconName,
    disabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> impl IntoElement {
    let palette = cx.palette();
    div()
        .id(id)
        .size(px(CONTROL_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_MD - 1.0))
        .border_1()
        .border_color(palette.border_strong)
        .bg(palette.raised)
        .child(Icon::new(icon).small())
        .when(disabled, |button| button.opacity(DISABLED_OPACITY))
        .when(!disabled, |button| {
            button
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover_over(palette.raised)))
                .active(|style| style.bg(palette.pressed_over(palette.raised)))
                .on_click(move |_, window, cx| on_click(window, cx))
        })
}

impl RenderOnce for NumberStepper {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let readout = self.readout();
        let mono_font = cx.theme().mono_font_family.clone();
        let at_min = self.value <= self.min;
        let at_max = self.value >= self.max;
        let button = |suffix: &'static str, icon, delta: f32, disabled| {
            let handler = self.on_change.clone();
            let (value, step, min, max) = (self.value, self.step, self.min, self.max);
            stepper_button(
                (self.id.clone(), suffix),
                icon,
                disabled,
                move |window, cx| {
                    if let Some(handler) = &handler {
                        handler(step_value(value, delta, step, min, max), window, cx);
                    }
                },
                cx,
            )
        };

        h_flex()
            .items_center()
            .gap(px(STEPPER_GAP))
            .child(button("minus", IconName::Minus, -self.step, at_min))
            .child(
                div()
                    .w(px(STEPPER_READOUT_WIDTH))
                    .text_center()
                    .font_family(mono_font)
                    .child(readout),
            )
            .child(button("plus", IconName::Plus, self.step, at_max))
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberStepper, shown_decimals, step_decimals, step_value};

    #[test]
    fn stepping_adds_the_step() {
        assert_eq!(step_value(13.0, 1.0, 1.0, 10.0, 20.0), 14.0);
        assert_eq!(step_value(13.0, -1.0, 1.0, 10.0, 20.0), 12.0);
    }

    #[test]
    fn stepping_stops_at_the_bounds() {
        assert_eq!(step_value(20.0, 1.0, 1.0, 10.0, 20.0), 20.0);
        assert_eq!(step_value(10.0, -1.0, 1.0, 10.0, 20.0), 10.0);
        // A value already outside the range comes back in.
        assert_eq!(step_value(30.0, 1.0, 1.0, 10.0, 20.0), 20.0);
    }

    #[test]
    fn stepping_rounds_float_noise_away() {
        assert_eq!(step_value(12.5, 0.5, 0.5, 10.0, 20.0), 13.0);
        assert_eq!(step_value(13.0, -0.5, 0.5, 10.0, 20.0), 12.5);
        assert_eq!(step_value(0.1, 0.2, 0.1, 0.0, 1.0), 0.3);
        assert_eq!(step_value(0.7, 0.1, 0.1, 0.0, 1.0), 0.8);
    }

    #[test]
    fn a_step_knows_its_decimals() {
        assert_eq!(step_decimals(1.0), 0);
        assert_eq!(step_decimals(5.0), 0);
        assert_eq!(step_decimals(0.5), 1);
        assert_eq!(step_decimals(0.1), 1);
        assert_eq!(step_decimals(0.25), 2);
    }

    #[test]
    fn the_readout_shows_only_the_decimals_it_needs() {
        assert_eq!(shown_decimals(13.0, 0.5), 0);
        assert_eq!(shown_decimals(12.5, 0.5), 1);
        assert_eq!(shown_decimals(12.25, 0.25), 2);
        assert_eq!(shown_decimals(12.5, 1.0), 0);
    }

    #[test]
    fn the_readout_follows_unit_decimals_and_format() {
        let stepper = NumberStepper::new("s", 12.5).step(0.5).unit("px");
        assert_eq!(stepper.readout(), "12.5 px");
        let stepper = NumberStepper::new("s", 13.0).step(0.5).unit("px");
        assert_eq!(stepper.readout(), "13 px");
        let stepper = NumberStepper::new("s", 13.0).decimals(2);
        assert_eq!(stepper.readout(), "13.00");
        let stepper = NumberStepper::new("s", 13.0)
            .unit("px")
            .format(|v| format!("<{v}>"));
        assert_eq!(stepper.readout(), "<13>");
    }
}
