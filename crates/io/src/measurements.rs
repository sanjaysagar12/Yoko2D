use std::collections::HashMap; // the flat name -> value map both public functions here build and return
use std::path::Path; // the parameter type for load_measurements_from_file, generic over any path-like input

use crate::error::MeasurementError; // the error type both public functions here can fail with

/// The top-level shape of a measurement JSON file: just a list of entries.
///
/// `Deserialize`-only: this crate reads measurement files but doesn't
/// write them (no export/save feature exists yet), so a `Serialize` impl
/// isn't needed and isn't added speculatively.
#[derive(serde::Deserialize)] // lets serde_json parse a JSON object with a "measurements" array directly into this shape
struct MeasurementFile {
    measurements: Vec<MeasurementEntry>, // every measurement listed in the file, in file order
}

/// One measurement entry as it appears in the file, before validation.
///
/// Deliberately a separate, lower-trust type from `core::Variable`: this
/// is what the file *says*, not yet a value the formula engine has
/// accepted. `parse_measurements_str` is what turns a `Vec` of these into
/// a validated `HashMap<String, f64>`.
#[derive(serde::Deserialize)] // lets serde_json parse one JSON object of this shape into a MeasurementEntry
struct MeasurementEntry {
    name: String, // the variable name this measurement will be stored under, e.g. "height_scapula"
    value: f64,   // the measurement's numeric value

    // Accepted so a measurement file can document what each entry means
    // (see fixtures/measurements/sample.json), but not read anywhere
    // else in the engine yet — there's no UI or export path that
    // surfaces it. `#[serde(default)]` makes the field optional in the
    // JSON itself, defaulting to `None` when omitted (as in the fixture's
    // second entry, which has no "description").
    #[serde(default)] // absent "description" in the JSON deserializes to None instead of an error
    #[allow(dead_code)]
    // intentionally unread: kept only so files can self-document (see the TODO below), not a bug
    description: Option<String>, // free-text documentation for this entry, currently unused by the engine
}

// TODO(future phase): `value` above is assumed to already be in the
// pattern's working unit (cm, inches, whatever the document is configured
// for) — there is no unit field on `MeasurementEntry` and no conversion
// logic anywhere in this module. Unit handling is explicitly out of scope
// for this phase.

/// Parses `json` as a measurement file and validates its contents,
/// returning a flat `name -> value` map suitable for
/// `core::Document::apply_measurements`.
///
/// Validation (empty names, non-finite values, duplicate names within the
/// file) happens here, in `io`, rather than in `core` — `core` only ever
/// sees an already-validated `HashMap<String, f64>`, never raw file data.
pub fn parse_measurements_str(json: &str) -> Result<HashMap<String, f64>, MeasurementError> {
    let file: MeasurementFile = serde_json::from_str(json)?; // parse the raw JSON text into the expected shape, or propagate the parse error
    validate_entries(file.measurements) // hand the deserialized entries off to the shared validation logic below
}

/// Validates a list of already-deserialized entries and flattens them into
/// a `name -> value` map.
///
/// Split out from `parse_measurements_str` so this logic is directly
/// testable with hand-built `MeasurementEntry` values, independent of
/// whether JSON text can actually produce them. In particular, the
/// `!value.is_finite()` check below can never be reached through
/// `parse_measurements_str`'s own JSON input: JSON has no literal for NaN
/// at all, and this project's pinned `serde_json` version rejects (with a
/// parse error) any numeric literal that would overflow `f64` to infinity,
/// rather than silently producing one. The check is kept anyway as defense
/// in depth against a future alternate entry point, and is exercised
/// directly in this module's tests.
fn validate_entries(
    entries: Vec<MeasurementEntry>,
) -> Result<HashMap<String, f64>, MeasurementError> {
    let mut map = HashMap::new(); // accumulates validated name -> value pairs as entries are checked

    for entry in entries {
        // walk every entry, in the order given
        if entry.name.is_empty() {
            // an empty name can never be looked up as a formula variable
            return Err(MeasurementError::EmptyName); // reject the whole file rather than silently skipping this entry
        }
        if !entry.value.is_finite() {
            // NaN or +/-infinity would poison any formula that references this measurement
            return Err(MeasurementError::InvalidValue {
                name: entry.name.clone(),
            }); // name the offending entry
        }
        if map.contains_key(&entry.name) {
            // this name was already inserted by an earlier entry in the same file
            return Err(MeasurementError::DuplicateName(entry.name.clone())); // reject rather than let the later value silently win
        }
        map.insert(entry.name, entry.value); // name and value both validated: record this measurement
    }

    Ok(map) // every entry validated successfully
}

/// Reads the file at `path` and parses it the same way as
/// `parse_measurements_str`.
pub fn load_measurements_from_file(path: &Path) -> Result<HashMap<String, f64>, MeasurementError> {
    let contents = std::fs::read_to_string(path)?; // read the whole file into memory, or propagate the I/O error
    parse_measurements_str(&contents) // delegate all parsing/validation to the string-based entry point
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_distinct_entries() {
        let json = r#"{
            "measurements": [
                { "name": "height_scapula", "value": 40.0, "description": "Scapula height" },
                { "name": "waist_circ", "value": 72.5 }
            ]
        }"#;

        let map = parse_measurements_str(json).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("height_scapula"), Some(&40.0));
        assert_eq!(map.get("waist_circ"), Some(&72.5));
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_measurements_str("{ not valid json").unwrap_err();
        assert!(matches!(err, MeasurementError::Parse(_)));
    }

    #[test]
    fn rejects_duplicate_names_within_one_file() {
        let json = r#"{
            "measurements": [
                { "name": "waist_circ", "value": 70.0 },
                { "name": "waist_circ", "value": 72.5 }
            ]
        }"#;

        let err = parse_measurements_str(json).unwrap_err();
        match err {
            MeasurementError::DuplicateName(name) => assert_eq!(name, "waist_circ"),
            other => panic!("expected DuplicateName, got {other:?}"),
        }
    }

    #[test]
    fn rejects_empty_name() {
        let json = r#"{ "measurements": [ { "name": "", "value": 1.0 } ] }"#;
        let err = parse_measurements_str(json).unwrap_err();
        assert!(matches!(err, MeasurementError::EmptyName));
    }

    #[test]
    fn rejects_non_finite_value() {
        // JSON has no NaN/Infinity literal, and serde_json itself rejects (as a
        // parse error) any numeric literal that would overflow f64 to infinity
        // rather than producing one — confirmed empirically, e.g. "1e309" fails
        // to parse with "number out of range". So this validation branch can
        // never actually be reached via parse_measurements_str's JSON input;
        // exercise the shared validation logic directly instead, bypassing JSON.
        let entries = vec![MeasurementEntry {
            name: "bad".to_string(),
            value: f64::NAN,
            description: None,
        }];
        let err = validate_entries(entries).unwrap_err();
        match err {
            MeasurementError::InvalidValue { name } => assert_eq!(name, "bad"),
            other => panic!("expected InvalidValue, got {other:?}"),
        }
    }

    #[test]
    fn load_from_nonexistent_path_is_an_io_error() {
        let path = Path::new("this/path/definitely/does/not/exist.json");
        let err = load_measurements_from_file(path).unwrap_err();
        assert!(matches!(err, MeasurementError::Io(_)));
    }

    #[test]
    fn load_from_fixture_returns_expected_values() {
        // CARGO_MANIFEST_DIR is crates/io at compile time; the fixtures directory
        // lives at the workspace root, two levels up from there.
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/measurements/sample.json");

        let map = load_measurements_from_file(&path).unwrap();
        assert_eq!(map.get("height_scapula"), Some(&40.0));
        assert_eq!(map.get("waist_circ"), Some(&72.5));
    }
}
