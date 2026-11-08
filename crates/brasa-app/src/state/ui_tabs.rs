//! The inner tabs of the request editor and response viewer panels.
//!
//! These are plain, `Copy` enums so the currently selected tab can be stored directly on
//! [`crate::views::AppView`] without any `gpui` type.

use rust_i18n::t;

/// The request editor's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestTab {
    /// The `::: query` section, as an editable key-value table.
    #[default]
    Params,
    /// The header list, as an editable key-value table.
    Headers,
    /// The `::: auth` section: type selector plus its fields.
    Auth,
    /// The body: type selector plus a code editor (or a key-value table for a Form body).
    Body,
    /// The `::: pre` section, a JavaScript code editor.
    Pre,
    /// The `::: post` section, a JavaScript code editor.
    Post,
    /// The `::: docs` section, free text notes.
    Docs,
}

impl RequestTab {
    /// Every tab, in the order the tab bar lists them.
    pub const ALL: [RequestTab; 7] = [
        RequestTab::Params,
        RequestTab::Headers,
        RequestTab::Auth,
        RequestTab::Body,
        RequestTab::Pre,
        RequestTab::Post,
        RequestTab::Docs,
    ];

    /// The label shown on the tab.
    pub fn label(self) -> String {
        match self {
            RequestTab::Params => t!("shell.tab.params"),
            RequestTab::Headers => t!("common.headers"),
            RequestTab::Auth => t!("shell.tab.auth"),
            RequestTab::Body => t!("common.body"),
            RequestTab::Pre => t!("shell.tab.pre"),
            RequestTab::Post => t!("shell.tab.post"),
            RequestTab::Docs => t!("shell.tab.docs"),
        }
        .into_owned()
    }
}

/// The request tab to show when opening a request for the first time (not when re-selecting an
/// already open one), given `current` (the tab shared across every open tab, see
/// `crate::views::root::AppView::active_request_tab`'s doc comment) and the request being
/// opened. Jumps to [`RequestTab::Body`] when `current` is still the default [`RequestTab::
/// Params`] and the request has no query params to show there but does have a body, since
/// `Params` would otherwise land on an empty table (a `POST` with a body opens on
/// `Body`). Any other
/// current tab is left alone: the user's own tab choice from a previous request is not
/// second-guessed.
pub fn tab_to_show_on_open(current: RequestTab, query_len: usize, has_body: bool) -> RequestTab {
    if current == RequestTab::Params && query_len == 0 && has_body {
        RequestTab::Body
    } else {
        current
    }
}

/// The response viewer's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResponseTab {
    /// The response body, pretty-printed when it is JSON, with a raw toggle.
    #[default]
    Body,
    /// The response headers.
    Headers,
    /// The post-response script's `test()` results.
    Tests,
    /// The pre/post scripts' `console.*` output.
    Console,
}

impl ResponseTab {
    /// Every tab, in the order the tab bar lists them.
    pub const ALL: [ResponseTab; 4] = [
        ResponseTab::Body,
        ResponseTab::Headers,
        ResponseTab::Tests,
        ResponseTab::Console,
    ];

    /// The label shown on the tab.
    pub fn label(self) -> String {
        match self {
            ResponseTab::Body => t!("common.body"),
            ResponseTab::Headers => t!("common.headers"),
            ResponseTab::Tests => t!("response.tab.tests"),
            ResponseTab::Console => t!("response.tab.console"),
        }
        .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_request_tab_is_params() {
        assert_eq!(RequestTab::default(), RequestTab::Params);
    }

    #[test]
    fn default_response_tab_is_body() {
        assert_eq!(ResponseTab::default(), ResponseTab::Body);
    }

    #[test]
    fn tab_to_show_on_open_jumps_to_body_from_params_with_no_query_and_a_body() {
        assert_eq!(
            tab_to_show_on_open(RequestTab::Params, 0, true),
            RequestTab::Body
        );
    }

    #[test]
    fn tab_to_show_on_open_keeps_params_when_it_has_query_entries() {
        assert_eq!(
            tab_to_show_on_open(RequestTab::Params, 2, true),
            RequestTab::Params
        );
    }

    #[test]
    fn tab_to_show_on_open_keeps_params_when_there_is_no_body() {
        assert_eq!(
            tab_to_show_on_open(RequestTab::Params, 0, false),
            RequestTab::Params
        );
    }

    #[test]
    fn tab_to_show_on_open_does_not_override_a_non_default_current_tab() {
        assert_eq!(
            tab_to_show_on_open(RequestTab::Headers, 0, true),
            RequestTab::Headers
        );
    }

    #[test]
    fn every_request_tab_has_a_label() {
        for tab in RequestTab::ALL {
            assert!(!tab.label().is_empty());
        }
    }

    #[test]
    fn every_response_tab_has_a_label() {
        for tab in ResponseTab::ALL {
            assert!(!tab.label().is_empty());
        }
    }
}
