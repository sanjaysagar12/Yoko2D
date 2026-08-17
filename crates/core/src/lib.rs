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
