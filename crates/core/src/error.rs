use thiserror::Error;

use crate::id::ObjectId;

/// Everything that can go wrong when looking something up in a
/// [`crate::PatternData`].
///
/// All three variants are plain "lookup failed" cases, not panics — every
/// `PatternData` accessor that can fail returns one of these in a `Result`
/// rather than unwrapping internally, which is also what
/// `#![deny(clippy::unwrap_used, ...)]` in `lib.rs` enforces for
/// non-test code.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ContainerError {
    /// No object exists under this id at all — it was never inserted, or it
    /// was and has since been removed via `remove_object`/`clear_objects`.
    #[error("object {0:?} not found")]
    ObjectNotFound(ObjectId),

    /// The id exists, but the stored [`crate::GeoObject`] is a different
    /// variant than the one asked for (e.g. calling `get_line` on an id that
    /// holds a `Point`). Kept distinct from `ObjectNotFound` because it
    /// means the caller has a bug (wrong id, wrong assumption), not that the
    /// data is missing.
    #[error("object {id:?} has wrong type, expected {expected}")]
    WrongObjectType {
        id: ObjectId,
        expected: &'static str,
    },

    /// No variable is registered under this name — either it was never
    /// added, or it belonged to a kind that was removed by
    /// `clear_variables`/`clear_all`.
    #[error("variable {0:?} not found")]
    VariableNotFound(String),

    /// `insert_with_id` was asked to insert an object under an id that
    /// already names something in the container. Returned instead of
    /// silently overwriting, since a collision here means the caller (the
    /// Phase 3 recompute engine) has a bug — two `ToolRecord`s sharing one
    /// id — that should surface loudly rather than quietly clobber data.
    #[error("object {0:?} already exists")]
    DuplicateId(ObjectId),
}
