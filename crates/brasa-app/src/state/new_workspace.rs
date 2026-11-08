//! The "New workspace" dialog's plain logic (GitHub #88): where a new workspace is suggested by
//! default and which folder a typed name creates. Creating the folder itself is
//! `brasa_workspace::create_workspace`. Unit tested without a window.

use std::path::{Path, PathBuf};

use brasa_workspace::sanitize_file_name;

/// The folder a new workspace is suggested in: `documents` (the user's Documents folder), `home`
/// when there is none, the current directory as a last resort. The dialog's "Change..." button
/// picks another one.
pub fn default_location(documents: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    documents.or(home).unwrap_or_else(|| PathBuf::from("."))
}

/// [`default_location`] for the current user.
pub fn default_location_for_user() -> PathBuf {
    default_location(dirs::document_dir(), dirs::home_dir())
}

/// The folder `name` creates inside `location`: the trimmed name, made safe as a folder name on
/// every platform. `None` while the name is blank, so the dialog's Create button stays disabled.
pub fn target(location: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    Some(location.join(sanitize_file_name(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_location_prefers_documents_then_home() {
        let documents = PathBuf::from("/home/ana/Documents");
        let home = PathBuf::from("/home/ana");
        assert_eq!(
            default_location(Some(documents.clone()), Some(home.clone())),
            documents
        );
        assert_eq!(default_location(None, Some(home.clone())), home);
        assert_eq!(default_location(None, None), PathBuf::from("."));
    }

    #[test]
    fn a_blank_name_has_no_target() {
        let location = Path::new("/home/ana/Documents");
        assert_eq!(target(location, ""), None);
        assert_eq!(target(location, "   "), None);
    }

    #[test]
    fn the_target_is_the_trimmed_name_inside_the_location() {
        let location = Path::new("/home/ana/Documents");
        assert_eq!(target(location, "  my-api "), Some(location.join("my-api")));
    }

    #[test]
    fn the_target_name_is_sanitized() {
        let location = Path::new("/home/ana/Documents");
        assert_eq!(
            target(location, "shop/api: v2"),
            Some(location.join("shop_api_ v2"))
        );
    }
}
