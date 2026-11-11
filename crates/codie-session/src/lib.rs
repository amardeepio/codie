//! Append-only JSONL session storage for codie.
//!
//! Phase 0 fixes the on-disk format version only. The header, reader, writer,
//! and projection are implemented in Phase 1 and the format is frozen in
//! Phase 3; see `specs/session/` for the normative contract.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// On-disk session format version understood by this build.
///
/// Version 1 is the initial format. Readers must refuse unknown newer versions
/// and migrate older ones on read.
pub const SESSION_FORMAT_VERSION: u32 = 1;

/// Returns the session format version.
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
