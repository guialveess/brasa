//! Exports a folder (or the whole workspace) of `.brasa` requests to another tool's collection
//! format, the reverse direction of `crate::postman` and `crate::bruno`'s import. Pure: callers
//! build an [`ExportNode`] tree from their own workspace and get back the text to write.
//!
//! Only the request shape (method, URL, headers, body, auth) round-trips exactly. The `pre`/
//! `post` scripts use Brasa's own API (`docs/scripting.md`), not the target tool's, so they are
//! carried over as is with a comment banner, instead of attempting a translation that could
//! silently produce a script that runs differently than it reads.

use brasa_core::Request;

pub mod bruno;
pub mod insomnia;
pub mod postman;

/// One node of the tree being exported: a folder of children, or a request with its parsed body.
#[derive(Debug, Clone)]
pub enum ExportNode {
    /// A folder, in the same order as the collection tree.
    Folder {
        /// The folder name.
        name: String,
        /// Its direct children, in order.
        children: Vec<ExportNode>,
    },
    /// A request.
    Request {
        /// The request name (the file name without the `.brasa` extension).
        name: String,
        /// The parsed request.
        request: Request,
    },
}
