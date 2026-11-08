//! The welcome tab, redesigned from scratch around one job: get the person into a workspace.
//! A quiet inline brand mark, a single full-width primary action ("New workspace"), the two
//! quieter paths (open a folder, import) tucked into one line under it, the recent workspaces,
//! and a footer with the links, a compact theme toggle and the "show at startup" checkbox. No
//! big stacked hero and no theme preview cards competing with the one action: those earlier
//! choices read as "the same screen, just resized" and are gone.
//!
//! Content fades and rises into place once, staggered, when the tab first paints
//! ([`fade_rise`]); it settles and repaints nothing afterward, so it never breaks the "an idle
//! window does no work" rule.
//!
//! The recent workspaces come from the tab's own [`WelcomeTab`], read when it opens, so render
//! touches no file.

use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_ui::components::{MODIFIER_KEY, labelled_checkbox, open_hint};

use crate::state::settings::ThemeChoice;
use crate::state::welcome::{QUICK_START_URL, RecentWorkspace, WelcomeTab};
use crate::theme::metrics::{
    RADIUS_MD, RADIUS_SM, WELCOME_COLUMN_WIDTH, WELCOME_COLUMN_WIDTH_NARROW, WELCOME_LOGO_SIZE,
    WELCOME_MIN_WIDTH, WELCOME_RECENT_NAME_WIDTH, WELCOME_ROW_HEIGHT, WELCOME_TOP_PADDING,
    WELCOME_TOP_PADDING_NARROW,
};
use crate::theme::{Palette, PaletteExt, BrasaColors};

use super::components::{PrimaryButton, SecondaryButton, SectionLabel};
use super::new_workspace;
use super::root::AppView;
use super::sidebar;

/// A click handler of an action, a link or the checkbox.
type ClickHandler = Box<dyn Fn(&mut Window, &mut App)>;

/// Length of the one-shot entrance animation. Chosen short enough to feel instant, not watched.
const ENTRANCE_MS: u64 = 380;
/// How far a block rises into place, in px, fading in over the same span.
const ENTRANCE_RISE: f32 = 8.0;

impl AppView {
    /// Opens the welcome tab, or brings it to the front with its recent workspaces read again.
    /// Also the command palette's "Show welcome" action.
    pub(crate) fn open_welcome_tab(&mut self, cx: &mut Context<Self>) {
        let welcome = WelcomeTab::load(&self.recent_workspaces, dirs::home_dir().as_deref());
        self.state.tabs.open_welcome(welcome);
        log::debug!("opened the welcome tab");
        cx.notify();
    }

    /// Renders the active welcome tab.
    pub(crate) fn render_welcome(&self, welcome: &WelcomeTab, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let weak = cx.weak_entity();
        // Next to the sidebar (shown while a workspace is open or being opened, see
        // `render_body`) the column is a little narrower and starts a little higher.
        let (column_width, top_padding) =
            if self.state.workspace.is_some() || self.opening.is_some() {
                (WELCOME_COLUMN_WIDTH_NARROW, WELCOME_TOP_PADDING_NARROW)
            } else {
                (WELCOME_COLUMN_WIDTH, WELCOME_TOP_PADDING)
            };

        let column = v_flex()
            .w_full()
            .max_w(px(column_width))
            // The design's line height; gpui's default is taller and spreads the blocks apart.
            .line_height(relative(1.25))
            .pt(px(top_padding))
            .px(px(40.0))
            .pb(px(32.0))
            .child(fade_rise("welcome-in-hero", 0.0, render_hero(&palette)))
            .child(fade_rise(
                "welcome-in-action",
                0.16,
                render_primary_action(weak.clone(), &palette),
            ))
            .children((!welcome.recents.is_empty()).then(|| {
                fade_rise(
                    "welcome-in-recent",
                    0.3,
                    div().mt(px(32.0)).child(render_recents(
                        weak.clone(),
                        &welcome.recents,
                        &palette,
                        &mono_font,
                    )),
                )
            }))
            .child(div().flex_1().min_h(px(24.0)))
            .child(fade_rise(
                "welcome-in-footer",
                0.42,
                render_footer(
                    weak,
                    self.state.settings.theme,
                    self.state.settings.show_welcome,
                    &palette,
                    &mono_font,
                ),
            ));

        let area = div()
            .id("welcome-tab")
            .size_full()
            .bg(palette.bg)
            // Both ways: below its least width the column scrolls sideways instead of being cut.
            .overflow_scroll()
            .track_scroll(&self.welcome_scroll)
            // Stretched, not centered: the column starts at its top padding and its footer
            // sits at the bottom of the tab.
            .child(
                h_flex()
                    .w_full()
                    .min_w(px(WELCOME_MIN_WIDTH))
                    .min_h_full()
                    .items_stretch()
                    .justify_center()
                    .child(column),
            );
        // Not `overflow_scrollbar`: its wrapper stretches the content to the area's height, so
        // the tab would no longer scroll down.
        div()
            .relative()
            .size_full()
            .child(area)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::new(&self.welcome_scroll)),
            )
            .into_any_element()
    }

    /// The main area with no workspace and no tab open, once the welcome tab is closed: a line
    /// and the two ways to get a workspace.
    pub(crate) fn render_no_workspace(
        &self,
        weak: WeakEntity<Self>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let new_weak = weak.clone();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(px(16.0))
            .bg(palette.bg)
            .child(
                div()
                    .text_color(palette.fg_muted)
                    .child(t!("welcome.no_workspace")),
            )
            .child(
                h_flex()
                    .gap(px(8.0))
                    .child(
                        PrimaryButton::new("empty-new-workspace", t!("common.new_workspace"))
                            .icon(Icon::new(Lucide::Plus))
                            .on_click(move |_, window, cx| {
                                new_workspace::open_new_workspace_dialog(
                                    new_weak.clone(),
                                    window,
                                    cx,
                                );
                            }),
                    )
                    .child(
                        SecondaryButton::new("empty-open-folder", t!("common.open_folder"))
                            .icon(Icon::new(Lucide::FolderOpen))
                            .on_click(move |_, window, cx| {
                                sidebar::pick_workspace_folder(weak.clone(), window, cx);
                            }),
                    ),
            )
            .into_any_element()
    }
}

/// Delays the start of an `ease_in_out` curve by `delay` (0.0..1.0 of the whole span), so a
/// later block settles in just after an earlier one from one shared, already-running
/// [`Animation`], instead of needing a delay primitive of its own.
fn staggered(delay: f32) -> impl Fn(f32) -> f32 {
    move |delta: f32| {
        let t = ((delta - delay) / (1.0 - delay)).clamp(0.0, 1.0);
        ease_in_out(t)
    }
}

/// Wraps `content` in a one-shot fade-and-rise, `delay` into the shared entrance span (see
/// [`staggered`]). Runs once when `id` first paints; gpui stops asking for more frames once it
/// reaches its end, so an idle, already-settled welcome tab costs nothing (`AGENTS.md`,
/// Performance).
fn fade_rise(id: impl Into<ElementId>, delay: f32, content: impl IntoElement) -> AnyElement {
    div()
        .relative()
        .child(content)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(ENTRANCE_MS)).with_easing(staggered(delay)),
            |this, delta| this.opacity(delta).top(px((1.0 - delta) * ENTRANCE_RISE)),
        )
        .into_any_element()
}

/// One spark of [`render_ember_mark`]: its size, position and color, all fractions of the mark's
/// own `size` so the whole cluster scales cleanly.
struct Spark {
    /// Diameter, as a fraction of the mark's size.
    diameter: f32,
    /// Left and top, as fractions of the mark's size.
    left: f32,
    top: f32,
}

/// The cluster of sparks a [`render_ember_mark`] is built from, biggest to smallest, each
/// overlapping the one before it closely enough that the group reads as one rounded mass, not a
/// thin trail of shrinking dots. Four, not five: a cluster reads cleanly at this count; a fifth
/// spark was too small to register as its own shape.
const SPARKS: [Spark; 4] = [
    Spark {
        diameter: 0.62,
        left: 0.08,
        top: 0.22,
    },
    Spark {
        diameter: 0.40,
        left: 0.48,
        top: 0.0,
    },
    Spark {
        diameter: 0.30,
        left: 0.50,
        top: 0.50,
    },
    Spark {
        diameter: 0.17,
        left: 0.80,
        top: 0.10,
    },
];

/// The brand mark: not Zed's nested squares, and not a literal flame either. A tight cluster of
/// four overlapping embers, biggest to smallest, in the mark's own
/// [`crate::theme::BrasaColors::ember_rings`] gradient: a deep, dominant ember anchors the group,
/// a small hot, pale spark catches the eye at its edge, reading as one rounded mass rather than a
/// thin trailing line. Plain native shapes, not an SVG: crisp at any size, recolors for free with
/// the palette, and animates with gpui's own `Animation`/`AnimationExt`, no asset or image
/// pipeline needed.
///
/// With `animate`, plays a one-shot catch once, when the mark first paints: the dominant ember
/// appears first, each smaller one popping in and drifting a touch further up a beat later, all
/// from one shared [`Animation`] (see [`staggered`]). Settles after its span and repaints nothing
/// again, like the rest of this tab's entrance (`AGENTS.md`, Performance).
fn render_ember_mark(palette: &Palette, size: f32, animate: bool) -> AnyElement {
    /// How small a spark starts, as a fraction of its final size, before it grows into place.
    const GROW_FROM: f32 = 0.4;
    /// How far a spark drifts up from its final position as it catches, in px at `size` 32 (it
    /// scales with the mark).
    const DRIFT: f32 = 5.0;

    let colors = palette.ember_rings();
    let mut mark = div().relative().flex_none().size(px(size));
    for (index, spark) in SPARKS.iter().enumerate() {
        // Skips `ember_rings`' darkest ring: the base spark already reads as the ember without
        // it, and using all five crowded the trail's gradient into too few visible steps.
        let color = colors[index + 1];
        let diameter = size * spark.diameter;
        let left = size * spark.left;
        let top = size * spark.top;
        let drift = DRIFT * (size / 32.0);
        let one = div()
            .absolute()
            .size(px(diameter))
            .left(px(left))
            .top(px(top))
            .rounded_full()
            .bg(color);
        let one = if animate {
            let delay = index as f32 / SPARKS.len() as f32 * 0.5;
            one.with_animation(
                ("ember-spark", index),
                Animation::new(Duration::from_millis(ENTRANCE_MS + 200))
                    .with_easing(staggered(delay)),
                move |this, delta| {
                    let grown = diameter * (GROW_FROM + (1.0 - GROW_FROM) * delta);
                    // Grows from its own center, and settles into place from a touch lower, so
                    // each spark reads as catching and rising rather than just fading in.
                    let center_shift = (diameter - grown) / 2.0;
                    this.opacity(delta)
                        .size(px(grown))
                        .left(px(left + center_shift))
                        .top(px(top + center_shift + drift * (1.0 - delta)))
                },
            )
            .into_any_element()
        } else {
            one.into_any_element()
        };
        mark = mark.child(one);
    }
    mark.into_any_element()
}

/// The quiet brand mark: the ember mark inline with the wordmark, and the tagline under it.
/// Deliberately small, so the primary action below it is the page's actual focus.
fn render_hero(palette: &Palette) -> impl IntoElement {
    v_flex()
        .gap(px(8.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(10.0))
                .child(render_ember_mark(palette, WELCOME_LOGO_SIZE, true))
                .child(
                    div()
                        .text_size(px(19.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(palette.fg)
                        .child("Brasa"),
                ),
        )
        .child(
            div()
                .text_size(px(13.5))
                .text_color(palette.fg_muted)
                .child(t!("welcome.tagline")),
        )
}

/// The one clear move: a full-width primary button. "Open a folder" and the two import paths
/// are tucked into a single quiet line underneath instead of competing with it for attention.
fn render_primary_action(weak: WeakEntity<AppView>, palette: &Palette) -> impl IntoElement {
    let new_weak = weak.clone();
    let open_weak = weak.clone();
    let openapi_weak = weak.clone();
    let postman_weak = weak;

    let quiet_link = |id: &'static str, label: SharedString, on_click: ClickHandler| -> AnyElement {
        let hover_color = palette.fg;
        div()
            .id(id)
            .cursor_pointer()
            .text_color(palette.fg_subtle)
            .hover(move |style| style.text_color(hover_color))
            .child(label)
            .on_click(move |_, window, cx| on_click(window, cx))
            .into_any_element()
    };

    v_flex()
        .mt(px(28.0))
        .gap(px(12.0))
        .child(
            PrimaryButton::new("welcome-new-workspace", t!("common.new_workspace"))
                .icon(Icon::new(Lucide::Plus))
                .height(42.0)
                .full_width()
                .on_click(move |_, window, cx| {
                    new_workspace::open_new_workspace_dialog(new_weak.clone(), window, cx);
                }),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(12.5))
                .child(quiet_link(
                    "welcome-open-folder",
                    t!("common.open_folder").into(),
                    Box::new(move |window, cx| {
                        sidebar::pick_workspace_folder(open_weak.clone(), window, cx);
                    }),
                ))
                .child(separator_dot(palette))
                .child(quiet_link(
                    "welcome-import",
                    t!("welcome.import_postman").into(),
                    Box::new(move |window, cx| {
                        super::openapi_import::open_postman_wizard(
                            postman_weak.clone(),
                            window,
                            cx,
                        );
                    }),
                ))
                .child(separator_dot(palette))
                .child(quiet_link(
                    "welcome-import-openapi",
                    t!("openapi.welcome").into(),
                    Box::new(move |window, cx| {
                        super::openapi_import::open_openapi_wizard(
                            openapi_weak.clone(),
                            crate::state::openapi_import::SourceKind::File,
                            window,
                            cx,
                        );
                    }),
                )),
        )
}

/// The faint `·` between the quiet links and between the footer links.
fn separator_dot(palette: &Palette) -> impl IntoElement {
    div()
        .text_color(palette.fg_subtle.opacity(0.6))
        .child("\u{b7}")
}

/// The "Recent" list: one row per recent workspace, name and shortened path. A click opens it.
fn render_recents(
    weak: WeakEntity<AppView>,
    recents: &[RecentWorkspace],
    palette: &Palette,
    mono_font: &SharedString,
) -> impl IntoElement {
    v_flex()
        .gap(px(6.0))
        .child(
            div()
                .px(px(10.0))
                .pb(px(6.0))
                .child(SectionLabel::new(t!("welcome.recent"))),
        )
        .children(recents.iter().enumerate().map(|(index, recent)| {
            let weak = weak.clone();
            let path = recent.path.clone();
            list_row(("welcome-recent", index), palette)
                .gap(px(12.0))
                .child(
                    div()
                        .flex_none()
                        .w(px(WELCOME_RECENT_NAME_WIDTH))
                        .truncate()
                        .text_color(palette.fg)
                        .child(recent.name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font.clone())
                        .text_size(px(12.0))
                        .text_color(palette.fg_subtle)
                        .child(recent.path_label.clone()),
                )
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.open_workspace_at(&path, window, cx));
                })
        }))
}

/// A clickable row of the Recent list, with the hover and pressed backgrounds.
fn list_row(id: impl Into<ElementId>, palette: &Palette) -> Stateful<Div> {
    h_flex()
        .id(id)
        .h(px(WELCOME_ROW_HEIGHT))
        .items_center()
        .px(px(10.0))
        .rounded(px(RADIUS_MD - 1.0))
        .cursor_pointer()
        .hover(|style| style.bg(palette.hover))
        .active(|style| style.bg(palette.pressed))
}

/// A compact icon-only theme toggle (System/Light/Dark), instead of three large preview cards:
/// the welcome tab's job is to get the person into a workspace, not to also be where themes are
/// browsed (Settings, Appearance already does that in full, with the same three cards).
fn render_theme_toggle(
    weak: WeakEntity<AppView>,
    current: ThemeChoice,
    palette: &Palette,
) -> impl IntoElement {
    let items = [
        (ThemeChoice::System, Lucide::Monitor, "welcome-theme-system"),
        (ThemeChoice::Light, Lucide::Sun, "welcome-theme-light"),
        (ThemeChoice::Dark, Lucide::Moon, "welcome-theme-dark"),
    ];
    h_flex()
        .flex_none()
        .gap(px(2.0))
        .p(px(2.0))
        .rounded(px(RADIUS_SM))
        .bg(palette.surface)
        .border_1()
        .border_color(palette.border)
        .children(items.into_iter().map(|(choice, icon, id)| {
            let selected = choice == current;
            let weak = weak.clone();
            div()
                .id(id)
                .size(px(24.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(RADIUS_SM - 1.0))
                .cursor_pointer()
                .when(selected, |this| {
                    this.bg(palette.raised).text_color(palette.fg)
                })
                .when(!selected, |this| {
                    this.text_color(palette.fg_subtle).hover(|style| {
                        style.bg(palette.hover).text_color(palette.fg)
                    })
                })
                .active(|style| style.bg(palette.pressed))
                .child(Icon::new(icon).with_size(px(13.0)))
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.set_theme_choice(choice, window, cx));
                })
        }))
}

fn render_footer(
    weak: WeakEntity<AppView>,
    theme: ThemeChoice,
    show_welcome: bool,
    palette: &Palette,
    mono_font: &SharedString,
) -> impl IntoElement {
    let shortcuts_weak = weak.clone();
    let search_weak = weak.clone();
    let link =
        |id: &'static str, label: SharedString, keys: Option<String>, on_click: ClickHandler| {
            let hover_color = palette.fg;
            h_flex()
                .id(id)
                .gap(px(4.0))
                .cursor_pointer()
                .text_color(palette.accent_text)
                .hover(move |style| style.text_color(hover_color))
                .child(label)
                .children(keys.map(|keys| {
                    div()
                        .font_family(mono_font.clone())
                        .text_size(px(11.0))
                        .child(format!("({keys})"))
                }))
                .on_click(move |_, window, cx| on_click(window, cx))
        };
    let dot = || separator_dot(palette);

    let checkbox_weak = weak.clone();

    h_flex()
        .flex_wrap()
        .items_center()
        .gap(px(16.0))
        .pt(px(16.0))
        .border_t_1()
        .border_color(palette.border)
        .text_size(px(12.5))
        .child(
            h_flex()
                .items_center()
                .gap(px(10.0))
                .child(link(
                    "welcome-quick-start",
                    t!("welcome.quick_start").into(),
                    None,
                    Box::new(|_, cx| cx.open_url(QUICK_START_URL)),
                ))
                .child(dot())
                .child(link(
                    "welcome-shortcuts",
                    t!("welcome.shortcuts").into(),
                    Some(open_hint()),
                    Box::new(move |window, cx| {
                        let _ =
                            shortcuts_weak.update(cx, |view, cx| view.open_shortcuts(window, cx));
                    }),
                ))
                .child(dot())
                .child(link(
                    "welcome-search",
                    t!("welcome.search").into(),
                    Some(format!("{MODIFIER_KEY}+K")),
                    Box::new(move |window, cx| {
                        let _ = search_weak
                            .update(cx, |view, cx| view.open_command_palette(window, cx));
                    }),
                )),
        )
        .child(div().flex_1())
        .child(render_theme_toggle(weak, theme, palette))
        .child(
            labelled_checkbox(
                "welcome-show-on-start",
                show_welcome,
                t!("welcome.show_on_start"),
                palette,
            )
            .on_change(move |checked, window, cx| {
                let checked = *checked;
                let _ =
                    checkbox_weak.update(cx, |view, cx| view.set_show_welcome(checked, window, cx));
            }),
        )
}
