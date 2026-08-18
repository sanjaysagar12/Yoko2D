use std::collections::HashMap; // backs `base_variables`, the same collection type `PatternData` uses for variables
use std::collections::HashSet; // backs `history_ids`, an O(1) existence index parallel to `history`

use thiserror::Error; // brings in the Error derive macro used by DocumentError below

use crate::id::ObjectId; // the id type every tool record and geometric reference is built from
use crate::variable::{Variable, VariableKind}; // the variable payload type, and its kind tag used to filter measurements out

/// Everything that can go wrong constructing a [`Document`] from parts that
/// didn't come from `Document`'s own `add_tool`/`add_*` methods (currently:
/// only [`Document::from_parts`], used by `io`'s Phase 8 pattern-file
/// deserialization).
#[derive(Debug, Clone, PartialEq, Error)] // Debug/Clone/PartialEq mirror this crate's other error types; Error implements std::error::Error via thiserror
pub enum DocumentError {
    /// Two entries in the input history share an `id`. `add_tool` can
    /// never produce this (its counter only ever climbs), so it can only
    /// happen when reconstructing a `Document` from external data (e.g. a
    /// corrupted or hand-edited pattern file) that wasn't validated by
    /// `Document` itself as it was built.
    #[error("duplicate object id {0:?} in restored history")]
    DuplicateId(ObjectId), // the id that appeared more than once

    /// [`Document::remove_tool`] was asked to remove a tool that some
    /// other, still-present tool depends on (per [`references_of`]).
    /// Refusing this is what keeps every reference in `history` always
    /// resolvable — a later `recompute_all` should never have to discover
    /// a dangling reference that `Document` itself could have prevented.
    #[error("tool {id:?} cannot be removed: still used by {used_by:?}")]
    ToolInUse {
        id: ObjectId,      // the tool that was asked to be removed
        used_by: ObjectId, // the other tool that still references it
    },

    /// [`Document::replace_tool_kind`] was asked to install a `ToolKind`
    /// that references an id not present in this `Document`. Named and
    /// shaped identically to [`crate::PatternError::MissingDependency`]
    /// (same underlying id, same "the reference doesn't exist" meaning),
    /// kept as a separate variant here because it's `Document`'s own
    /// mutation-validation failing, not a `recompute_all` failure.
    #[error("missing dependency: object {0:?} does not exist in this document")]
    MissingDependency(ObjectId), // the referenced id that doesn't exist in this document

    /// [`Document::remove_tool`]/[`Document::replace_tool_kind`] were
    /// asked to act on an id that isn't actually present in `history`.
    /// Should be unreachable in normal use (callers are expected to check
    /// [`Document::contains`] first, the same convention `add_end_line`/
    /// `add_line` already follow), but returned rather than panicking or
    /// indexing unsafely if it somehow happens anyway.
    #[error("no tool with id {0:?} exists in this document")]
    ToolNotFound(ObjectId), // the id that was looked up and not found in history
}

/// Returns every `ObjectId` that `kind` depends on: none for a
/// [`ToolKind::BasePoint`], `base_point` for an [`ToolKind::EndLine`], or
/// `p1`/`p2` for a [`ToolKind::Line`].
///
/// A plain function, not a method, since it only needs the `ToolKind`
/// itself, not any `Document` state. Shared by [`Document::remove_tool`]
/// (to check whether removing a tool would orphan a dependent) and
/// [`Document::replace_tool_kind`] (to validate a replacement kind's own
/// references before committing it) — factored out so this match doesn't
/// have to be kept in sync by hand in two separate places.
fn references_of(kind: &ToolKind) -> Vec<ObjectId> {
    match kind {
        // dispatch on which kind of tool this is
        ToolKind::BasePoint { .. } => Vec::new(), // no dependencies: a literal starting point
        ToolKind::EndLine { base_point, .. } => vec![*base_point], // depends on exactly one earlier point
        ToolKind::Line { p1, p2 } => vec![*p1, *p2], // depends on exactly two earlier points
        ToolKind::AlongLine { p1, p2, .. } => vec![*p1, *p2], // depends on the measured-from point and the direction point
        ToolKind::Normal { p1, p2, .. } => vec![*p1, *p2], // depends on the measured-from point and the point defining the perpendicular line
        ToolKind::Bisector { p1, p2, p3, .. } => vec![*p1, *p2, *p3], // depends on both angle rays plus the vertex
        ToolKind::Height {
            point,
            line_p1,
            line_p2,
            ..
        } => vec![*point, *line_p1, *line_p2], // depends on the projected point plus both points defining the line
        ToolKind::Midpoint { p1, p2, .. } => vec![*p1, *p2], // depends on both segment endpoints
        ToolKind::Piece { nodes, .. } => nodes.iter().map(|node| node.point).collect(), // every point still used as a piece boundary vertex, so remove_tool correctly blocks deleting any of them
        ToolKind::ShoulderPoint {
            p1_line,
            p2_line,
            shoulder,
            ..
        } => vec![*p1_line, *p2_line, *shoulder], // depends on both ray-defining points plus the circle-center point
        ToolKind::LineIntersect {
            p1_line1,
            p2_line1,
            p1_line2,
            p2_line2,
            ..
        } => vec![*p1_line1, *p2_line1, *p1_line2, *p2_line2], // depends on all four points defining the two lines
        ToolKind::PointOfIntersection { p1, p2, .. } => vec![*p1, *p2], // depends on the point supplying x and the point supplying y
        ToolKind::Triangle {
            axis_p1,
            axis_p2,
            hypotenuse_p1,
            hypotenuse_p2,
            ..
        } => vec![*axis_p1, *axis_p2, *hypotenuse_p1, *hypotenuse_p2], // depends on both axis points plus both hypotenuse points
        ToolKind::PointOfContact { center, p1, p2, .. } => vec![*center, *p1, *p2], // depends on the circle-center point plus both segment endpoints
        ToolKind::Arc { center, .. } => vec![*center], // depends on the circle's own center point
        ToolKind::Spline { p1, p4, .. } => vec![*p1, *p4], // depends on both curve endpoints
    }
}

/// One entry in a [`Document`]'s ordered tool history: the id it was
/// assigned when added, plus what kind of tool it is.
///
/// Mirrors Seamly2D's XML tool nodes — each one is a formula-carrying
/// instruction, not a resolved coordinate. [`crate::recompute::recompute_all`]
/// is what turns a sequence of these into actual [`crate::GeoObject`]s.
#[derive(Debug, Clone, PartialEq)] // Debug: printable in test failures; Clone: Document itself needs to be cloneable; PartialEq: enables assert_eq! in tests
pub struct ToolRecord {
    pub id: ObjectId, // the id this tool's resulting geometry will be stored under on recompute
    pub kind: ToolKind, // which kind of tool this is, and its formula/literal inputs
}

// TODO(phase 10b+): ShoulderPoint, PointOfContact, intersection tools,
// curve tools (Arc, Spline, ...), and transform operations remain for
// later phases — the intersection/curve/transform tools involve
// multi-solution ambiguity or new GeoObject variants that are explicitly
// out of scope for Phase 10a.
/// One boundary vertex in a [`ToolKind::Piece`]'s ordered node list: which
/// existing point it references, and whether that vertex's adjacent edges
/// should be excluded from seam-allowance offsetting.
///
/// `excluded_from_seam_allowance` is READ but not yet acted upon by
/// [`crate::recompute::recompute_all`]'s `Piece` arm in this phase — see
/// that arm's own doc comment for why proper per-edge exclusion is a
/// future refinement.
#[derive(Debug, Clone, PartialEq)] // same rationale as ToolRecord/ToolKind's derives above
pub struct PieceNode {
    pub point: ObjectId, // the existing point this boundary vertex references
    pub excluded_from_seam_allowance: bool, // whether this vertex's adjacent edges should be excluded from offsetting (not yet implemented; see the doc comment above)
}

/// The kind of a [`ToolRecord`], carrying whatever inputs that tool needs
/// to produce its geometry. Phase 10a adds the five single-point
/// construction tools (`AlongLine`/`Normal`/`Bisector`/`Height`/
/// `Midpoint`) alongside the `BasePoint`/`EndLine`/`Line` trio from
/// earlier phases, establishing the repeatable pattern (a `ToolKind`
/// variant, a `references_of` arm, a `Document::add_*` constructor, a
/// `recompute_all` arm, and XML read/write support) later phases reuse for
/// further tool types.
#[derive(Debug, Clone, PartialEq)] // same rationale as the derives on `ToolRecord` above
pub enum ToolKind {
    /// A literal starting point: no formulas, no dependency on any other
    /// tool. Every pattern's history has to start with at least one of
    /// these, since every other tool kind depends on some earlier point.
    BasePoint {
        name: String, // the point's user-facing label (not used for lookup; ids are)
        x: f64,       // the point's literal x coordinate
        y: f64,       // the point's literal y coordinate
    },

    /// A point placed at a given angle and length from an existing point.
    /// Both `angle_formula` and `length_formula` are formula *strings*,
    /// re-evaluated from scratch on every recompute rather than cached,
    /// since the variables they reference can change between recomputes.
    EndLine {
        name: String,           // the point's user-facing label
        base_point: ObjectId,   // which earlier tool's point this one is measured from
        angle_formula: String, // a formula string evaluating to the angle, in degrees (Phase 2's trig convention)
        length_formula: String, // a formula string evaluating to the distance from `base_point`
    },

    /// A straight line between two existing points, referenced by id.
    Line {
        p1: ObjectId, // the line's first endpoint
        p2: ObjectId, // the line's second endpoint
    },

    /// A new point measured `length_formula` units from `p1`, continuing
    /// straight along the direction from `p1` toward `p2`. A common
    /// pattern-drafting move: "extend this line segment by a further
    /// amount" (e.g. lengthening a hem allowance past an existing edge).
    AlongLine {
        name: String,           // the point's user-facing label
        p1: ObjectId,           // the point measured from
        p2: ObjectId,           // the point that defines the direction to continue in
        length_formula: String, // a formula string evaluating to the distance from `p1`, along the p1->p2 direction
    },

    /// A new point offset from `p1` along the direction perpendicular to
    /// the `p1`-`p2` line, additionally rotated by `angle_formula`
    /// degrees, at `length_formula` distance. Used for drafting seam
    /// allowances and construction lines that need to run perpendicular
    /// (or near-perpendicular) to an existing edge.
    Normal {
        name: String,           // the point's user-facing label
        p1: ObjectId,           // the point measured from
        p2: ObjectId, // the point that, together with p1, defines the line to be perpendicular to
        length_formula: String, // a formula string evaluating to the distance from `p1`
        angle_formula: String, // a formula string evaluating to an additional rotation, in degrees, applied on top of the perpendicular direction
    },

    /// A new point along the bisector of the angle `p1`-`p2`-`p3` (`p2` is
    /// the angle's vertex), at `length_formula` distance from `p2`. Used
    /// for symmetric darts and other constructions that need to split an
    /// angle evenly.
    Bisector {
        name: String,           // the point's user-facing label
        p1: ObjectId,           // one ray of the angle, measured from the vertex p2
        p2: ObjectId,           // the angle's vertex
        p3: ObjectId,           // the other ray of the angle, measured from the vertex p2
        length_formula: String, // a formula string evaluating to the distance from p2, along the bisecting direction
    },

    /// A new point: the perpendicular projection of `point` onto the line
    /// through `line_p1`/`line_p2` — the foot of the altitude from `point`
    /// to that line. Pure geometry, no formula: used to drop a
    /// perpendicular from a point onto an existing edge, e.g. to find
    /// where a dart's centerline meets a hem.
    Height {
        name: String,      // the point's user-facing label
        point: ObjectId,   // the point being projected
        line_p1: ObjectId, // one point defining the line being projected onto
        line_p2: ObjectId, // the other point defining the line being projected onto
    },

    /// A new point exactly halfway between `p1` and `p2`. Pure geometry,
    /// no formula: used whenever a construction needs the midpoint of an
    /// existing segment, e.g. to center a dart or find a fold line.
    Midpoint {
        name: String, // the point's user-facing label
        p1: ObjectId, // the segment's first endpoint
        p2: ObjectId, // the segment's second endpoint
    },

    /// A closed straight-line boundary built from existing points, plus a
    /// formula controlling how far (if at all) that boundary is offset
    /// outward into a seam allowance on recompute. A deliberately
    /// simplified subset of Seamly2D's `VPiecePath`: straight edges only
    /// (no curve/spline boundary segments), no notch generation — see this
    /// phase's own scope note.
    Piece {
        name: String,                   // the piece's user-facing label
        nodes: Vec<PieceNode>, // the boundary vertices, in order, each referencing an existing point
        seam_allowance_formula: String, // a formula string evaluating to the seam-allowance width; 0.0 means no seam allowance
    },

    /// A new point along the ray from `p1_line` through `p2_line` (extended
    /// past `p2_line`), at `length_formula` distance from `shoulder` —
    /// mirrors Seamly2D's `VToolShoulderPoint::FindPoint`: intersect a
    /// circle of radius `length_formula` centered at `shoulder` with that
    /// ray, and pick whichever candidate lies farther from `p1_line` than
    /// `p2_line` does, in the ray's own forward direction. Used to find a
    /// garment's shoulder point: a fixed distance from a reference point
    /// (`shoulder`), landing on the shoulder-slope line extended past its
    /// nearer endpoint.
    ShoulderPoint {
        name: String,           // the point's user-facing label
        p1_line: ObjectId,      // the ray's own starting point
        p2_line: ObjectId, // the point defining the ray's direction (and the "already past this" threshold)
        shoulder: ObjectId, // the circle's center
        length_formula: String, // a formula string evaluating to the circle's radius
    },

    /// A new point where the infinite lines through `p1_line1`/`p2_line1`
    /// and `p1_line2`/`p2_line2` cross. Mirrors Seamly2D's
    /// `VToolLineIntersect::Create`. Pure geometry, no formula: the two
    /// lines either cross at a unique point or don't (parallel/collinear).
    LineIntersect {
        name: String,       // the point's user-facing label
        p1_line1: ObjectId, // the first line's first defining point
        p2_line1: ObjectId, // the first line's second defining point
        p1_line2: ObjectId, // the second line's first defining point
        p2_line2: ObjectId, // the second line's second defining point
    },

    /// A new point combining `p1`'s x coordinate with `p2`'s y coordinate —
    /// mirrors Seamly2D's `PointIntersectXYTool::Create` (internally
    /// `"intersectXY"`), which is the plain, axis-aligned member of the
    /// "PointOfIntersection*" tool family (distinct from
    /// `PointOfIntersectionArcs`/`Circles`/`Curves`, which remain out of
    /// scope). Pure geometry, no formula, and never degenerate: any two
    /// points (even coincident ones) always produce a well-defined answer.
    PointOfIntersection {
        name: String, // the point's user-facing label
        p1: ObjectId, // the point this one takes its x coordinate from
        p2: ObjectId, // the point this one takes its y coordinate from
    },

    /// A new point on the infinite line through `axis_p1`/`axis_p2` where
    /// that line first sees `hypotenuse_p1`-`hypotenuse_p2` at a right
    /// angle, in the forward `axis_p1`-\>`axis_p2` direction from where the
    /// axis crosses the hypotenuse. Mirrors Seamly2D's
    /// `VToolTriangle::FindPoint`'s actual geometric effect — Seamly2D
    /// itself computes this via a fragile 1-pixel-per-step numeric search
    /// for the first point where the law-of-cosines angle at that point
    /// drops to <=90 degrees; this is the exact closed-form equivalent
    /// (Thales' theorem: that condition holds exactly on the circle whose
    /// diameter is the `hypotenuse_p1`-`hypotenuse_p2` segment), computed
    /// directly rather than searched for. Used to drop a right-angle
    /// construction point onto a reference axis from a diagonal edge.
    Triangle {
        name: String,            // the point's user-facing label
        axis_p1: ObjectId,       // the reference line's first defining point
        axis_p2: ObjectId, // the reference line's second defining point, also the forward-search direction
        hypotenuse_p1: ObjectId, // one endpoint of the segment this point forms a right angle with
        hypotenuse_p2: ObjectId, // the other endpoint of that segment
    },

    /// A new point where a circle of radius `radius_formula` centered at
    /// `center` crosses the segment `p1`-`p2` — mirrors Seamly2D's
    /// `VToolPointOfContact::FindPoint`. When the circle crosses the
    /// segment twice, prefers whichever candidate actually lies within the
    /// finite segment (not just the infinite line through it); if both or
    /// neither qualify, prefers whichever is closer to `p1`.
    PointOfContact {
        name: String,           // the point's user-facing label
        center: ObjectId,       // the circle's center
        p1: ObjectId,           // the segment's first endpoint
        p2: ObjectId,           // the segment's second endpoint
        radius_formula: String, // a formula string evaluating to the circle's radius
    },

    /// A circular arc: a center point plus radius/start-angle/end-angle
    /// formulas — mirrors Seamly2D's `VToolArc::Create`/`VArc`. A first-class
    /// curve `ToolKind`, unlike every construction tool above (which all
    /// produce a single `Point`): this produces a `GeoObject::Arc`, sampled
    /// into a polyline for rendering/export by `crate::geometry::
    /// direction_from_angle_deg`'s callers in the `render`/`cli` crates.
    Arc {
        name: String,                // the arc's user-facing label
        center: ObjectId,            // the circle's center
        radius_formula: String, // a formula string evaluating to the circle's radius; must evaluate > 0.0
        start_angle_formula: String, // a formula string evaluating to the sweep's starting angle, in degrees
        end_angle_formula: String, // a formula string evaluating to the sweep's ending angle, in degrees
    },

    /// A single-segment cubic Bezier curve between two existing points,
    /// mirrors Seamly2D's `VToolSpline::Create`/`VSpline`: each endpoint
    /// gets its own independent tangent-angle formula (degrees) and
    /// handle-length formula, from which the two interior Bezier control
    /// points are derived (`crate::recompute`'s `ToolKind::Spline` arm).
    /// Like `Arc` above, this produces a curve `GeoObject` (`Spline`), not a
    /// `Point`.
    Spline {
        name: String,            // the curve's user-facing label
        p1: ObjectId,            // the curve's first endpoint
        p4: ObjectId,            // the curve's second endpoint
        angle1_formula: String, // a formula string evaluating to p1's own tangent angle, in degrees
        length1_formula: String, // a formula string evaluating to p1's own control-handle length
        angle2_formula: String, // a formula string evaluating to p4's own tangent angle, in degrees
        length2_formula: String, // a formula string evaluating to p4's own control-handle length
    },
}

/// The persistent, ordered source of truth for a pattern: every tool the
/// user has added, in the order they added them, plus the base variable
/// table (measurements and user-defined custom variables) formulas can
/// reference.
///
/// Unlike [`crate::PatternData`], which is a disposable cache of *resolved*
/// geometry rebuilt from scratch by [`crate::recompute::recompute_all`],
/// `Document` never gets thrown away — it is what recompute reads *from*.
/// Its own `next_id` counter (separate from `PatternData`'s) is what makes
/// ids stable across recomputes: a `PatternData` gets rebuilt every time,
/// but a `ToolRecord`'s id, once assigned, never changes.
#[derive(Debug, Clone, PartialEq, Default)] // Default gives an empty history, empty ids, empty variables, and next_id starting at 0 — an empty pattern
pub struct Document {
    history: Vec<ToolRecord>, // every tool added so far, in the exact order it was added
    history_ids: HashSet<ObjectId>, // every id ever allocated by this Document, for O(1) "does this id exist" checks
    base_variables: HashMap<String, Variable>, // measurements/custom variables formulas can reference, independent of any tool
    next_id: u32, // this Document's own id counter; distinct from any PatternData's, since PatternData is rebuilt from scratch each recompute
}

impl Document {
    /// Appends a new tool of kind `kind` to the history and returns the id
    /// it was assigned.
    ///
    /// Ids come from `Document`'s own counter, not from whatever
    /// `PatternData` a later recompute happens to build — that's what lets
    /// a `ToolRecord`'s id stay the same across repeated recomputes, even
    /// though the `PatternData` it resolves into is thrown away and
    /// rebuilt from scratch every time.
    pub fn add_tool(&mut self, kind: ToolKind) -> ObjectId {
        let id = ObjectId::new(self.next_id); // allocate the next id from this Document's own counter
        self.next_id += 1; // advance the counter so the next add_tool call gets a different id
        self.history_ids.insert(id); // record the id as allocated, so `contains` can answer instantly without scanning history
        self.history.push(ToolRecord { id, kind }); // record this tool, in order, at the end of the history
        id // hand the newly assigned id back to the caller
    }

    /// Inserts `var` under `name` in the base variable table, overwriting
    /// any existing entry with the same name.
    ///
    /// Same overwrite policy as [`crate::PatternData::add_variable`], for
    /// the same reason: reloading a measurement file is expected to
    /// replace the previous value under a given name, not accumulate
    /// stale duplicates alongside it.
    pub fn set_variable(&mut self, name: impl Into<String>, var: Variable) {
        self.base_variables.insert(name.into(), var); // overwrite (or newly insert) the entry for this name
    }

    /// Removes every entry from `base_variables` whose [`Variable::kind`]
    /// is [`VariableKind::Measurement`], leaving all other kinds (`Custom`,
    /// `LineLength`, etc.) untouched.
    ///
    /// Ports the same filtering approach as
    /// [`crate::PatternData::clear_variables`] (Phase 1), just applied to
    /// `Document`'s own `base_variables` map instead of `PatternData`'s.
    /// Private: the only caller is [`Self::apply_measurements`] below,
    /// which always follows this with a fresh load — there's no standalone
    /// use case for clearing measurements without also reloading them.
    fn clear_measurement_variables(&mut self) {
        self.base_variables
            .retain(|_, var| var.kind() != VariableKind::Measurement); // keep everything except Measurement-kind entries
    }

    /// Replaces every currently-loaded measurement with the contents of
    /// `measurements`.
    ///
    /// This is "replace, not merge" on purpose, mirroring
    /// `MeasurementDoc::readMeasurements()`'s "clear then reload" semantics
    /// in the original C++ app: `clear_measurement_variables` runs first so
    /// a name present in the *previous* file but absent from this one
    /// doesn't linger with its stale value — reloading a smaller or
    /// different measurement file must make the old, no-longer-present
    /// names genuinely disappear, not just get shadowed by later entries
    /// that happen not to overwrite them. Custom/derived variables are
    /// untouched, since `clear_measurement_variables` only removes
    /// `Measurement`-kind entries.
    pub fn apply_measurements(&mut self, measurements: std::collections::HashMap<String, f64>) {
        self.clear_measurement_variables(); // drop every previously loaded measurement before loading the new ones
        for (name, value) in measurements {
            // walk the freshly parsed measurement map, in whatever order the HashMap yields it
            self.set_variable(name, Variable::Measurement { value }); // set_variable's overwrite semantics naturally handle insertion here
        }
    }

    /// Returns the full tool history, in the order tools were added.
    ///
    /// Read-only: mutating the history is only ever done through
    /// `add_tool`, so every `ToolRecord` in it is guaranteed to have come
    /// from a real `add_tool` call (or, in tests within this crate, direct
    /// construction — see `document.rs`'s own test module for why that
    /// matters).
    pub fn history(&self) -> &[ToolRecord] {
        &self.history // borrow out the backing Vec as a slice, so callers can't push/remove through this accessor
    }

    /// Returns whether `id` has ever been allocated by this Document (via
    /// `add_tool` or one of the `add_*` convenience constructors below).
    ///
    /// An O(1) lookup into `history_ids`, deliberately separate from
    /// `recompute_all`'s validation: this lets a caller (e.g. a GUI tool
    /// about to create an `EndLine`) check "does this dependency exist"
    /// *before* appending anything to history, instead of only finding out
    /// after a full recompute.
    pub fn contains(&self, id: ObjectId) -> bool {
        self.history_ids.contains(&id) // O(1) set membership check
    }

    /// Looks up the [`ToolRecord`] with the given `id`, if this Document
    /// has one.
    ///
    /// A plain linear scan over `history` — simple and correct, and at
    /// this stage of the project not worth a second `HashMap<ObjectId,
    /// usize>` index just to make it O(1); `history` isn't expected to be
    /// large enough yet for that tradeoff to matter.
    pub fn get_tool(&self, id: ObjectId) -> Option<&ToolRecord> {
        self.history.iter().find(|record| record.id == id) // scan for the first (and only, since ids never repeat) matching record
    }

    /// Adds a literal starting point with no dependencies.
    ///
    /// Mirrors `VToolLine::Create`'s contract of "validate immediately,
    /// only register on success" — trivially here, since a `BasePoint` has
    /// no referenced ids to validate, so this can never fail and returns a
    /// bare `ObjectId` rather than a `Result`.
    pub fn add_base_point(&mut self, name: impl Into<String>, x: f64, y: f64) -> ObjectId {
        let kind = ToolKind::BasePoint {
            name: name.into(),
            x,
            y,
        }; // build the tool's data; nothing here can be invalid
        self.add_tool(kind) // register it and hand back the id it was assigned
    }

    /// Adds a point at `angle_formula` degrees and `length_formula` units
    /// from `base_point`.
    ///
    /// Mirrors `VToolLine::Create`'s contract of "validate immediately,
    /// only register on success": `base_point` is checked against this
    /// Document's known ids *before* anything is appended to `history`, so
    /// a bad reference fails loudly right away instead of surfacing only
    /// on the next `recompute_all`, and leaves history completely
    /// unchanged on failure.
    pub fn add_end_line(
        &mut self,
        name: impl Into<String>,
        base_point: ObjectId,
        angle_formula: impl Into<String>,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(base_point) {
            // `base_point` isn't a real id from this Document: refuse before touching history
            return Err(crate::PatternError::MissingDependency(base_point)); // precise, actionable error naming the bad reference
        }
        let kind = ToolKind::EndLine {
            name: name.into(), // convert the caller's name into an owned String
            base_point,        // already validated above
            angle_formula: angle_formula.into(), // convert the caller's angle formula into an owned String
            length_formula: length_formula.into(), // convert the caller's length formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a straight line between two existing points.
    ///
    /// Mirrors `VToolLine::Create`'s contract of "validate immediately,
    /// only register on success": both `p1` and `p2` are checked against
    /// this Document's known ids *before* anything is appended to
    /// `history`, `p1` first so a failure reports exactly which one was
    /// bad rather than always blaming `p2`.
    pub fn add_line(
        &mut self,
        p1: ObjectId,
        p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first, so a p1 failure is reported as p1's fault, not silently shadowed by a p2 check
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            // p1 was fine; now check p2 on its own
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        let kind = ToolKind::Line { p1, p2 }; // both endpoints validated above
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point measured `length_formula` units from `p1`, continuing
    /// along the p1->p2 direction.
    ///
    /// Same "validate immediately, only register on success" contract as
    /// `add_end_line`/`add_line`: `p1` is checked before `p2`, and the
    /// first missing id is reported, without touching `history` at all on
    /// failure.
    pub fn add_along_line(
        &mut self,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first, so a p1 failure is reported as p1's fault, not silently shadowed by a p2 check
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            // p1 was fine; now check p2 on its own
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        let kind = ToolKind::AlongLine {
            name: name.into(), // convert the caller's name into an owned String
            p1,                // already validated above
            p2,                // already validated above
            length_formula: length_formula.into(), // convert the caller's length formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point offset from `p1` perpendicular to the `p1`-`p2` line
    /// (further rotated by `angle_formula` degrees), at `length_formula`
    /// distance.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `p1` then `p2` in order.
    pub fn add_normal(
        &mut self,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
        length_formula: impl Into<String>,
        angle_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first, so a p1 failure is reported as p1's fault, not silently shadowed by a p2 check
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            // p1 was fine; now check p2 on its own
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        let kind = ToolKind::Normal {
            name: name.into(), // convert the caller's name into an owned String
            p1,                // already validated above
            p2,                // already validated above
            length_formula: length_formula.into(), // convert the caller's length formula into an owned String
            angle_formula: angle_formula.into(), // convert the caller's angle formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point along the bisector of the angle `p1`-`p2`-`p3` (`p2`
    /// the vertex), at `length_formula` distance from `p2`.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `p1`, then `p2`, then `p3` in order — the ids are checked
    /// in the exact order they appear in this method's signature, and the
    /// first missing one is reported.
    pub fn add_bisector(
        &mut self,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
        p3: ObjectId,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first: it's the first id in this method's signature
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            // p1 was fine; check p2 next, matching signature order
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        if !self.contains(p3) {
            // p1 and p2 were fine; check p3 last, matching signature order
            return Err(crate::PatternError::MissingDependency(p3)); // precise, actionable error naming p3
        }
        let kind = ToolKind::Bisector {
            name: name.into(), // convert the caller's name into an owned String
            p1,                // already validated above
            p2,                // already validated above
            p3,                // already validated above
            length_formula: length_formula.into(), // convert the caller's length formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point: the perpendicular projection of `point` onto the line
    /// through `line_p1`/`line_p2`.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `point`, then `line_p1`, then `line_p2` in order (the
    /// order they appear in this method's signature).
    pub fn add_height(
        &mut self,
        name: impl Into<String>,
        point: ObjectId,
        line_p1: ObjectId,
        line_p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(point) {
            // check `point` first: it's the first id in this method's signature
            return Err(crate::PatternError::MissingDependency(point)); // precise, actionable error naming `point`
        }
        if !self.contains(line_p1) {
            // `point` was fine; check `line_p1` next, matching signature order
            return Err(crate::PatternError::MissingDependency(line_p1)); // precise, actionable error naming `line_p1`
        }
        if !self.contains(line_p2) {
            // `point`/`line_p1` were fine; check `line_p2` last, matching signature order
            return Err(crate::PatternError::MissingDependency(line_p2)); // precise, actionable error naming `line_p2`
        }
        let kind = ToolKind::Height {
            name: name.into(), // convert the caller's name into an owned String
            point,             // already validated above
            line_p1,           // already validated above
            line_p2,           // already validated above
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point exactly halfway between `p1` and `p2`.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `p1` then `p2` in order.
    pub fn add_midpoint(
        &mut self,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first, so a p1 failure is reported as p1's fault, not silently shadowed by a p2 check
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            // p1 was fine; now check p2 on its own
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        let kind = ToolKind::Midpoint {
            name: name.into(),
            p1,
            p2,
        }; // both endpoints validated above
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a piece contour built from `nodes`, each referencing an
    /// existing point by id, with `seam_allowance_formula` controlling how
    /// far (if at all) the contour is offset outward on recompute.
    ///
    /// Same "validate immediately, only register on success" contract as
    /// every other `add_*` constructor: every node's `point` id is checked
    /// against this Document's known ids *before* anything is appended to
    /// `history`, in node order, so the first missing one is reported and
    /// nothing is mutated on failure.
    pub fn add_piece(
        &mut self,
        name: impl Into<String>,
        nodes: Vec<PieceNode>,
        seam_allowance_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        for node in &nodes {
            // check every referenced point before touching history, in node order
            if !self.contains(node.point) {
                // this node's point isn't a real id from this Document: refuse before touching history
                return Err(crate::PatternError::MissingDependency(node.point)); // precise, actionable error naming the first missing node's point
            }
        }
        let kind = ToolKind::Piece {
            name: name.into(), // convert the caller's name into an owned String
            nodes,             // every node already validated above
            seam_allowance_formula: seam_allowance_formula.into(), // convert the caller's formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point on the ray from `p1_line` through `p2_line` (extended
    /// past `p2_line`), `length_formula` units from `shoulder`.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `p1_line`, then `p2_line`, then `shoulder`, in that order
    /// (the order they appear in this method's signature).
    pub fn add_shoulder_point(
        &mut self,
        name: impl Into<String>,
        p1_line: ObjectId,
        p2_line: ObjectId,
        shoulder: ObjectId,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1_line) {
            // check p1_line first: it's the first id in this method's signature
            return Err(crate::PatternError::MissingDependency(p1_line)); // precise, actionable error naming p1_line
        }
        if !self.contains(p2_line) {
            // p1_line was fine; check p2_line next, matching signature order
            return Err(crate::PatternError::MissingDependency(p2_line)); // precise, actionable error naming p2_line
        }
        if !self.contains(shoulder) {
            // p1_line/p2_line were fine; check shoulder last, matching signature order
            return Err(crate::PatternError::MissingDependency(shoulder)); // precise, actionable error naming shoulder
        }
        let kind = ToolKind::ShoulderPoint {
            name: name.into(), // convert the caller's name into an owned String
            p1_line,           // already validated above
            p2_line,           // already validated above
            shoulder,          // already validated above
            length_formula: length_formula.into(), // convert the caller's length formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point where the infinite lines through `p1_line1`/`p2_line1`
    /// and `p1_line2`/`p2_line2` cross.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking all four ids in the order they appear in this method's
    /// signature.
    pub fn add_line_intersect(
        &mut self,
        name: impl Into<String>,
        p1_line1: ObjectId,
        p2_line1: ObjectId,
        p1_line2: ObjectId,
        p2_line2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1_line1) {
            // check p1_line1 first: it's the first id in this method's signature
            return Err(crate::PatternError::MissingDependency(p1_line1)); // precise, actionable error naming p1_line1
        }
        if !self.contains(p2_line1) {
            return Err(crate::PatternError::MissingDependency(p2_line1)); // precise, actionable error naming p2_line1
        }
        if !self.contains(p1_line2) {
            return Err(crate::PatternError::MissingDependency(p1_line2)); // precise, actionable error naming p1_line2
        }
        if !self.contains(p2_line2) {
            return Err(crate::PatternError::MissingDependency(p2_line2)); // precise, actionable error naming p2_line2
        }
        let kind = ToolKind::LineIntersect {
            name: name.into(), // convert the caller's name into an owned String
            p1_line1,          // already validated above
            p2_line1,          // already validated above
            p1_line2,          // already validated above
            p2_line2,          // already validated above
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point combining `p1`'s x coordinate with `p2`'s y coordinate.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `p1` then `p2` in order.
    pub fn add_point_of_intersection(
        &mut self,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first, so a p1 failure is reported as p1's fault, not silently shadowed by a p2 check
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            // p1 was fine; now check p2 on its own
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        let kind = ToolKind::PointOfIntersection {
            name: name.into(),
            p1,
            p2,
        }; // both endpoints validated above
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point on the infinite line through `axis_p1`/`axis_p2` that
    /// forms a right angle with `hypotenuse_p1`/`hypotenuse_p2`, in the
    /// forward `axis_p1`-\>`axis_p2` direction.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking all four ids in the order they appear in this method's
    /// signature.
    pub fn add_triangle(
        &mut self,
        name: impl Into<String>,
        axis_p1: ObjectId,
        axis_p2: ObjectId,
        hypotenuse_p1: ObjectId,
        hypotenuse_p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(axis_p1) {
            // check axis_p1 first: it's the first id in this method's signature
            return Err(crate::PatternError::MissingDependency(axis_p1)); // precise, actionable error naming axis_p1
        }
        if !self.contains(axis_p2) {
            return Err(crate::PatternError::MissingDependency(axis_p2)); // precise, actionable error naming axis_p2
        }
        if !self.contains(hypotenuse_p1) {
            return Err(crate::PatternError::MissingDependency(hypotenuse_p1)); // precise, actionable error naming hypotenuse_p1
        }
        if !self.contains(hypotenuse_p2) {
            return Err(crate::PatternError::MissingDependency(hypotenuse_p2)); // precise, actionable error naming hypotenuse_p2
        }
        let kind = ToolKind::Triangle {
            name: name.into(), // convert the caller's name into an owned String
            axis_p1,           // already validated above
            axis_p2,           // already validated above
            hypotenuse_p1,     // already validated above
            hypotenuse_p2,     // already validated above
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a point where a circle of radius `radius_formula` centered at
    /// `center` crosses the segment `p1`-`p2`.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `center`, then `p1`, then `p2`, in that order (the order
    /// they appear in this method's signature).
    pub fn add_point_of_contact(
        &mut self,
        name: impl Into<String>,
        center: ObjectId,
        p1: ObjectId,
        p2: ObjectId,
        radius_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(center) {
            // check center first: it's the first id in this method's signature
            return Err(crate::PatternError::MissingDependency(center)); // precise, actionable error naming center
        }
        if !self.contains(p1) {
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p2) {
            return Err(crate::PatternError::MissingDependency(p2)); // precise, actionable error naming p2
        }
        let kind = ToolKind::PointOfContact {
            name: name.into(), // convert the caller's name into an owned String
            center,            // already validated above
            p1,                // already validated above
            p2,                // already validated above
            radius_formula: radius_formula.into(), // convert the caller's radius formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a circular arc centered at `center`, with `radius_formula`/
    /// `start_angle_formula`/`end_angle_formula` controlling its shape on
    /// recompute.
    ///
    /// Same "validate immediately, only register on success" contract as
    /// every other `add_*` constructor: `center` is checked against this
    /// Document's known ids *before* anything is appended to `history`.
    pub fn add_arc(
        &mut self,
        name: impl Into<String>,
        center: ObjectId,
        radius_formula: impl Into<String>,
        start_angle_formula: impl Into<String>,
        end_angle_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(center) {
            // `center` isn't a real id from this Document: refuse before touching history
            return Err(crate::PatternError::MissingDependency(center)); // precise, actionable error naming the bad reference
        }
        let kind = ToolKind::Arc {
            name: name.into(), // convert the caller's name into an owned String
            center,            // already validated above
            radius_formula: radius_formula.into(), // convert the caller's radius formula into an owned String
            start_angle_formula: start_angle_formula.into(), // convert the caller's start-angle formula into an owned String
            end_angle_formula: end_angle_formula.into(), // convert the caller's end-angle formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Adds a single-segment cubic Bezier curve between `p1` and `p4`, with
    /// each endpoint's own tangent-angle/handle-length formulas controlling
    /// the two interior control points on recompute.
    ///
    /// Same "validate immediately, only register on success" contract,
    /// checking `p1` then `p4` in order (both via `self.contains(..)`
    /// before calling `add_tool`, matching this method's own doc comment).
    #[allow(clippy::too_many_arguments)] // mirrors VSpline's own constructor shape (two independent angle+length pairs, one per endpoint); splitting this into a builder would be more invasive than the flat parameter list every other add_* constructor in this file already uses
    pub fn add_spline(
        &mut self,
        name: impl Into<String>,
        p1: ObjectId,
        p4: ObjectId,
        angle1_formula: impl Into<String>,
        length1_formula: impl Into<String>,
        angle2_formula: impl Into<String>,
        length2_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        if !self.contains(p1) {
            // check p1 first, so a p1 failure is reported as p1's fault, not silently shadowed by a p4 check
            return Err(crate::PatternError::MissingDependency(p1)); // precise, actionable error naming p1
        }
        if !self.contains(p4) {
            // p1 was fine; now check p4 on its own
            return Err(crate::PatternError::MissingDependency(p4)); // precise, actionable error naming p4
        }
        let kind = ToolKind::Spline {
            name: name.into(), // convert the caller's name into an owned String
            p1,                // already validated above
            p4,                // already validated above
            angle1_formula: angle1_formula.into(), // convert the caller's angle1 formula into an owned String
            length1_formula: length1_formula.into(), // convert the caller's length1 formula into an owned String
            angle2_formula: angle2_formula.into(), // convert the caller's angle2 formula into an owned String
            length2_formula: length2_formula.into(), // convert the caller's length2 formula into an owned String
        };
        Ok(self.add_tool(kind)) // validation passed: register the tool and hand back its id
    }

    /// Iterates every `(name, Variable)` pair in the base variable table,
    /// in unspecified order (the same order `HashMap::iter` yields).
    ///
    /// Public (widened from `pub(crate)` in Phase 8): originally only
    /// `recompute::recompute_all` needed this, to seed a fresh
    /// `PatternData` with `Document`'s variables before walking the tool
    /// history; the `io` crate's pattern-file serialization now needs the
    /// same read access from outside this crate.
    pub fn base_variables(&self) -> impl Iterator<Item = (&String, &Variable)> {
        self.base_variables.iter() // hand back the map's own iterator; nothing to transform here
    }

    /// Rebuilds a `Document` from an already-assembled `history` and
    /// `base_variables`, validating that no two records share an `id` and
    /// restoring the id counter to continue past whatever was loaded.
    ///
    /// This is the deserialization-side counterpart to `add_tool`: where
    /// `add_tool` can never produce a duplicate id (its counter only
    /// climbs), a `history` built from external data (e.g. a hand-edited
    /// or corrupted pattern file) has no such guarantee, so it's validated
    /// here instead.
    pub fn from_parts(
        history: Vec<ToolRecord>, // the tool records to adopt, in order, exactly as given
        base_variables: HashMap<String, Variable>, // the variable table to adopt as-is (pattern files don't embed variable values — see io's Phase 8 docs)
    ) -> Result<Self, DocumentError> {
        let mut history_ids = HashSet::new(); // tracks every id seen so far in `history`, to catch duplicates
        let mut max_id = 0u32; // tracks the highest raw id value seen; start at 0 so an empty history still yields a valid next_id
        for record in &history {
            // walk the caller-supplied history in order, validating as we go
            if history_ids.contains(&record.id) {
                // this id already appeared earlier in the same history: a corrupted/hand-edited file
                return Err(DocumentError::DuplicateId(record.id)); // reject rather than silently accept the collision
            }
            history_ids.insert(record.id); // record this id as seen, for the next iteration's duplicate check
            max_id = max_id.max(record.id.raw()); // keep track of the running maximum raw id value across all records
        }
        Ok(Document {
            history,             // adopt the caller-supplied history as-is, now proven duplicate-free
            history_ids,         // the id-presence index built above, kept in sync with `history`
            base_variables,      // adopt the caller-supplied variable table as-is
            next_id: max_id + 1, // ensures a later add_tool on this restored Document never collides with any id loaded from the file
        })
    }

    /// Removes the tool with `id` from the history, failing if any other
    /// tool still depends on it.
    ///
    /// Mirrors the original C++ app's undo/redo philosophy (see
    /// `crate::undo`'s module docs): this mutates the ordered tool-record
    /// list — our DOM-equivalent — and nothing else. It never touches
    /// resolved geometry, and callers are expected to call `recompute_all`
    /// again afterward to see the effect; this method has no dependency
    /// on the recompute engine at all.
    pub fn remove_tool(&mut self, id: ObjectId) -> Result<(ToolRecord, usize), DocumentError> {
        for other in &self.history {
            // scan every record for one that depends on `id`
            if other.id == id {
                // this is the record about to be removed itself, not some other tool: skip it
                continue; // a tool never counts as depending on itself
            }
            if references_of(&other.kind).contains(&id) {
                // some other tool still references `id`: removing it would leave that tool dangling
                return Err(DocumentError::ToolInUse {
                    id,
                    used_by: other.id,
                }); // report exactly which tool is blocking the removal, without mutating anything
            }
        }

        let index = self
            .history
            .iter() // scan for the record's current position
            .position(|record| record.id == id) // find where this id actually lives in history
            .ok_or(DocumentError::ToolNotFound(id))?; // defensive: should be guaranteed present if history_ids.contains(id), but never index unsafely

        let record = self.history.remove(index); // remove it; Vec::remove shifts later elements left, preserving their relative order
        self.history_ids.remove(&id); // this id is no longer present in the document

        // next_id is deliberately NOT rolled back here: ids are never
        // reused once allocated, which is exactly what makes it safe to
        // later redo an "add" of this same id without any collision risk
        // — even though the id was just removed and the counter may keep
        // climbing for unrelated adds in the meantime.
        Ok((record, index)) // hand back both the removed record and its original position, for undo to restore it there later
    }

    /// Replaces the `ToolKind` stored under `id` with `new_kind`,
    /// returning the previous `ToolKind`.
    ///
    /// Validates `new_kind`'s own references first, the same way
    /// `add_end_line`/`add_line` validate before appending — a bad
    /// replacement (referencing an id that doesn't exist in this
    /// document) fails loudly here rather than silently installing a
    /// dangling reference that only `recompute_all` would later catch.
    pub fn replace_tool_kind(
        &mut self,
        id: ObjectId,
        new_kind: ToolKind,
    ) -> Result<ToolKind, DocumentError> {
        for dependency in references_of(&new_kind) {
            // check every id the replacement kind would depend on
            if !self.contains(dependency) {
                // this dependency doesn't exist anywhere in the document: refuse before touching anything
                return Err(DocumentError::MissingDependency(dependency)); // precise, actionable error naming the bad reference
            }
        }

        let record = self
            .history
            .iter_mut() // need a mutable reference to swap the kind in place
            .find(|record| record.id == id) // locate the record being modified
            .ok_or(DocumentError::ToolNotFound(id))?; // defensive: id must actually exist in this document

        let old_kind = std::mem::replace(&mut record.kind, new_kind); // swap in the new kind, capturing the previous one in a single step
        Ok(old_kind) // hand back the replaced-out kind, e.g. so undo can restore it later
    }

    /// Inserts `record` back into the history at `index`, failing if its
    /// id is already present.
    ///
    /// The undo-side counterpart to `remove_tool`: restoring a removed
    /// tool to its *exact original position* matters, not just re-adding
    /// it somewhere — `recompute_all` processes history strictly in
    /// order, so inserting it at a different position could change
    /// dependency-resolution behavior for every tool between the old and
    /// new positions, even if every individual reference still resolves.
    pub fn insert_tool_record_at(
        &mut self,
        index: usize,
        record: ToolRecord,
    ) -> Result<(), DocumentError> {
        if self.history_ids.contains(&record.id) {
            // this id is already present: refuse rather than create a duplicate
            return Err(DocumentError::DuplicateId(record.id)); // reject before mutating anything
        }
        self.history_ids.insert(record.id); // record this id as present again

        if record.id.raw() >= self.next_id {
            // Defensive guard, not expected to normally trigger:
            // `remove_tool`'s "never roll back next_id" policy already
            // guarantees ids removed from THIS document stay below
            // next_id. Kept anyway so this method stays correct as a
            // standalone primitive regardless of caller discipline (e.g.
            // if it's ever used to insert a record sourced elsewhere).
            self.next_id = record.id.raw() + 1; // keep the counter strictly ahead of every known id
        }

        self.history.insert(index, record); // insert at the exact original position; Vec::insert shifts later elements right, preserving strict document order
        Ok(()) // restored successfully
    }

    /// Test-only escape hatch: pushes `record` directly onto the history,
    /// bypassing `add_tool`'s id allocation entirely.
    ///
    /// Exists solely so `recompute`'s tests can build a deliberately
    /// corrupted `Document` — one whose history references an id that was
    /// never actually produced by any `add_tool` call — to verify
    /// `recompute_all` reports `PatternError::MissingDependency` instead of
    /// panicking. `#[cfg(test)]` keeps this out of non-test builds
    /// entirely, so it can never be used to corrupt a real `Document`.
    #[cfg(test)] // compiled only for tests, so this can't reach production code paths
    pub(crate) fn push_record_for_test(&mut self, record: ToolRecord) {
        self.history.push(record); // bypasses add_tool's id counter on purpose, to simulate a corrupted document
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{recompute_all, PatternError};

    #[test]
    fn add_base_point_registers_id_immediately() {
        let mut doc = Document::default();
        let id = doc.add_base_point("A", 1.0, 2.0);
        assert!(doc.contains(id));
    }

    #[test]
    fn add_line_between_two_valid_points_succeeds() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 1.0, 1.0);

        let line = doc.add_line(a, b).unwrap();
        assert!(line > a);
        assert!(line > b);
    }

    #[test]
    fn add_line_with_unknown_second_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_line(a, bogus).unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_end_line_with_unknown_base_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_end_line("A1", bogus, "0", "10").unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_end_line_with_valid_base_point_succeeds() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let result = doc.add_end_line("A1", a, "0", "10");
        assert!(result.is_ok());
    }

    #[test]
    fn get_tool_finds_records_created_by_each_constructor() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 1.0, 2.0);
        let a1 = doc.add_end_line("A1", a, "0", "10").unwrap();
        let line = doc.add_line(a, a1).unwrap();

        assert!(matches!(
            doc.get_tool(a).unwrap().kind,
            ToolKind::BasePoint { x, y, .. } if x == 1.0 && y == 2.0
        ));
        assert!(matches!(
            doc.get_tool(a1).unwrap().kind,
            ToolKind::EndLine { base_point, .. } if base_point == a
        ));
        assert!(matches!(
            doc.get_tool(line).unwrap().kind,
            ToolKind::Line { p1, p2 } if p1 == a && p2 == a1
        ));
    }

    #[test]
    fn get_tool_returns_none_for_an_id_never_created() {
        let doc = Document::default();
        assert!(doc.get_tool(ObjectId::new(123)).is_none());
    }

    #[test]
    fn full_cascade_via_ergonomic_constructors_matches_expected_geometry() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        doc.set_variable("height_scapula", Variable::Measurement { value: 40.0 });
        let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();
        let line = doc.add_line(a, a1).unwrap();

        let data = recompute_all(&doc).unwrap();
        let resolved_line = data.get_line(line).unwrap();

        let p1 = data.get_point(resolved_line.p1).unwrap();
        assert!((p1.x - 0.0).abs() < 1e-9);
        assert!((p1.y - 0.0).abs() < 1e-9);

        let p2 = data.get_point(resolved_line.p2).unwrap();
        assert!((p2.x - 4.0).abs() < 1e-9);
        assert!((p2.y - 0.0).abs() < 1e-9);
    }

    // Builds two structurally identical Documents (BasePoint "A" at the
    // origin, BasePoint "B" at a different literal x, then a Line from A
    // to B), differing only in B's literal x. Proves geometry is always
    // freshly derived from the BasePoint's current literal values on
    // recompute, never cached on the Line record itself.
    #[test]
    fn recompute_never_caches_stale_geometry_on_the_line_record() {
        let mut doc_10 = Document::default();
        let a10 = doc_10.add_base_point("A", 0.0, 0.0);
        let b10 = doc_10.add_base_point("B", 10.0, 0.0);
        let line_10 = doc_10.add_line(a10, b10).unwrap();

        let mut doc_20 = Document::default();
        let a20 = doc_20.add_base_point("A", 0.0, 0.0);
        let b20 = doc_20.add_base_point("B", 20.0, 0.0);
        let line_20 = doc_20.add_line(a20, b20).unwrap();

        // Both Documents' Line records have the same structural shape: a
        // Line referencing exactly two known ids.
        assert!(matches!(
            doc_10.get_tool(line_10).unwrap().kind,
            ToolKind::Line { .. }
        ));
        assert!(matches!(
            doc_20.get_tool(line_20).unwrap().kind,
            ToolKind::Line { .. }
        ));

        let data_10 = recompute_all(&doc_10).unwrap();
        let data_20 = recompute_all(&doc_20).unwrap();

        let resolved_10 = data_10.get_line(line_10).unwrap();
        let endpoint_10 = data_10.get_point(resolved_10.p2).unwrap();
        assert!((endpoint_10.x - 10.0).abs() < 1e-9);

        let resolved_20 = data_20.get_line(line_20).unwrap();
        let endpoint_20 = data_20.get_point(resolved_20.p2).unwrap();
        assert!((endpoint_20.x - 20.0).abs() < 1e-9);
    }

    #[test]
    fn add_tool_returns_strictly_increasing_never_repeating_ids() {
        let mut doc = Document::default();
        let ids: Vec<ObjectId> = (0..5)
            .map(|i| {
                doc.add_tool(ToolKind::BasePoint {
                    name: format!("P{i}"),
                    x: 0.0,
                    y: 0.0,
                })
            })
            .collect();

        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1]);
        }

        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn add_tool_appends_to_history_in_order() {
        let mut doc = Document::default();
        let a = doc.add_tool(ToolKind::BasePoint {
            name: "A".to_string(),
            x: 0.0,
            y: 0.0,
        });
        let b = doc.add_tool(ToolKind::Line { p1: a, p2: a });

        let history = doc.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, a);
        assert_eq!(history[1].id, b);
        assert!(matches!(history[1].kind, ToolKind::Line { .. }));
    }

    #[test]
    fn set_variable_twice_keeps_most_recent_value() {
        let mut doc = Document::default();
        doc.set_variable("waist", Variable::Measurement { value: 70.0 });
        doc.set_variable("waist", Variable::Measurement { value: 72.5 });

        let stored: Vec<&Variable> = doc
            .base_variables()
            .filter(|(name, _)| *name == "waist")
            .map(|(_, var)| var)
            .collect();
        assert_eq!(stored, vec![&Variable::Measurement { value: 72.5 }]);
    }

    #[test]
    fn apply_measurements_feeds_the_full_pipeline_to_resolved_geometry() {
        let mut doc = Document::default();
        let mut measurements = HashMap::new();
        measurements.insert("height_scapula".to_string(), 40.0);
        doc.apply_measurements(measurements);

        let a = doc.add_base_point("A", 0.0, 0.0);
        let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(a1).unwrap();
        assert!((point.x - 4.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn apply_measurements_replaces_rather_than_merges() {
        let mut doc = Document::default();
        let mut first = HashMap::new();
        first.insert("height_scapula".to_string(), 40.0);
        first.insert("waist_circ".to_string(), 72.5);
        doc.apply_measurements(first);

        let mut second = HashMap::new();
        second.insert("waist_circ".to_string(), 70.0); // "height_scapula" deliberately absent this time
        doc.apply_measurements(second);

        // Not merely shadowed: the name must be genuinely gone from the variable table.
        let still_present = doc
            .base_variables()
            .any(|(name, _)| name == "height_scapula");
        assert!(!still_present);

        // Confirm through the formula engine too: a tool referencing the
        // now-missing name must fail to recompute rather than resolve
        // against a stale cached value.
        let a = doc.add_base_point("A", 0.0, 0.0);
        doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();
        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::Formula(_)));
    }

    #[test]
    fn apply_measurements_only_clears_measurement_kind_variables() {
        let mut doc = Document::default();
        doc.set_variable(
            "half_waist",
            Variable::Custom {
                formula: "waist_circ / 2".to_string(),
                cached_value: 36.25,
            },
        );

        let mut measurements = HashMap::new();
        measurements.insert("waist_circ".to_string(), 72.5);
        doc.apply_measurements(measurements);

        let custom = doc
            .base_variables()
            .find(|(name, _)| *name == "half_waist")
            .map(|(_, var)| var.clone());
        assert_eq!(
            custom,
            Some(Variable::Custom {
                formula: "waist_circ / 2".to_string(),
                cached_value: 36.25,
            })
        );
    }

    #[test]
    fn remove_tool_on_an_unreferenced_id_succeeds() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 1.0, 1.0); // not referenced by anything

        let (removed, _index) = doc.remove_tool(b).unwrap();
        assert_eq!(removed.id, b);
        assert!(!doc.contains(b));
        assert!(doc.get_tool(b).is_none());
        assert!(doc.contains(a)); // unrelated tool is untouched
    }

    #[test]
    fn remove_tool_still_referenced_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let a1 = doc.add_end_line("A1", a, "0", "10").unwrap(); // depends on `a`

        let len_before = doc.history().len();
        let err = doc.remove_tool(a).unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, DocumentError::ToolInUse { id: a, used_by: a1 });
        assert_eq!(len_before, len_after);
        assert!(doc.contains(a)); // still present: the failed removal changed nothing
    }

    #[test]
    fn replace_tool_kind_with_valid_kind_returns_the_previous_kind() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);

        let new_kind = ToolKind::BasePoint {
            name: "A".to_string(),
            x: 9.0,
            y: 9.0,
        };
        let old_kind = doc.replace_tool_kind(a, new_kind.clone()).unwrap();

        assert_eq!(
            old_kind,
            ToolKind::BasePoint {
                name: "A".to_string(),
                x: 0.0,
                y: 0.0,
            }
        );
        assert_eq!(doc.get_tool(a).unwrap().kind, new_kind);
    }

    #[test]
    fn replace_tool_kind_with_missing_dependency_fails_and_leaves_original_kind() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let bad_kind = ToolKind::EndLine {
            name: "A1".to_string(),
            base_point: bogus,
            angle_formula: "0".to_string(),
            length_formula: "10".to_string(),
        };
        let err = doc.replace_tool_kind(a, bad_kind).unwrap_err();
        assert_eq!(err, DocumentError::MissingDependency(bogus));

        // the record being "modified" is untouched: it still shows its original kind
        assert_eq!(
            doc.get_tool(a).unwrap().kind,
            ToolKind::BasePoint {
                name: "A".to_string(),
                x: 0.0,
                y: 0.0,
            }
        );
    }

    #[test]
    fn insert_tool_record_at_with_duplicate_id_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);

        let len_before = doc.history().len();
        let duplicate = ToolRecord {
            id: a, // already present
            kind: ToolKind::BasePoint {
                name: "Duplicate".to_string(),
                x: 5.0,
                y: 5.0,
            },
        };
        let err = doc.insert_tool_record_at(0, duplicate).unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, DocumentError::DuplicateId(a));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_along_line_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_along_line("N", a, bogus, "10").unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_normal_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_normal("N", a, bogus, "10", "0").unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_bisector_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 1.0, 1.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_bisector("N", a, b, bogus, "10").unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_height_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 1.0, 1.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_height("N", a, b, bogus).unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_midpoint_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_midpoint("N", a, bogus).unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    // Proves `references_of` was correctly extended for `Normal` in a real
    // dependency chain, not just checked in isolation: A is used by the
    // Normal tool N, and N is itself used by a downstream Line. Removing A
    // must be blocked because N (not the Line) still depends on it.
    #[test]
    fn remove_tool_blocked_by_a_normal_tools_dependency() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 1.0, 1.0);
        let n = doc.add_normal("N", a, b, "10", "0").unwrap(); // depends on both a and b
        let c = doc.add_base_point("C", 5.0, 5.0);
        doc.add_line(n, c).unwrap(); // a downstream tool depending on N, not directly on A

        let err = doc.remove_tool(a).unwrap_err();
        assert_eq!(err, DocumentError::ToolInUse { id: a, used_by: n });
        assert!(doc.contains(a)); // the failed removal changed nothing
    }

    #[test]
    fn add_piece_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc
            .add_piece(
                "Piece1",
                vec![
                    PieceNode {
                        point: a,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: bogus,
                        excluded_from_seam_allowance: false,
                    },
                ],
                "1.0",
            )
            .unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    // Point `a` is used both directly (in a Line) and as a Piece boundary
    // vertex. The Piece is added FIRST, so it's the record `remove_tool`
    // encounters first in its history scan — this is what actually proves
    // `references_of` was correctly extended for `Piece`, rather than the
    // removal merely being blocked by the (also-present) Line dependency.
    #[test]
    fn remove_tool_blocked_by_a_piece_nodes_dependency() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 10.0, 0.0);
        let c = doc.add_base_point("C", 0.0, 10.0);
        let piece = doc
            .add_piece(
                "Piece1",
                vec![
                    PieceNode {
                        point: a,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: b,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: c,
                        excluded_from_seam_allowance: false,
                    },
                ],
                "1.0",
            )
            .unwrap(); // added before the line below, so the piece is found first during remove_tool's scan
        doc.add_line(a, b).unwrap(); // `a` is ALSO used directly here, matching the "used both ways" scenario

        let err = doc.remove_tool(a).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: a,
                used_by: piece
            }
        );
        assert!(doc.contains(a)); // the failed removal changed nothing
    }

    // Part C: one remove_tool-blocked-by-dependency test per newly added
    // tool, proving `references_of` was correctly extended for each new
    // `ToolKind` variant — matching the exact pattern the pre-existing
    // `remove_tool_blocked_by_a_normal_tools_dependency`/
    // `remove_tool_blocked_by_a_piece_nodes_dependency` tests already use.

    #[test]
    fn remove_tool_blocked_by_a_shoulder_points_dependency() {
        let mut doc = Document::default();
        let p1_line = doc.add_base_point("P1Line", 0.0, 0.0);
        let p2_line = doc.add_base_point("P2Line", 10.0, 0.0);
        let shoulder = doc.add_base_point("Shoulder", 15.0, 8.0);
        let result = doc
            .add_shoulder_point("S", p1_line, p2_line, shoulder, "10")
            .unwrap();

        let err = doc.remove_tool(shoulder).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: shoulder,
                used_by: result
            }
        );
        assert!(doc.contains(shoulder)); // the failed removal changed nothing
    }

    #[test]
    fn remove_tool_blocked_by_a_line_intersects_dependency() {
        let mut doc = Document::default();
        let p1_line1 = doc.add_base_point("P1L1", 0.0, 0.0);
        let p2_line1 = doc.add_base_point("P2L1", 3.0, 3.0);
        let p1_line2 = doc.add_base_point("P1L2", 0.0, 4.0);
        let p2_line2 = doc.add_base_point("P2L2", 4.0, 0.0);
        let result = doc
            .add_line_intersect("X", p1_line1, p2_line1, p1_line2, p2_line2)
            .unwrap();

        // p2_line2 is the FOURTH id in the signature, proving every one of
        // the four references (not just the first) is actually tracked.
        let err = doc.remove_tool(p2_line2).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: p2_line2,
                used_by: result
            }
        );
        assert!(doc.contains(p2_line2)); // the failed removal changed nothing
    }

    #[test]
    fn remove_tool_blocked_by_a_point_of_intersections_dependency() {
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 3.0, 7.0);
        let p2 = doc.add_base_point("P2", 9.0, -2.0);
        let result = doc.add_point_of_intersection("X", p1, p2).unwrap();

        let err = doc.remove_tool(p2).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: p2,
                used_by: result
            }
        );
        assert!(doc.contains(p2)); // the failed removal changed nothing
    }

    #[test]
    fn remove_tool_blocked_by_a_triangles_dependency() {
        let mut doc = Document::default();
        let axis_p1 = doc.add_base_point("AxisP1", 0.0, 0.0);
        let axis_p2 = doc.add_base_point("AxisP2", 20.0, 0.0);
        let hyp_p1 = doc.add_base_point("HypP1", 8.0, -3.0);
        let hyp_p2 = doc.add_base_point("HypP2", 14.0, 9.0);
        let result = doc
            .add_triangle("T", axis_p1, axis_p2, hyp_p1, hyp_p2)
            .unwrap();

        // hyp_p2 is the FOURTH id in the signature, proving every one of
        // the four references is actually tracked.
        let err = doc.remove_tool(hyp_p2).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: hyp_p2,
                used_by: result
            }
        );
        assert!(doc.contains(hyp_p2)); // the failed removal changed nothing
    }

    #[test]
    fn remove_tool_blocked_by_a_point_of_contacts_dependency() {
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 5.0, 3.0);
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p2 = doc.add_base_point("P2", 10.0, 0.0);
        let result = doc.add_point_of_contact("X", center, p1, p2, "4").unwrap();

        let err = doc.remove_tool(center).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: center,
                used_by: result
            }
        );
        assert!(doc.contains(center)); // the failed removal changed nothing
    }

    #[test]
    fn add_arc_with_unknown_center_fails_without_mutating_history() {
        let mut doc = Document::default();
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc.add_arc("A", bogus, "10", "0", "90").unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_arc_with_valid_center_succeeds() {
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 0.0, 0.0);
        let result = doc.add_arc("A", center, "10", "0", "90");
        assert!(result.is_ok());
    }

    #[test]
    fn add_spline_with_unknown_point_fails_without_mutating_history() {
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = doc.history().len();
        let err = doc
            .add_spline("S", a, bogus, "90", "3", "90", "3")
            .unwrap_err();
        let len_after = doc.history().len();

        assert_eq!(err, PatternError::MissingDependency(bogus));
        assert_eq!(len_before, len_after);
    }

    #[test]
    fn add_spline_with_valid_points_succeeds() {
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p4 = doc.add_base_point("P4", 10.0, 0.0);
        let result = doc.add_spline("S", p1, p4, "90", "3", "90", "3");
        assert!(result.is_ok());
    }

    #[test]
    fn remove_tool_blocked_by_an_arcs_dependency() {
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 0.0, 0.0);
        let result = doc.add_arc("A", center, "10", "0", "90").unwrap();

        let err = doc.remove_tool(center).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: center,
                used_by: result
            }
        );
        assert!(doc.contains(center)); // the failed removal changed nothing
    }

    #[test]
    fn remove_tool_blocked_by_a_splines_dependency() {
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p4 = doc.add_base_point("P4", 10.0, 0.0);
        let result = doc.add_spline("S", p1, p4, "90", "3", "90", "3").unwrap();

        let err = doc.remove_tool(p1).unwrap_err();
        assert_eq!(
            err,
            DocumentError::ToolInUse {
                id: p1,
                used_by: result
            }
        );
        assert!(doc.contains(p1)); // the failed removal changed nothing
    }
}
