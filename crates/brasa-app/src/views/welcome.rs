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

use crate::state::format::relative_time;
use crate::state::number::format_integer;
use crate::state::settings::ThemeChoice;
use crate::state::welcome::{QUICK_START_URL, RecentWorkspace, RecentWorkspaceStats, WelcomeTab};
use crate::theme::metrics::{
    RADIUS_MD, RADIUS_SM, WELCOME_COLUMN_WIDTH, WELCOME_COLUMN_WIDTH_NARROW, WELCOME_LOGO_SIZE,
    WELCOME_MIN_WIDTH, WELCOME_TOP_PADDING, WELCOME_TOP_PADDING_NARROW,
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
    /// Also the command palette's "Show welcome" action and the rail's brand button
    /// ([`super::navigation::render_rail`]).
    pub(crate) fn open_welcome_tab(&mut self, cx: &mut Context<Self>) {
        let welcome = WelcomeTab::load(&self.recent_workspaces, dirs::home_dir().as_deref());
        let paths: Vec<_> = welcome.recents.iter().map(|recent| recent.path.clone()).collect();
        self.state.tabs.open_welcome(welcome);
        log::debug!("opened the welcome tab");
        self.load_recent_workspace_stats(paths, cx);
        cx.notify();
    }

    /// Fills each recent workspace card's request count and modified time, read on the
    /// background executor ([`crate::state::welcome::load_stats`]) so opening the tab itself
    /// never touches disk. A no-op once the tab closes or moves on before this finishes.
    fn load_recent_workspace_stats(&mut self, paths: Vec<std::path::PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx| {
            let stats = cx
                .background_spawn(async move {
                    paths
                        .iter()
                        .map(|path| crate::state::welcome::load_stats(path))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                let Some(index) = view.state.tabs.index_of(crate::state::welcome::TAB_ID) else {
                    return;
                };
                let Some(welcome) = view
                    .state
                    .tabs
                    .get_mut(index)
                    .and_then(|tab| tab.welcome_mut())
                else {
                    return;
                };
                for (recent, stats) in welcome.recents.iter_mut().zip(stats) {
                    recent.stats = stats;
                }
                cx.notify();
            });
        })
        .detach();
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

/// How many nested rings [`render_ember_mark`] draws, outermost first.
const RING_COUNT: usize = 4;
/// Asymmetric inset fractions per ring step, the same ratios the real app icon is built from
/// (`packaging/icons`'s own generator): more taken from the bottom-left than the top-right each
/// step, so the stack drifts toward one corner instead of sitting as a flat concentric target.
const RING_LEFT: f32 = 0.62;
const RING_RIGHT: f32 = 0.38;
const RING_TOP: f32 = 0.40;
const RING_BOTTOM: f32 = 0.60;
/// How much of the mark's size one inset step takes, and the outer ring's own corner radius, both
/// fractions of `size`.
const RING_STEP: f32 = 0.16;
const RING_OUTER_RADIUS: f32 = 0.32;

/// The brand mark, matching the real app icon: nested rounded squares, outermost first, in the
/// mark's own [`crate::theme::BrasaColors::ember_rings`] gradient, drifting toward one corner as
/// they nest in rather than sitting as a flat concentric target. Plain native shapes, not an SVG:
/// crisp at any size, recolors for free with the palette, and animates with gpui's own
/// `Animation`/`AnimationExt`, no asset or image pipeline needed.
///
/// With `animate`, plays a one-shot catch once, when the mark first paints: the outer ring appears
/// first, each smaller one popping in a beat later, all from one shared [`Animation`] (see
/// [`staggered`]). Settles after its span and repaints nothing again, like the rest of this tab's
/// entrance (`AGENTS.md`, Performance).
fn render_ember_mark(palette: &Palette, size: f32, animate: bool) -> AnyElement {
    /// How small a ring starts, as a fraction of its final size, before it grows into place.
    const GROW_FROM: f32 = 0.55;

    // Skips `ember_rings`' darkest ring, the same way the old spark cluster did: the outer ring
    // already reads as the ember without it.
    let colors = palette.ember_rings();
    let mut mark = div().relative().flex_none().size(px(size));
    for index in 0..RING_COUNT {
        let color = colors[index + 1];
        let step = size * RING_STEP * index as f32;
        let left = step * RING_LEFT;
        let top = step * RING_TOP;
        let width = size - left - step * RING_RIGHT;
        let height = size - top - step * RING_BOTTOM;
        let radius = (size * RING_OUTER_RADIUS - step * 0.5).max(size * 0.08);
        let one = div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .h(px(height))
            .rounded(px(radius))
            .bg(color);
        let one = if animate {
            let delay = index as f32 / RING_COUNT as f32 * 0.5;
            one.with_animation(
                ("ember-ring", index),
                Animation::new(Duration::from_millis(ENTRANCE_MS + 200))
                    .with_easing(staggered(delay)),
                move |this, delta| {
                    let grown_w = width * (GROW_FROM + (1.0 - GROW_FROM) * delta);
                    let grown_h = height * (GROW_FROM + (1.0 - GROW_FROM) * delta);
                    // Grows from its own center, so each ring reads as catching into place
                    // rather than just fading in.
                    let shift_x = (width - grown_w) / 2.0;
                    let shift_y = (height - grown_h) / 2.0;
                    this.opacity(delta)
                        .w(px(grown_w))
                        .h(px(grown_h))
                        .left(px(left + shift_x))
                        .top(px(top + shift_y))
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

/// The "Recent" section: a quiet two-column grid of cards, one per recent workspace, instead of
/// the earlier plain list rows. Each card carries its name, path, request count and last
/// modified time — nothing a flat row can show without starting to feel like a table. The count
/// and time show once [`AppView::load_recent_workspace_stats`] fills them in, which is why
/// [`RecentWorkspace::stats`] starts `None`: this renders instantly, from the paths alone.
fn render_recents(
    weak: WeakEntity<AppView>,
    recents: &[RecentWorkspace],
    palette: &Palette,
    mono_font: &SharedString,
) -> impl IntoElement {
    v_flex()
        .gap(px(10.0))
        .child(
            div()
                .px(px(10.0))
                .child(SectionLabel::new(t!("welcome.recent"))),
        )
        .children(recents.chunks(2).enumerate().map(|(row_index, row)| {
            h_flex()
                .gap(px(10.0))
                .children(row.iter().enumerate().map(|(column_index, recent)| {
                    render_recent_card(
                        weak.clone(),
                        row_index * 2 + column_index,
                        recent,
                        palette,
                        mono_font,
                    )
                }))
        }))
}

/// One card of [`render_recents`]: an icon, the workspace name and path, and, once loaded, its
/// request count and last modified time on their own quiet line.
fn render_recent_card(
    weak: WeakEntity<AppView>,
    index: usize,
    recent: &RecentWorkspace,
    palette: &Palette,
    mono_font: &SharedString,
) -> AnyElement {
    let path = recent.path.clone();
    let meta = recent
        .stats
        .map(|stats| recent_card_meta(stats))
        .unwrap_or_default();
    h_flex()
        .id(("welcome-recent-card", index))
        .flex_1()
        .min_w_0()
        .items_start()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(RADIUS_MD))
        .cursor_pointer()
        .border_1()
        .border_color(palette.border)
        .bg(palette.surface)
        .hover(|style| style.border_color(palette.border_strong).bg(palette.hover))
        .active(|style| style.bg(palette.pressed))
        .child(
            div()
                .flex_none()
                .size(px(28.0))
                .rounded(px(RADIUS_SM))
                .flex()
                .items_center()
                .justify_center()
                .bg(palette.accent_subtle)
                .child(
                    Icon::new(Lucide::FolderTree)
                        .with_size(px(14.0))
                        .text_color(palette.accent_text),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .truncate()
                        .text_color(palette.fg)
                        .child(recent.name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(mono_font.clone())
                        .text_size(px(11.5))
                        .text_color(palette.fg_subtle)
                        .child(recent.path_label.clone()),
                )
                .child(
                    div()
                        .h(px(15.0))
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(palette.fg_subtle)
                        .child(meta),
                ),
        )
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |view, cx| view.open_workspace_at(&path, window, cx));
        })
        .into_any_element()
}

/// The card's third line, once its stats load: `"N requests · modified 3 h ago"` (or "No
/// requests" for an empty workspace). Empty before they load, so the card's layout does not jump
/// when they arrive.
fn recent_card_meta(stats: RecentWorkspaceStats) -> String {
    let count = if stats.request_count == 1 {
        t!("welcome.recent_one_request").into_owned()
    } else {
        t!(
            "welcome.recent_request_count",
            count = format_integer(stats.request_count as u64).as_str()
        )
        .into_owned()
    };
    match stats.modified_unix_seconds {
        Some(modified) => {
            let now = chrono::Utc::now().timestamp();
            format!("{count} \u{b7} {}", relative_time(modified, now))
        }
        None => count,
    }
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
