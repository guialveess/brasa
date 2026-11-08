//! Application settings, persisted to `<OS config dir>/brasa/settings.toml`
//! and applied live: there is no explicit
//! "Save" action, every change made through the Settings view writes
//! the file immediately.

use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use brasa_runner::InvalidCertificates;
use rust_i18n::t;
use serde::Serialize;

use super::config::config_dir;
use super::locale::LanguageChoice;

pub use telaio_app::logging::LogLevel;

/// The path, relative to the config directory ([`config_dir`]), of the settings file.
pub(crate) const SETTINGS_FILE: &str = "settings.toml";

/// Valid range of [`Settings::ui_font_size`], in points.
pub const UI_FONT_SIZE_RANGE: RangeInclusive<f32> = 11.0..=16.0;

/// Step of the UI font size stepper in the Settings view.
pub const UI_FONT_SIZE_STEP: f32 = 1.0;

/// The choices offered for [`Settings::max_response_mb`], in MB. A value outside this list that
/// was written by hand in `settings.toml` is kept as it is.
pub const MAX_RESPONSE_MB_OPTIONS: [u32; 5] = [10, 20, 50, 100, 200];

/// Valid range of [`Settings::max_response_mb`], in MB.
pub const MAX_RESPONSE_MB_RANGE: RangeInclusive<u32> = 1..=1000;

/// Valid range of [`Settings::mono_font_size`], in points.
pub const MONO_FONT_SIZE_RANGE: RangeInclusive<f32> = 10.0..=18.0;

/// Step of the monospace font size stepper in the Settings view.
pub const MONO_FONT_SIZE_STEP: f32 = 0.5;

/// The choices for [`Settings::json_indent`], in spaces per level. Any other value read from
/// `settings.toml` falls back to the default.
pub const JSON_INDENT_OPTIONS: [usize; 2] = [2, 4];

/// Which theme mode the app follows, chosen in Settings, "Appearance".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Follow the OS light/dark setting.
    #[default]
    System,
    /// Always use the light theme.
    Light,
    /// Always use the dark theme.
    Dark,
}

impl From<ThemeChoice> for telaio_ui::ThemeChoice {
    fn from(choice: ThemeChoice) -> Self {
        match choice {
            ThemeChoice::System => telaio_ui::ThemeChoice::System,
            ThemeChoice::Light => telaio_ui::ThemeChoice::Light,
            ThemeChoice::Dark => telaio_ui::ThemeChoice::Dark,
        }
    }
}

impl ThemeChoice {
    /// Parses a theme name as read from the settings file, case-insensitive. Returns `None` for
    /// anything else, so the caller can fall back to the default.
    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "system" => Some(ThemeChoice::System),
            "light" => Some(ThemeChoice::Light),
            "dark" => Some(ThemeChoice::Dark),
            _ => None,
        }
    }
}

/// A category of the Settings view's left nav.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsCategory {
    /// Theme, fonts and sizes.
    #[default]
    Appearance,
    /// How requests are sent (TLS certificates).
    Requests,
    /// The git executable, syncing, pulling and the safety checks.
    Git,
    /// Diagnostics: the log level and the log file.
    Advanced,
    /// The version, the release links and the updater.
    About,
}

impl SettingsCategory {
    /// Every category, in the order the nav lists them.
    pub const ALL: [SettingsCategory; 5] = [
        SettingsCategory::Appearance,
        SettingsCategory::Requests,
        SettingsCategory::Git,
        SettingsCategory::Advanced,
        SettingsCategory::About,
    ];

    /// The nav item label, also the pane's title.
    pub fn label(self) -> String {
        match self {
            SettingsCategory::Appearance => t!("settings.category.appearance"),
            SettingsCategory::Requests => t!("common.requests"),
            SettingsCategory::Git => t!("settings.category.git"),
            SettingsCategory::Advanced => t!("settings.category.advanced"),
            SettingsCategory::About => t!("settings.category.about"),
        }
        .into_owned()
    }
}

/// What to do when a server's TLS certificate is invalid, chosen in Settings, "Requests". Stored
/// in `settings.toml` as `invalid_tls_certificates = "warn" | "reject" | "accept"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InvalidTlsCertificates {
    /// Send the request anyway and show a warning in the response.
    #[default]
    Warn,
    /// Fail the request with an error.
    Reject,
    /// Accept any certificate without a warning.
    Accept,
}

impl InvalidTlsCertificates {
    /// Every choice, in the order the Settings view lists them.
    pub const ALL: [InvalidTlsCertificates; 3] = [
        InvalidTlsCertificates::Warn,
        InvalidTlsCertificates::Reject,
        InvalidTlsCertificates::Accept,
    ];

    /// Parses a value as read from the settings file, case-insensitive. Returns `None` for
    /// anything else, so the caller can fall back to the default.
    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "warn" => Some(InvalidTlsCertificates::Warn),
            "reject" => Some(InvalidTlsCertificates::Reject),
            "accept" => Some(InvalidTlsCertificates::Accept),
            _ => None,
        }
    }

    /// The label shown in the Settings view.
    pub fn label(self) -> String {
        match self {
            InvalidTlsCertificates::Warn => t!("settings.tls.warn"),
            InvalidTlsCertificates::Reject => t!("settings.tls.reject"),
            InvalidTlsCertificates::Accept => t!("settings.tls.accept"),
        }
        .into_owned()
    }

    /// The matching `brasa-http` mode.
    pub fn to_http(self) -> InvalidCertificates {
        match self {
            InvalidTlsCertificates::Warn => InvalidCertificates::SendWithWarning,
            InvalidTlsCertificates::Reject => InvalidCertificates::Reject,
            InvalidTlsCertificates::Accept => InvalidCertificates::Accept,
        }
    }
}

/// How "Pull" integrates the remote changes, chosen in Settings, "Git". Stored in `settings.toml`
/// as `git_pull = "follow_config" | "merge" | "rebase"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitPull {
    /// Respect `pull.rebase` and `pull.ff` from git config, merge when unset.
    #[default]
    FollowConfig,
    /// Always merge.
    Merge,
    /// Always rebase.
    Rebase,
}

impl GitPull {
    /// Every choice, in the order the Settings view lists them.
    pub const ALL: [GitPull; 3] = [GitPull::FollowConfig, GitPull::Merge, GitPull::Rebase];

    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "follow_config" => Some(GitPull::FollowConfig),
            "merge" => Some(GitPull::Merge),
            "rebase" => Some(GitPull::Rebase),
            _ => None,
        }
    }

    /// The label shown in the Settings view.
    pub fn label(self) -> String {
        match self {
            GitPull::FollowConfig => t!("settings.git.pull_follow"),
            GitPull::Merge => t!("settings.git.pull_merge"),
            GitPull::Rebase => t!("settings.git.pull_rebase"),
        }
        .into_owned()
    }
}

/// What Ctrl+Enter does in the commit message, chosen in Settings, "Git". Stored in
/// `settings.toml` as `git_commit_action = "commit" | "commit_and_push"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GitCommitAction {
    /// Commit only.
    #[default]
    Commit,
    /// Commit, then push.
    CommitAndPush,
}

impl GitCommitAction {
    /// Every choice, in the order the Settings view lists them.
    pub const ALL: [GitCommitAction; 2] = [GitCommitAction::Commit, GitCommitAction::CommitAndPush];

    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "commit" => Some(GitCommitAction::Commit),
            "commit_and_push" => Some(GitCommitAction::CommitAndPush),
            _ => None,
        }
    }

    /// The label shown in the Settings view.
    pub fn label(self) -> String {
        match self {
            GitCommitAction::Commit => t!("settings.git.action_commit"),
            GitCommitAction::CommitAndPush => t!("settings.git.action_commit_and_push"),
        }
        .into_owned()
    }
}

/// How the diff tab lays out the two versions of a file, switched from its toolbar and
/// remembered. Stored as `git_diff_mode = "split" | "unified" | "request"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GitDiffMode {
    /// The two versions side by side (design 7a).
    #[default]
    Split,
    /// One column, removed lines before added ones.
    Unified,
    /// For a `.brasa` file, the changes grouped by part of the request (design 2c). Other
    /// files show side by side.
    Request,
}

impl GitDiffMode {
    fn parse(text: &str) -> Option<Self> {
        match text.to_ascii_lowercase().as_str() {
            "split" => Some(GitDiffMode::Split),
            "unified" => Some(GitDiffMode::Unified),
            "request" => Some(GitDiffMode::Request),
            _ => None,
        }
    }
}

/// User-configurable settings: the UI language, the theme mode, the two fonts (UI and editor) with their sizes,
/// the JSON indentation, which certificates requests trust and what they do with invalid ones, the log level, the update
/// check and the welcome tab.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    /// The UI language: automatic (the system's) or a fixed one.
    pub language: LanguageChoice,
    /// The theme mode: system, light or dark.
    pub theme: ThemeChoice,
    /// The UI font family name.
    pub ui_font: String,
    /// The UI font size, in points, clamped to [`UI_FONT_SIZE_RANGE`].
    pub ui_font_size: f32,
    /// The monospace font family name used in code editors.
    pub mono_font: String,
    /// The monospace font size, in points, clamped to [`MONO_FONT_SIZE_RANGE`].
    pub mono_font_size: f32,
    /// Spaces per nesting level when Brasa formats JSON: the request body's Format button and the
    /// response's Pretty view. One of [`JSON_INDENT_OPTIONS`].
    pub json_indent: usize,
    /// What to do with an invalid TLS certificate.
    pub invalid_tls_certificates: InvalidTlsCertificates,
    /// Whether certificates are verified against the OS certificate store instead of the bundled
    /// Mozilla roots (`SendOptions::system_certificates`). Stored as `system_tls_certificates`.
    #[serde(rename = "system_tls_certificates")]
    pub system_certificates: bool,
    /// The largest response body that is read, in MB (1 MB is 1,000,000 bytes, like the sizes the
    /// app shows), clamped to [`MAX_RESPONSE_MB_RANGE`].
    pub max_response_mb: u32,
    /// How much is written to the log file.
    pub log_level: LogLevel,
    /// Whether to check for a new version once at startup. Only used by builds that have an
    /// updater (`state::update::updater_enabled`).
    pub check_updates: bool,
    /// Whether the welcome tab opens at startup next to the reopened workspace. Without a
    /// workspace to open it always does (`state::welcome::show_at_startup`).
    pub show_welcome: bool,
    /// Whether the What's new tab opens once after an update (`state::whats_new`).
    pub show_whats_new: bool,
    /// The git executable to use, `None` to search the `PATH`. Stored as `git_executable`, absent
    /// when auto-detecting.
    #[serde(rename = "git_executable", skip_serializing_if = "Option::is_none")]
    pub git_executable: Option<PathBuf>,
    /// Whether to fetch when the window regains focus (at most every 5 minutes).
    pub git_fetch_automatically: bool,
    /// How Pull integrates the remote changes.
    pub git_pull: GitPull,
    /// What Ctrl+Enter does in the commit message.
    pub git_commit_action: GitCommitAction,
    /// Whether a commit that includes a `.local.env` file is blocked.
    pub git_block_secrets: bool,
    /// Whether discarding changes asks for confirmation first.
    pub git_confirm_discard: bool,
    /// The diff tab's layout, remembered from its toolbar.
    pub git_diff_mode: GitDiffMode,
    /// The branches starred in the branch menu, pinned first in their group. Names as the menu
    /// shows them (`main`, `origin/main`), shared by every repository. Absent when empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub git_favorite_branches: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            language: LanguageChoice::default(),
            theme: ThemeChoice::default(),
            ui_font: "Geist".to_string(),
            ui_font_size: 13.0,
            mono_font: "Geist Mono".to_string(),
            mono_font_size: 12.5,
            json_indent: 2,
            invalid_tls_certificates: InvalidTlsCertificates::default(),
            system_certificates: true,
            max_response_mb: 20,
            log_level: LogLevel::default(),
            check_updates: true,
            show_welcome: true,
            show_whats_new: true,
            git_executable: None,
            git_fetch_automatically: true,
            git_pull: GitPull::default(),
            git_commit_action: GitCommitAction::default(),
            git_block_secrets: true,
            git_confirm_discard: true,
            git_diff_mode: GitDiffMode::default(),
            git_favorite_branches: Vec::new(),
        }
    }
}

/// The text of a response limit choice: `20 MB`. The unit stays as it is in every language, like
/// the sizes of a response.
pub fn max_response_label(mb: u32) -> String {
    format!("{} MB", super::number::format_integer(u64::from(mb)))
}

impl Settings {
    /// [`Self::max_response_mb`] in bytes, for `SendOptions::max_response_size`.
    pub fn max_response_bytes(&self) -> u64 {
        u64::from(self.max_response_mb) * 1_000_000
    }
}

/// Loads the settings from `<OS config dir>/brasa/settings.toml`, or [`Settings::default`] if the
/// OS config directory is unknown, the file is missing, or its content is not valid TOML.
pub fn load_settings() -> Settings {
    match config_dir() {
        Some(base) => read_settings(&base),
        None => Settings::default(),
    }
}

/// Saves `settings` to `<OS config dir>/brasa/settings.toml`, creating the folder if needed.
/// Failing to persist this is not worth interrupting the user over, so any error (a missing
/// config directory, a read-only filesystem, ...) is only logged.
pub fn save_settings(settings: &Settings) {
    if let Some(base) = config_dir() {
        write_settings(&base, settings);
    }
}

/// The real path of the settings file, `None` when the OS config directory is unknown. Used by the
/// Settings view's "Saved to" line, shortened with `~` by `state::format::shorten_path`.
pub fn settings_path() -> Option<PathBuf> {
    config_dir().map(|base| base.join(SETTINGS_FILE))
}

/// Builds the options for a font picker: `bundled` first, always, then every other entry of
/// `installed` (as returned by `cx.text_system().all_font_names()`, already sorted), skipping the
/// bundled family's own name if it repeats there and de-duplicating.
pub fn font_options(bundled: &str, installed: &[String]) -> Vec<String> {
    let mut options = vec![bundled.to_string()];
    for name in installed {
        if name != bundled && !options.contains(name) {
            options.push(name.clone());
        }
    }
    options
}

/// Reads and parses `<base>/settings.toml`. Split out from [`load_settings`] so tests can
/// point `base` at a temporary directory instead of the real, per-user OS config directory.
///
/// Every field is read and validated independently: a missing file, a field of the wrong type,
/// or an out-of-range number falls back to its own default instead of discarding the whole file,
/// and a key this version does not know about is silently ignored.
fn read_settings(base: &Path) -> Settings {
    let Ok(content) = fs::read_to_string(base.join(SETTINGS_FILE)) else {
        return Settings::default();
    };
    parse_settings(&content)
}

/// Parses the content of a settings file into [`Settings`], falling back field by field to
/// [`Settings::default`] as documented on [`read_settings`].
fn parse_settings(content: &str) -> Settings {
    let defaults = Settings::default();
    let Ok(value) = content.parse::<toml::Value>() else {
        return defaults;
    };
    let table = value.as_table();

    let language = table
        .and_then(|table| table.get("language"))
        .and_then(toml::Value::as_str)
        .and_then(LanguageChoice::parse)
        .unwrap_or(defaults.language);
    let theme = table
        .and_then(|table| table.get("theme"))
        .and_then(toml::Value::as_str)
        .and_then(ThemeChoice::parse)
        .unwrap_or(defaults.theme);
    let ui_font = table
        .and_then(|table| table.get("ui_font"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or(defaults.ui_font);
    let ui_font_size = table
        .and_then(|table| table.get("ui_font_size"))
        .and_then(as_f32)
        .map(|size| size.clamp(*UI_FONT_SIZE_RANGE.start(), *UI_FONT_SIZE_RANGE.end()))
        .unwrap_or(defaults.ui_font_size);
    let mono_font = table
        .and_then(|table| table.get("mono_font"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or(defaults.mono_font);
    let mono_font_size = table
        .and_then(|table| table.get("mono_font_size"))
        .and_then(as_f32)
        .map(|size| size.clamp(*MONO_FONT_SIZE_RANGE.start(), *MONO_FONT_SIZE_RANGE.end()))
        .unwrap_or(defaults.mono_font_size);
    let json_indent = table
        .and_then(|table| table.get("json_indent"))
        .and_then(toml::Value::as_integer)
        .and_then(|spaces| usize::try_from(spaces).ok())
        .filter(|spaces| JSON_INDENT_OPTIONS.contains(spaces))
        .unwrap_or(defaults.json_indent);
    let invalid_tls_certificates = table
        .and_then(|table| table.get("invalid_tls_certificates"))
        .and_then(toml::Value::as_str)
        .and_then(InvalidTlsCertificates::parse)
        .unwrap_or(defaults.invalid_tls_certificates);
    let system_certificates = table
        .and_then(|table| table.get("system_tls_certificates"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.system_certificates);
    let max_response_mb = table
        .and_then(|table| table.get("max_response_mb"))
        .and_then(toml::Value::as_integer)
        .and_then(|mb| u32::try_from(mb).ok())
        .map(|mb| mb.clamp(*MAX_RESPONSE_MB_RANGE.start(), *MAX_RESPONSE_MB_RANGE.end()))
        .unwrap_or(defaults.max_response_mb);
    let log_level = table
        .and_then(|table| table.get("log_level"))
        .and_then(toml::Value::as_str)
        .and_then(LogLevel::parse)
        .unwrap_or(defaults.log_level);

    let check_updates = table
        .and_then(|table| table.get("check_updates"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.check_updates);
    let show_welcome = table
        .and_then(|table| table.get("show_welcome"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.show_welcome);
    let show_whats_new = table
        .and_then(|table| table.get("show_whats_new"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.show_whats_new);
    let git_executable = table
        .and_then(|table| table.get("git_executable"))
        .and_then(toml::Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from);
    let git_fetch_automatically = table
        .and_then(|table| table.get("git_fetch_automatically"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.git_fetch_automatically);
    let git_pull = table
        .and_then(|table| table.get("git_pull"))
        .and_then(toml::Value::as_str)
        .and_then(GitPull::parse)
        .unwrap_or(defaults.git_pull);
    let git_commit_action = table
        .and_then(|table| table.get("git_commit_action"))
        .and_then(toml::Value::as_str)
        .and_then(GitCommitAction::parse)
        .unwrap_or(defaults.git_commit_action);
    let git_block_secrets = table
        .and_then(|table| table.get("git_block_secrets"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.git_block_secrets);
    let git_confirm_discard = table
        .and_then(|table| table.get("git_confirm_discard"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(defaults.git_confirm_discard);
    let git_diff_mode = table
        .and_then(|table| table.get("git_diff_mode"))
        .and_then(toml::Value::as_str)
        .and_then(GitDiffMode::parse)
        .unwrap_or(defaults.git_diff_mode);
    let git_favorite_branches = table
        .and_then(|table| table.get("git_favorite_branches"))
        .and_then(toml::Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(toml::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Settings {
        language,
        theme,
        ui_font,
        ui_font_size,
        mono_font,
        mono_font_size,
        json_indent,
        invalid_tls_certificates,
        system_certificates,
        max_response_mb,
        log_level,
        check_updates,
        show_welcome,
        show_whats_new,
        git_executable,
        git_fetch_automatically,
        git_pull,
        git_commit_action,
        git_block_secrets,
        git_confirm_discard,
        git_diff_mode,
        git_favorite_branches,
    }
}

/// Reads a TOML value as an `f32`, accepting both a float and an integer.
fn as_f32(value: &toml::Value) -> Option<f32> {
    value
        .as_float()
        .map(|value| value as f32)
        .or_else(|| value.as_integer().map(|value| value as f32))
}

/// Writes `settings` to `<base>/settings.toml`, creating the folder if needed. Split out
/// from [`save_settings`] for the same testing reason as [`read_settings`].
fn write_settings(base: &Path, settings: &Settings) {
    let path = base.join(SETTINGS_FILE);
    let Some(parent) = path.parent() else {
        return;
    };
    if let Err(error) = fs::create_dir_all(parent) {
        log::warn!("could not create {}: {error}", parent.display());
        return;
    }
    let text = match toml::to_string_pretty(settings) {
        Ok(text) => text,
        Err(error) => {
            log::warn!("could not serialize the settings: {error}");
            return;
        }
    };
    if let Err(error) = fs::write(&path, text) {
        log::warn!("could not save {}: {error}", path.display());
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn defaults_are_the_documented_values() {
        let settings = Settings::default();
        assert_eq!(settings.language, LanguageChoice::Auto);
        assert_eq!(settings.theme, ThemeChoice::System);
        assert_eq!(settings.ui_font, "Geist");
        assert_eq!(settings.ui_font_size, 13.0);
        assert_eq!(settings.mono_font, "Geist Mono");
        assert_eq!(settings.mono_font_size, 12.5);
        assert_eq!(settings.json_indent, 2);
        assert_eq!(
            settings.invalid_tls_certificates,
            InvalidTlsCertificates::Warn
        );
        assert!(settings.system_certificates);
        assert_eq!(settings.max_response_mb, 20);
        assert_eq!(settings.log_level, LogLevel::Info);
        assert!(settings.check_updates);
        assert!(settings.show_welcome);
        assert_eq!(settings.git_executable, None);
        assert!(settings.git_fetch_automatically);
        assert_eq!(settings.git_pull, GitPull::FollowConfig);
        assert_eq!(settings.git_commit_action, GitCommitAction::Commit);
        assert!(settings.git_block_secrets);
        assert!(settings.git_confirm_discard);
    }

    #[test]
    fn git_settings_parse_and_fall_back_one_by_one() {
        let settings = parse_settings(
            "git_executable = \"/opt/git/bin/git\"\ngit_fetch_automatically = false\n\
             git_pull = \"REBASE\"\ngit_commit_action = \"commit_and_push\"\n\
             git_block_secrets = false\ngit_confirm_discard = false\n",
        );
        assert_eq!(
            settings.git_executable,
            Some(PathBuf::from("/opt/git/bin/git"))
        );
        assert!(!settings.git_fetch_automatically);
        assert_eq!(settings.git_pull, GitPull::Rebase);
        assert_eq!(settings.git_commit_action, GitCommitAction::CommitAndPush);
        assert!(!settings.git_block_secrets);
        assert!(!settings.git_confirm_discard);

        let settings = parse_settings(
            "git_executable = \"\"\ngit_pull = \"squash\"\ngit_commit_action = 3\n\
             git_block_secrets = \"no\"\n",
        );
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn auto_detected_git_is_not_written_to_the_file() {
        let text = toml::to_string_pretty(&Settings::default()).expect("serialize");
        assert!(!text.contains("git_executable"));
        assert!(text.contains("git_pull = \"follow_config\""));
    }

    #[test]
    fn missing_file_reads_as_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(read_settings(dir.path()), Settings::default());
    }

    #[test]
    fn round_trips_through_a_config_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = Settings {
            language: LanguageChoice::Gl,
            theme: ThemeChoice::Dark,
            ui_font: "Inter".to_string(),
            ui_font_size: 14.0,
            mono_font: "Fira Code".to_string(),
            mono_font_size: 13.5,
            json_indent: 4,
            invalid_tls_certificates: InvalidTlsCertificates::Reject,
            system_certificates: false,
            max_response_mb: 100,
            log_level: LogLevel::Trace,
            check_updates: false,
            show_welcome: true,
            show_whats_new: false,
            git_executable: Some(PathBuf::from("/usr/local/bin/git")),
            git_fetch_automatically: false,
            git_pull: GitPull::Rebase,
            git_commit_action: GitCommitAction::CommitAndPush,
            git_block_secrets: false,
            git_confirm_discard: false,
            git_diff_mode: GitDiffMode::Unified,
            git_favorite_branches: vec!["main".to_string(), "origin/release/0.4".to_string()],
        };

        write_settings(dir.path(), &settings);

        assert_eq!(read_settings(dir.path()), settings);
    }

    #[test]
    fn invalid_toml_reads_as_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join(SETTINGS_FILE), "not valid toml {{{").expect("write file");

        assert_eq!(read_settings(dir.path()), Settings::default());
    }

    #[test]
    fn an_invalid_field_falls_back_on_its_own_without_discarding_the_rest() {
        let settings = parse_settings("theme = \"dark\"\nui_font_size = \"not a number\"\n");
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.ui_font_size, Settings::default().ui_font_size);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let settings = parse_settings("theme = \"light\"\nsome_future_field = 42\n");
        assert_eq!(settings.theme, ThemeChoice::Light);
    }

    #[test]
    fn ui_font_size_is_clamped_to_its_range() {
        assert_eq!(parse_settings("ui_font_size = 3\n").ui_font_size, 11.0);
        assert_eq!(parse_settings("ui_font_size = 999\n").ui_font_size, 16.0);
    }

    #[test]
    fn mono_font_size_is_clamped_to_its_range() {
        assert_eq!(parse_settings("mono_font_size = 1\n").mono_font_size, 10.0);
        assert_eq!(
            parse_settings("mono_font_size = 999\n").mono_font_size,
            18.0
        );
    }

    #[test]
    fn theme_choice_parses_case_insensitively() {
        assert_eq!(ThemeChoice::parse("DARK"), Some(ThemeChoice::Dark));
        assert_eq!(ThemeChoice::parse("Light"), Some(ThemeChoice::Light));
        assert_eq!(ThemeChoice::parse("nonsense"), None);
    }

    #[test]
    fn json_indent_accepts_only_its_options() {
        let parse = |text: &str| parse_settings(text).json_indent;
        assert_eq!(parse(""), 2);
        assert_eq!(parse("json_indent = 4\n"), 4);
        assert_eq!(parse("json_indent = 2\n"), 2);
        assert_eq!(parse("json_indent = 3\n"), 2);
        assert_eq!(parse("json_indent = -4\n"), 2);
        assert_eq!(parse("json_indent = \"4\"\n"), 2);
        let settings = parse_settings("json_indent = 8\ntheme = \"dark\"\n");
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert_eq!(settings.json_indent, 2);
    }

    #[test]
    fn max_response_bytes_uses_decimal_megabytes() {
        let settings = Settings {
            max_response_mb: 20,
            ..Settings::default()
        };
        assert_eq!(settings.max_response_bytes(), 20_000_000);
        assert_eq!(max_response_label(20), "20 MB");
    }

    #[test]
    fn max_response_mb_is_read_and_clamped() {
        let parse = |text: &str| parse_settings(text).max_response_mb;
        assert_eq!(parse("max_response_mb = 50\n"), 50);
        assert_eq!(
            parse("max_response_mb = 35\n"),
            35,
            "a hand written value is kept"
        );
        assert_eq!(parse("max_response_mb = 0\n"), 1);
        assert_eq!(parse("max_response_mb = 99999\n"), 1000);
    }

    #[test]
    fn max_response_mb_falls_back_to_the_default() {
        let parse = |text: &str| parse_settings(text).max_response_mb;
        assert_eq!(parse(""), 20);
        assert_eq!(parse("max_response_mb = -5\n"), 20);
        assert_eq!(parse("max_response_mb = \"big\"\n"), 20);
        assert_eq!(parse("max_response_mb = 2.5\n"), 20);
    }

    #[test]
    fn invalid_tls_certificates_parses_every_value() {
        let parse = |text: &str| parse_settings(text).invalid_tls_certificates;
        assert_eq!(
            parse("invalid_tls_certificates = \"warn\"\n"),
            InvalidTlsCertificates::Warn
        );
        assert_eq!(
            parse("invalid_tls_certificates = \"reject\"\n"),
            InvalidTlsCertificates::Reject
        );
        assert_eq!(
            parse("invalid_tls_certificates = \"ACCEPT\"\n"),
            InvalidTlsCertificates::Accept
        );
    }

    #[test]
    fn invalid_tls_certificates_falls_back_to_warn() {
        let parse = |text: &str| parse_settings(text).invalid_tls_certificates;
        assert_eq!(parse(""), InvalidTlsCertificates::Warn);
        assert_eq!(
            parse("invalid_tls_certificates = \"nonsense\"\n"),
            InvalidTlsCertificates::Warn
        );
        assert_eq!(
            parse("invalid_tls_certificates = 3\n"),
            InvalidTlsCertificates::Warn
        );
    }

    #[test]
    fn invalid_tls_certificates_serializes_as_its_toml_value() {
        let settings = Settings {
            invalid_tls_certificates: InvalidTlsCertificates::Accept,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(
            text.contains("invalid_tls_certificates = \"accept\""),
            "{text}"
        );
    }

    #[test]
    fn invalid_tls_certificates_maps_to_the_http_modes() {
        assert_eq!(
            InvalidTlsCertificates::Warn.to_http(),
            InvalidCertificates::SendWithWarning
        );
        assert_eq!(
            InvalidTlsCertificates::Reject.to_http(),
            InvalidCertificates::Reject
        );
        assert_eq!(
            InvalidTlsCertificates::Accept.to_http(),
            InvalidCertificates::Accept
        );
    }

    #[test]
    fn system_certificates_default_to_on_and_round_trip_by_name() {
        let parse = |text: &str| parse_settings(text).system_certificates;
        assert!(parse(""));
        assert!(parse("system_tls_certificates = \"no\"\n"));
        assert!(!parse("system_tls_certificates = false\n"));
        let settings = Settings {
            system_certificates: false,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(text.contains("system_tls_certificates = false"), "{text}");
    }

    #[test]
    fn log_level_parses_every_value_and_falls_back_to_info() {
        let parse = |text: &str| parse_settings(text).log_level;
        for level in LogLevel::ALL {
            let text = format!("log_level = \"{level:?}\"\n");
            assert_eq!(parse(&text), level);
        }
        assert_eq!(parse(""), LogLevel::Info);
        assert_eq!(parse("log_level = \"loud\"\n"), LogLevel::Info);
        assert_eq!(LogLevel::parse(" warning "), Some(LogLevel::Warn));
    }

    #[test]
    fn check_updates_defaults_to_on_and_falls_back_per_field() {
        let parse = |text: &str| parse_settings(text).check_updates;
        assert!(parse(""));
        assert!(!parse("check_updates = false\n"));
        assert!(parse("check_updates = \"no\"\n"));
        let settings = parse_settings("theme = \"dark\"\ncheck_updates = 3\n");
        assert_eq!(settings.theme, ThemeChoice::Dark);
        assert!(settings.check_updates);
    }

    #[test]
    fn show_welcome_defaults_to_on_and_falls_back_per_field() {
        let parse = |text: &str| parse_settings(text).show_welcome;
        assert!(parse(""));
        assert!(!parse("show_welcome = false\n"));
        assert!(parse("show_welcome = \"no\"\n"));
        let settings = parse_settings("show_welcome = 0\ntheme = \"light\"\n");
        assert_eq!(settings.theme, ThemeChoice::Light);
        assert!(settings.show_welcome);
    }

    #[test]
    fn show_whats_new_defaults_to_on_and_falls_back_per_field() {
        let parse = |text: &str| parse_settings(text).show_whats_new;
        assert!(parse(""));
        assert!(!parse("show_whats_new = false\n"));
        assert!(parse("show_whats_new = 1\n"));
    }

    #[test]
    fn check_updates_serializes_as_a_toml_bool() {
        let settings = Settings {
            check_updates: false,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(text.contains("check_updates = false"), "{text}");
    }

    #[test]
    fn log_level_serializes_as_its_toml_value() {
        let settings = Settings {
            log_level: LogLevel::Debug,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(text.contains("log_level = \"debug\""), "{text}");
    }

    #[test]
    fn categories_start_on_appearance() {
        assert_eq!(SettingsCategory::default(), SettingsCategory::Appearance);
        assert_eq!(SettingsCategory::Requests.label(), "Requests");
    }

    #[test]
    fn language_parses_every_value_and_falls_back_to_auto() {
        let parse = |text: &str| parse_settings(text).language;
        assert_eq!(parse("language = \"es\"\n"), LanguageChoice::Es);
        assert_eq!(parse("language = \"IT\"\n"), LanguageChoice::It);
        assert_eq!(parse("language = \"auto\"\n"), LanguageChoice::Auto);
        assert_eq!(parse(""), LanguageChoice::Auto);
        assert_eq!(parse("language = \"klingon\"\n"), LanguageChoice::Auto);
        assert_eq!(parse("language = 3\n"), LanguageChoice::Auto);
    }

    #[test]
    fn language_serializes_as_its_toml_value() {
        let settings = Settings {
            language: LanguageChoice::Gl,
            ..Settings::default()
        };
        let text = toml::to_string_pretty(&settings).expect("serialize");
        assert!(text.contains("language = \"gl\""), "{text}");
    }

    #[test]
    fn labels_are_translated_with_the_requested_locale() {
        assert_eq!(t!("common.requests", locale = "es"), "Peticiones");
        assert_eq!(t!("settings.tls.reject", locale = "gl"), "Rexeitar");
    }

    #[test]
    fn font_options_puts_the_bundled_family_first() {
        let installed = vec![
            "Arial".to_string(),
            "Geist".to_string(),
            "Fira Code".to_string(),
        ];
        assert_eq!(
            font_options("Geist", &installed),
            vec![
                "Geist".to_string(),
                "Arial".to_string(),
                "Fira Code".to_string()
            ]
        );
    }

    #[test]
    fn font_options_deduplicates() {
        let installed = vec!["Arial".to_string(), "Arial".to_string()];
        assert_eq!(
            font_options("Geist", &installed),
            vec!["Geist".to_string(), "Arial".to_string()]
        );
    }
}
