//! Checks of telaio's own strings (`locales/telaio.yml`). The tables are loaded by the
//! `rust_i18n::i18n!` call in `lib.rs`; this module only holds tests.

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use pretty_assertions::assert_eq;
    use rust_i18n::t;

    /// The languages every key must have, the same four as the apps.
    const LANGUAGES: [&str; 4] = ["en", "es", "gl", "it"];

    /// Every message of `locale` defined by telaio's own table.
    fn messages(locale: &str) -> BTreeMap<String, String> {
        crate::_rust_i18n_backend()
            .messages_for_locale(locale)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn every_key_is_translated_in_every_language() {
        let english = messages("en");
        assert!(!english.is_empty(), "no English messages were loaded");
        for language in LANGUAGES {
            let table = messages(language);
            for key in english.keys() {
                let text = table
                    .get(key)
                    .unwrap_or_else(|| panic!("{key} is missing in {language}"));
                assert!(!text.trim().is_empty(), "{key} is empty in {language}");
            }
            for key in table.keys() {
                assert!(english.contains_key(key), "{key} is only in {language}");
            }
        }
    }

    #[test]
    fn no_translation_contains_the_em_dash() {
        for language in LANGUAGES {
            for (key, text) in messages(language) {
                assert!(!text.contains('\u{2014}'), "{key} in {language}");
            }
        }
    }

    #[test]
    fn a_key_reads_in_the_asked_language() {
        assert_eq!(t!("telaio.close", locale = "es"), "Cerrar");
        assert_eq!(t!("telaio.close", locale = "it"), "Chiudi");
    }

    /// Every `.rs` file under `dir`, recursively.
    fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).expect("read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// The literal keys of every `t!` call in `source` whose first argument is a string literal.
    fn literal_keys(source: &str) -> Vec<String> {
        let mut keys = Vec::new();
        let mut from = 0;
        while let Some(found) = source[from..].find("t!(\"") {
            let start = from + found;
            let body = start + 4;
            from = body;
            // `format!(`, `assert!(` ... also end in `t!(`: only a standalone `t!` counts.
            let previous = source[..start].chars().next_back();
            if previous.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            if let Some(end) = source[body..].find('"') {
                keys.push(source[body..body + end].to_string());
            }
        }
        keys
    }

    #[test]
    fn every_key_used_in_the_sources_exists() {
        let english = messages("en");
        let mut files = Vec::new();
        rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        assert!(!files.is_empty());
        let mut missing = Vec::new();
        for file in files {
            let source = fs::read_to_string(&file).expect("read source");
            for key in literal_keys(&source) {
                if !english.contains_key(&key) {
                    missing.push(format!("{}: {key}", file.display()));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "keys missing from locales/: {missing:#?}"
        );
    }
}
