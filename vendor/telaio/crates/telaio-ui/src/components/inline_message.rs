//! [`InlineMessage`]: the warning/danger/success/info strip used instead of a full-width error
//! banner (see `docs/design-system.md`).
//!
//! Custom element: a colored icon plus text plus an optional right-aligned action link, on a
//! `*-subtle` background, is a plain `div` row colored from [`crate::Palette`].

use std::rc::Rc;

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::metrics::RADIUS_MD;

/// Which of the four kinds an [`InlineMessage`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineMessageKind {
    /// For example an unknown variable.
    Warning,
    /// For example a send failure.
    Danger,
    /// For example "N of N tests passed".
    Success,
    /// A neutral, informational note.
    Info,
}

impl InlineMessageKind {
    /// The icon this kind shows, for each message kind.
    fn icon(self) -> Icon {
        match self {
            InlineMessageKind::Warning => Icon::new(IconName::TriangleAlert),
            InlineMessageKind::Danger => Icon::new(IconName::CircleX),
            InlineMessageKind::Success => Icon::new(IconName::Check),
            InlineMessageKind::Info => Icon::new(gpui_kit::assets::IconName::Info),
        }
    }
}

/// A colored strip: icon, text, and an optional action link on the right.
#[derive(IntoElement)]
pub struct InlineMessage {
    kind: InlineMessageKind,
    text: SharedString,
    /// An optional trailing fragment shown in the mono font, for example a variable name in
    /// `"Unknown variable "` + `"username"`.
    mono_suffix: Option<SharedString>,
    action: Option<(SharedString, ActionHandler)>,
}

/// An action link's click handler, factored out because clippy's `type_complexity` flags the
/// inline form.
type ActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

impl InlineMessage {
    /// A message of `kind` showing `text`.
    pub fn new(kind: InlineMessageKind, text: impl Into<SharedString>) -> Self {
        Self {
            kind,
            text: text.into(),
            mono_suffix: None,
            action: None,
        }
    }

    /// Appends `suffix` in the mono font right after the text (for example a variable name).
    pub fn mono_suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.mono_suffix = Some(suffix.into());
        self
    }

    /// Adds a right-aligned action link, for example "Define" on an unknown-variable warning.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), Rc::new(handler)));
        self
    }
}

impl RenderOnce for InlineMessage {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let (icon_color, bg) = match self.kind {
            InlineMessageKind::Warning => (palette.warning, palette.warning_subtle),
            InlineMessageKind::Danger => (palette.danger, palette.danger_subtle),
            InlineMessageKind::Success => (palette.success, palette.success_subtle),
            InlineMessageKind::Info => (palette.info, palette.info_subtle),
        };

        let mut row = h_flex()
            .items_center()
            .gap(px(10.0))
            .px_3()
            .py_2()
            .rounded(px(RADIUS_MD))
            .bg(bg)
            .text_color(palette.fg)
            .child(self.kind.icon().small().text_color(icon_color));

        // `min_w_0` on the line and the text, so a long message wraps inside the strip instead
        // of running past its right edge.
        let mut text_line = h_flex()
            .flex_1()
            .min_w_0()
            .gap(px(4.0))
            .child(div().min_w_0().child(self.text));
        if let Some(suffix) = self.mono_suffix {
            text_line = text_line.child(div().font_family(mono_font).child(suffix));
        }
        row = row.child(text_line);

        if let Some((label, handler)) = self.action {
            row = row.child(
                div()
                    .id("inline-message-action")
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.accent_text)
                    .hover(|style| style.text_decoration_1())
                    .active(|style| style.opacity(0.7))
                    .on_click(move |_, window, cx| handler(window, cx))
                    .child(label),
            );
        }

        row
    }
}
