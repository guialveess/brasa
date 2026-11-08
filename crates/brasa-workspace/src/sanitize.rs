//! A file name sanitization helper shared by [`crate::Workspace`] (new requests and folders) and
//! the Postman importer.

/// Device names reserved by Windows, checked case-insensitively. A file or folder named exactly
/// one of these (ignoring case) cannot be created on Windows, regardless of extension.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters not allowed in a file or folder name on Windows, plus control characters.
const FORBIDDEN_CHARACTERS: &str = "<>:\"/\\|?*";

/// The maximum length, in characters, of a sanitized name.
const MAX_NAME_LENGTH: usize = 120;

/// The fallback name used when sanitizing leaves nothing usable, for example because the input
/// was only dots and spaces.
const FALLBACK_NAME: &str = "untitled";

/// Turns an arbitrary string into a name that is safe to use as a file or folder name on Linux,
/// Windows and macOS.
///
/// This only sanitizes the name itself, it does not add an extension: a caller creating a
/// `.brasa` file appends `.brasa` after calling this function. The transformation, in order:
///
/// 1. Every control character and every character forbidden on Windows (`< > : " / \ | ? *`) is
///    replaced with `_`.
/// 2. Trailing dots and spaces are trimmed, since Windows does not allow them at the end of a
///    name.
/// 3. The name is truncated to [`MAX_NAME_LENGTH`] characters (counting Unicode scalar values,
///    not bytes, so a multi-byte character is never split), then trimmed again in case the
///    truncation left new trailing dots or spaces.
/// 4. If the result, compared case-insensitively, is a Windows-reserved device name (`CON`,
///    `PRN`, `COM1`, ...), an underscore is appended so it is safe to use.
/// 5. If nothing is left at this point, the name becomes `"untitled"`.
pub fn sanitize_file_name(name: &str) -> String {
    let mut sanitized: String = name
        .chars()
        .map(|character| {
            if character.is_control() || FORBIDDEN_CHARACTERS.contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect();

    sanitized = trim_trailing_dots_and_spaces(&sanitized);

    if sanitized.chars().count() > MAX_NAME_LENGTH {
        sanitized = sanitized.chars().take(MAX_NAME_LENGTH).collect();
        sanitized = trim_trailing_dots_and_spaces(&sanitized);
    }

    if is_windows_reserved_name(&sanitized) {
        sanitized.push('_');
    }

    if sanitized.is_empty() {
        sanitized = FALLBACK_NAME.to_string();
    }

    sanitized
}

/// Trims trailing `.` and ` ` characters from `name`.
fn trim_trailing_dots_and_spaces(name: &str) -> String {
    name.trim_end_matches([' ', '.']).to_string()
}

/// Whether `name` is a Windows-reserved device name, compared case-insensitively.
fn is_windows_reserved_name(name: &str) -> bool {
    WINDOWS_RESERVED_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn leaves_a_plain_name_untouched() {
        assert_eq!(sanitize_file_name("Login request"), "Login request");
    }

    #[test]
    fn replaces_forbidden_characters() {
        assert_eq!(
            sanitize_file_name("a/b\\c:d*e?f\"g<h>i|j"),
            "a_b_c_d_e_f_g_h_i_j"
        );
    }

    #[test]
    fn replaces_control_characters() {
        assert_eq!(sanitize_file_name("a\tb\nc"), "a_b_c");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_file_name("name..  "), "name");
    }

    #[test]
    fn keeps_leading_and_interior_dots() {
        assert_eq!(sanitize_file_name("..hidden.name.."), "..hidden.name");
    }

    #[test]
    fn escapes_windows_reserved_names_case_insensitively() {
        assert_eq!(sanitize_file_name("CON"), "CON_");
        assert_eq!(sanitize_file_name("con"), "con_");
        assert_eq!(sanitize_file_name("LPT3"), "LPT3_");
    }

    #[test]
    fn does_not_treat_a_reserved_name_as_a_prefix() {
        assert_eq!(sanitize_file_name("CONtacts"), "CONtacts");
    }

    #[test]
    fn truncates_to_the_max_length() {
        let long_name = "a".repeat(200);
        let sanitized = sanitize_file_name(&long_name);
        assert_eq!(sanitized.chars().count(), MAX_NAME_LENGTH);
        assert_eq!(sanitized, "a".repeat(MAX_NAME_LENGTH));
    }

    #[test]
    fn truncation_does_not_split_a_multi_byte_character() {
        let long_name = "é".repeat(200);
        let sanitized = sanitize_file_name(&long_name);
        assert_eq!(sanitized.chars().count(), MAX_NAME_LENGTH);
    }

    #[test]
    fn falls_back_to_untitled_when_nothing_is_left() {
        assert_eq!(sanitize_file_name(""), "untitled");
        assert_eq!(sanitize_file_name("..."), "untitled");
        assert_eq!(sanitize_file_name("   "), "untitled");
    }
}
