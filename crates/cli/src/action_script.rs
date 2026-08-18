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

    /// Maps to `Document::remove_tool(id)`, resolved from `name`. Modifies
    /// the loaded/starting Document IN PLACE rather than rebuilding it: the
    /// named tool's `ToolRecord` is deleted from history, and every other
    /// record (loaded or added earlier in this same script) is left
    /// completely untouched.
    #[serde(rename = "remove_tool")]
    RemoveTool {
        name: String, // the NAME of an earlier (or loaded) point; resolved to the id passed to remove_tool
    },

    /// Maps to `Document::replace_tool_kind(id, ToolKind::BasePoint { .. })`,
    /// resolved from `name`. Both `x`/`y` are optional so a script can move
    /// a point along just one axis without needing to already know and
    /// repeat the other axis's current value; whichever field is `None`
    /// falls back to that point's CURRENT stored value.
    #[serde(rename = "modify_base_point")]
    ModifyBasePoint {
        name: String, // the NAME of an earlier (or loaded) point; must already resolve to a ToolKind::BasePoint
        x: Option<f64>, // new literal x coordinate; `None` keeps the current value
        y: Option<f64>, // new literal y coordinate; `None` keeps the current value
    },

    /// Maps to `Document::replace_tool_kind(id, ToolKind::EndLine { .. })`,
    /// resolved from `name`. Same "optional field falls back to the
    /// current stored value" contract as `ModifyBasePoint`, applied to the
    /// angle/length formula strings instead of literal coordinates.
    #[serde(rename = "modify_end_line")]
    ModifyEndLine {
        name: String, // the NAME of an earlier (or loaded) point; must already resolve to a ToolKind::EndLine
        angle_formula: Option<String>, // new angle formula string; `None` keeps the current formula
        length_formula: Option<String>, // new length formula string; `None` keeps the current formula
    },

    /// Maps to `Document::add_shoulder_point(name, p1_line, p2_line, shoulder, length_formula)`.
    #[serde(rename = "add_shoulder_point")]
    AddShoulderPoint {
        name: String,           // -> add_shoulder_point's `name` parameter
        p1_line: String, // the NAME of an earlier point; resolved to add_shoulder_point's `p1_line` parameter
        p2_line: String, // the NAME of an earlier point; resolved to add_shoulder_point's `p2_line` parameter
        shoulder: String, // the NAME of an earlier point; resolved to add_shoulder_point's `shoulder` parameter
        length_formula: String, // -> add_shoulder_point's `length_formula` parameter
    },

    /// Maps to `Document::add_line_intersect(name, p1_line1, p2_line1, p1_line2, p2_line2)`.
    #[serde(rename = "add_line_intersect")]
    AddLineIntersect {
        name: String,     // -> add_line_intersect's `name` parameter
        p1_line1: String, // the NAME of an earlier point; resolved to add_line_intersect's `p1_line1` parameter
        p2_line1: String, // the NAME of an earlier point; resolved to add_line_intersect's `p2_line1` parameter
        p1_line2: String, // the NAME of an earlier point; resolved to add_line_intersect's `p1_line2` parameter
        p2_line2: String, // the NAME of an earlier point; resolved to add_line_intersect's `p2_line2` parameter
    },

    /// Maps to `Document::add_point_of_intersection(name, p1, p2)`.
    #[serde(rename = "add_point_of_intersection")]
    AddPointOfIntersection {
        name: String, // -> add_point_of_intersection's `name` parameter
        p1: String, // the NAME of an earlier point; resolved to add_point_of_intersection's `p1` parameter
        p2: String, // the NAME of an earlier point; resolved to add_point_of_intersection's `p2` parameter
    },

    /// Maps to `Document::add_triangle(name, axis_p1, axis_p2, hypotenuse_p1, hypotenuse_p2)`.
    #[serde(rename = "add_triangle")]
    AddTriangle {
        name: String,          // -> add_triangle's `name` parameter
        axis_p1: String, // the NAME of an earlier point; resolved to add_triangle's `axis_p1` parameter
        axis_p2: String, // the NAME of an earlier point; resolved to add_triangle's `axis_p2` parameter
        hypotenuse_p1: String, // the NAME of an earlier point; resolved to add_triangle's `hypotenuse_p1` parameter
        hypotenuse_p2: String, // the NAME of an earlier point; resolved to add_triangle's `hypotenuse_p2` parameter
    },

    /// Maps to `Document::add_point_of_contact(name, center, p1, p2, radius_formula)`.
    #[serde(rename = "add_point_of_contact")]
    AddPointOfContact {
        name: String,           // -> add_point_of_contact's `name` parameter
        center: String, // the NAME of an earlier point; resolved to add_point_of_contact's `center` parameter
        p1: String, // the NAME of an earlier point; resolved to add_point_of_contact's `p1` parameter
        p2: String, // the NAME of an earlier point; resolved to add_point_of_contact's `p2` parameter
        radius_formula: String, // -> add_point_of_contact's `radius_formula` parameter
    },

    /// Maps to `Document::add_piece(name, nodes, seam_allowance_formula)`.
    /// `points` is an ordered list of earlier points' NAMES (not raw
    /// `PieceNode`s): each name is resolved to a `PieceNode` with
    /// `excluded_from_seam_allowance` always `false` — this action format
    /// has no way to express per-node exclusion, matching every other
    /// action's "plain name list, no extra per-reference flags" shape;
    /// a script needing that finer control would need a new, separate
    /// action variant, not a change to this one.
    #[serde(rename = "add_piece")]
    AddPiece {
        name: String,                   // -> add_piece's `name` parameter
        points: Vec<String>, // the NAMEs of earlier points, in perimeter order; each resolved to a PieceNode
        seam_allowance_formula: String, // -> add_piece's `seam_allowance_formula` parameter
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
    /// Path to an existing pattern XML file (the format
    /// `io::serialize_document`/`io::deserialize_document` read/write) to
    /// load as the STARTING `Document`, instead of starting from
    /// `core_lib::Document::default()`. When present, every action in this
    /// script is applied ON TOP OF the loaded document's existing
    /// history, in order, exactly like actions are applied when there's
    /// no starting pattern — the only difference is the starting point
    /// isn't empty. `None` means this script builds a brand-new pattern
    /// from scratch, the original (Phase 9) behavior.
    pub load_pattern_path: Option<String>,
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

    /// Loading/parsing the referenced `load_pattern_path` pattern file
    /// failed. Wrapped via `#[from]` so `?` converts an
    /// `io::PatternFileError` automatically.
    #[error("failed to load pattern: {0}")]
    PatternFile(#[from] io::PatternFileError), // the underlying pattern-file load/parse failure

    /// A `Document::remove_tool`/`Document::replace_tool_kind` call (from a
    /// `remove_tool`/`modify_base_point`/`modify_end_line` action) was
    /// rejected. Wrapped via `#[from]` so `?` converts a
    /// `core_lib::DocumentError` automatically. Distinct from `Pattern`
    /// above: that variant wraps `core_lib::PatternError`, the error type
    /// the `add_*` constructors return, which is unrelated to `Document`'s
    /// own `DocumentError`.
    #[error("failed to modify document: {0}")]
    Document(#[from] core_lib::DocumentError), // the underlying Document-mutation failure

    /// A `modify_base_point`/`modify_end_line` action targeted a name that
    /// resolves to a tool of a different `ToolKind` than the action
    /// expects, e.g. `modify_base_point` on a name that's actually an
    /// `EndLine`.
    #[error("action expected {name:?} to be a {expected}, but it is a {found}")]
    WrongToolType {
        name: String,     // the name that was targeted
        expected: String, // the ToolKind variant the action required
        found: String,    // the ToolKind variant the name actually resolved to
    },
}

/// Reads and parses the action script at `path`.
pub fn load_action_script(path: &Path) -> Result<ActionScript, ActionScriptError> {
    let contents = std::fs::read_to_string(path)?; // read the whole file into memory; propagate an I/O failure (e.g. file not found) via ActionScriptError::Io
    let script: ActionScript = serde_json::from_str(&contents)?; // parse the JSON text into the expected shape; propagate a malformed-JSON failure via ActionScriptError::Json
    Ok(script) // successfully loaded and parsed
}

/// Extracts a `ToolKind`'s `name` field, if it has one.
///
/// `Document` itself has no built-in name index — names only exist as a
/// field inside each `ToolKind` — so anything that wants to resolve "the
/// point named B" back to an `ObjectId` has to scan history and extract
/// this field itself. Used both when building the initial `names` map from
/// a freshly loaded pattern file (below) and could equally be reused by
/// any future caller needing the same lookup (e.g. `main.rs`'s `view`
/// subcommand, which reuses this exact function rather than duplicating
/// the match).
pub(crate) fn tool_name(kind: &core_lib::ToolKind) -> Option<&str> {
    match kind {
        // every point-producing variant carries a user-facing `name` field
        core_lib::ToolKind::BasePoint { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::EndLine { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::AlongLine { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Normal { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Bisector { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Height { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Midpoint { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Piece { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::ShoulderPoint { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::LineIntersect { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::PointOfIntersection { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Triangle { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::PointOfContact { name, .. } => Some(name.as_str()),
        core_lib::ToolKind::Line { .. } => None, // the only variant with no user-facing name
    }
}

/// Returns the bare variant name of `kind` (e.g. `"BasePoint"`,
/// `"EndLine"`), for `ActionScriptError::WrongToolType`'s `found` field —
/// a human-readable label naming what a name actually resolved to, when a
/// `modify_*` action expected a different `ToolKind`.
pub(crate) fn tool_kind_variant_name(kind: &core_lib::ToolKind) -> &'static str {
    match kind {
        // one arm per ToolKind variant, returning its bare name as a string literal
        core_lib::ToolKind::BasePoint { .. } => "BasePoint",
        core_lib::ToolKind::EndLine { .. } => "EndLine",
        core_lib::ToolKind::Line { .. } => "Line",
        core_lib::ToolKind::AlongLine { .. } => "AlongLine",
        core_lib::ToolKind::Normal { .. } => "Normal",
        core_lib::ToolKind::Bisector { .. } => "Bisector",
        core_lib::ToolKind::Height { .. } => "Height",
        core_lib::ToolKind::Midpoint { .. } => "Midpoint",
        core_lib::ToolKind::Piece { .. } => "Piece",
        core_lib::ToolKind::ShoulderPoint { .. } => "ShoulderPoint",
        core_lib::ToolKind::LineIntersect { .. } => "LineIntersect",
        core_lib::ToolKind::PointOfIntersection { .. } => "PointOfIntersection",
        core_lib::ToolKind::Triangle { .. } => "Triangle",
        core_lib::ToolKind::PointOfContact { .. } => "PointOfContact",
    }
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
    // `document`/`names`/`embedded_measurements_path` are built up
    // differently depending on whether `load_pattern_path` is set, then
    // used identically below regardless of which branch produced them.
    let (mut document, mut names, embedded_measurements_path): (
        core_lib::Document,
        HashMap<String, core_lib::ObjectId>,
        Option<String>,
    ) = match &script.load_pattern_path {
        Some(path) => {
            // Resolved relative to the CURRENT WORKING DIRECTORY, same
            // convention as `measurements_path` just below (see that
            // branch's own comment for the rationale).
            let contents = std::fs::read_to_string(Path::new(path))?; // propagate a missing/unreadable file via ActionScriptError::Io
            let (loaded_document, embedded_path) = io::deserialize_document(&contents)?; // propagate a malformed pattern file via ActionScriptError::PatternFile

            let mut names = HashMap::new(); // built fresh from the loaded document's own history
            for record in loaded_document.history() {
                // every named tool in the loaded file gets indexed, so later actions can reference it by name
                if let Some(name) = tool_name(&record.kind) {
                    // Deliberately permissive: if a hand-edited or
                    // externally-produced XML file somehow contains two
                    // tools sharing a name (which a session built entirely
                    // through this executor's own DuplicateName check
                    // below could never produce), the LATER one in history
                    // order simply wins the map entry here, rather than
                    // rejecting the load outright. This is distinct from —
                    // and more permissive than — the stricter
                    // DuplicateName rejection this same function still
                    // applies below to any NEW name a script action itself
                    // tries to introduce.
                    names.insert(name.to_string(), record.id);
                }
            }
            (loaded_document, names, embedded_path)
        }
        None => (core_lib::Document::default(), HashMap::new(), None), // unchanged original behavior: start from a completely empty pattern document
    };

    // Script-level `measurements_path` takes precedence (unchanged
    // existing behavior); otherwise, fall back to whatever
    // `<measurementsPath>` the loaded pattern file itself referenced, if
    // any — a saved pattern file may reference the measurement file it was
    // built against, and using it when the action script doesn't specify
    // its own override is a reasonable default. If neither is present, no
    // measurements are applied, exactly as before this capability existed.
    let effective_measurements_path = script
        .measurements_path
        .clone()
        .or(embedded_measurements_path);

    if let Some(path) = &effective_measurements_path {
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
            Action::RemoveTool { name } => {
                // MODIFIES the loaded/starting Document IN PLACE by
                // deleting one ToolRecord — it must never rebuild the
                // Document from scratch or lose any other unrelated
                // ToolRecord already in history.
                let id = resolve_name(&names, name)?; // look up the referenced tool's id, or report exactly which name is missing
                document.remove_tool(id)?; // propagate a Document-side failure (e.g. ToolInUse) via ActionScriptError::Document; called directly, not through UndoStack, since this executor builds a standalone Document with no UndoStack involved
                names.remove(name); // the tool this name pointed to no longer exists, so leaving the stale mapping would let a LATER action wrongly reference a since-removed tool as if it still existed
            }
            Action::ModifyBasePoint { name, x, y } => {
                // MODIFIES an existing ToolRecord's kind in place — it must
                // never rebuild the Document from scratch or lose any
                // other unrelated ToolRecord already in history.
                let id = resolve_name(&names, name)?; // look up the referenced tool's id, or report exactly which name is missing
                let current = document.get_tool(id).ok_or(ActionScriptError::Document(
                    core_lib::DocumentError::ToolNotFound(id),
                ))?; // defensive: `id` just came from `names`, which is always kept in sync with the Document
                let core_lib::ToolKind::BasePoint {
                    name: current_name,
                    x: current_x,
                    y: current_y,
                } = &current.kind
                else {
                    // this name resolves to a tool of a different kind: refuse rather than silently misinterpreting it
                    return Err(ActionScriptError::WrongToolType {
                        name: name.clone(),
                        expected: "BasePoint".to_string(),
                        found: tool_kind_variant_name(&current.kind).to_string(),
                    });
                };
                let new_kind = core_lib::ToolKind::BasePoint {
                    name: current_name.clone(), // the name itself is never changed by this action
                    x: x.unwrap_or(*current_x), // override only if the action provided a new x, otherwise keep the current value
                    y: y.unwrap_or(*current_y), // same, for y: this is what lets a script move a point along just one axis
                };
                document.replace_tool_kind(id, new_kind)?; // propagate a Document-side validation failure via ActionScriptError::Document
            }
            Action::ModifyEndLine {
                name,
                angle_formula,
                length_formula,
            } => {
                // MODIFIES an existing ToolRecord's kind in place — same
                // "never rebuild from scratch" contract as ModifyBasePoint
                // above.
                let id = resolve_name(&names, name)?; // look up the referenced tool's id, or report exactly which name is missing
                let current = document.get_tool(id).ok_or(ActionScriptError::Document(
                    core_lib::DocumentError::ToolNotFound(id),
                ))?; // defensive: `id` just came from `names`, which is always kept in sync with the Document
                let core_lib::ToolKind::EndLine {
                    name: current_name,
                    base_point: current_base_point,
                    angle_formula: current_angle_formula,
                    length_formula: current_length_formula,
                } = &current.kind
                else {
                    // this name resolves to a tool of a different kind: refuse rather than silently misinterpreting it
                    return Err(ActionScriptError::WrongToolType {
                        name: name.clone(),
                        expected: "EndLine".to_string(),
                        found: tool_kind_variant_name(&current.kind).to_string(),
                    });
                };
                let new_kind = core_lib::ToolKind::EndLine {
                    name: current_name.clone(), // the name itself is never changed by this action
                    base_point: *current_base_point, // which point this is measured from is never changed by this action
                    angle_formula: angle_formula
                        .clone()
                        .unwrap_or_else(|| current_angle_formula.clone()), // override only if the action provided a new angle formula, otherwise keep the current one
                    length_formula: length_formula
                        .clone()
                        .unwrap_or_else(|| current_length_formula.clone()), // same, for the length formula
                };
                document.replace_tool_kind(id, new_kind)?; // propagate a Document-side validation failure via ActionScriptError::Document
            }
            Action::AddShoulderPoint {
                name,
                p1_line,
                p2_line,
                shoulder,
                length_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_line_id = resolve_name(&names, p1_line)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_line_id = resolve_name(&names, p2_line)?; // same, for the second referenced point
                let shoulder_id = resolve_name(&names, shoulder)?; // same, for the circle-center point
                let id = document.add_shoulder_point(
                    name.clone(),
                    p1_line_id,
                    p2_line_id,
                    shoulder_id,
                    length_formula.clone(),
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddLineIntersect {
                name,
                p1_line1,
                p2_line1,
                p1_line2,
                p2_line2,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_line1_id = resolve_name(&names, p1_line1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_line1_id = resolve_name(&names, p2_line1)?; // same, for the first line's second point
                let p1_line2_id = resolve_name(&names, p1_line2)?; // same, for the second line's first point
                let p2_line2_id = resolve_name(&names, p2_line2)?; // same, for the second line's second point
                let id = document.add_line_intersect(
                    name.clone(),
                    p1_line1_id,
                    p2_line1_id,
                    p1_line2_id,
                    p2_line2_id,
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddPointOfIntersection { name, p1, p2 } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let p1_id = resolve_name(&names, p1)?; // look up the referenced point's id, or report exactly which name is missing
                let p2_id = resolve_name(&names, p2)?; // same, for the second referenced point
                let id = document.add_point_of_intersection(name.clone(), p1_id, p2_id)?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddTriangle {
                name,
                axis_p1,
                axis_p2,
                hypotenuse_p1,
                hypotenuse_p2,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let axis_p1_id = resolve_name(&names, axis_p1)?; // look up the referenced point's id, or report exactly which name is missing
                let axis_p2_id = resolve_name(&names, axis_p2)?; // same, for the axis's second point
                let hypotenuse_p1_id = resolve_name(&names, hypotenuse_p1)?; // same, for the hypotenuse's first point
                let hypotenuse_p2_id = resolve_name(&names, hypotenuse_p2)?; // same, for the hypotenuse's second point
                let id = document.add_triangle(
                    name.clone(),
                    axis_p1_id,
                    axis_p2_id,
                    hypotenuse_p1_id,
                    hypotenuse_p2_id,
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddPointOfContact {
                name,
                center,
                p1,
                p2,
                radius_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let center_id = resolve_name(&names, center)?; // look up the referenced point's id, or report exactly which name is missing
                let p1_id = resolve_name(&names, p1)?; // same, for the segment's first endpoint
                let p2_id = resolve_name(&names, p2)?; // same, for the segment's second endpoint
                let id = document.add_point_of_contact(
                    name.clone(),
                    center_id,
                    p1_id,
                    p2_id,
                    radius_formula.clone(),
                )?; // propagate a Document-side validation failure via ActionScriptError::Pattern
                names.insert(name.clone(), id); // record this name so later actions can reference it
            }
            Action::AddPiece {
                name,
                points,
                seam_allowance_formula,
            } => {
                reject_if_duplicate(&names, name)?; // refuse before touching the Document if this name is already taken
                let mut nodes = Vec::with_capacity(points.len()); // the resolved PieceNode list, built up in perimeter order below
                for point_name in points {
                    // resolve every perimeter point name to an id, in the order this action lists them, matching add_piece's own node-order contract
                    let point_id = resolve_name(&names, point_name)?; // look up the referenced point's id, or report exactly which name is missing
                    nodes.push(core_lib::PieceNode {
                        point: point_id,                     // this boundary vertex's resolved point id
                        excluded_from_seam_allowance: false, // this action format has no way to express per-node exclusion; see AddPiece's own doc comment
                    });
                }
                let id = document.add_piece(name.clone(), nodes, seam_allowance_formula.clone())?; // propagate a Document-side validation failure via ActionScriptError::Pattern
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
            load_pattern_path: None, // this test builds a document from scratch
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
            load_pattern_path: None, // this test builds a document from scratch
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
            load_pattern_path: None, // this test builds a document from scratch
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

    /// Writes `doc` out to a fresh temp-directory pattern XML file and
    /// returns both the `TempDir` guard (the CALLER must keep it bound to a
    /// named variable so the directory isn't deleted before the test uses
    /// the path) and that file's path as a `String`, ready to plug straight
    /// into `ActionScript::load_pattern_path`. Shared by every test below
    /// that needs a small starting pattern to load.
    fn write_pattern_to_tempfile(doc: &core_lib::Document) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap(); // the directory this pattern file lives in, kept alive by the caller
        let path = dir.path().join("loaded.xml");
        let xml = io::serialize_document(doc, None).unwrap();
        std::fs::write(&path, xml).unwrap();
        let path_string = path.to_string_lossy().into_owned(); // owned, so it doesn't borrow from `path`/`dir`
        (dir, path_string)
    }

    #[test]
    fn load_pattern_path_preserves_loaded_point_and_adds_a_new_one() {
        let mut loaded_doc = core_lib::Document::default();
        let loaded_id = loaded_doc.add_base_point("Existing", 3.0, 4.0); // the point this test proves survives untouched
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![Action::AddBasePoint {
                name: "New".to_string(),
                x: 1.0,
                y: 1.0,
            }],
        };

        let document = execute_action_script(&script).unwrap();
        assert_eq!(document.history().len(), 2); // the loaded point plus the newly added one

        // the loaded point's ORIGINAL id and kind are preserved exactly
        let preserved = document.get_tool(loaded_id).unwrap();
        assert_eq!(
            preserved.kind,
            core_lib::ToolKind::BasePoint {
                name: "Existing".to_string(),
                x: 3.0,
                y: 4.0,
            }
        );

        let new_tool = document
            .history()
            .iter()
            .find(
                |r| matches!(&r.kind, core_lib::ToolKind::BasePoint { name, .. } if name == "New"),
            )
            .unwrap();
        assert!(
            matches!(&new_tool.kind, core_lib::ToolKind::BasePoint { x, y, .. } if *x == 1.0 && *y == 1.0)
        );
    }

    #[test]
    fn actions_can_reference_a_loaded_points_original_name() {
        let mut loaded_doc = core_lib::Document::default();
        loaded_doc.add_base_point("A", 0.0, 0.0); // never redefined by the script below
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![Action::AddEndLine {
                name: "A1".to_string(),
                base_point: "A".to_string(), // resolves against the LOADED document's name index
                angle_formula: "0".to_string(),
                length_formula: "10".to_string(),
            }],
        };

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();
        let a1_id = document
            .history()
            .iter()
            .find(|r| matches!(&r.kind, core_lib::ToolKind::EndLine { name, .. } if name == "A1"))
            .unwrap()
            .id;
        let point = data.get_point(a1_id).unwrap();
        assert!((point.x - 10.0).abs() < 1e-9); // angle 0, length 10, from A (0,0)
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn remove_tool_removes_only_the_named_tool() {
        let mut loaded_doc = core_lib::Document::default();
        let a = loaded_doc.add_base_point("A", 0.0, 0.0); // to be removed
        let b = loaded_doc.add_base_point("B", 1.0, 1.0); // unrelated, unconnected: must survive untouched
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![Action::RemoveTool {
                name: "A".to_string(),
            }],
        };

        let document = execute_action_script(&script).unwrap();
        assert!(document.get_tool(a).is_none()); // A is genuinely gone
        assert!(!document
            .history()
            .iter()
            .any(|r| matches!(&r.kind, core_lib::ToolKind::BasePoint { name, .. } if name == "A")));

        // B is completely unaffected: same id, same kind data as before
        let preserved_b = document.get_tool(b).unwrap();
        assert_eq!(
            preserved_b.kind,
            core_lib::ToolKind::BasePoint {
                name: "B".to_string(),
                x: 1.0,
                y: 1.0,
            }
        );
    }

    #[test]
    fn remove_tool_still_referenced_fails_as_a_document_error_and_leaves_document_unchanged() {
        let mut loaded_doc = core_lib::Document::default();
        let a = loaded_doc.add_base_point("A", 0.0, 0.0);
        let a1 = loaded_doc.add_end_line("A1", a, "0", "10").unwrap(); // depends on A, blocking its removal
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let failing_script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path.clone()),
            actions: vec![Action::RemoveTool {
                name: "A".to_string(),
            }],
        };
        let err = execute_action_script(&failing_script).unwrap_err();
        assert!(matches!(
            err,
            ActionScriptError::Document(core_lib::DocumentError::ToolInUse { id, used_by })
                if id == a && used_by == a1
        ));

        // execute_action_script never writes to disk, and Document::remove_tool
        // is proven atomic (document.rs's own tests) — so a completely fresh
        // load of the same file, with no actions at all, reflects exactly the
        // same state the failed script's Document was in right before its one
        // (rejected) action: reloading it here and comparing against the
        // originally-built loaded_doc proves nothing was left half-mutated.
        let reload_only_script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![],
        };
        let reloaded = execute_action_script(&reload_only_script).unwrap();
        assert_eq!(reloaded.history(), loaded_doc.history());
    }

    #[test]
    fn modify_base_point_overrides_only_the_provided_axis() {
        let mut loaded_doc = core_lib::Document::default();
        let a = loaded_doc.add_base_point("A", 0.0, 0.0);
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![Action::ModifyBasePoint {
                name: "A".to_string(),
                x: Some(5.0), // overridden
                y: None,      // kept as-is
            }],
        };

        let document = execute_action_script(&script).unwrap();
        let tool = document.get_tool(a).unwrap(); // same id as before the modification
        assert_eq!(
            tool.kind,
            core_lib::ToolKind::BasePoint {
                name: "A".to_string(),
                x: 5.0,
                y: 0.0, // unchanged from its original value
            }
        );
    }

    #[test]
    fn modify_base_point_on_a_non_base_point_is_reported_as_wrong_tool_type() {
        let mut loaded_doc = core_lib::Document::default();
        let a = loaded_doc.add_base_point("A", 0.0, 0.0);
        loaded_doc.add_end_line("B", a, "0", "10").unwrap(); // "B" is an EndLine, not a BasePoint
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![Action::ModifyBasePoint {
                name: "B".to_string(),
                x: Some(1.0),
                y: None,
            }],
        };

        let err = execute_action_script(&script).unwrap_err();
        match err {
            ActionScriptError::WrongToolType {
                name,
                expected,
                found,
            } => {
                assert_eq!(name, "B");
                assert_eq!(expected, "BasePoint");
                assert_eq!(found, "EndLine");
            }
            other => panic!("expected WrongToolType, got {other:?}"),
        }
    }

    #[test]
    fn modify_end_line_overrides_only_the_provided_formula() {
        let mut loaded_doc = core_lib::Document::default();
        let a = loaded_doc.add_base_point("A", 0.0, 0.0);
        let b = loaded_doc.add_end_line("B", a, "0", "10").unwrap();
        let (_dir, pattern_path) = write_pattern_to_tempfile(&loaded_doc); // `_dir` must stay bound: dropping it would delete the file `pattern_path` names

        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: Some(pattern_path),
            actions: vec![Action::ModifyEndLine {
                name: "B".to_string(),
                angle_formula: None,                    // kept as-is
                length_formula: Some("20".to_string()), // overridden
            }],
        };

        let document = execute_action_script(&script).unwrap();
        assert!(document.get_tool(b).is_some()); // still present under its ORIGINAL id, not removed and re-added under a new one
        let data = core_lib::recompute_all(&document).unwrap();
        let point = data.get_point(b).unwrap();
        assert!((point.x - 20.0).abs() < 1e-9); // angle 0 (unchanged), length 20 (overridden), from A (0,0)
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    // THE key end-to-end test for this whole feature: an "L" shape (two
    // legs, an open corner) loaded from a fixture file, then turned into a
    // closed rectangle purely by ADDING a fourth point and two closing
    // edges — never by deleting and redrawing the original shape.
    #[test]
    fn loading_l_shape_and_applying_l_to_rectangle_script_closes_it_additively() {
        // CARGO_MANIFEST_DIR is crates/cli at compile time; fixtures live at
        // the workspace root, two levels up — same convention as this
        // module's other fixture-based test above.
        let fixtures_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let pattern_path = fixtures_dir.join("patterns/l_shape.xml");
        let script_path = fixtures_dir.join("actions/l_to_rectangle.json");

        let mut script = load_action_script(&script_path).unwrap();
        // Rewritten to an absolute path for the same cwd-relative-resolution
        // reason load_action_script_from_fixture_recomputes_to_correct_coordinates
        // rewrites measurements_path above: `cargo test`'s default working
        // directory is this crate's own manifest directory, not the
        // workspace root the fixture's relative load_pattern_path assumes.
        script.load_pattern_path = Some(pattern_path.to_string_lossy().into_owned());

        let document = execute_action_script(&script).unwrap();
        // 5 original records (A, B, C, line A-B, line B-C) + 3 new ones (D,
        // line C-D, line D-A) = 8.
        assert_eq!(document.history().len(), 8);

        // Every ORIGINAL record survives with an IDENTICAL id and kind,
        // proven by loading the raw fixture completely independently (no
        // script involved at all) and comparing each of its records
        // directly against the corresponding one in the script-modified
        // Document.
        let raw_xml = std::fs::read_to_string(&pattern_path).unwrap();
        let (raw_l_shape, _measurements_path) = io::deserialize_document(&raw_xml).unwrap();
        for original_record in raw_l_shape.history() {
            let modified_record = document.get_tool(original_record.id).unwrap();
            assert_eq!(&modified_record.kind, &original_record.kind);
        }

        let data = core_lib::recompute_all(&document).unwrap();

        let point_id = |name: &str| {
            document
                .history()
                .iter()
                .find(|r| tool_name(&r.kind) == Some(name))
                .unwrap()
                .id
        };
        let a = data.get_point(point_id("A")).unwrap();
        let b = data.get_point(point_id("B")).unwrap();
        let c = data.get_point(point_id("C")).unwrap();
        let d = data.get_point(point_id("D")).unwrap();

        let close =
            |x: f64, y: f64, px: f64, py: f64| (x - px).abs() < 1e-9 && (y - py).abs() < 1e-9; // shared coordinate-comparison helper for every check below

        assert!(close(a.x, a.y, 0.0, 0.0));
        assert!(close(b.x, b.y, 10.0, 0.0));
        assert!(close(c.x, c.y, 10.0, 5.0));
        assert!(close(d.x, d.y, 0.0, 5.0));

        // Exactly 4 resolved lines, together tracing the rectangle's full
        // perimeter — checked by coordinates, not assuming any particular
        // ordering among them.
        let resolved_edges: Vec<((f64, f64), (f64, f64))> = data
            .objects()
            .filter_map(|(_, object)| match object {
                core_lib::GeoObject::Line(line) => {
                    let p1 = data.get_point(line.p1).unwrap();
                    let p2 = data.get_point(line.p2).unwrap();
                    Some(((p1.x, p1.y), (p2.x, p2.y)))
                }
                _ => None,
            })
            .collect();
        assert_eq!(resolved_edges.len(), 4);

        let expected_edges = [
            ((0.0, 0.0), (10.0, 0.0)),  // A-B
            ((10.0, 0.0), (10.0, 5.0)), // B-C
            ((10.0, 5.0), (0.0, 5.0)),  // C-D
            ((0.0, 5.0), (0.0, 0.0)),   // D-A
        ];
        for (expected_p1, expected_p2) in expected_edges {
            let found = resolved_edges.iter().any(|&(p1, p2)| {
                (close(p1.0, p1.1, expected_p1.0, expected_p1.1)
                    && close(p2.0, p2.1, expected_p2.0, expected_p2.1))
                    || (close(p1.0, p1.1, expected_p2.0, expected_p2.1)
                        && close(p2.0, p2.1, expected_p1.0, expected_p1.1))
            }); // either endpoint order counts as the same edge
            assert!(found, "missing edge {expected_p1:?}-{expected_p2:?}");
        }
    }

    // Part C end-to-end tests: one per newly added tool, confirming the
    // new Action variant, invoked from an action script referencing
    // earlier points by name, produces correct geometry — same golden
    // values as recompute.rs's own Part C tests, exercised through this
    // layer instead.

    #[test]
    fn add_shoulder_point_action_produces_correct_geometry() {
        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: None,
            actions: vec![
                Action::AddBasePoint {
                    name: "P1Line".to_string(),
                    x: 0.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "P2Line".to_string(),
                    x: 10.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "Shoulder".to_string(),
                    x: 15.0,
                    y: 8.0,
                },
                Action::AddShoulderPoint {
                    name: "S".to_string(),
                    p1_line: "P1Line".to_string(),
                    p2_line: "P2Line".to_string(),
                    shoulder: "Shoulder".to_string(),
                    length_formula: "10".to_string(),
                },
            ],
        };

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();
        let s_id = document
            .history()
            .iter()
            .find(|r| tool_name(&r.kind) == Some("S"))
            .unwrap()
            .id;
        let point = data.get_point(s_id).unwrap();
        assert!((point.x - 21.0).abs() < 1e-9); // same hand-calculated value as recompute.rs's own ShoulderPoint golden test
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn add_line_intersect_action_produces_correct_geometry() {
        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: None,
            actions: vec![
                Action::AddBasePoint {
                    name: "P1L1".to_string(),
                    x: 0.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "P2L1".to_string(),
                    x: 3.0,
                    y: 3.0,
                },
                Action::AddBasePoint {
                    name: "P1L2".to_string(),
                    x: 0.0,
                    y: 4.0,
                },
                Action::AddBasePoint {
                    name: "P2L2".to_string(),
                    x: 4.0,
                    y: 0.0,
                },
                Action::AddLineIntersect {
                    name: "X".to_string(),
                    p1_line1: "P1L1".to_string(),
                    p2_line1: "P2L1".to_string(),
                    p1_line2: "P1L2".to_string(),
                    p2_line2: "P2L2".to_string(),
                },
            ],
        };

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();
        let x_id = document
            .history()
            .iter()
            .find(|r| tool_name(&r.kind) == Some("X"))
            .unwrap()
            .id;
        let point = data.get_point(x_id).unwrap();
        assert!((point.x - 2.0).abs() < 1e-9); // same hand-calculated value as recompute.rs's own LineIntersect golden test
        assert!((point.y - 2.0).abs() < 1e-9);
    }

    #[test]
    fn add_point_of_intersection_action_produces_correct_geometry() {
        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: None,
            actions: vec![
                Action::AddBasePoint {
                    name: "P1".to_string(),
                    x: 3.0,
                    y: 7.0,
                },
                Action::AddBasePoint {
                    name: "P2".to_string(),
                    x: 9.0,
                    y: -2.0,
                },
                Action::AddPointOfIntersection {
                    name: "X".to_string(),
                    p1: "P1".to_string(),
                    p2: "P2".to_string(),
                },
            ],
        };

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();
        let x_id = document
            .history()
            .iter()
            .find(|r| tool_name(&r.kind) == Some("X"))
            .unwrap()
            .id;
        let point = data.get_point(x_id).unwrap();
        assert!((point.x - 3.0).abs() < 1e-9); // P1's x
        assert!((point.y - -2.0).abs() < 1e-9); // P2's y
    }

    #[test]
    fn add_triangle_action_produces_correct_geometry() {
        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: None,
            actions: vec![
                Action::AddBasePoint {
                    name: "AxisP1".to_string(),
                    x: 0.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "AxisP2".to_string(),
                    x: 20.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "HypP1".to_string(),
                    x: 8.0,
                    y: -3.0,
                },
                Action::AddBasePoint {
                    name: "HypP2".to_string(),
                    x: 14.0,
                    y: 9.0,
                },
                Action::AddTriangle {
                    name: "T".to_string(),
                    axis_p1: "AxisP1".to_string(),
                    axis_p2: "AxisP2".to_string(),
                    hypotenuse_p1: "HypP1".to_string(),
                    hypotenuse_p2: "HypP2".to_string(),
                },
            ],
        };

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();
        let t_id = document
            .history()
            .iter()
            .find(|r| tool_name(&r.kind) == Some("T"))
            .unwrap()
            .id;
        let point = data.get_point(t_id).unwrap();
        assert!((point.x - 17.0).abs() < 1e-9); // same hand-calculated value as recompute.rs's own Triangle golden test
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    // This is the ONLY test in this module where a new tool depends on
    // output from ANOTHER new tool, exercising named-reference resolution
    // across two Part C actions in the same script: the ShoulderPoint
    // result "S" becomes PointOfContact's own segment endpoint.
    #[test]
    fn add_point_of_contact_action_depending_on_a_shoulder_point_action_produces_correct_geometry()
    {
        let script = ActionScript {
            measurements_path: None,
            load_pattern_path: None,
            actions: vec![
                Action::AddBasePoint {
                    name: "P1Line".to_string(),
                    x: 0.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "P2Line".to_string(),
                    x: 10.0,
                    y: 0.0,
                },
                Action::AddBasePoint {
                    name: "Shoulder".to_string(),
                    x: 15.0,
                    y: 8.0,
                },
                Action::AddShoulderPoint {
                    name: "S".to_string(), // resolves to (21,0), per the golden test above
                    p1_line: "P1Line".to_string(),
                    p2_line: "P2Line".to_string(),
                    shoulder: "Shoulder".to_string(),
                    length_formula: "10".to_string(),
                },
                Action::AddBasePoint {
                    name: "Center".to_string(),
                    x: 21.0,
                    y: 3.0,
                },
                Action::AddPointOfContact {
                    name: "Contact".to_string(),
                    center: "Center".to_string(),
                    p1: "P1Line".to_string(),
                    p2: "S".to_string(), // depends on the ShoulderPoint action's own result, by name
                    radius_formula: "4".to_string(),
                },
            ],
        };

        let document = execute_action_script(&script).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();

        // Center=(21,3), radius=4: circle meets y=0 at x=21+-sqrt(7)
        // (from (x-21)^2+9=16). Segment is now [0,21] (P1Line to S), so
        // both candidates (21-sqrt(7)~18.35 and 21+sqrt(7)~23.65) fall on
        // opposite sides of that check: 21-sqrt(7) is on [0,21], 21+sqrt(7)
        // is not, so exactly one qualifies.
        let sqrt7 = 7.0_f64.sqrt();
        let contact_id = document
            .history()
            .iter()
            .find(|r| tool_name(&r.kind) == Some("Contact"))
            .unwrap()
            .id;
        let point = data.get_point(contact_id).unwrap();
        assert!((point.x - (21.0 - sqrt7)).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }
}
