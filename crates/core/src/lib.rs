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
pub mod document;
pub mod error;
pub mod formula;
pub mod geo;
pub mod id;
pub mod recompute;
pub mod variable;

pub use container::PatternData;
pub use document::{Document, ToolKind, ToolRecord};
pub use error::ContainerError;
pub use geo::{GeoObject, LineData, PointData};
pub use id::ObjectId;
pub use recompute::{recompute_all, PatternError};
pub use variable::{Variable, VariableKind};
