pub mod error; // MeasurementError
pub mod measurements; // parse_measurements_str, load_measurements_from_file
pub mod pattern_file; // serialize_document, deserialize_document, PatternFileError

pub use error::MeasurementError; // re-exported so callers can write `io::MeasurementError`
pub use measurements::{load_measurements_from_file, parse_measurements_str}; // re-exported so callers can write `io::parse_measurements_str` etc.
pub use pattern_file::{deserialize_document, serialize_document, PatternFileError}; // re-exported so callers can write `io::serialize_document` etc.
