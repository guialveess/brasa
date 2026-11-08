//! A theme card (System, Light or Dark): a mini window previewing the theme inside a recessed
//! frame, then a radio and the theme's name. Settings, Appearance and the welcome tab both show the
//! three cards side by side.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::metrics::{
    CHECKBOX_HALO_PADDING, RADIUS_LG, RADIUS_SM, THEME_PREVIEW_HEIGHT, THEME_PREVIEW_PADDING,
};
use crate::{PaletteExt, ThemeChoice};

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One theme card. It fills the width its parent gives it (`flex_1`).
#[derive(IntoElement)]
pub struct ThemeCard {
    id: ElementId,
    choice: ThemeChoice,
    selected: bool,
    on_click: Option<ClickHandler>,
}

impl ThemeCard {
    /// The card for `choice`, labelled with the theme's translated name.
    pub fn new(id: impl Into<ElementId>, choice: ThemeChoice) -> Self {
        Self {
            id: id.into(),
            choice,
            selected: false,
            on_click: None,
        }
    }

    /// Whether this is the current theme: accent frame and a filled radio.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Runs on a click anywhere on the card.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for ThemeCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let selected = self.selected;
        let label = match self.choice {
            ThemeChoice::System => t!("telaio.theme.system"),
            ThemeChoice::Light => t!("telaio.theme.light"),
            ThemeChoice::Dark => t!("telaio.theme.dark"),
        }
        .into_owned();
        // System's preview is half light, half dark; Light and Dark show their own single mode on
        // both sides (System is `[light, dark]`, Light is `[light, light]`, Dark is
        // `[dark, dark]`: the two palettes passed to each card's left and right half).
        let palettes = cx.palettes();
        let (left, right) = match self.choice {
            ThemeChoice::System => (palettes.light.clone(), palettes.dark.clone()),
            ThemeChoice::Light => (palettes.light.clone(), palettes.light.clone()),
            ThemeChoice::Dark => (palettes.dark.clone(), palettes.dark.clone()),
        };
        // Every card's left half draws its accent line in the dark palette's accent color,
        // regardless of which theme the card represents: a fixed swatch color, for visual
        // consistency across the three previews, not the current theme mode's own accent.
        let accent_line = palettes.dark.accent;

        let ring_color = if selected {
            palette.accent
        } else {
            palette.border_strong
        };

        // Hovering anywhere on the card lights its frame and the halo around its radio (Postino
        // #138). The selected card's frame is already `accent`, so only the halo changes there.
        let group = SharedString::from(match self.choice {
            ThemeChoice::System => "theme-card-system",
            ThemeChoice::Light => "theme-card-light",
            ThemeChoice::Dark => "theme-card-dark",
        });
        let (frame_hover, frame_pressed) = (palette.fg_subtle, palette.fg_muted);
        let (halo_hover, halo_pressed) = (palette.pressed, palette.border_strong);

        let mut radio = div().size(px(14.0)).rounded_full().border_color(ring_color);
        radio = if selected {
            radio.border_4()
        } else {
            // Brighter on hover, or the ring would melt into the halo around it.
            radio
                .border_1()
                .group_hover(group.clone(), move |style| style.border_color(frame_hover))
        };
        // The checkbox's halo: 3 px around the ring, kept out of the layout by a negative margin.
        let radio = div()
            .id(SharedString::from(format!("{group}-radio")))
            .flex_none()
            .p(px(CHECKBOX_HALO_PADDING))
            .m(px(-CHECKBOX_HALO_PADDING))
            .rounded_full()
            .group_hover(group.clone(), move |style| style.bg(halo_hover))
            .group_active(group.clone(), move |style| style.bg(halo_pressed))
            .child(radio);

        // A real border, not a `box-shadow` ring (an `N px` spread shadow): a spread, zero-blur,
        // zero-offset shadow on a `rounded()` + `overflow_hidden()` box rendered as a filled halo
        // covering the box's own content here, not a thin outline as CSS would, so the border
        // achieves the same "ring" look without that. The frame is a recessed well (`surface`,
        // `accent_subtle` when selected) around the mini window, which keeps its own rounded
        // corners and border. The selected frame's thicker border eats one pixel of padding, so
        // the mini window stays put when the selection changes.
        let mut preview_box = div()
            .id(SharedString::from(format!("{group}-frame")))
            .flex()
            .h(px(THEME_PREVIEW_HEIGHT))
            .rounded(px(RADIUS_LG))
            .border_color(ring_color)
            .bg(if selected {
                palette.accent_subtle
            } else {
                palette.surface
            });
        preview_box = if selected {
            preview_box.border_2().p(px(THEME_PREVIEW_PADDING - 1.0))
        } else {
            preview_box
                .border_1()
                .p(px(THEME_PREVIEW_PADDING))
                .group_hover(group.clone(), move |style| style.border_color(frame_hover))
                .group_active(group.clone(), move |style| {
                    style.border_color(frame_pressed)
                })
        };
        // gpui clips children to a rectangle, not to rounded corners, so each half rounds its own
        // outer corners inside the mini window's 1 px border.
        let mini_radius = px(RADIUS_SM - 1.0);
        let mini_window = h_flex()
            .flex_1()
            .items_stretch()
            .rounded(px(RADIUS_SM))
            .border_1()
            .border_color(palette.border_strong)
            .child(
                v_flex()
                    .flex_1()
                    .rounded_l(mini_radius)
                    .bg(left.surface)
                    .p(px(8.0))
                    .gap(px(5.0))
                    .child(preview_line(0.7, left.border_strong))
                    .child(preview_line(0.5, accent_line))
                    .child(preview_line(0.6, left.border_strong)),
            )
            .child(
                v_flex()
                    .flex_grow(2.0)
                    .flex_shrink(1.0)
                    .flex_basis(relative(0.0))
                    .rounded_r(mini_radius)
                    .bg(right.bg)
                    .p(px(8.0))
                    .gap(px(6.0))
                    .child(
                        div()
                            .h(px(12.0))
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(right.border_strong),
                    )
                    .child(preview_line(0.8, right.border_strong))
                    .child(preview_line(0.55, right.border_strong)),
            );

        let on_click = self.on_click;
        v_flex()
            .id(self.id)
            .flex_1()
            .min_w_0()
            .gap(px(8.0))
            .group(group)
            .cursor_pointer()
            .child(preview_box.child(mini_window))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(radio)
                    .child(label),
            )
            .when_some(on_click, |card, on_click| {
                card.on_click(move |event, window, cx| on_click(event, window, cx))
            })
    }
}

/// A mini preview "line": a thin, colored, rounded bar `width_fraction` of its parent's width.
fn preview_line(width_fraction: f32, color: Hsla) -> impl IntoElement {
    div()
        .h(px(5.0))
        .w(relative(width_fraction))
        .rounded(px(3.0))
        .bg(color)
}
