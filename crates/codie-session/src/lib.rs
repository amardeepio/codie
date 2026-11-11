//! Append-only JSONL session storage for codie.
//!
//! A session file starts with a [`SessionHeader`] and continues with one
//! [`Entry`] per line. Entries form a tree through `parent_id`; the
//! [`Session::projection`] method renders the branch from the current leaf back
//! to its root as a transcript.
//!
//! Persistence is lazy (SESS-002): [`Session::create`] prepares the session
//! directory but the file appears only when the first message is appended.
//! Format version 1 is the only version this build writes or accepts; newer
//! versions are refused with a clear error (SESS-007).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod entry;
mod session;
mod time;

pub use entry::{Entry, HeaderType, SESSION_FORMAT_VERSION, SessionHeader};
pub use session::{Session, SessionSummary, TreeEntry};

/// Returns the session format version written by this build.
#[must_use]
pub const fn format_version() -> u32 {
    SESSION_FORMAT_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_version_is_one() {
        assert_eq!(SESSION_FORMAT_VERSION, 1);
        assert_eq!(format_version(), SESSION_FORMAT_VERSION);
    }
}
