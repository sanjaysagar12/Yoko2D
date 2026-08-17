pub mod error; // MeasurementError
pub mod measurements; // parse_measurements_str, load_measurements_from_file

pub use error::MeasurementError; // re-exported so callers can write `io::MeasurementError`
pub use measurements::{load_measurements_from_file, parse_measurements_str}; // re-exported so callers can write `io::parse_measurements_str` etc.

/// Stands in for this crate's eventual pattern file format read/write
/// support (`.val`/`.smis`, etc. — distinct from the measurement file
/// format in `measurements.rs`, which is already implemented).
///
/// Takes `&core_lib::PatternData` purely to prove the dependency wiring from
/// Phase 0 works end-to-end — `io` can see and reference `core`'s public
/// types (under the `core_lib` alias; see the comment on this dependency in
/// `Cargo.toml`) — without touching the container's fields, which are
/// private by design (see `core_lib::container::PatternData`).
pub fn placeholder(_data: &core_lib::PatternData) -> &'static str {
    "io placeholder"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_uses_core_pattern_data() {
        let data = core_lib::PatternData::default();
        assert!(placeholder(&data).contains("io placeholder"));
    }
}
