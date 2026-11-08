//! The response status badge shown in the response pane's tab bar: telaio's [`StatusBadge`]
//! colored by the status code ([`status_tone`]).

use rust_i18n::t;
use telaio_ui::components::StatusBadge;

use crate::state::response_render;
use crate::theme::palette::status_tone;

/// What a response status badge reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusState {
    /// A response was received with this HTTP status code.
    Code(u16),
    /// No request has been sent yet, or the last send failed before a status was known.
    NotSent,
    /// A request is in flight.
    Sending,
}

/// The badge for `state`, labelled with the code and its reason phrase ("200 OK", or the bare
/// code when it has none), "Not sent" or "Sending...".
pub fn status_badge(state: &StatusState) -> StatusBadge {
    match state {
        StatusState::Code(code) => {
            let label = match response_render::reason_phrase(*code) {
                Some(reason) => format!("{code} {reason}"),
                None => code.to_string(),
            };
            StatusBadge::new(label).tone(status_tone(Some(*code)))
        }
        StatusState::NotSent => StatusBadge::new(t!("response.status.not_sent").into_owned()),
        StatusState::Sending => StatusBadge::new(t!("common.sending").into_owned()).busy(true),
    }
}
