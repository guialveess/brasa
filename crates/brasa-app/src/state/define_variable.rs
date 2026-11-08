//! Plain logic behind defining an unknown variable from a request (a red chip, or the response's
//! "Define" action): where the user is taken, and the secret-name heuristic that puts a new
//! row in `.local.env`. `views/define_variable.rs` carries it out; this module holds only what can
//! be unit tested without a window.

/// Case-insensitive substrings that mark a variable name as likely sensitive. A new row for a
/// variable whose name contains one of these is stored in `.local.env` (not versioned).
const SENSITIVE_NEEDLES: [&str; 4] = ["token", "secret", "password", "key"];

/// Whether `name` looks like it holds a secret, by a case-insensitive substring check against
/// [`SENSITIVE_NEEDLES`].
pub fn looks_sensitive(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SENSITIVE_NEEDLES
        .iter()
        .any(|needle| lower.contains(needle))
}

/// Where defining a variable takes the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefineTarget {
    /// No environment is active: the Environments panel, to pick or create one.
    ChooseEnvironment,
    /// The editor tab of the active environment, on the variable's row.
    Environment(String),
}

/// The target for the `active` environment, among the workspace's `environments`. An active
/// environment whose files are gone counts as none.
pub fn target(active: Option<&str>, environments: &[String]) -> DefineTarget {
    match active {
        Some(active) if environments.iter().any(|name| name == active) => {
            DefineTarget::Environment(active.to_string())
        }
        _ => DefineTarget::ChooseEnvironment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn recognizes_every_sensitive_needle_case_insensitively() {
        assert!(looks_sensitive("authToken"));
        assert!(looks_sensitive("API_SECRET"));
        assert!(looks_sensitive("password"));
        assert!(looks_sensitive("apiKey"));
    }

    #[test]
    fn an_ordinary_name_is_not_sensitive() {
        assert!(!looks_sensitive("baseUrl"));
        assert!(!looks_sensitive("userId"));
    }

    #[test]
    fn the_active_environment_is_the_target() {
        let environments = vec!["local".to_string(), "prod".to_string()];
        assert_eq!(
            target(Some("prod"), &environments),
            DefineTarget::Environment("prod".to_string())
        );
    }

    #[test]
    fn without_an_active_environment_the_user_chooses_one() {
        let environments = vec!["local".to_string()];
        assert_eq!(target(None, &environments), DefineTarget::ChooseEnvironment);
        assert_eq!(target(None, &[]), DefineTarget::ChooseEnvironment);
    }

    #[test]
    fn an_active_environment_that_no_longer_exists_counts_as_none() {
        let environments = vec!["local".to_string()];
        assert_eq!(
            target(Some("stale"), &environments),
            DefineTarget::ChooseEnvironment
        );
    }
}
