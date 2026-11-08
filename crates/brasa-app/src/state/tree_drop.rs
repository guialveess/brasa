//! Moving a request or a folder by dragging it onto another folder of the sidebar tree. Pure logic
//! over workspace ids, so it is unit-tested without `gpui`; `views/sidebar.rs` tracks the folder
//! under the pointer and `views/root.rs` moves the entry. The workspace root is the folder `""`.

/// The folder holding `id`, `""` at the workspace root. `a/b/c.brasa` gives `a/b`.
pub fn parent_folder(id: &str) -> &str {
    id.rfind('/').map_or("", |ix| &id[..ix])
}

/// The folder a request dropped on the row `row_id` goes to: the row itself for a folder, the
/// folder holding it for a request.
pub fn row_drop_folder(row_id: &str, is_request: bool) -> &str {
    if is_request {
        parent_folder(row_id)
    } else {
        row_id
    }
}

/// Whether `id` is the entry `ancestor` itself or lies anywhere below it.
pub fn is_within(id: &str, ancestor: &str) -> bool {
    id.strip_prefix(ancestor)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Where the entry `id` (a request or a folder) goes when dropped with the pointer over `hovered`
/// (`None` outside the tree): `None` as well when it is the folder the entry is already in, since
/// nothing would change, and for a folder when it is the folder itself or one of its subfolders.
pub fn drop_target(id: &str, hovered: Option<String>) -> Option<String> {
    hovered.filter(|folder| folder != parent_folder(id) && !is_within(folder, id))
}

/// Whether the row `row_id` is drawn as part of the drop target `folder`: the folder's own row
/// and every row below it. The root (`""`) highlights the whole tree instead, never single rows.
pub fn in_drop_target(row_id: &str, folder: &str) -> bool {
    !folder.is_empty() && is_within(row_id, folder)
}

/// The new id of `id` after the entry `old` moved to `new`: `id` itself when it is not `old` nor
/// below it.
pub fn moved_id(id: &str, old: &str, new: &str) -> String {
    if is_within(id, old) {
        format!("{new}{}", &id[old.len()..])
    } else {
        id.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_folder_of_nested_and_root_ids() {
        assert_eq!(parent_folder("a/b/c.brasa"), "a/b");
        assert_eq!(parent_folder("a/b"), "a");
        assert_eq!(parent_folder("c.brasa"), "");
    }

    #[test]
    fn a_folder_row_is_its_own_drop_folder_and_a_request_row_gives_its_parent() {
        assert_eq!(row_drop_folder("auth", false), "auth");
        assert_eq!(row_drop_folder("auth/login.brasa", true), "auth");
        assert_eq!(row_drop_folder("ping.brasa", true), "");
    }

    #[test]
    fn drop_target_skips_the_folder_the_request_is_already_in() {
        assert_eq!(drop_target("auth/login.brasa", Some("auth".into())), None);
        assert_eq!(drop_target("ping.brasa", Some(String::new())), None);
        assert_eq!(drop_target("auth/login.brasa", None), None);
        assert_eq!(
            drop_target("auth/login.brasa", Some("users".into())),
            Some("users".to_string())
        );
        assert_eq!(
            drop_target("auth/login.brasa", Some(String::new())),
            Some(String::new())
        );
        assert_eq!(
            drop_target("ping.brasa", Some("auth/admin".into())),
            Some("auth/admin".to_string())
        );
    }

    #[test]
    fn a_folder_never_targets_itself_nor_its_subfolders() {
        assert_eq!(drop_target("auth", Some("auth".into())), None);
        assert_eq!(drop_target("auth", Some("auth/admin".into())), None);
        assert_eq!(drop_target("auth", Some(String::new())), None);
        assert_eq!(drop_target("a/auth", Some("a".into())), None);
        assert_eq!(
            drop_target("auth", Some("auth-extra".into())),
            Some("auth-extra".to_string())
        );
        assert_eq!(
            drop_target("a/auth", Some(String::new())),
            Some(String::new())
        );
        assert_eq!(
            drop_target("auth", Some("users/auth".into())),
            Some("users/auth".to_string())
        );
    }

    #[test]
    fn moved_id_follows_the_entry_and_everything_below_it() {
        assert_eq!(moved_id("auth", "auth", "users/auth"), "users/auth");
        assert_eq!(
            moved_id("auth/admin/reset.brasa", "auth", "users/auth"),
            "users/auth/admin/reset.brasa"
        );
        assert_eq!(
            moved_id("auth-extra/x.brasa", "auth", "users/auth"),
            "auth-extra/x.brasa"
        );
        assert_eq!(
            moved_id("a/ping.brasa", "a/ping.brasa", "ping.brasa"),
            "ping.brasa"
        );
    }

    #[test]
    fn in_drop_target_covers_the_folder_row_and_everything_below_it() {
        assert!(in_drop_target("auth", "auth"));
        assert!(in_drop_target("auth/login.brasa", "auth"));
        assert!(in_drop_target("auth/admin/reset.brasa", "auth"));
        assert!(!in_drop_target("auth-extra", "auth"));
        assert!(!in_drop_target("users/list.brasa", "auth"));
        assert!(!in_drop_target("auth", "auth/admin"));
    }

    #[test]
    fn the_root_target_highlights_no_single_row() {
        assert!(!in_drop_target("ping.brasa", ""));
        assert!(!in_drop_target("auth", ""));
    }
}
