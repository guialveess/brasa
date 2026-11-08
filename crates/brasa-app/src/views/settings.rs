//! The Settings modal: opened from the title bar gear (`views/title_bar.rs`), the command palette
//! and `Ctrl ,` / `Cmd ,` (`main.rs`'s `bind_keys`,
//! `actions::OpenSettings`). Closed with the `x`, Escape, or a click on the dimmed backdrop, all
//! handled by gpui-component's own `Dialog`.
//!
//! Built as a `Dialog` with fully custom content: a left nav ("Settings" title, the single
//! "Appearance" item) and a right column (header, scrollable body, footer), styled per
//! `docs/design-system.md`. Every control writes straight to
//! `AppState::settings` and calls [`AppView::apply_settings_live`], so it updates the global
//! `Theme` and persists to `settings.toml` immediately: there is no "Save" button.

use std::ops::RangeInclusive;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use brasa_cli::IDENTITY;
use rust_i18n::t;
use telaio_app::logging;
use telaio_ui::components::{
    LabeledRow, NumberStepper, SectionHeading, SettingsCategory as NavCategory, SettingsLayout,
    settings_dialog,
};

use crate::state;
use crate::state::locale::{self, LanguageChoice};
use crate::state::number::format_decimal;
use crate::state::settings::{
    InvalidTlsCertificates, LogLevel, Settings, SettingsCategory, ThemeChoice,
};
use crate::theme::metrics::RADIUS_MD;
use crate::theme::{self, Palette, PaletteExt, BrasaColors as _};

use super::components::{GhostButton, ThemeCard, switch_halo};
use super::root::AppView;

mod about;
mod git;
mod selects;

pub(crate) use selects::SettingsSelects;
use selects::{SelectHandles, SettingsSelect, render_select};

impl AppView {
    /// Opens the Settings modal. A no-op when one is already open, so the gear and `Ctrl ,` / `Cmd
    /// ,` never stack a second modal on top (GitHub #18).
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            return;
        }
        let installed = cx.text_system().all_font_names();
        self.settings_selects = Some(SettingsSelects::new(
            &self.state.settings,
            &installed,
            window,
            cx,
        ));
        let weak = cx.weak_entity();
        window.open_dialog(cx, move |dialog, window, _cx| {
            let weak = weak.clone();
            settings_dialog(dialog, window, move |_window, cx| {
                render_settings_body(weak.clone(), cx)
            })
        });
    }

    /// Applies `self.state.settings` to the live `Theme` (colors, fonts and sizes) and to the theme
    /// mode, refreshes `window`, and persists the file. Called after every control in the Settings
    /// view changes a value; there is no separate "Save" action.
    pub(crate) fn apply_settings_live(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        theme::apply_settings(cx, &self.state.settings);
        telaio_ui::ThemeChoice::from(self.state.settings.theme).apply(Some(window), cx);
        // The next request, normal or load test, reads these options.
        self.send_options.invalid_certificates =
            self.state.settings.invalid_tls_certificates.to_http();
        self.send_options.system_certificates = self.state.settings.system_certificates;
        self.send_options.max_response_size = self.state.settings.max_response_bytes();
        log::info!("settings changed: {:?}", self.state.settings);
        state::settings::save_settings(&self.state.settings);
        self.sync_settings_selects(false, window, cx);
        cx.notify();
    }

    /// Picks the Settings category shown in the right column.
    fn set_settings_category(&mut self, category: SettingsCategory, cx: &mut Context<Self>) {
        self.settings_category = category;
        cx.notify();
    }

    /// Picks the UI language: applies it process wide, lets every view re-apply the strings it
    /// cached outside render ([`Self::relocalize`]), and repaints every window.
    fn set_language(
        &mut self,
        choice: LanguageChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.language = choice;
        self.apply_language(window, cx);
        self.apply_settings_live(window, cx);
    }

    /// Applies `self.state.settings.language` live, without persisting it.
    fn apply_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = locale::resolve(self.state.settings.language);
        locale::apply(language);
        log::info!("language changed to {}", language.code());
        self.relocalize(window, cx);
        self.sync_settings_selects(true, window, cx);
        cx.refresh_windows();
    }

    /// Shows the current settings in the Settings selects, when the modal has been opened.
    fn sync_settings_selects(
        &mut self,
        relabel: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selects) = self.settings_selects.take() {
            selects.sync(&self.state.settings, relabel, window, cx);
            self.settings_selects = Some(selects);
        }
    }

    /// Picks what to do with an invalid TLS certificate.
    fn set_invalid_tls_certificates(
        &mut self,
        choice: InvalidTlsCertificates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.invalid_tls_certificates = choice;
        self.apply_settings_live(window, cx);
    }

    /// Turns on or off verifying certificates against the OS certificate store.
    fn set_system_certificates(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.system_certificates = enabled;
        self.apply_settings_live(window, cx);
    }

    /// Picks the largest response body that is read.
    /// Picks the JSON indentation, used by the next Format and by every response's Pretty view
    /// (its text cache is keyed on it, so the shown response is re-indented once).
    fn set_json_indent(&mut self, spaces: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.json_indent = spaces;
        self.apply_settings_live(window, cx);
    }

    fn set_max_response_mb(&mut self, mb: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.max_response_mb = mb;
        self.apply_settings_live(window, cx);
    }

    /// Picks the log level, applied to the logger at once.
    fn set_log_level(&mut self, level: LogLevel, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.log_level = level;
        logging::set_level(level.to_filter());
        self.apply_settings_live(window, cx);
    }

    /// Turns the automatic update check at startup on or off.
    fn set_check_updates(&mut self, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.check_updates = enabled;
        self.apply_settings_live(window, cx);
    }

    /// Turns the welcome tab at startup on or off, from Settings or the tab's own checkbox.
    pub(crate) fn set_show_welcome(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.show_welcome = enabled;
        self.apply_settings_live(window, cx);
    }

    /// Turns the What's new tab after an update on or off, from Settings or the tab's checkbox.
    pub(crate) fn set_show_whats_new(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.show_whats_new = enabled;
        self.apply_settings_live(window, cx);
    }

    /// Toggles between the light and dark theme (the command palette's "Toggle theme" action): picks the opposite of whichever mode is currently
    /// showing, so it also does the sensible thing from "System" (moving to whichever of Light or
    /// Dark the OS is not currently showing). Always lands on an explicit choice, like clicking a
    /// theme card, so "System" stops following the OS once toggled.
    pub(crate) fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if cx.theme().is_dark() {
            ThemeChoice::Light
        } else {
            ThemeChoice::Dark
        };
        self.set_theme_choice(next, window, cx);
    }

    /// Picks a theme card, in Settings or in the welcome tab.
    pub(crate) fn set_theme_choice(
        &mut self,
        choice: ThemeChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.settings.theme = choice;
        self.apply_settings_live(window, cx);
    }

    /// Picks the UI font family.
    fn set_ui_font(&mut self, font: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.ui_font = font;
        self.apply_settings_live(window, cx);
    }

    /// Sets the UI font size, already within its range.
    fn set_ui_font_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.ui_font_size = size;
        self.apply_settings_live(window, cx);
    }

    /// Picks the monospace font family.
    fn set_mono_font(&mut self, font: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.mono_font = font;
        self.apply_settings_live(window, cx);
    }

    /// Sets the monospace font size, already within its range.
    fn set_mono_font_size(&mut self, size: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.state.settings.mono_font_size = size;
        self.apply_settings_live(window, cx);
    }

    /// Restores `Settings::default()` ("Reset to
    /// defaults").
    fn reset_settings_to_defaults(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let git_executable = self.state.settings.git_executable.take();
        self.state.settings = Settings::default();
        logging::set_level(self.state.settings.log_level.to_filter());
        self.apply_language(window, cx);
        self.apply_settings_live(window, cx);
        if git_executable.is_some() {
            self.detect_git(cx);
        }
    }
}

/// Renders the dialog's whole content with telaio's [`SettingsLayout`]: the categories, the
/// "Saved to" note, the selected category's sections and its footer. `weak` lets every control
/// reach back into [`AppView`] to change a setting; `cx` is re-read fresh every time the window
/// repaints the dialog (the content builder gpui-component calls is a plain `Fn`, not a `FnMut`,
/// so nothing here is memoized across frames), which is what makes every change appear
/// immediately.
fn render_settings_body(weak: WeakEntity<AppView>, cx: &mut App) -> AnyElement {
    let Some(view) = weak.upgrade() else {
        return div().into_any_element();
    };
    let settings = view.read(cx).state.settings.clone();
    let category = view.read(cx).settings_category;
    let about =
        (category == SettingsCategory::About).then(|| about::AboutPane::capture(view.read(cx)));
    let git = (category == SettingsCategory::Git).then(|| git::GitPane::capture(view.read(cx)));
    let palette = cx.palette();
    let mono_font_family = cx.theme().mono_font_family.clone();
    let Some(selects) = view
        .read(cx)
        .settings_selects
        .as_ref()
        .map(|selects| selects.handles.clone())
    else {
        return div().into_any_element();
    };

    let path_label = state::settings::settings_path()
        .map(|path| state::format::shorten_path(&path, dirs::home_dir().as_deref()))
        .unwrap_or_else(|| t!("common.unknown").into_owned());
    let categories = SettingsCategory::ALL.map(|category| {
        let icon = match category {
            SettingsCategory::Appearance => gpui_kit::assets::IconName::Palette,
            SettingsCategory::Requests => gpui_kit::assets::IconName::Globe,
            SettingsCategory::Git => gpui_kit::assets::IconName::GitBranch,
            SettingsCategory::Advanced => gpui_kit::assets::IconName::Wrench,
            SettingsCategory::About => gpui_kit::assets::IconName::Info,
        };
        NavCategory::new(icon, category.label())
    });
    let selected = SettingsCategory::ALL
        .iter()
        .position(|candidate| *candidate == category)
        .unwrap_or(0);
    let select_weak = weak.clone();

    let layout = SettingsLayout::new("settings")
        .title(t!("settings.title"))
        .categories(categories)
        .selected(selected)
        .on_select(move |index, _, cx| {
            if let Some(&category) = SettingsCategory::ALL.get(index) {
                let _ = select_weak.update(cx, |view, cx| view.set_settings_category(category, cx));
            }
        })
        .nav_footer(
            v_flex().child(t!("settings.saved_to")).child(
                div()
                    .font_family(mono_font_family.clone())
                    .child(path_label),
            ),
        )
        .heading(category.label())
        .sections(render_sections(
            weak.clone(),
            category,
            &palette,
            &settings,
            &selects,
            &mono_font_family,
            about.as_ref(),
            git.as_ref(),
        ));
    let layout = if about.is_some() {
        layout.footer(about::footer_text(), div())
    } else {
        layout.footer(
            t!("settings.footer.applied"),
            GhostButton::new("settings-reset", t!("settings.footer.reset")).on_click(
                move |_, window, cx| {
                    let _ = weak.update(cx, |view, cx| view.reset_settings_to_defaults(window, cx));
                },
            ),
        )
    };
    layout.into_any_element()
}

/// The selected category's sections, which the layout splits with dividers.
#[allow(clippy::too_many_arguments)]
fn render_sections(
    weak: WeakEntity<AppView>,
    category: SettingsCategory,
    palette: &Palette,
    settings: &Settings,
    selects: &SelectHandles,
    mono_font_family: &SharedString,
    about: Option<&about::AboutPane>,
    git: Option<&git::GitPane>,
) -> Vec<AnyElement> {
    match category {
        SettingsCategory::Appearance => vec![
            render_language_section(&selects.language),
            render_theme_section(weak.clone(), settings.theme),
            render_interface_section(weak.clone(), palette, settings, selects),
            render_editor_section(weak, palette, settings, selects, mono_font_family),
        ],
        SettingsCategory::Requests => vec![
            render_tls_section(weak, palette, settings, &selects.tls),
            render_response_section(&selects.max_response),
        ],
        SettingsCategory::Git => git
            .map(|pane| {
                git::render_git(
                    weak,
                    palette,
                    settings,
                    &selects.git_executable,
                    mono_font_family,
                    pane,
                )
            })
            .unwrap_or_default(),
        SettingsCategory::Advanced => vec![render_logging_section(
            palette,
            &selects.log_level,
            mono_font_family,
        )],
        SettingsCategory::About => about
            .map(|pane| about::render_about(weak, palette, mono_font_family, pane))
            .unwrap_or_default(),
    }
}

/// The dropdown label of `choice`: a language's name in itself, or "Automatic" with the language
/// the system resolves to.
fn language_choice_label(choice: LanguageChoice) -> String {
    match choice.language() {
        Some(language) => language.native_name().to_string(),
        None => t!(
            "settings.language.auto",
            language = locale::resolve(LanguageChoice::Auto).native_name()
        )
        .into_owned(),
    }
}

/// The "Language" section: a dropdown with Automatic and the four languages.
fn render_language_section(select: &SettingsSelect) -> AnyElement {
    let control = render_select("settings-language", select, None);

    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.language.title")))
        .child(
            LabeledRow::new(t!("settings.language.label"), control)
                .description(t!("settings.language.description")),
        )
        .into_any_element()
}

/// The "Theme" section: the description and the three theme cards.
/// The [`ThemeCard`] for `choice`, selected when it is `current`, that picks it on click. Shared by
/// Settings and the welcome tab; `id_prefix` keeps their element ids apart.
pub(crate) fn theme_card(
    weak: WeakEntity<AppView>,
    id_prefix: &str,
    choice: ThemeChoice,
    current: ThemeChoice,
) -> ThemeCard {
    let id = match choice {
        ThemeChoice::System => "system",
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
    };
    ThemeCard::new(
        SharedString::from(format!("{id_prefix}-{id}")),
        choice.into(),
    )
    .selected(choice == current)
    .on_click(move |_, window, cx| {
        let _ = weak.update(cx, |view, cx| view.set_theme_choice(choice, window, cx));
    })
}

fn render_theme_section(weak: WeakEntity<AppView>, current: ThemeChoice) -> AnyElement {
    v_flex()
        .gap(px(10.0))
        .child(
            SectionHeading::new(t!("settings.theme.title"))
                .description(t!("settings.theme.description")),
        )
        .child(
            h_flex().gap(px(12.0)).children(
                [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
                    .into_iter()
                    .map(|choice| theme_card(weak.clone(), "settings-theme", choice, current)),
            ),
        )
        .into_any_element()
}

/// The "Interface" section: the UI font picker, its size stepper and the welcome tab switch.
fn render_interface_section(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    settings: &Settings,
    selects: &SelectHandles,
) -> AnyElement {
    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.interface.title")))
        .child(
            LabeledRow::new(
                t!("settings.font"),
                render_select("settings-ui-font", &selects.ui_font, None),
            )
            .description(t!("settings.interface.font_description")),
        )
        .child(LabeledRow::new(
            t!("settings.font_size"),
            font_size_stepper(
                "settings-ui-size",
                weak.clone(),
                settings.ui_font_size,
                state::settings::UI_FONT_SIZE_STEP,
                state::settings::UI_FONT_SIZE_RANGE,
                |view, size, window, cx| view.set_ui_font_size(size, window, cx),
            ),
        ))
        .child(
            LabeledRow::new(
                t!("settings.interface.welcome"),
                switch_halo(
                    "settings-show-welcome-halo",
                    Switch::new("settings-show-welcome")
                        .checked(settings.show_welcome)
                        .on_click({
                            let weak = weak.clone();
                            move |checked, window, cx| {
                                let checked = *checked;
                                let _ = weak.update(cx, |view, cx| {
                                    view.set_show_welcome(checked, window, cx)
                                });
                            }
                        }),
                    palette,
                ),
            )
            .description(t!("settings.interface.welcome_description")),
        )
        .child(
            LabeledRow::new(
                t!("settings.interface.whats_new"),
                switch_halo(
                    "settings-show-whats-new-halo",
                    Switch::new("settings-show-whats-new")
                        .checked(settings.show_whats_new)
                        .on_click(move |checked, window, cx| {
                            let checked = *checked;
                            let _ = weak.update(cx, |view, cx| {
                                view.set_show_whats_new(checked, window, cx)
                            });
                        }),
                    palette,
                ),
            )
            .description(t!("settings.interface.whats_new_description")),
        )
        .into_any_element()
}

/// The "Editor" section: the monospace font picker, its size stepper, and the live JSON preview.
fn render_editor_section(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    settings: &Settings,
    selects: &SelectHandles,
    mono_font_family: &SharedString,
) -> AnyElement {
    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.editor.title")))
        .child(LabeledRow::new(
            t!("settings.editor.mono_font"),
            render_select(
                "settings-mono-font",
                &selects.mono_font,
                Some(mono_font_family.clone()),
            ),
        ))
        .child(LabeledRow::new(
            t!("settings.font_size"),
            font_size_stepper(
                "settings-mono-size",
                weak.clone(),
                settings.mono_font_size,
                state::settings::MONO_FONT_SIZE_STEP,
                state::settings::MONO_FONT_SIZE_RANGE,
                |view, size, window, cx| view.set_mono_font_size(size, window, cx),
            ),
        ))
        .child(
            LabeledRow::new(
                t!("settings.editor.indent"),
                render_select("settings-json-indent", &selects.json_indent, None),
            )
            .description(t!("settings.editor.indent_description")),
        )
        .child(render_json_preview(
            palette,
            mono_font_family.clone(),
            settings.mono_font_size,
            settings.json_indent,
        ))
        .into_any_element()
}

/// The "TLS" section of the Requests pane: which certificates are trusted and what to do with an
/// invalid one.
fn render_tls_section(
    weak: WeakEntity<AppView>,
    palette: &Palette,
    settings: &Settings,
    select: &SettingsSelect,
) -> AnyElement {
    let control = render_select("settings-invalid-certificates", select, None);

    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.tls.title")))
        .child(
            LabeledRow::new(
                t!("settings.tls.system_certificates"),
                switch_halo(
                    "settings-system-certificates-halo",
                    Switch::new("settings-system-certificates")
                        .checked(settings.system_certificates)
                        .on_click(move |checked, window, cx| {
                            let checked = *checked;
                            let _ = weak.update(cx, |view, cx| {
                                view.set_system_certificates(checked, window, cx)
                            });
                        }),
                    palette,
                ),
            )
            .description(t!("settings.tls.system_certificates_description")),
        )
        .child(
            LabeledRow::new(t!("settings.tls.invalid_certificates"), control)
                .description(t!("settings.tls.invalid_certificates_description")),
        )
        .into_any_element()
}

/// The "Responses" section of the Requests pane: the largest response body that is read.
fn render_response_section(select: &SettingsSelect) -> AnyElement {
    let control = render_select("settings-max-response", select, None);

    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.response.title")))
        .child(
            LabeledRow::new(t!("settings.response.max_size"), control)
                .description(t!("settings.response.max_size_description")),
        )
        .into_any_element()
}

/// The "Logging" section of the Advanced pane: the log level and where the log file is.
fn render_logging_section(
    palette: &Palette,
    select: &SettingsSelect,
    mono_font_family: &SharedString,
) -> AnyElement {
    let control = render_select("settings-log-level", select, None);

    let log_dir = logging::log_dir(&IDENTITY);
    let path_label = logging::log_path(&IDENTITY)
        .map(|path| state::format::shorten_path(&path, dirs::home_dir().as_deref()))
        .unwrap_or_else(|| t!("common.unknown").into_owned());
    let open_folder = GhostButton::new(
        "settings-open-log-folder",
        t!("settings.logging.open_folder"),
    )
    .icon(Icon::new(gpui_kit::assets::IconName::FolderOpen))
    .disabled(log_dir.is_none())
    .on_click(move |_, _, cx| {
        if let Some(dir) = &log_dir {
            cx.open_with_system(dir);
        }
    });

    v_flex()
        .gap(px(14.0))
        .child(SectionHeading::new(t!("settings.logging.title")))
        .child(
            LabeledRow::new(t!("settings.logging.level"), control)
                .description(t!("settings.logging.level_description")),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap(px(16.0))
                .child(
                    v_flex()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(div().child(t!("settings.logging.file")))
                        .child(
                            div()
                                .text_color(palette.fg_muted)
                                .text_size(px(12.0))
                                .font_family(mono_font_family.clone())
                                .child(path_label),
                        ),
                )
                .child(open_folder),
        )
        .child(
            div()
                .text_color(palette.fg_subtle)
                .text_size(px(12.0))
                .child(t!(
                    "settings.logging.rotation",
                    size = logging::MAX_FILE_SIZE / (1024 * 1024),
                    files = logging::MAX_FILES,
                    variable = logging::env_var(&IDENTITY),
                )),
        )
        .into_any_element()
}

/// A font size stepper: `-`, the size in the user's number format and `+`, within `range`.
fn font_size_stepper(
    id: &'static str,
    weak: WeakEntity<AppView>,
    value: f32,
    step: f32,
    range: RangeInclusive<f32>,
    set: impl Fn(&mut AppView, f32, &mut Window, &mut Context<AppView>) + 'static,
) -> NumberStepper {
    NumberStepper::new(id, value)
        .step(step)
        .range(*range.start(), *range.end())
        .format(|value| {
            format!(
                "{} px",
                format_decimal(f64::from(value), usize::from(value.fract() != 0.0))
            )
        })
        .on_change(move |size, window, cx| {
            let _ = weak.update(cx, |view, cx| set(view, size, window, cx));
        })
}

/// The Editor section's live preview: a syntax-highlighted JSON snippet in the chosen monospace
/// font and size, indented by `indent` spaces as the Format button and the Pretty view would.
fn render_json_preview(
    palette: &Palette,
    mono_font_family: SharedString,
    size: f32,
    indent: usize,
) -> AnyElement {
    let pad = || json_span(" ".repeat(indent), palette.fg_muted);
    let line = || h_flex().items_baseline();
    div()
        .border_1()
        .border_color(palette.border)
        .rounded(px(RADIUS_MD))
        .bg(palette.bg)
        .px(px(14.0))
        .py(px(10.0))
        .child(
            v_flex()
                .font_family(mono_font_family)
                .text_size(px(size))
                .line_height(relative(1.6))
                .child(line().child(json_span("{", palette.fg_muted)))
                .child(
                    line()
                        .child(pad())
                        .child(json_span("\"token\"", palette.syntax_color("property")))
                        .child(json_span(": ", palette.fg_muted))
                        .child(json_span(
                            "\"eyJhbGciOi\u{2026}\"",
                            palette.syntax_color("string"),
                        ))
                        .child(json_span(",", palette.fg_muted)),
                )
                .child(
                    line()
                        .child(pad())
                        .child(json_span("\"expiresIn\"", palette.syntax_color("property")))
                        .child(json_span(": ", palette.fg_muted))
                        .child(json_span("3600", palette.syntax_color("number"))),
                )
                .child(line().child(json_span("}", palette.fg_muted))),
        )
        .into_any_element()
}

/// One colored fragment of a [`render_json_preview`] line.
fn json_span(text: impl Into<SharedString>, color: Hsla) -> impl IntoElement {
    div().text_color(color).child(text.into())
}
