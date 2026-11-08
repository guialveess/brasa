//! The color of an environment: the dot in the title bar's environment pill and menu, the
//! Environments panel, the environment editor and the command palette (see
//! `docs/design-system.md`).
//!
//! Each environment is drawn in one of nine named colors. The user may choose one (#217), saved
//! by name in `.brasa/project.toml`; otherwise it is derived from the name ([`auto_color`]).
//! "No environment" has no color: the views draw a hollow ring for it instead.

use std::collections::HashMap;

use brasa_workspace::Workspace;

/// One of the colors an environment can be drawn in. Each has a light and a dark value in
/// `theme::palette` (`BrasaColors::env_color`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvColor {
    /// The `danger` token. Automatic for names with "prod".
    Red,
    /// A color of Brasa's own.
    Orange,
    /// The `warning` token. Automatic for names with "stag", "test", "qa" or "uat".
    Amber,
    /// The `success` token. Automatic for every other name.
    Green,
    /// A color of Brasa's own.
    Teal,
    /// The `info` token.
    Blue,
    /// The PATCH method's color.
    Violet,
    /// A color of Brasa's own.
    Pink,
    /// A color of Brasa's own.
    Grey,
}

impl EnvColor {
    /// Every color, in the order the color picker shows them.
    pub const ALL: [EnvColor; 9] = [
        EnvColor::Red,
        EnvColor::Orange,
        EnvColor::Amber,
        EnvColor::Green,
        EnvColor::Teal,
        EnvColor::Blue,
        EnvColor::Violet,
        EnvColor::Pink,
        EnvColor::Grey,
    ];

    /// The name written in `.brasa/project.toml`.
    pub fn key(self) -> &'static str {
        match self {
            EnvColor::Red => "red",
            EnvColor::Orange => "orange",
            EnvColor::Amber => "amber",
            EnvColor::Green => "green",
            EnvColor::Teal => "teal",
            EnvColor::Blue => "blue",
            EnvColor::Violet => "violet",
            EnvColor::Pink => "pink",
            EnvColor::Grey => "grey",
        }
    }

    /// The color named `key` in the file, ignoring ASCII case. `None` for an unknown name, which
    /// leaves the environment on its automatic color.
    pub fn from_key(key: &str) -> Option<EnvColor> {
        EnvColor::ALL
            .into_iter()
            .find(|color| color.key().eq_ignore_ascii_case(key.trim()))
    }

    /// The color nearest to a CSS hex color (`#4caf50`, `#4CAF50`, `4caf50` or `#fff`), for the
    /// colors other apps give their environments (a Bruno import). By hue, and grey for colors
    /// with little saturation. `None` for text that is not a hex color.
    pub fn nearest(hex: &str) -> Option<EnvColor> {
        let digits = hex.trim().trim_start_matches('#');
        let digits: String = match digits.len() {
            3 => digits.chars().flat_map(|digit| [digit, digit]).collect(),
            6 | 8 => digits[..6].to_string(),
            _ => return None,
        };
        let channel = |at: usize| {
            u8::from_str_radix(digits.get(at..at + 2)?, 16)
                .ok()
                .map(|value| f32::from(value) / 255.0)
        };
        let (red, green, blue) = (channel(0)?, channel(2)?, channel(4)?);
        let max = red.max(green).max(blue);
        let min = red.min(green).min(blue);
        let lightness = (max + min) / 2.0;
        let delta = max - min;
        let saturation = if delta == 0.0 {
            0.0
        } else {
            delta / (1.0 - (2.0 * lightness - 1.0).abs())
        };
        if saturation < 0.15 || delta < 0.08 {
            return Some(EnvColor::Grey);
        }
        let hue = if max == red {
            60.0 * ((green - blue) / delta).rem_euclid(6.0)
        } else if max == green {
            60.0 * ((blue - red) / delta + 2.0)
        } else {
            60.0 * ((red - green) / delta + 4.0)
        };
        Some(match hue {
            hue if !(15.0..345.0).contains(&hue) => EnvColor::Red,
            hue if hue < 38.0 => EnvColor::Orange,
            hue if hue < 65.0 => EnvColor::Amber,
            hue if hue < 160.0 => EnvColor::Green,
            hue if hue < 195.0 => EnvColor::Teal,
            hue if hue < 255.0 => EnvColor::Blue,
            hue if hue < 310.0 => EnvColor::Violet,
            _ => EnvColor::Pink,
        })
    }

    /// The translation key of the color's name, shown in the picker's tooltips.
    pub fn label_key(self) -> &'static str {
        match self {
            EnvColor::Red => "environments.color.red",
            EnvColor::Orange => "environments.color.orange",
            EnvColor::Amber => "environments.color.amber",
            EnvColor::Green => "environments.color.green",
            EnvColor::Teal => "environments.color.teal",
            EnvColor::Blue => "environments.color.blue",
            EnvColor::Violet => "environments.color.violet",
            EnvColor::Pink => "environments.color.pink",
            EnvColor::Grey => "environments.color.grey",
        }
    }
}

/// The name fragments, checked case-insensitively, that make an environment amber.
const WARNING_NEEDLES: [&str; 4] = ["stag", "test", "qa", "uat"];

/// The automatic color of an environment, from its name, case-insensitive: red for "prod",
/// amber for "stag", "test", "qa" or "uat", green for anything else.
pub fn auto_color(name: &str) -> EnvColor {
    let lower = name.to_ascii_lowercase();
    if lower.contains("prod") {
        EnvColor::Red
    } else if WARNING_NEEDLES.iter().any(|needle| lower.contains(needle)) {
        EnvColor::Amber
    } else {
        EnvColor::Green
    }
}

/// The colors the user chose, by environment name, as read from `.brasa/project.toml`.
/// Environments without an entry use [`auto_color`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvColors {
    chosen: HashMap<String, EnvColor>,
}

impl EnvColors {
    /// Reads the chosen colors of `names` from the workspace configuration. A file that cannot
    /// be read, or a color name Brasa does not know, leaves the automatic colors. File IO:
    /// call it when the environments are loaded, never from render.
    pub fn load(workspace: &Workspace, names: &[String]) -> Self {
        let config = match workspace.load_project() {
            Ok(config) => config,
            Err(error) => {
                log::warn!("ignoring the environment colors: {error}");
                return Self::default();
            }
        };
        let chosen = names
            .iter()
            .filter_map(|name| {
                let key = config.environment_color(name)?;
                match EnvColor::from_key(key) {
                    Some(color) => Some((name.clone(), color)),
                    None => {
                        log::warn!("unknown color {key:?} for environment {name:?}, ignoring it");
                        None
                    }
                }
            })
            .collect();
        EnvColors { chosen }
    }

    /// The color `name` is drawn in: the chosen one, or its automatic color.
    pub fn color(&self, name: &str) -> EnvColor {
        self.chosen(name).unwrap_or_else(|| auto_color(name))
    }

    /// The color the user chose for `name`, `None` when it is automatic.
    pub fn chosen(&self, name: &str) -> Option<EnvColor> {
        self.chosen.get(name).copied()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_workspace::ProjectWrite;
    use pretty_assertions::assert_eq;

    #[test]
    fn production_is_red() {
        assert_eq!(auto_color("production"), EnvColor::Red);
        assert_eq!(auto_color("PROD"), EnvColor::Red);
    }

    #[test]
    fn staging_test_qa_and_uat_are_amber() {
        assert_eq!(auto_color("staging"), EnvColor::Amber);
        assert_eq!(auto_color("Test"), EnvColor::Amber);
        assert_eq!(auto_color("qa-env"), EnvColor::Amber);
        assert_eq!(auto_color("UAT"), EnvColor::Amber);
    }

    #[test]
    fn anything_else_is_green() {
        assert_eq!(auto_color("local"), EnvColor::Green);
        assert_eq!(auto_color("dev"), EnvColor::Green);
    }

    #[test]
    fn prod_wins_over_warning_needles_when_both_are_present() {
        assert_eq!(auto_color("staging-prod-mirror"), EnvColor::Red);
    }

    #[test]
    fn keys_round_trip_and_ignore_case() {
        for color in EnvColor::ALL {
            assert_eq!(EnvColor::from_key(color.key()), Some(color));
        }
        assert_eq!(EnvColor::from_key(" Teal "), Some(EnvColor::Teal));
        assert_eq!(EnvColor::from_key("gray"), None);
    }

    #[test]
    fn hex_colors_map_to_the_nearest_named_one() {
        for (hex, color) in [
            ("#f44336", EnvColor::Red),
            ("#ff9800", EnvColor::Orange),
            ("#FFC107", EnvColor::Amber),
            ("#4CAF50", EnvColor::Green),
            ("#009688", EnvColor::Teal),
            ("2196f3", EnvColor::Blue),
            ("#9c27b0", EnvColor::Violet),
            ("#e91e63", EnvColor::Pink),
            ("#9e9e9e", EnvColor::Grey),
            ("#fff", EnvColor::Grey),
            ("#000000", EnvColor::Grey),
            ("#4caf50ff", EnvColor::Green),
        ] {
            assert_eq!(EnvColor::nearest(hex), Some(color), "{hex}");
        }
        assert_eq!(EnvColor::nearest("green"), None);
        assert_eq!(EnvColor::nearest("#12"), None);
        assert_eq!(EnvColor::nearest("#zzzzzz"), None);
    }

    #[test]
    fn chosen_colors_come_from_the_project_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workspace = Workspace::open(dir.path()).expect("open");
        let names = vec!["dev".to_string(), "prod".to_string(), "qa".to_string()];

        let colors = EnvColors::load(&workspace, &names);
        assert_eq!(colors.color("prod"), EnvColor::Red);
        assert_eq!(colors.chosen("prod"), None);

        workspace
            .update_project(ProjectWrite::Create, |config| {
                config.set_environment_color("dev", Some("teal"));
                config.set_environment_color("qa", Some("ultraviolet"));
            })
            .expect("update");
        let colors = EnvColors::load(&workspace, &names);
        assert_eq!(colors.color("dev"), EnvColor::Teal);
        assert_eq!(colors.chosen("dev"), Some(EnvColor::Teal));
        // An unknown color name keeps the automatic one.
        assert_eq!(colors.color("qa"), EnvColor::Amber);
        assert_eq!(colors.chosen("qa"), None);
    }
}
