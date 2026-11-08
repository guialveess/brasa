//! Parses the `BRASA_OPEN` debug hook's value, the same style as `views/root.rs`'s existing
//! `apply_debug_autosend`: a hidden hook to screenshot a specific UI state at
//! startup, not a supported feature, not surfaced in any menu.

/// A UI state `BRASA_OPEN` can request at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugOpenTarget {
    /// Renders the components gallery in the main area, to check every widget
    /// against `docs/design-system.md`.
    Components,
    /// Opens the Settings modal, to check it against `docs/design-system.md`.
    Settings,
    /// Opens the Settings modal on its "Requests" pane.
    SettingsRequests,
    /// Opens the Settings modal on its "Advanced" pane.
    SettingsAdvanced,
    /// Opens the Settings modal on its "About" pane.
    SettingsAbout,
    /// Opens the command palette.
    Palette,
    /// Opens the Code snippet dialog for the active tab's request. A no-op (like every
    /// `BRASA_OPEN` target with nothing to show) when no tab is open.
    Snippet,
    /// Opens a load test tab preselecting the first request found in the workspace tree, for
    /// screenshotting the load test config panel. A no-op when no workspace is open or it has no
    /// requests.
    LoadTest,
}

/// Parses `BRASA_OPEN`'s value into a [`DebugOpenTarget`]. An unrecognized value is `None`
/// (silently ignored, same convention as the rest of `BRASA_OPEN`'s callers).
pub fn parse(value: &str) -> Option<DebugOpenTarget> {
    match value {
        "components" => Some(DebugOpenTarget::Components),
        "settings" => Some(DebugOpenTarget::Settings),
        "settings-requests" => Some(DebugOpenTarget::SettingsRequests),
        "settings-advanced" => Some(DebugOpenTarget::SettingsAdvanced),
        "settings-about" => Some(DebugOpenTarget::SettingsAbout),
        "palette" => Some(DebugOpenTarget::Palette),
        "snippet" => Some(DebugOpenTarget::Snippet),
        "loadtest" => Some(DebugOpenTarget::LoadTest),
        _ => None,
    }
}

/// The environment name of a `BRASA_OPEN=env:<name>` value, which opens that environment's
/// editor tab. Separate from [`parse`] because it carries a name.
pub fn parse_env(value: &str) -> Option<&str> {
    value.strip_prefix("env:").filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_an_environment_target() {
        assert_eq!(parse_env("env:local"), Some("local"));
        assert_eq!(parse_env("env:"), None);
        assert_eq!(parse_env("settings"), None);
    }

    #[test]
    fn parses_components() {
        assert_eq!(parse("components"), Some(DebugOpenTarget::Components));
    }

    #[test]
    fn parses_settings() {
        assert_eq!(parse("settings"), Some(DebugOpenTarget::Settings));
    }

    #[test]
    fn parses_settings_requests() {
        assert_eq!(
            parse("settings-requests"),
            Some(DebugOpenTarget::SettingsRequests)
        );
    }

    #[test]
    fn parses_settings_advanced() {
        assert_eq!(
            parse("settings-advanced"),
            Some(DebugOpenTarget::SettingsAdvanced)
        );
    }

    #[test]
    fn parses_settings_about() {
        assert_eq!(
            parse("settings-about"),
            Some(DebugOpenTarget::SettingsAbout)
        );
    }

    #[test]
    fn parses_palette() {
        assert_eq!(parse("palette"), Some(DebugOpenTarget::Palette));
    }

    #[test]
    fn parses_snippet() {
        assert_eq!(parse("snippet"), Some(DebugOpenTarget::Snippet));
    }

    #[test]
    fn parses_loadtest() {
        assert_eq!(parse("loadtest"), Some(DebugOpenTarget::LoadTest));
    }

    #[test]
    fn unrecognized_value_is_none() {
        assert_eq!(parse("load-test"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("Components"), None);
    }
}
