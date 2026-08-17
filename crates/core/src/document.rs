use std::collections::HashMap; // backs `base_variables`, the same collection type `PatternData` uses for variables
use std::collections::HashSet; // backs `history_ids`, an O(1) existence index parallel to `history`

use crate::id::ObjectId; // the id type every tool record and geometric reference is built from
use crate::variable::{Variable, VariableKind}; // the variable payload type, and its kind tag used to filter measurements out

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

// TODO(phase 10): AlongLine, Normal, Bisector, Arc, Spline, ...
/// The kind of a [`ToolRecord`], carrying whatever inputs that tool needs
/// to produce its geometry. Only the three tools needed to prove the
/// recompute engine end-to-end are implemented so far; more tool kinds
/// arrive in later phases.
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

    /// Iterates every `(name, Variable)` pair in the base variable table,
    /// in unspecified order (the same order `HashMap::iter` yields).
    ///
    /// `pub(crate)`: only `recompute::recompute_all` needs this, to seed a
    /// fresh `PatternData` with `Document`'s variables before walking the
    /// tool history.
    pub(crate) fn base_variables(&self) -> impl Iterator<Item = (&String, &Variable)> {
        self.base_variables.iter() // hand back the map's own iterator; nothing to transform here
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
}
