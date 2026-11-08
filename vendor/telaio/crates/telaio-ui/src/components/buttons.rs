//! [`PrimaryButton`], [`SecondaryButton`], [`GhostButton`] and [`DangerButton`]: the four button
//! kinds.
//!
//! `PrimaryButton`/`SecondaryButton`/`DangerButton` wrap gpui-kit's [`Button`] on its matching
//! built-in variant (`.primary()`/`.secondary()`/`.danger()`, plus `SecondaryButton`'s own
//! `border_strong`-weight border, drawn by hand since no built-in variant auto-draws one outside
//! `Default`/outline): their normal, hover and pressed fills all come from the `button.primary.*`
//! /`button.secondary.*`/`button.danger.*` theme JSON keys
//! (`palette::theme_colors_and_highlight`), set straight from [`crate::Palette`].
//! An earlier version of this file started every kind from `.ghost()` instead, restyling through
//! `Styled`, to dodge `Danger`/`Custom`'s background formula (`ButtonVariant::bg_color`,
//! `gpui-component-0.6.6/src/button/button.rs`, `Self::Custom(colors) =>
//! colors.color.mix_oklab(cx.theme().transparent, 0.2)`): that formula only actually applies to
//! `Custom`, not to `Danger` (`Self::Danger => cx.theme().tokens.button_danger.into()`, no
//! mixing), so it was never a reason to avoid `Danger` specifically, and it never applied to
//! `Primary`/`Secondary` either. The real problem it hid (Postino #17) was that `.ghost()`'s own
//! hover/pressed are hardcoded to an accent-tinted formula (`ButtonVariant::hovered`/`active`)
//! regardless of a `Styled` background override, which made a solid-filled `.bg(accent)` button
//! flip to an unrelated muted color on hover, reading as disabled rather than highlighted.
//!
//! `GhostButton` still starts transparent, so `Custom`'s 20%-toward-transparent mix is harmless
//! there (mixing an already-transparent color further toward transparent is a no-op): it uses
//! `.custom(ButtonCustomVariant::new(cx).color(transparent).hover(palette.hover)
//! .active(palette.pressed))` instead, for the same `hover`/`pressed` tokens every hand-rolled
//! row and icon button already uses (`IconButton`, tree rows, ...), rather than `Ghost`'s own accent-tinted hover.

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::{CONTROL_HEIGHT, RADIUS_MD};

/// Opacity applied to a disabled button (45%).
pub(crate) const DISABLED_OPACITY: f32 = 0.45;

/// Wraps a solid `button`: used to add a hover glow, now plain. `Button` already draws its own
/// hover and pressed fills (`button.primary.hover.background`, from the theme), so a primary or
/// danger button needs nothing more here; kept as a wrapper so `full_width` still has somewhere
/// to go and callers do not change.
fn with_hover_glow(button: Button, _color: Hsla, _disabled: bool, full_width: bool) -> Div {
    div()
        .flex_none()
        .rounded(px(RADIUS_MD))
        .when(full_width, |this| this.w_full())
        .child(button)
}

/// Builder state shared by the four button kinds in this file. Not public: each kind exposes its
/// own constructor and forwards its builder methods to this.
struct ButtonSpec {
    id: ElementId,
    label: SharedString,
    key_hint: Option<SharedString>,
    icon: Option<Icon>,
    height: f32,
    min_width: Option<f32>,
    full_width: bool,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

/// A boxed click handler, factored out because clippy's `type_complexity` flags the inline form.
type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

impl ButtonSpec {
    fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            key_hint: None,
            icon: None,
            height: CONTROL_HEIGHT,
            min_width: None,
            full_width: false,
            disabled: false,
            on_click: None,
        }
    }

    /// Renders `self` as a gpui-kit [`Button`] with `apply` picking the variant (and, through it,
    /// every color: this no longer takes `bg`/`fg` params, see the module doc comment).
    fn render(self, cx: &mut App, apply: impl FnOnce(Button) -> Button) -> Button {
        let key_hint_font = cx.theme().mono_font_family.clone();
        let mut button = apply(Button::new(self.id))
            .h(px(self.height))
            .when_some(self.min_width, |button, width| button.min_w(px(width)))
            .when(self.full_width, |button| button.w_full())
            .rounded(px(RADIUS_MD))
            .font_weight(FontWeight::MEDIUM)
            .disabled(self.disabled)
            .when(self.disabled, |button| button.opacity(DISABLED_OPACITY))
            // `Button`'s own root only turns the pointer into a hand for its link and text
            // variants (`gpui-component-0.8.0/src/button/button.rs`); every solid kind here
            // (primary, secondary, danger, ghost) kept the plain arrow cursor otherwise.
            .when(!self.disabled, |button| button.cursor_pointer())
            .when_some(self.icon, |button, icon| button.icon(icon))
            .label(self.label);
        if let Some(hint) = self.key_hint {
            button = button.child(
                div()
                    // The mono hint is smaller than the label, so centering their boxes leaves
                    // its baseline about a pixel above the label's; nudge it down to match.
                    .relative()
                    .top(px(1.0))
                    .font_family(key_hint_font)
                    .text_size(px(10.5))
                    .opacity(0.75)
                    .child(hint),
            );
        }
        if let Some(handler) = self.on_click {
            button = button.on_click(move |event, window, cx| handler(event, window, cx));
        }
        button
    }
}

/// The Send button and other primary actions: `accent` background, `accent_fg` text.
#[derive(IntoElement)]
pub struct PrimaryButton {
    spec: ButtonSpec,
}

impl PrimaryButton {
    /// A primary button labeled `label`.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            spec: ButtonSpec::new(id, label),
        }
    }

    /// Sets a trailing mono key hint (for example `"Ctrl+\u{21b5}"`).
    pub fn key_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.spec.key_hint = Some(hint.into());
        self
    }

    /// Sets the button's height (defaults to [`CONTROL_HEIGHT`]).
    pub fn height(mut self, height: f32) -> Self {
        self.spec.height = height;
        self
    }

    /// Sets the button's minimum width in px.
    pub fn min_width(mut self, width: f32) -> Self {
        self.spec.min_width = Some(width);
        self
    }

    /// Sets a leading icon.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.spec.icon = Some(icon.into());
        self
    }

    /// Makes the button fill the width of its parent.
    pub fn full_width(mut self) -> Self {
        self.spec.full_width = true;
        self
    }

    /// Disables the button (45% opacity, no click).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.spec.disabled = disabled;
        self
    }

    /// Sets the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.spec.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for PrimaryButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (color, disabled, full_width) = (
            cx.palette().accent,
            self.spec.disabled,
            self.spec.full_width,
        );
        with_hover_glow(
            self.spec.render(cx, |button| button.primary()),
            color,
            disabled,
            full_width,
        )
    }
}

/// A secondary action: `raised` background, `border_strong` border, default text color.
#[derive(IntoElement)]
pub struct SecondaryButton {
    spec: ButtonSpec,
}

impl SecondaryButton {
    /// A secondary button labeled `label`.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            spec: ButtonSpec::new(id, label),
        }
    }

    /// Sets a leading icon.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.spec.icon = Some(icon.into());
        self
    }

    /// Sets the button's height (defaults to [`CONTROL_HEIGHT`]).
    pub fn height(mut self, height: f32) -> Self {
        self.spec.height = height;
        self
    }

    /// Disables the button (45% opacity, no click).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.spec.disabled = disabled;
        self
    }

    /// Sets the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.spec.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for SecondaryButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        self.spec
            .render(cx, |button| button.secondary())
            .border_1()
            .border_color(palette.border_strong)
    }
}

/// A low-emphasis action: transparent background, `fg_muted` text, optional icon (for example
/// the "Format"/"Import" buttons).
#[derive(IntoElement)]
pub struct GhostButton {
    spec: ButtonSpec,
}

impl GhostButton {
    /// A ghost button labeled `label`.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            spec: ButtonSpec::new(id, label),
        }
    }

    /// Sets a leading icon.
    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.spec.icon = Some(icon.into());
        self
    }

    /// Sets the button's height (defaults to [`CONTROL_HEIGHT`]).
    pub fn height(mut self, height: f32) -> Self {
        self.spec.height = height;
        self
    }

    /// Disables the button (45% opacity, no click).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.spec.disabled = disabled;
        self
    }

    /// Sets the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.spec.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for GhostButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let custom = ButtonCustomVariant::new(cx)
            .color(palette.bg.opacity(0.0))
            .foreground(palette.fg_muted)
            .hover(palette.hover)
            .active(palette.pressed);
        self.spec.render(cx, move |button| button.custom(custom))
    }
}

/// Neutral hover and pressed fills for a gpui-kit [`Button`] built by hand (icon buttons,
/// pickers that open a dropdown menu), the same `hover`/`pressed` tokens as [`GhostButton`] and
/// every hand-rolled row (Postino #138). `.ghost()` is not used for these: its hover is
/// gpui-component's accent tinted formula, and it swaps the text color
/// (`ButtonVariant::hovered`).
pub trait NeutralButtonExt: Sized {
    /// Transparent at rest, `foreground` text, `hover` and `pressed` fills.
    fn neutral_ghost(self, cx: &App, foreground: Hsla) -> Self;

    /// A bordered control with the opaque `rest` fill (a picker box): `hover` and `pressed`
    /// composited over `rest`, `foreground` text, a `border` border at rest and `border_strong`
    /// while hovered or pressed, like a text input. The caller sets the border width. `Custom`
    /// paints its border in its `color` on hover and press (`ButtonVariant::border_color`), so
    /// `color` is `border_strong` here; the resting border and fill set on the instance win over
    /// the ones `Custom` derives from it.
    fn neutral_bordered(self, cx: &App, rest: Hsla, border: Hsla, foreground: Hsla) -> Self;
}

impl NeutralButtonExt for Button {
    fn neutral_ghost(self, cx: &App, foreground: Hsla) -> Self {
        let palette = cx.palette();
        let transparent = palette.bg.opacity(0.0);
        self.custom(
            ButtonCustomVariant::new(cx)
                .color(transparent)
                .foreground(foreground)
                .hover(palette.hover)
                .active(palette.pressed),
        )
        .bg(transparent)
        // `Button`'s own root only turns the pointer into a hand for its link and text variants,
        // not for a `custom` one like this: every icon button, the workspace switcher and the
        // method picker kept the plain arrow cursor otherwise.
        .cursor_pointer()
    }

    fn neutral_bordered(self, cx: &App, rest: Hsla, border: Hsla, foreground: Hsla) -> Self {
        let palette = cx.palette();
        self.custom(
            ButtonCustomVariant::new(cx)
                .color(palette.border_strong)
                .foreground(foreground)
                .hover(palette.hover_over(rest))
                .active(palette.pressed_over(rest)),
        )
        .bg(rest)
        .border_color(border)
        .cursor_pointer()
    }
}

/// A destructive action: `danger` background, white text.
#[derive(IntoElement)]
pub struct DangerButton {
    spec: ButtonSpec,
}

impl DangerButton {
    /// A danger button labeled `label`.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            spec: ButtonSpec::new(id, label),
        }
    }

    /// Sets the button's height (defaults to [`CONTROL_HEIGHT`]).
    pub fn height(mut self, height: f32) -> Self {
        self.spec.height = height;
        self
    }

    /// Disables the button (45% opacity, no click).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.spec.disabled = disabled;
        self
    }

    /// Sets the click handler.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.spec.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for DangerButton {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (color, disabled, full_width) = (
            cx.palette().danger,
            self.spec.disabled,
            self.spec.full_width,
        );
        with_hover_glow(
            self.spec.render(cx, |button| button.danger()),
            color,
            disabled,
            full_width,
        )
    }
}
