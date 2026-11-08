//! The `BRASA_OPEN=components` debug view: every component in this module, in every state, to
//! check against `docs/design-system.md`, to screenshot and compare. Not
//! reachable from any menu.

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use brasa_core::{KeyValue, Method, variable_spans};

use super::{
    Card, DangerButton, DocumentTab, DocumentTabs, EnvMenuItem, EnvPill, GhostButton, IconButton,
    InlineMessage, InlineMessageKind, KeyValueRow, KeyValueTable, MethodBadge, PrimaryButton,
    SecondaryButton, SectionLabel, SegmentedControl, SegmentedItem, StatusState, ThemeCard,
    UnderlineTabItem, UnderlineTabs, UrlBar, VariableChip, select_label_row, select_trigger,
    status_badge, switch_halo,
};
use crate::state::settings::ThemeChoice;
use crate::state::variable_hint::VariableContext;
use crate::theme::PaletteExt;
use crate::theme::metrics::{CONTROL_HEIGHT, RADIUS_MD};
use crate::views::root::AppView;

impl AppView {
    /// Renders the whole gallery: one section per kind of component, plus the
    /// extra components (`UrlBar`, `EnvPill`, `SectionLabel`, `Card`,
    /// `DocumentTabs`).
    pub(crate) fn render_components_gallery(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();

        div()
            .id("components-gallery")
            .size_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .bg(palette.bg)
            .p_6()
            .child(
                v_flex()
                    .gap_6()
                    .max_w(px(720.0))
                    .child(section("Buttons", render_buttons()))
                    .child(section("Inputs", render_inputs(&palette, mono_font)))
                    .child(section("Tabs", render_tabs()))
                    .child(section("Open tabs", render_document_tabs()))
                    .child(section("Methods", render_methods()))
                    .child(section("Status", render_status()))
                    .child(section("Controls", render_controls(&palette)))
                    .child(section("Select", render_select(&palette, cx)))
                    .child(section("Menu", render_menu(&palette)))
                    .child(section("Key-value table", render_key_value_table()))
                    .child(section("Inline messages", render_inline_messages()))
                    .child(section("URL bar", self.render_url_bar_demo(window, cx)))
                    .child(section("Env pill", render_env_pill()))
                    .child(section("Card", render_card()))
                    .child(section("Theme cards", render_theme_cards())),
            )
            .into_any_element()
    }

    /// The "URL bar" section: one `UrlBar` with a known (`{{baseUrl}}`) and an unknown
    /// (`{{missing}}`) chip, backed by [`AppView::gallery_url_input`] (created once by
    /// `apply_debug_open`).
    fn render_url_bar_demo(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> AnyElement {
        let Some(input_state) = self.gallery_url_input.clone() else {
            return div().into_any_element();
        };
        let text = "{{baseUrl}}/users/{{missing}}";
        let spans = variable_spans(text);
        let environment = [KeyValue {
            key: "baseUrl".to_string(),
            value: "https://api.example.com".to_string(),
            enabled: true,
        }];
        let context =
            VariableContext::new(Some("demo".to_string()), &environment, &[], HashSet::new());

        UrlBar::new(Method::Post, text, spans, Rc::new(context), input_state).into_any_element()
    }
}

/// Wraps `content` under a [`SectionLabel`] titled `title`, with a 10 px
/// gap.
fn section(title: &str, content: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_2p5()
        .child(SectionLabel::new(title))
        .child(content)
}

/// The "Buttons" swatch: Primary (with a key hint), Secondary, Ghost, Danger, a disabled
/// Primary, and the small/large `IconButton`.
fn render_buttons() -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .items_center()
        .child(PrimaryButton::new("gallery-primary", "Send").key_hint("Ctrl+\u{21b5}"))
        .child(SecondaryButton::new("gallery-secondary", "Secondary"))
        .child(
            GhostButton::new("gallery-ghost", "Ghost").icon(gpui_kit::assets::IconName::Download),
        )
        .child(DangerButton::new("gallery-danger", "Delete"))
        .child(PrimaryButton::new("gallery-disabled", "Disabled").disabled(true))
        .child(IconButton::new("gallery-icon-btn-sm", IconName::Plus).tooltip("New"))
        .child(IconButton::new("gallery-icon-btn-lg", IconName::Ellipsis).large())
}

/// The "Inputs" swatch: a standalone defined and undefined [`VariableChip`], and a Pretty/Raw/
/// Preview [`SegmentedControl`].
fn render_inputs(palette: &crate::theme::Palette, mono_font: SharedString) -> impl IntoElement {
    let cell = |border: Hsla| {
        h_flex()
            .flex_1()
            .h(px(CONTROL_HEIGHT))
            .items_center()
            .rounded(px(RADIUS_MD))
            .border_1()
            .border_color(border)
            .bg(palette.raised)
            .px_2p5()
    };

    let placeholder_cell = cell(palette.border_strong)
        .text_color(palette.fg_subtle)
        .child("Request name");

    let focused_cell = cell(palette.accent)
        .shadow(vec![
            BoxShadow::new(px(0.0), px(0.0), palette.accent_subtle).spread_radius(px(3.0)),
        ])
        .child("Focused")
        .child(div().w(px(1.0)).h(px(14.0)).ml(px(1.0)).bg(palette.fg));

    let defined_cell = cell(palette.border_strong)
        .font_family(mono_font.clone())
        .text_size(px(12.5))
        .gap(px(1.0))
        .child(VariableChip::new("{{baseUrl}}", true))
        .child("/users");

    let undefined_cell = cell(palette.danger)
        .font_family(mono_font.clone())
        .text_size(px(12.5))
        .gap(px(1.0))
        .child(VariableChip::new("{{apiUrl}}", false))
        .child("/users");

    let select_cell = cell(palette.border_strong)
        .justify_between()
        .child("JSON")
        .child(
            Icon::new(IconName::ChevronsUpDown)
                .small()
                .text_color(palette.fg_subtle),
        );

    let segmented_cell = div().flex_1().child(
        SegmentedControl::new("gallery-segmented")
            .item(SegmentedItem::new("Pretty").selected(true))
            .item(SegmentedItem::new("Raw"))
            .item(SegmentedItem::new("Preview")),
    );

    v_flex()
        .gap(px(10.0))
        .child(
            h_flex()
                .gap(px(10.0))
                .child(placeholder_cell)
                .child(focused_cell),
        )
        .child(
            h_flex()
                .gap(px(10.0))
                .child(defined_cell)
                .child(undefined_cell),
        )
        .child(
            h_flex()
                .gap(px(10.0))
                .child(select_cell)
                .child(segmented_cell),
        )
}

/// The "Tabs" swatch: request-editor style [`UnderlineTabs`], Params/Headers(2)/Body(active)/
/// Pre-request.
fn render_tabs() -> impl IntoElement {
    UnderlineTabs::new("gallery-underline-tabs")
        .item(UnderlineTabItem::new("Params"))
        .item(UnderlineTabItem::new("Headers").count("2"))
        .item(UnderlineTabItem::new("Body").selected(true))
        .item(UnderlineTabItem::new("Pre-request"))
}

/// The open-tabs strip swatch: a dirty selected POST tab, a clean GET tab, and a load test tab.
fn render_document_tabs() -> impl IntoElement {
    DocumentTabs::new("gallery-document-tabs")
        .item(
            DocumentTab::new("login")
                .prefix(|| MethodBadge::inline(Method::Post))
                .dirty(true)
                .selected(true),
        )
        .item(DocumentTab::new("list").prefix(|| MethodBadge::inline(Method::Get)))
        .item(DocumentTab::new("Load test \u{b7} users").icon(gpui_kit::assets::IconName::Gauge))
}

/// The "Methods" swatch: a [`MethodBadge`] pill for every standard method plus HEAD/OPTIONS.
fn render_methods() -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(MethodBadge::pill(Method::Get))
        .child(MethodBadge::pill(Method::Post))
        .child(MethodBadge::pill(Method::Put))
        .child(MethodBadge::pill(Method::Patch))
        .child(MethodBadge::pill(Method::Delete))
        .child(MethodBadge::pill(Method::Head))
        .child(MethodBadge::pill(Method::Options))
}

/// The "Status" swatch: every status color plus "Not sent" and "Sending...".
fn render_status() -> impl IntoElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(status_badge(&StatusState::Code(200)))
        .child(status_badge(&StatusState::Code(301)))
        .child(status_badge(&StatusState::Code(404)))
        .child(status_badge(&StatusState::Code(500)))
        .child(status_badge(&StatusState::NotSent))
        .child(status_badge(&StatusState::Sending))
}

/// The "Controls" swatch: checked/unchecked [`Checkbox`], a checked [`Radio`], on/off
/// [`Switch`], and a small folder/request tree mock (reusing [`MethodBadge`]'s `label` variant).
fn render_controls(palette: &crate::theme::Palette) -> impl IntoElement {
    v_flex()
        .gap_3()
        .child(
            h_flex()
                .gap_4()
                .items_center()
                .child(Checkbox::new("gallery-checkbox-on").checked(true))
                .child(Checkbox::new("gallery-checkbox-off").checked(false))
                .child(Radio::new("gallery-radio").checked(true))
                .child(switch_halo(
                    "gallery-switch-on-halo",
                    Switch::new("gallery-switch-on").checked(true),
                    palette,
                ))
                .child(switch_halo(
                    "gallery-switch-off-halo",
                    Switch::new("gallery-switch-off").checked(false),
                    palette,
                )),
        )
        .child(
            v_flex()
                .gap(px(1.0))
                .p_1()
                .child(
                    h_flex()
                        .h(px(26.0))
                        .items_center()
                        .gap_2()
                        .child(Icon::new(IconName::ChevronDown).small())
                        .child("auth"),
                )
                .child(
                    h_flex()
                        .h(px(26.0))
                        .items_center()
                        .gap_2()
                        .pl(px(27.0))
                        .child(MethodBadge::label(Method::Post))
                        .child("login"),
                )
                .child(
                    h_flex()
                        .h(px(26.0))
                        .items_center()
                        .gap_2()
                        .pl(px(27.0))
                        .child(MethodBadge::label(Method::Post))
                        .child("refresh"),
                ),
        )
}

/// The "Select" swatch: a `select_trigger` with its label row, as in the Settings selects.
fn render_select(palette: &crate::theme::Palette, cx: &App) -> impl IntoElement {
    h_flex().child(
        select_trigger("gallery-select", palette, cx)
            .w(px(220.0))
            .text_color(palette.fg)
            .child(select_label_row(div().child("staging"), palette)),
    )
}

/// The "Menu" swatch: a static reproduction of the environment menu's rows (a checked "No
/// environment", a hovered "local", and "production" with a shortcut hint).
/// `EnvPill`'s own dropdown uses the same layout, but only ever renders it
/// while actually open, which a static screenshot cannot show.
fn render_menu(palette: &crate::theme::Palette) -> impl IntoElement {
    v_flex()
        .p_1()
        .rounded(px(10.0))
        .bg(palette.overlay)
        .shadow(palette.shadow.clone())
        .child(
            h_flex()
                .h(px(28.0))
                .items_center()
                .gap_2()
                .px_2()
                .child(
                    Icon::new(IconName::Check)
                        .small()
                        .text_color(palette.accent_text),
                )
                .child("No environment"),
        )
        .child(div().h(px(1.0)).my_1().bg(palette.border))
        .child(
            h_flex()
                .h(px(28.0))
                .items_center()
                .gap_2()
                .px_2()
                .rounded(px(6.0))
                .bg(palette.hover)
                .child(div().size(px(7.0)).rounded_full().bg(palette.success))
                .child("local"),
        )
        .child(
            h_flex()
                .h(px(28.0))
                .items_center()
                .gap_2()
                .px_2()
                .child(div().size(px(7.0)).rounded_full().bg(palette.danger))
                .child(div().flex_1().child("production"))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(palette.fg_subtle)
                        .child("Ctrl+3"),
                ),
        )
}

/// The "Key-value table" swatch: an enabled `page=1` row and a disabled, struck-through
/// `limit=50` row.
fn render_key_value_table() -> impl IntoElement {
    KeyValueTable::new("gallery-kv-table")
        .row(KeyValueRow::new("page", "1"))
        .row(KeyValueRow::new("limit", "50").enabled(false))
}

/// The "Inline messages" swatch: warning (with a "Define" action), danger, and success.
fn render_inline_messages() -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(
            InlineMessage::new(InlineMessageKind::Warning, "Unknown variable")
                .mono_suffix("username")
                .action("Define", |_, _| {}),
        )
        .child(InlineMessage::new(
            InlineMessageKind::Danger,
            "Sending failed: invalid URI character",
        ))
        .child(InlineMessage::new(
            InlineMessageKind::Success,
            "3 of 3 tests passed",
        ))
        .child(InlineMessage::new(
            InlineMessageKind::Info,
            "Runs on this machine. Results are saved to .brasa/runs/",
        ))
}

/// The title bar's `EnvPill`, closed, showing "local" as the active environment.
fn render_env_pill() -> impl IntoElement {
    // `EnvPill` wraps gpui-kit's `Popover`/`dropdown_menu`, whose trigger wrapper stretches to
    // fill a column flex parent's cross axis regardless of the trigger's own width styling (no
    // public style hook reaches that wrapper); it never stretches in the title bar's own row
    // layout, so this `max_w` is only to keep the gallery's demo pill compact.
    div().max_w(px(220.0)).child(EnvPill::new(
        "gallery-env-pill",
        vec![
            EnvMenuItem::none(),
            EnvMenuItem::named("local").active(true),
            EnvMenuItem::named("production").shortcut("Ctrl+3"),
        ],
    ))
}

/// The three [`ThemeCard`]s, Dark selected, as in Settings and the welcome tab. Clicks do
/// nothing here.
fn render_theme_cards() -> impl IntoElement {
    h_flex().max_w(px(640.0)).gap(px(12.0)).children(
        [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark].map(|choice| {
            ThemeCard::new(
                SharedString::from(format!("gallery-theme-{choice:?}")),
                choice.into(),
            )
            .selected(choice == ThemeChoice::Dark)
        }),
    )
}

/// A [`Card`] with a couple of KPI-like children, close enough to a dashboard
/// card to show the container's own look (the real KPI strip is in the load test dashboard).
fn render_card() -> impl IntoElement {
    Card::new().child(SectionLabel::new("Requests/s")).child(
        div()
            .text_size(px(22.0))
            .font_weight(FontWeight::SEMIBOLD)
            .child("128"),
    )
}
