//! Checks of telaio-app's own strings (`locales/telaio_app.yml`). The tables are loaded by the
//! `rust_i18n::i18n!` call in `lib.rs`; this module only holds tests.

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use pretty_assertions::assert_eq;
    use rust_i18n::t;

    use crate::language::Language;

    /// Every message of `locale` defined by telaio-app's own table.
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
        for language in Language::ALL {
            let table = messages(language.code());
            for key in english.keys() {
                let text = table
                    .get(key)
                    .unwrap_or_else(|| panic!("{key} is missing in {}", language.code()));
                assert!(
                    !text.trim().is_empty(),
                    "{key} is empty in {}",
                    language.code()
                );
            }
            for key in table.keys() {
                assert!(
                    english.contains_key(key),
                    "{key} is only in {}",
                    language.code()
                );
            }
        }
    }

    #[test]
    fn no_translation_contains_the_em_dash() {
        for language in Language::ALL {
            for (key, text) in messages(language.code()) {
                assert!(!text.contains('\u{2014}'), "{key} in {}", language.code());
            }
        }
    }

    #[test]
    fn a_key_reads_in_the_asked_language() {
        assert_eq!(t!("telaio_app.log_level.off", locale = "it"), "Disattivato");
        assert_eq!(t!("telaio_app.about.version", locale = "es"), "Versión");
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

    #[test]
    fn every_key_used_in_the_sources_exists() {
        let english = messages("en");
        let mut files = Vec::new();
        rust_files(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut missing = Vec::new();
        for file in files {
            let source = fs::read_to_string(&file).expect("read source");
            for part in source.split("telaio_app.").skip(1) {
                let key: String = part
                    .chars()
                    .take_while(|c| c.is_ascii_lowercase() || *c == '.' || *c == '_')
                    .collect();
                let key = format!("telaio_app.{}", key.trim_end_matches('.'));
                if key.matches('.').count() >= 2 && !english.contains_key(&key) {
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
