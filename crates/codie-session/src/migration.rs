//! Session format migration harness.
//!
//! Version 1 is the only format in the wild, so [`MIGRATIONS`] is currently
//! empty. The harness exists so the next format change adds one entry instead of
//! teaching every reader a one-off conversion. Migration happens on read; files
//! are never rewritten in place.

use serde_json::Value;

use crate::entry::SESSION_FORMAT_VERSION;
use codie_core::{Error, Result};

/// The oldest on-disk version this build can still read.
pub const MIN_SUPPORTED_VERSION: u32 = 1;

/// One read-time format conversion.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Version being converted from.
    pub from: u32,
    /// Version produced by the conversion.
    pub to: u32,
    /// Converts one entry object.
    pub apply: fn(Value) -> Result<Value>,
}

/// Read-time migrations, ordered from oldest to newest.
///
/// Adding a version bumps [`SESSION_FORMAT_VERSION`], appends a migration here,
/// and raises [`MIN_SUPPORTED_VERSION`] only when old files are no longer
/// readable.
pub const MIGRATIONS: &[Migration] = &[];

/// Returns true when this build can read `version`.
#[must_use]
pub const fn is_supported(version: u32) -> bool {
    version >= MIN_SUPPORTED_VERSION && version <= SESSION_FORMAT_VERSION
}

/// Validates the migration table at load time.
///
/// Returns an error when a step is non-monotonic or the table does not reach
/// the current version.
pub fn validate_table() -> Result<()> {
    let mut version = MIN_SUPPORTED_VERSION;
    for migration in MIGRATIONS {
        if migration.from != version || migration.to <= migration.from {
            return Err(Error::session(format!(
                "invalid session migration {} -> {}",
                migration.from, migration.to
            )));
        }
        version = migration.to;
    }
    if version != SESSION_FORMAT_VERSION {
        return Err(Error::session(format!(
            "session migrations stop at version {version}, expected {SESSION_FORMAT_VERSION}"
        )));
    }
    Ok(())
}

/// Applies every migration needed to reach the current version.
///
/// The value is returned unchanged for the current version.
pub fn migrate_value(mut value: Value, from: u32) -> Result<Value> {
    if !is_supported(from) {
        return Err(Error::session(format!(
            "unsupported session format version {from}"
        )));
    }
    let mut version = from;
    while version < SESSION_FORMAT_VERSION {
        let Some(migration) = MIGRATIONS
            .iter()
            .find(|migration| migration.from == version)
        else {
            return Err(Error::session(format!(
                "no migration from session format version {version}"
            )));
        };
        value = (migration.apply)(value)?;
        version = migration.to;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reaches_the_current_version() {
        validate_table().expect("valid empty table");
        assert_eq!(MIN_SUPPORTED_VERSION, 1);
        assert_eq!(SESSION_FORMAT_VERSION, 1);
    }
}
