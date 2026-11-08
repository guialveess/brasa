//! Small formatting helpers for paths and dates shown in the UI: home-relative paths and relative
//! day labels.

use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use rust_i18n::t;

/// Number of seconds in a day, used by [`relative_day`] to compare UTC day boundaries.
const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

/// Shortens `path` to start with `~` when it is inside `home`, for example `/home/tanis/dev`
/// with home `/home/tanis` becomes `~/dev`. Returns the path unchanged (as a string) if it is
/// not inside `home`, or if `home` is `None`. Every separator is the platform's own, so on
/// Windows `C:\Users\me\AppData\Roaming` plus `brasa/settings.toml` shows as
/// `~\AppData\Roaming\brasa\settings.toml`, not with a mix of `/` and `\`.
pub fn shorten_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(relative) if relative.as_os_str().is_empty() => "~".to_string(),
        Some(relative) => format!("~{MAIN_SEPARATOR}{}", with_native_separators(relative)),
        None => with_native_separators(path),
    }
}

/// `path` rebuilt from its components, which joins them with the platform's separator. Windows
/// splits components on both `/` and `\`, so this turns a mixed path into a `\` one; elsewhere
/// it only drops redundant separators.
fn with_native_separators(path: &Path) -> String {
    path.components().collect::<PathBuf>().display().to_string()
}

/// Elides the middle of a long `path` with `…` so it fits in `max_chars`, keeping its first
/// component and as many trailing ones as fit, for example `/tmp/…/scratchpad/ws`. A path that
/// already fits is returned unchanged. If even the last component is too long, the start of the
/// whole path is cut instead (`…tail`).
pub fn elide_path(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
        return path.to_string();
    }
    let segments: Vec<&str> = path.split(MAIN_SEPARATOR).collect();
    let head_len = segments
        .iter()
        .position(|segment| !segment.is_empty())
        .map_or(0, |index| index + 1);
    let head = segments[..head_len].join(std::path::MAIN_SEPARATOR_STR);
    let fixed = head.chars().count() + 2;
    let mut tail = String::new();
    for segment in segments[head_len..].iter().rev() {
        let candidate = format!("{MAIN_SEPARATOR}{segment}{tail}");
        if fixed + candidate.chars().count() > max_chars {
            break;
        }
        tail = candidate;
    }
    if tail.is_empty() {
        let keep = max_chars.saturating_sub(1);
        let skip = path.chars().count().saturating_sub(keep);
        return format!("…{}", path.chars().skip(skip).collect::<String>());
    }
    format!("{head}{MAIN_SEPARATOR}…{tail}")
}

/// The label shown in the open-tabs bar for an open request tab: the file stem (its name without
/// the `.brasa` extension), for example `"login"` for `"auth/login.brasa"`. The full id is
/// shown in the tab's tooltip instead.
pub fn tab_label(id: &str) -> &str {
    let name = id.rsplit('/').next().unwrap_or(id);
    name.strip_suffix(".brasa").unwrap_or(name)
}

/// A relative label for a day, comparing UTC day boundaries: `"today"`, `"yesterday"`, or `"N
/// days ago"` for anything older. A `then` on the same UTC day as `now`, or in the future, is
/// also `"today"`.
pub fn relative_day(then_unix_seconds: i64, now_unix_seconds: i64) -> String {
    relative_day_in(then_unix_seconds, now_unix_seconds, &rust_i18n::locale())
}

/// [`relative_day`] in an explicit `locale`, so tests need not touch the process wide one.
fn relative_day_in(then_unix_seconds: i64, now_unix_seconds: i64, locale: &str) -> String {
    let day_difference = now_unix_seconds.div_euclid(SECONDS_PER_DAY)
        - then_unix_seconds.div_euclid(SECONDS_PER_DAY);
    if day_difference <= 0 {
        t!("request.format.today", locale = locale).into_owned()
    } else if day_difference == 1 {
        t!("request.format.yesterday", locale = locale).into_owned()
    } else {
        t!(
            "request.format.days_ago",
            locale = locale,
            count = day_difference
        )
        .into_owned()
    }
}

/// A relative label for a moment, finer than [`relative_day`] within the last day: `"just now"`,
/// `"12 min ago"`, `"3 h ago"`, then [`relative_day`]'s labels up to 30 days, then the date as
/// `YYYY-MM-DD` in local time. Used for commits and branches.
pub fn relative_time(then_unix_seconds: i64, now_unix_seconds: i64) -> String {
    relative_time_in(then_unix_seconds, now_unix_seconds, &rust_i18n::locale())
}

/// [`relative_time`] in an explicit `locale`.
fn relative_time_in(then_unix_seconds: i64, now_unix_seconds: i64, locale: &str) -> String {
    let elapsed = now_unix_seconds - then_unix_seconds;
    if elapsed < 60 {
        t!("git.time.just_now", locale = locale).into_owned()
    } else if elapsed < 60 * 60 {
        t!(
            "git.time.minutes_ago",
            locale = locale,
            count = elapsed / 60
        )
        .into_owned()
    } else if elapsed < SECONDS_PER_DAY {
        t!(
            "git.time.hours_ago",
            locale = locale,
            count = elapsed / 3600
        )
        .into_owned()
    } else if elapsed < 30 * SECONDS_PER_DAY {
        relative_day_in(then_unix_seconds, now_unix_seconds, locale)
    } else {
        chrono::DateTime::from_timestamp(then_unix_seconds, 0)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn shorten_path_replaces_the_home_prefix() {
        assert_eq!(
            shorten_path(
                Path::new("/home/tanis/dev/brasa"),
                Some(Path::new("/home/tanis"))
            ),
            "~/dev/brasa"
        );
    }

    #[test]
    fn shorten_path_of_home_itself_is_a_bare_tilde() {
        assert_eq!(
            shorten_path(Path::new("/home/tanis"), Some(Path::new("/home/tanis"))),
            "~"
        );
    }

    #[test]
    fn shorten_path_outside_home_is_unchanged() {
        assert_eq!(
            shorten_path(Path::new("/var/log"), Some(Path::new("/home/tanis"))),
            "/var/log"
        );
    }

    #[test]
    fn shorten_path_without_a_home_is_unchanged() {
        assert_eq!(shorten_path(Path::new("/var/log"), None), "/var/log");
    }

    #[cfg(windows)]
    #[test]
    fn shorten_path_uses_backslashes_on_windows() {
        assert_eq!(
            shorten_path(
                &Path::new(r"C:\Users\me\AppData\Roaming").join("brasa/settings.toml"),
                Some(Path::new(r"C:\Users\me"))
            ),
            r"~\AppData\Roaming\brasa\settings.toml"
        );
    }

    #[test]
    fn elide_path_keeps_a_short_path() {
        assert_eq!(elide_path("~/dev/brasa", 40), "~/dev/brasa");
    }

    #[test]
    fn elide_path_keeps_the_head_and_the_tail() {
        assert_eq!(
            elide_path(
                "/tmp/claude-1000/-home-tanis-dev-brasa/5d5e073d-3b50/scratchpad/ws",
                30
            ),
            "/tmp/…/scratchpad/ws"
        );
        assert_eq!(
            elide_path("~/dev/some/deep/folder/project", 24),
            "~/…/deep/folder/project"
        );
    }

    #[test]
    fn elide_path_cuts_the_start_when_the_last_component_is_too_long() {
        assert_eq!(
            elide_path("/tmp/abcdefghijklmnopqrstuvwxyz", 10),
            "…rstuvwxyz"
        );
    }

    #[test]
    fn tab_label_is_the_file_stem() {
        assert_eq!(tab_label("auth/login.brasa"), "login");
        assert_eq!(tab_label("health.brasa"), "health");
    }

    #[test]
    fn relative_day_labels() {
        let now = 1_000_000i64;
        let day = SECONDS_PER_DAY;
        assert_eq!(relative_day_in(now, now, "en"), "today");
        assert_eq!(relative_day_in(now - day, now, "en"), "yesterday");
        assert_eq!(relative_day_in(now - 3 * day, now, "en"), "3 days ago");
        assert_eq!(relative_time_in(now - 30, now, "en"), "just now");
        assert_eq!(relative_time_in(now - 12 * 60, now, "en"), "12 min ago");
        assert_eq!(relative_time_in(now - 3 * 3600, now, "es"), "hace 3 h");
        assert_eq!(relative_time_in(now - 3 * day, now, "en"), "3 days ago");
        assert_eq!(relative_time_in(now - 90 * day, now, "en").len(), 10);
        assert_eq!(relative_day_in(now + day, now, "en"), "today");
    }

    #[test]
    fn relative_day_is_translated() {
        let now = 1_000_000i64;
        let day = SECONDS_PER_DAY;
        assert_eq!(relative_day_in(now - day, now, "es"), "ayer");
        assert_eq!(relative_day_in(now - 3 * day, now, "it"), "3 giorni fa");
    }
}
