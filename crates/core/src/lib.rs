// Applied only outside `#[cfg(test)]` builds: production code paths in this
// crate must handle every failure explicitly (via `Result`/`Option`, as
// `PatternData`'s methods do) rather than panicking, since `core` is meant
// to be usable from a GUI event loop where a panic would take down the
// whole app. Test code is exempted so `.unwrap()`/`assert!` stay ergonomic
// in `#[cfg(test)] mod tests` blocks throughout this crate.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod container;
pub mod error;
pub mod formula;
pub mod geo;
pub mod id;
pub mod variable;

pub use container::PatternData;
pub use error::ContainerError;
pub use geo::{GeoObject, LineData, PointData};
pub use id::ObjectId;
pub use variable::{Variable, VariableKind};

/// Placeholder for the pattern's document-level metadata (name, author,
/// units, measurement file reference, …) that will grow out in a later
/// phase. Not wired into `PatternData` yet — this phase only implements the
/// object/variable container itself.
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_placeholder_text() {
        assert_eq!(formula::placeholder(), "formula engine not yet implemented");
    }
}
