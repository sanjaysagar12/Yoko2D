use thiserror::Error; // brings in the `Error` derive macro used below

/// Everything that can go wrong loading and validating a measurement file.
#[derive(Debug, Error)] // Debug: printable in test failures; Error: implements std::error::Error via thiserror
pub enum MeasurementError {
    /// Reading the file from disk failed (not found, permission denied,
    /// etc). Wrapped via `#[from]` so `?` converts a `std::io::Error`
    /// automatically inside `load_measurements_from_file`.
    #[error("failed to read measurement file: {0}")]
    // {0} refers to the wrapped io::Error, whose own Display supplies detail
    Io(#[from] std::io::Error), // the underlying I/O failure

    /// The file's contents aren't valid JSON, or don't match the expected
    /// `MeasurementFile` shape. Wrapped via `#[from]` so `?` converts a
    /// `serde_json::Error` automatically inside `parse_measurements_str`.
    #[error("failed to parse measurement JSON: {0}")]
    // {0} refers to the wrapped serde_json::Error, whose Display gives line/column detail
    Parse(#[from] serde_json::Error), // the underlying JSON parse/shape failure

    /// Two entries in the same file share a `name`. Caught explicitly
    /// rather than letting the later entry silently win, since a silent
    /// overwrite here would hide what's likely a data-entry mistake in the
    /// measurement file itself.
    #[error("duplicate measurement name {0:?} in file")]
    // {0} is the name that appeared more than once
    DuplicateName(String), // the name that was seen twice

    /// An entry's `name` field is the empty string, which can't identify a
    /// variable in the formula engine (Phase 2's `Expr::Variable` requires
    /// a real identifier to look up).
    #[error("measurement entry has an empty name")]
    // no extra data: there's only one way for a name to be empty
    EmptyName,

    /// An entry's `value` is NaN or infinite, which Phase 2's formula
    /// engine already refuses to produce as a *result*
    /// (`FormulaError::InvalidResult`) — so it shouldn't be accepted as an
    /// *input* either.
    #[error("measurement {name:?} has an invalid (NaN or infinite) value")]
    // names the specific entry that failed
    InvalidValue {
        name: String, // which measurement's value was rejected
    },
}
