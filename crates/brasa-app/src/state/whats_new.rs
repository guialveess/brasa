//! The What's new tab (#189): the release notes of the running version, embedded at build time
//! from `release-notes/X.Y.Z.md` (`build.rs`), the same file the GitHub release is created
//! from. It opens by itself once after an update, never on a first install nor on a pre-release
//! build. Plain Rust, unit tested without a window. `views/whats_new.rs` renders it.

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/release_notes.rs"));
}

/// The release notes of this version, `None` when `release-notes/X.Y.Z.md` does not exist (a
/// development build before the notes are written).
pub const NOTES: Option<&str> = embedded::NOTES;

/// The running version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The id of the What's new tab. Never a request id: those end in `.brasa`.
pub const TAB_ID: &str = "whats-new";

/// The last version that ran, as `state::config::last_seen_version` reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LastSeen {
    /// Brasa never ran here: no settings, no recent workspaces, no recorded version.
    FirstRun,
    /// Brasa ran before, but a version older than 0.6.0 that did not record itself.
    Unrecorded,
    /// The last stable version that ran.
    Version(String),
}

/// Whether the What's new tab opens at startup: only for a stable version newer than the last
/// one that ran, with notes to show, when `enabled` (the `show_whats_new` setting) is on.
pub fn opens_at_startup(current: &str, last: &LastSeen, enabled: bool, has_notes: bool) -> bool {
    let Some(current) = stable(current) else {
        return false;
    };
    if !enabled || !has_notes {
        return false;
    }
    match last {
        LastSeen::FirstRun => false,
        LastSeen::Unrecorded => true,
        LastSeen::Version(seen) => stable(seen).is_none_or(|seen| current > seen),
    }
}

/// Whether `current` is recorded as the last version that ran: only stable versions, so a
/// release candidate never hides the notes of the release that follows it.
pub fn records(current: &str) -> bool {
    stable(current).is_some()
}

/// `X.Y.Z` as numbers, `None` for a pre-release (`0.6.0-rc.1`, `0.6.0-dev`) or anything else.
fn stable(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.trim().split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let parsed = (next()?, next()?, next()?);
    parts.next().is_none().then_some(parsed)
}

/// The GitHub release page of `version`.
pub fn release_url(repository: &str, version: &str) -> String {
    format!("{repository}/releases/tag/v{version}")
}

/// `markdown` with every issue reference (`#123`) turned into a link to it, except inside code
/// and where it is already a link (`[#123](...)`). GitHub sends `/issues/N` of a pull request on
/// to the pull request.
pub fn link_issues(markdown: &str, repository: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fenced = false;
    for line in markdown.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            out.push_str(line);
            continue;
        }
        if fenced {
            out.push_str(line);
            continue;
        }
        let mut in_code = false;
        let mut previous: Option<char> = None;
        let mut chars = line.char_indices().peekable();
        while let Some((index, character)) = chars.next() {
            if character == '`' {
                in_code = !in_code;
            }
            let starts_reference = character == '#'
                && !in_code
                && !previous.is_some_and(|before| before.is_alphanumeric() || before == '[')
                && chars.peek().is_some_and(|(_, next)| next.is_ascii_digit());
            if starts_reference {
                let digits: String = line[index + 1..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                let rest = &line[index + 1 + digits.len()..];
                if !rest.starts_with(|next: char| next.is_alphanumeric()) {
                    out.push_str(&format!("[#{digits}]({repository}/issues/{digits})"));
                    for _ in 0..digits.len() {
                        chars.next();
                    }
                    previous = digits.chars().last();
                    continue;
                }
            }
            out.push(character);
            previous = Some(character);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const REPO: &str = "https://github.com/tanisperez/brasa";

    #[test]
    fn opens_only_after_an_update_to_a_stable_version() {
        let seen = |version: &str| LastSeen::Version(version.to_string());
        assert!(opens_at_startup("0.6.0", &seen("0.5.0"), true, true));
        assert!(opens_at_startup("1.0.0", &seen("0.10.3"), true, true));
        assert!(opens_at_startup("0.6.0", &LastSeen::Unrecorded, true, true));
        assert!(
            !opens_at_startup("0.6.0", &seen("0.6.0"), true, true),
            "same version"
        );
        assert!(
            !opens_at_startup("0.5.0", &seen("0.6.0"), true, true),
            "a downgrade"
        );
        assert!(
            !opens_at_startup("0.6.0", &LastSeen::FirstRun, true, true),
            "first install"
        );
        assert!(
            !opens_at_startup("0.6.0", &seen("0.5.0"), false, true),
            "turned off"
        );
        assert!(
            !opens_at_startup("0.6.0", &seen("0.5.0"), true, false),
            "no notes"
        );
        assert!(
            !opens_at_startup("0.6.0-rc.1", &seen("0.5.0"), true, true),
            "an rc"
        );
        assert!(
            !opens_at_startup("0.6.0-dev", &seen("0.5.0"), true, true),
            "a dev build"
        );
        assert!(opens_at_startup("0.6.0", &seen("garbage"), true, true));
    }

    #[test]
    fn only_stable_versions_are_recorded() {
        assert!(records("0.6.0"));
        assert!(!records("0.6.0-rc.2"));
        assert!(!records("0.6.0-dev"));
        assert!(!records("0.6"));
        assert!(!records("0.6.0.1"));
    }

    #[test]
    fn a_stable_build_has_its_release_notes() {
        // Fails the release build when `release-notes/X.Y.Z.md` was not written.
        if records(VERSION) {
            assert!(NOTES.is_some(), "release-notes/{VERSION}.md is missing");
        }
    }

    #[test]
    fn issue_references_become_links() {
        assert_eq!(
            link_issues("- **Log** (#174, #186): history.\n", REPO),
            format!("- **Log** ([#174]({REPO}/issues/174), [#186]({REPO}/issues/186)): history.\n")
        );
        assert_eq!(
            link_issues("Fixed #12.", REPO),
            format!("Fixed [#12]({REPO}/issues/12).")
        );
    }

    #[test]
    fn code_headings_and_links_keep_their_hashes() {
        let text = "### Git\n`#12` and C#1 and #12a and [#7](x)\n```sh\necho #3\n```\n#4\n";
        assert_eq!(
            link_issues(text, REPO),
            format!(
                "### Git\n`#12` and C#1 and #12a and [#7](x)\n```sh\necho #3\n```\n[#4]({REPO}/issues/4)\n"
            )
        );
    }

    #[test]
    fn the_release_page_is_the_version_tag() {
        assert_eq!(
            release_url(REPO, "0.6.0"),
            format!("{REPO}/releases/tag/v0.6.0")
        );
    }
}
