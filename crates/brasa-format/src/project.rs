//! The workspace configuration file, `.brasa/project.toml` (#215): settings that belong to the
//! workspace and are shared with it through git, unlike the user's own `settings.toml`. Today the
//! order of the environments (#216) and their colors (#217).
//!
//! The file is optional: a workspace without it behaves as if it were empty. [`ProjectConfig`]
//! keeps the whole TOML table it was parsed from, so keys this version does not know (written by
//! a newer Brasa, or by hand) survive a save.

use toml::{Table, Value};

/// The path of the file, relative to the workspace root, with `/` separators.
pub const PROJECT_FILE: &str = ".brasa/project.toml";

/// The format version written in new files.
pub const PROJECT_VERSION: i64 = 1;

/// The comment on top of the file.
const HEADER: &str = "# Brasa workspace configuration, shared with everyone who uses this workspace.\n\n";

/// The table of the environment settings.
const ENVIRONMENTS: &str = "environments";
/// The key of the environment order, inside [`ENVIRONMENTS`].
const ORDER: &str = "order";
/// The table of the environment colors, inside [`ENVIRONMENTS`].
const COLORS: &str = "colors";

/// A `project.toml` that is not valid TOML.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{PROJECT_FILE}: {0}")]
pub struct ProjectError(String);

/// The content of a `project.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectConfig {
    table: Table,
}

impl Default for ProjectConfig {
    /// A new file: only the format version.
    fn default() -> Self {
        let mut table = Table::new();
        table.insert("version".to_string(), Value::Integer(PROJECT_VERSION));
        ProjectConfig { table }
    }
}

impl ProjectConfig {
    /// Parses the text of a `project.toml`. Values of the wrong type are ignored by the getters,
    /// not rejected here, so a hand edit never makes the whole file unreadable.
    pub fn parse(text: &str) -> Result<Self, ProjectError> {
        text.parse::<Table>()
            .map(|table| ProjectConfig { table })
            .map_err(|error| ProjectError(error.message().to_string()))
    }

    /// The text of the file: a short header, then the table with its keys sorted, so the same
    /// configuration always gives the same text.
    pub fn to_toml(&self) -> String {
        let body = toml::to_string(&self.table).unwrap_or_default();
        format!("{HEADER}{body}")
    }

    /// The environment names in the order the user chose, as written. Empty when the user never
    /// reordered them. May name environments that no longer exist.
    pub fn environment_order(&self) -> Vec<String> {
        self.environments()
            .and_then(|environments| environments.get(ORDER))
            .and_then(Value::as_array)
            .map(|order| {
                order
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Sets the order of the environments.
    pub fn set_environment_order(&mut self, order: &[String]) {
        let order = order.iter().cloned().map(Value::String).collect();
        self.update_environments(|environments| {
            environments.insert(ORDER.to_string(), Value::Array(order));
        });
    }

    /// The color chosen for the environment `name`, as written (a color name such as `"teal"`),
    /// `None` when it has none.
    pub fn environment_color(&self, name: &str) -> Option<&str> {
        self.environments()?
            .get(COLORS)?
            .as_table()?
            .get(name)?
            .as_str()
    }

    /// Sets the color of the environment `name`, or removes it with `None`.
    pub fn set_environment_color(&mut self, name: &str, color: Option<&str>) {
        self.update_environments(|environments| match color {
            Some(color) => {
                let colors = environments
                    .entry(COLORS)
                    .or_insert_with(|| Value::Table(Table::new()));
                if !colors.is_table() {
                    *colors = Value::Table(Table::new());
                }
                if let Some(colors) = colors.as_table_mut() {
                    colors.insert(name.to_string(), Value::String(color.to_string()));
                }
            }
            None => {
                if let Some(colors) = environments.get_mut(COLORS).and_then(Value::as_table_mut) {
                    colors.remove(name);
                    if colors.is_empty() {
                        environments.remove(COLORS);
                    }
                }
            }
        });
    }

    /// Follows the rename of an environment: its place in the order and its color move to `new`.
    pub fn rename_environment(&mut self, old: &str, new: &str) {
        let order = self.environment_order();
        if order.iter().any(|name| name == old) {
            let renamed: Vec<String> = order
                .into_iter()
                .map(|name| if name == old { new.to_string() } else { name })
                .collect();
            self.set_environment_order(&renamed);
        }
        if let Some(color) = self.environment_color(old).map(str::to_string) {
            self.set_environment_color(old, None);
            self.set_environment_color(new, Some(&color));
        }
    }

    /// Forgets a deleted environment: its place in the order and its color.
    pub fn remove_environment(&mut self, name: &str) {
        let order = self.environment_order();
        if order.iter().any(|entry| entry == name) {
            let kept: Vec<String> = order.into_iter().filter(|entry| entry != name).collect();
            self.set_environment_order(&kept);
        }
        self.set_environment_color(name, None);
    }

    fn environments(&self) -> Option<&Table> {
        self.table.get(ENVIRONMENTS)?.as_table()
    }

    /// Runs `edit` on the `[environments]` table (a new one when it is missing or not a table),
    /// and drops the table when it ends up empty.
    fn update_environments(&mut self, edit: impl FnOnce(&mut Table)) {
        let mut environments = match self.table.remove(ENVIRONMENTS) {
            Some(Value::Table(table)) => table,
            _ => Table::new(),
        };
        edit(&mut environments);
        if !environments.is_empty() {
            self.table
                .insert(ENVIRONMENTS.to_string(), Value::Table(environments));
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn a_new_file_only_has_the_version() {
        let text = ProjectConfig::default().to_toml();
        assert!(text.starts_with("# Brasa workspace configuration"));
        assert!(text.ends_with("version = 1\n"), "{text}");
        assert_eq!(ProjectConfig::parse(&text), Ok(ProjectConfig::default()));
    }

    #[test]
    fn order_and_colors_round_trip() {
        let mut config = ProjectConfig::default();
        config.set_environment_order(&names(&["local", "dev", "prod"]));
        config.set_environment_color("local", Some("teal"));
        config.set_environment_color("staging env", Some("violet"));
        let text = config.to_toml();
        assert!(
            text.contains("order = [\"local\", \"dev\", \"prod\"]"),
            "{text}"
        );
        assert!(text.contains("local = \"teal\""), "{text}");
        assert!(text.contains("\"staging env\" = \"violet\""), "{text}");

        let parsed = ProjectConfig::parse(&text).expect("parse");
        assert_eq!(parsed.environment_order(), names(&["local", "dev", "prod"]));
        assert_eq!(parsed.environment_color("local"), Some("teal"));
        assert_eq!(parsed.environment_color("staging env"), Some("violet"));
        assert_eq!(parsed.environment_color("prod"), None);
        assert_eq!(parsed.to_toml(), text);
    }

    #[test]
    fn unknown_keys_survive_a_save() {
        let text = "version = 2\nfuture = \"x\"\n\n[environments]\nicons = { dev = \"bug\" }\n";
        let mut config = ProjectConfig::parse(text).expect("parse");
        config.set_environment_color("dev", Some("blue"));
        let saved = ProjectConfig::parse(&config.to_toml()).expect("parse");
        assert_eq!(saved.table["version"].as_integer(), Some(2));
        assert_eq!(saved.table["future"].as_str(), Some("x"));
        assert_eq!(
            saved.environments().expect("environments")["icons"]["dev"].as_str(),
            Some("bug")
        );
        assert_eq!(saved.environment_color("dev"), Some("blue"));
    }

    #[test]
    fn values_of_the_wrong_type_are_ignored() {
        let config = ProjectConfig::parse(
            "[environments]\norder = [\"dev\", 3, \"prod\"]\ncolors = { dev = 5, prod = \"red\" }\n",
        )
        .expect("parse");
        assert_eq!(config.environment_order(), names(&["dev", "prod"]));
        assert_eq!(config.environment_color("dev"), None);
        assert_eq!(config.environment_color("prod"), Some("red"));

        let mut broken = ProjectConfig::parse("environments = \"oops\"\n").expect("parse");
        assert!(broken.environment_order().is_empty());
        broken.set_environment_color("dev", Some("blue"));
        assert_eq!(broken.environment_color("dev"), Some("blue"));
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(ProjectConfig::parse("[environments\n").is_err());
    }

    #[test]
    fn clearing_the_last_color_removes_the_table() {
        let mut config = ProjectConfig::default();
        config.set_environment_color("dev", Some("blue"));
        config.set_environment_color("dev", None);
        assert!(!config.to_toml().contains("colors"));
        // Clearing a color that was never set changes nothing.
        config.set_environment_color("prod", None);
    }

    #[test]
    fn rename_and_remove_follow_the_environment() {
        let mut config = ProjectConfig::default();
        config.set_environment_order(&names(&["local", "dev", "prod"]));
        config.set_environment_color("dev", Some("blue"));

        config.rename_environment("dev", "development");
        assert_eq!(
            config.environment_order(),
            names(&["local", "development", "prod"])
        );
        assert_eq!(config.environment_color("dev"), None);
        assert_eq!(config.environment_color("development"), Some("blue"));

        config.remove_environment("development");
        assert_eq!(config.environment_order(), names(&["local", "prod"]));
        assert_eq!(config.environment_color("development"), None);

        // An environment the file does not mention: nothing to do, and no empty keys appear.
        let mut empty = ProjectConfig::default();
        empty.rename_environment("a", "b");
        empty.remove_environment("a");
        assert_eq!(empty, ProjectConfig::default());
    }
}
