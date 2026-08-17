use std::collections::HashMap; // backs `base_variables`, the same collection type `PatternData` uses for variables

use crate::id::ObjectId; // the id type every tool record and geometric reference is built from
use crate::variable::Variable; // the variable payload type stored in `base_variables`

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
#[derive(Debug, Clone, PartialEq, Default)] // Default gives an empty history, empty variables, and next_id starting at 0 — an empty pattern
pub struct Document {
    history: Vec<ToolRecord>, // every tool added so far, in the exact order it was added
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
}
