use std::collections::HashMap; // backs the executor's name -> ObjectId tracking table
use std::path::Path; // the parameter type for load_action_script, matching io::load_measurements_from_file's own convention

/// One instruction in an action script: a direct, one-to-one mapping onto
/// an existing `core_lib::Document::add_*` constructor.
///
/// Internally tagged (`#[serde(tag = "op")]`): the JSON's `"op"` field
/// selects which variant this is, with that variant's own fields sitting
/// flat alongside `"op"` in the same JSON object — e.g.
/// `{"op":"add_base_point","name":"A","x":0.0,"y":0.0}`. Every point
/// reference here (`base_point`, `p1`, `p2`, `p3`, `point`, `line_p1`,
/// `line_p2`) is a `String` NAME, not a numeric id: a script author has no
/// way to know an `ObjectId` in advance, since ids are assigned at
/// runtime — `execute_action_script` below resolves each name to the
/// `ObjectId` an earlier action produced.
#[derive(Debug, Clone, serde::Deserialize)]
// Debug: printable in test failures; Clone: tests build one and reuse it; Deserialize: parsed directly from action-script JSON
#[serde(tag = "op")]
// the "op" field is the tagged-union discriminator
// Every variant deliberately shares the `Add` prefix, mirroring the
// `add_*` naming both the JSON `"op"` values and the `Document::add_*`
// constructors each variant maps onto use — clippy's "all variants share a
// prefix" lint is a false positive here, not an accidental naming smell.
#[allow(clippy::enum_variant_names)]
pub enum Action {
    /// Maps to `Document::add_base_point(name, x, y)`.
    #[serde(rename = "add_base_point")]
    AddBasePoint {
        name: String, // -> add_base_point's `name` parameter
        x: f64,       // -> add_base_point's `x` parameter
        y: f64,       // -> add_base_point's `y` parameter
    },

    /// Maps to `Document::add_end_line(name, base_point, angle_formula, length_formula)`.
    #[serde(rename = "add_end_line")]
    AddEndLine {
        name: String,           // -> add_end_line's `name` parameter
        base_point: String, // the NAME of an earlier point; resolved to add_end_line's `base_point` parameter
        angle_formula: String, // -> add_end_line's `angle_formula` parameter
        length_formula: String, // -> add_end_line's `length_formula` parameter
    },

    /// Maps to `Document::add_line(p1, p2)`. The only action here that
    /// introduces no new named point (a `Line` has no user-facing label in
    /// `core_lib`'s `ToolKind::Line`).
    #[serde(rename = "add_line")]
    AddLine {
        p1: String, // the NAME of an earlier point; resolved to add_line's `p1` parameter
        p2: String, // the NAME of an earlier point; resolved to add_line's `p2` parameter
    },

    /// Maps to `Document::add_along_line(name, p1, p2, length_formula)`.
    #[serde(rename = "add_along_line")]
    AddAlongLine {
        name: String,           // -> add_along_line's `name` parameter
        p1: String, // the NAME of an earlier point; resolved to add_along_line's `p1` parameter
        p2: String, // the NAME of an earlier point; resolved to add_along_line's `p2` parameter
        length_formula: String, // -> add_along_line's `length_formula` parameter
    },

    /// Maps to `Document::add_normal(name, p1, p2, length_formula, angle_formula)`.
    #[serde(rename = "add_normal")]
    AddNormal {
        name: String,           // -> add_normal's `name` parameter
        p1: String, // the NAME of an earlier point; resolved to add_normal's `p1` parameter
        p2: String, // the NAME of an earlier point; resolved to add_normal's `p2` parameter
        length_formula: String, // -> add_normal's `length_formula` parameter
        angle_formula: String, // -> add_normal's `angle_formula` parameter
    },

    /// Maps to `Document::add_bisector(name, p1, p2, p3, length_formula)`.
    #[serde(rename = "add_bisector")]
    AddBisector {
        name: String,           // -> add_bisector's `name` parameter
        p1: String, // the NAME of an earlier point; resolved to add_bisector's `p1` parameter
        p2: String, // the NAME of an earlier point; resolved to add_bisector's `p2` parameter (the angle's vertex)
        p3: String, // the NAME of an earlier point; resolved to add_bisector's `p3` parameter
        length_formula: String, // -> add_bisector's `length_formula` parameter
    },

    /// Maps to `Document::add_height(name, point, line_p1, line_p2)`.
    #[serde(rename = "add_height")]
    AddHeight {
        name: String,    // -> add_height's `name` parameter
        point: String,   // the NAME of an earlier point; resolved to add_height's `point` parameter
        line_p1: String, // the NAME of an earlier point; resolved to add_height's `line_p1` parameter
        line_p2: String, // the NAME of an earlier point; resolved to add_height's `line_p2` parameter
    },

    /// Maps to `Document::add_midpoint(name, p1, p2)`.
    #[serde(rename = "add_midpoint")]
    AddMidpoint {
        name: String, // -> add_midpoint's `name` parameter
        p1: String,   // the NAME of an earlier point; resolved to add_midpoint's `p1` parameter
        p2: String,   // the NAME of an earlier point; resolved to add_midpoint's `p2` parameter
    },
}

/// The top-level shape of an action-script JSON file: an optional
/// measurement file reference, plus the ordered list of actions to
/// replay against a fresh `Document`.
#[derive(Debug, Clone, serde::Deserialize)] // same derive rationale as Action above
pub struct ActionScript {
    /// Path to a Phase-5-format measurement JSON file, applied to the
    /// `Document` (via `apply_measurements`) BEFORE any action runs, so
    /// every action's formulas can already reference its measurements.
    /// `None` if this script needs no measurements.
    pub measurements_path: Option<String>,
    /// Every action to replay, in order — later actions may reference
    /// points earlier ones defined, by name.
    pub actions: Vec<Action>,
}

/// Everything that can go wrong loading or executing an action script.
#[derive(Debug, thiserror::Error)] // Debug: printable in test failures/CLI error messages; Error: implements std::error::Error via thiserror
pub enum ActionScriptError {
    /// Reading the script file from disk failed. Wrapped via `#[from]` so
    /// `?` converts a `std::io::Error` automatically.
    #[error("failed to read action script: {0}")]
    Io(#[from] std::io::Error), // the underlying I/O failure

    /// The file's contents aren't valid JSON, or don't match `ActionScript`'s
    /// shape. Wrapped via `#[from]` so `?` converts a `serde_json::Error`
    /// automatically.
    #[error("failed to parse action script JSON: {0}")]
    Json(#[from] serde_json::Error), // the underlying JSON parse/shape failure

    /// An action referenced a point name that no earlier action defined.
    #[error("action referenced point {0:?}, which was never defined by an earlier action")]
    UnknownPointName(String), // the missing name

    /// Two actions both tried to introduce a point under the same name.
    ///
    /// Caught explicitly rather than letting the later action's point
    /// silently shadow the earlier one in the name -> id tracking map:
    /// that would make every earlier action's references to that name
    /// retroactively ambiguous (which point did they actually mean?), so
    /// this is rejected loudly instead of resolved by "last write wins".
    #[error("action defines point {0:?}, but that name is already in use")]
    DuplicateName(String), // the name that was already taken

    /// A `Document::add_*` constructor rejected an action (e.g. a
    /// genuinely invalid reference slipping past name resolution, though
    /// that shouldn't normally happen given the checks above). Wrapped via
    /// `#[from]` so `?` converts a `core_lib::PatternError` automatically.
    #[error("failed to build pattern: {0}")]
    Pattern(#[from] core_lib::PatternError), // the underlying Document-constructor failure

    /// Loading/parsing/validating the referenced measurement file failed.
    /// Wrapped via `#[from]` so `?` converts an `io::MeasurementError`
    /// automatically.
    #[error("failed to load measurements: {0}")]
    Measurement(#[from] io::MeasurementError), // the underlying measurement-file failure
}

/// Reads and parses the action script at `path`.
pub fn load_action_script(path: &Path) -> Result<ActionScript, ActionScriptError> {
    let contents = std::fs::read_to_string(path)?; // read the whole file into memory; propagate an I/O failure (e.g. file not found) via ActionScriptError::Io
    let script: ActionScript = serde_json::from_str(&contents)?; // parse the JSON text into the expected shape; propagate a malformed-JSON failure via ActionScriptError::Json
    Ok(script) // successfully loaded and parsed
}

/// Looks up `name` in the executor's name -> id tracking table, reporting
/// exactly which name is missing if it was never defined by an earlier
/// action.
fn resolve_name(
    names: &HashMap<String, core_lib::ObjectId>, // the tracking table built up so far by execute_action_script
    name: &str,                                  // the name this action referenced
) -> Result<core_lib::ObjectId, ActionScriptError> {
    names
        .get(name) // Option<&ObjectId>
        .copied() // Option<ObjectId>: ObjectId is Copy, so this avoids borrowing from `names`
        .ok_or_else(|| ActionScriptError::UnknownPointName(name.to_string())) // not found: report exactly which name is missing
}

/// Rejects `name` if it's already a key in the tracking table, since
/// letting a later action's point silently shadow an earlier same-named
/// one would make earlier actions' references to that name ambiguous.
fn reject_if_duplicate(
    names: &HashMap<String, core_lib::ObjectId>, // the tracking table built up so far
    name: &str,                                  // the name this action is about to introduce
) -> Result<(), ActionScriptError> {
    if names.contains_key(name) {
        // this name was already introduced by an earlier action
        return Err(ActionScriptError::DuplicateName(name.to_string())); // reject loudly rather than silently overwrite the earlier mapping
    }
    Ok(()) // name is free to use
}

/// Builds a fresh `core_lib::Document` by replaying every action in
/// `script.actions`, in order, resolving each named point reference
/// against the ids earlier actions produced.
pub fn execute_action_script(
    script: &ActionScript,
) -> Result<core_lib::Document, ActionScriptError> {
    let mut document = core_lib::Document::default(); // start from a completely empty pattern document
    let mut names: HashMap<String, core_lib::ObjectId> = HashMap::new(); // tracks every named point defined so far, built up as actions are processed in order

    if let Some(path) = &script.measurements_path {
        // Resolved relative to the CURRENT WORKING DIRECTORY, not the
        // action script file's own location: resolving relative to the
        // script file would be more robust (the script would keep working
        // regardless of where the CLI is invoked from), but this
        // implementation picks the simpler cwd-relative resolution, which
        // matches how this binary's other file-path CLI arguments (e.g.
        // `--output`) are also resolved relative to cwd.
        let measurements = io::load_measurements_from_file(Path::new(path))?; // propagate a missing/malformed measurement file via ActionScriptError::Measurement
        document.apply_measurements(measurements); // reuse Document's own existing "replace, not merge" measurement logic (Phase 5), rather than duplicating it here
    }

    for action in &script.actions {
        // process every action in order, so later actions can reference points earlier ones defined
        match action {
            Action::AddBasePoint { name, x, y } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let id = document.add_base_point(name.clone(), *x, *y); // infallible: a literal starting point has no references to validate
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddEndLine {
                name,
                base_point,
                angle_formula,
                length_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let base_point_id = resolve_name(&names, base_point)?; // look up the referenced point's id, or report exactly which name is missing
                let id = document.add_end_line(
                    name.clone(),
                    base_point_id,
                    angle_formula.clone(),
                    length_formula.clone(),
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddLine { p1, p2 } => {
                // add_line introduces no new named point: nothing to check for duplication, and nothing to insert into `names` afterward
                let p1_id = resolve_name(&names, p1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_id = resolve_name(&names, p2)?; // same, for the second referenced point
                document.add_line(p1_id, p2_id)?; // propagate a Document-side validation failure via ActionScriptError::Pattern
            }
            Action::AddAlongLine {
                name,
                p1,
                p2,
                length_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_id = resolve_name(&names, p1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_id = resolve_name(&names, p2)?; // same, for the second referenced point
                let id =
                    document.add_along_line(name.clone(), p1_id, p2_id, length_formula.clone())?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddNormal {
                name,
                p1,
                p2,
                length_formula,
                angle_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_id = resolve_name(&names, p1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_id = resolve_name(&names, p2)?; // same, for the second referenced point
                let id = document.add_normal(
                    name.clone(),
                    p1_id,
                    p2_id,
                    length_formula.clone(),
                    angle_formula.clone(),
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddBisector {
                name,
                p1,
                p2,
                p3,
                length_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_id = resolve_name(&names, p1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_id = resolve_name(&names, p2)?; // same, for the second referenced point (the angle's vertex)
                let p3_id = resolve_name(&names, p3)?; // same, for the third referenced point
                let id = document.add_bisector(
                    name.clone(),
                    p1_id,
                    p2_id,
                    p3_id,
                    length_formula.clone(),
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddHeight {
                name,
                point,
                line_p1,
                line_p2,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let point_id = resolve_name(&names, point)?; // look up the referenced point's id, or report exactly which name is missing
                let line_p1_id = resolve_name(&names, line_p1)?; // same, for the line's first defining point
                let line_p2_id = resolve_name(&names, line_p2)?; // same, for the line's second defining point
                let id = document.add_height(name.clone(), point_id, line_p1_id, line_p2_id)?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddMidpoint { name, p1, p2 } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_id = resolve_name(&names, p1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_id = resolve_name(&names, p2)?; // same, for the second referenced point
                let id = document.add_midpoint(name.clone(), p1_id, p2_id)?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
        }
    }

    Ok(document) // every action executed successfully
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_action_script_builds_correct_geometry_from_named_references() {
        let script = ActionScript {
            measurements_path: None, // seeded directly on the Document below instead, for this inline test
            actions: vec![
                Action::AddBasePoint {
                    name: "A".to_string(),
                    x: 0.0,
                    y: 0.0,
                },
                Action::AddEndLine {
                    name: "A1".to_string(),
                    base_point: "A".to_string(),
                    angle_formula: "0".to_string(),
                    length_formula: "height_scapula/10".to_string(),
                },
                Action::AddLine {
                    p1: "A".to_string(),
                    p2: "A1".to_string(),
                },
            ],
        };

        let mut document = execute_action_script(&script).unwrap();
        document.set_variable(
            "height_scapula",
            core_lib::Variable::Measurement { value: 40.0 },
        ); // seed the measurement execute_action_script didn't load (measurements_path was None here)

        let data = core_lib::recompute_all(&document).unwrap();
        let a1_id = document
            .history()
            .iter()
            .find(|record| matches!(&record.kind, core_lib::ToolKind::EndLine { name, .. } if name == "A1"))
            .unwrap()
            .id;
        let point = data.get_point(a1_id).unwrap();
        assert!((point.x - 4.0).abs() < 1e-9); // height_scapula/10 = 4.0, at angle 0, from (0,0)
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn action_referencing_undefined_name_is_reported() {
        let script = ActionScript {
            measurements_path: None,
            actions: vec![Action::AddEndLine {
                name: "A1".to_string(),
                base_point: "does_not_exist".to_string(),
                angle_formula: "0".to_string(),
                length_formula: "10".to_string(),
            }],
        };

        let err = execute_action_script(&script).unwrap_err();
        assert!(
            matches!(err, ActionScriptError::UnknownPointName(name) if name == "does_not_exist")
        );
    }

    #[test]
    fn two_actions_defining_the_same_name_is_reported() {
        let script = ActionScript {
            measurements_path: None,
            actions: vec![
                Action::AddBasePoint {
                    name: "A".to_string(),
                    x: 0.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "A".to_string(),
                    x: 1.0,
                    y: 1.0,
                },
            ],
        };

        let err = execute_action_script(&script).unwrap_err();
        assert!(matches!(err, ActionScriptError::DuplicateName(name) if name == "A"));
    }

    #[test]
    fn load_action_script_from_fixture_recomputes_to_correct_coordinates() {
        // CARGO_MANIFEST_DIR is crates/cli at compile time; the fixtures directory
        // lives at the workspace root, two levels up from there (same pattern as io's own tests).
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/actions/simple_line.json");

        let mut script = load_action_script(&path).unwrap();
        // execute_action_script resolves measurements_path relative to the
        // CURRENT WORKING DIRECTORY (see its own doc comment), but `cargo
        // test`'s default working directory is this crate's own manifest
        // directory, not the workspace root the fixture's relative path
        // assumes — so it's rewritten here to an absolute path, the same
        // CARGO_MANIFEST_DIR-relative way every other fixture path in this
        // test suite is resolved. The real cwd-relative resolution itself
        // is exercised end-to-end by crates/cli/tests/run_command.rs,
        // which runs the actual built binary from the workspace root.
        script.measurements_path = Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/measurements/sample.json")
                .to_string_lossy()
                .into_owned(),
        );

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();

        let a1_id = document
            .history()
            .iter()
            .find(|record| matches!(&record.kind, core_lib::ToolKind::EndLine { name, .. } if name == "A1"))
            .unwrap()
            .id;
        let point = data.get_point(a1_id).unwrap();
        // fixtures/measurements/sample.json has height_scapula = 40.0, so height_scapula/10 = 4.0, at angle 0, from A (0,0).
        assert!((point.x - 4.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn load_action_script_on_nonexistent_path_is_an_io_error() {
        let path = std::path::Path::new("this/path/definitely/does/not/exist.json");
        let err = load_action_script(path).unwrap_err();
        assert!(matches!(err, ActionScriptError::Io(_)));
    }

    #[test]
    fn load_action_script_on_malformed_json_is_a_json_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "{ not valid json").unwrap();

        let err = load_action_script(&path).unwrap_err();
        assert!(matches!(err, ActionScriptError::Json(_)));
    }
}
