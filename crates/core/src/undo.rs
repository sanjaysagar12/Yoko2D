use crate::document::{DocumentError, ToolKind, ToolRecord}; // the Document error/data types every Edit variant carries
use crate::id::ObjectId; // the id type ModifyFormula records which tool it changed
use crate::Document; // the type every Edit variant is applied to/reverted against

/// One reversible change to a [`Document`]'s tool history.
///
/// Each variant carries everything needed to BOTH re-apply it (redo) and
/// undo it — that's what makes undo/redo symmetric without needing a
/// separate hierarchy of command objects the way some undo frameworks use;
/// a single `Edit` value drives both directions via [`apply_edit`] and
/// [`revert_edit`] below.
///
/// Mirrors the original C++ app's undo/redo philosophy: an `Edit` only
/// ever describes a change to `Document` (our DOM-equivalent, the ordered
/// tool-record list), never to resolved geometry. Neither this type nor
/// [`UndoStack`] below ever calls `recompute_all` — callers are expected
/// to do that themselves after an undo/redo, the same way they would after
/// any other `Document` edit.
#[derive(Debug, Clone, PartialEq)] // Debug: printable in test failures; Clone: UndoStack needs to clone edits between its two stacks; PartialEq: enables assert_eq! in tests
pub enum Edit {
    /// A tool was added. Reverting removes it (by id, via
    /// [`Document::remove_tool`]); re-applying inserts it back at the end
    /// of history (via [`Document::insert_tool_record_at`]).
    AddTool(ToolRecord),

    /// A tool was removed. Reverting re-inserts `record` at `index` — its
    /// exact original position; re-applying removes it again by
    /// `record.id`.
    RemoveTool {
        record: ToolRecord, // the full record as it existed right before removal
        index: usize,       // exactly where in history it was removed from
    },

    /// A tool's kind (its formulas/literal values) was replaced. Reverting
    /// restores `old_kind`; re-applying restores `new_kind`.
    ModifyFormula {
        id: ObjectId,       // which tool was modified
        old_kind: ToolKind, // its kind before the edit
        new_kind: ToolKind, // its kind after the edit
    },
}

/// Re-applies `edit` to `doc` — the "redo" direction.
///
/// Private: only [`UndoStack::redo`] should call this, so an `Edit` is
/// never applied to a `Document` it wasn't recorded/validated against.
fn apply_edit(edit: &Edit, doc: &mut Document) -> Result<(), DocumentError> {
    match edit {
        // dispatch on which kind of edit this is
        Edit::AddTool(record) => {
            // Re-applying an add always means putting it back at the
            // current end of history: it was originally appended there by
            // add_tool, and by construction of how an undo stack works,
            // only the most recently undone action can be the one being
            // redone — nothing can have been inserted after the position
            // this record used to occupy without first clearing the redo
            // stack (see UndoStack::record).
            doc.insert_tool_record_at(doc.history().len(), record.clone())?; // insert a clone at the current end of history
        }
        Edit::RemoveTool { record, .. } => {
            // Re-applying a removal just means removing it again. We
            // already know, from when this Edit was first created, that
            // the removal was valid (nothing depended on it) — UNLESS
            // some undo/redo sequence in between somehow reintroduced a
            // new dependent. That's a known theoretical edge case a real
            // UI layer must guard against, e.g. by clearing the redo
            // stack whenever an unrelated new edit is made, which
            // UndoStack::record already does below.
            doc.remove_tool(record.id)?; // remove by id; the returned (record, index) is discarded, since we already have both
        }
        Edit::ModifyFormula { id, new_kind, .. } => {
            doc.replace_tool_kind(*id, new_kind.clone())?; // restore the "after" kind; the returned old value is discarded, since we already have new_kind
        }
    }
    Ok(()) // the edit was successfully re-applied
}

/// Reverts `edit` on `doc` — the "undo" direction.
///
/// Private: only [`UndoStack::undo`] should call this.
fn revert_edit(edit: &Edit, doc: &mut Document) -> Result<(), DocumentError> {
    match edit {
        // dispatch on which kind of edit this is
        Edit::AddTool(record) => {
            doc.remove_tool(record.id)?; // reverting an add means removing it; the returned (record, index) is discarded, since we already have record
        }
        Edit::RemoveTool { record, index } => {
            doc.insert_tool_record_at(*index, record.clone())?; // reverting a removal means putting it back at its original position
        }
        Edit::ModifyFormula { id, old_kind, .. } => {
            doc.replace_tool_kind(*id, old_kind.clone())?; // reverting a modification means restoring the old kind; the returned new value is discarded, since we already have new_kind
        }
    }
    Ok(()) // the edit was successfully reverted
}

/// A linear undo/redo history of [`Edit`]s applied to a [`Document`].
///
/// Deliberately holds no reference to any particular `Document` — the
/// same `UndoStack` value has no fixed association with one `Document`; it
/// just records/replays `Edit`s against whichever `&mut Document` is
/// passed to each call. Callers are responsible for always passing the
/// same `Document` an `UndoStack`'s edits were originally recorded
/// against.
#[derive(Debug, Clone, Default, PartialEq)] // Debug: printable in test failures; Clone: tests capture snapshots to compare against; Default: an empty stack, no edits yet; PartialEq: enables assert_eq! in tests
pub struct UndoStack {
    undo: Vec<Edit>, // edits that can be undone, most-recent last
    redo: Vec<Edit>, // edits that have been undone and can be redone, most-recently-undone last
}

impl UndoStack {
    /// Pushes `edit` onto the undo stack and clears the redo stack.
    ///
    /// Making any new edit after having undone something invalidates the
    /// ability to redo the undone actions — the same convention most
    /// undo/redo implementations (including Qt's `QUndoStack`) follow:
    /// there is only one "future" at a time, and recording a new edit
    /// replaces whatever future the redo stack was pointing at.
    pub fn record(&mut self, edit: Edit) {
        self.undo.push(edit); // this edit becomes the most recent undoable action
        self.redo.clear(); // any previously-undone actions are no longer redoable
    }

    /// Undoes the most recent edit, if any.
    ///
    /// Returns `Ok(true)` if something was undone, `Ok(false)` if there
    /// was nothing to undo. An empty undo stack is a completely normal,
    /// expected state (e.g. right after opening a document) — not an
    /// error condition — hence a bool return rather than a distinct
    /// "nothing to undo" error variant.
    pub fn undo(&mut self, doc: &mut Document) -> Result<bool, DocumentError> {
        let Some(edit) = self.undo.pop() else {
            return Ok(false); // nothing recorded: nothing to undo, and doc is left untouched
        };
        revert_edit(&edit, doc)?; // if this fails, `edit` is intentionally NOT pushed back anywhere and is lost — should not happen if every Edit only ever comes from this struct's own do_* methods (which validate before recording), but handled without panicking regardless
        self.redo.push(edit); // successfully undone: it becomes redoable
        Ok(true) // something was undone
    }

    /// Redoes the most recently undone edit, if any.
    ///
    /// Exact mirror of [`Self::undo`]: `Ok(true)` if something was redone,
    /// `Ok(false)` if the redo stack was empty.
    pub fn redo(&mut self, doc: &mut Document) -> Result<bool, DocumentError> {
        let Some(edit) = self.redo.pop() else {
            return Ok(false); // nothing undone: nothing to redo, and doc is left untouched
        };
        apply_edit(&edit, doc)?; // if this fails, `edit` is lost, for the same reason as in undo() above
        self.undo.push(edit); // successfully redone: it becomes undoable again
        Ok(true) // something was redone
    }

    /// Number of edits currently undoable. Exposed for tests/debugging,
    /// so callers can assert on stack depth without reaching into private
    /// fields.
    pub fn undo_len(&self) -> usize {
        self.undo.len() // how many edits are on the undo stack right now
    }

    /// Number of edits currently redoable. Exposed for tests/debugging,
    /// same rationale as [`Self::undo_len`].
    pub fn redo_len(&self) -> usize {
        self.redo.len() // how many edits are on the redo stack right now
    }

    /// Adds a literal starting point to `doc` and records the edit.
    ///
    /// Performs the `Document` mutation FIRST and only records if it
    /// succeeded — a failed operation must never leave a phantom entry on
    /// the undo stack. `Document::add_base_point` itself can never fail,
    /// so this always records.
    pub fn do_add_base_point(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        x: f64,
        y: f64,
    ) -> ObjectId {
        let id = doc.add_base_point(name, x, y); // perform the mutation first; add_base_point can never fail
        if let Some(record) = doc.get_tool(id).cloned() {
            // Look up what was just added, to capture its exact ToolRecord
            // for the Edit. This is always `Some` in practice — we hold
            // exclusive access to `doc` and just inserted this id — but
            // handled as an `Option` (rather than `.unwrap()`/`.expect()`)
            // to respect this crate's `#![deny(clippy::unwrap_used, ...)]`
            // policy for non-test code.
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        id // hand back the new id, same as the underlying Document method
    }

    /// Adds an angle/length-formula-driven point to `doc` and records the
    /// edit.
    ///
    /// NOTE: returns `Result<ObjectId, crate::PatternError>`, not
    /// `DocumentError` — `Document::add_end_line` (Phase 4) itself returns
    /// `PatternError`, a distinct, unrelated error type from `Document`'s
    /// own `DocumentError` (which Phase 9's new `remove_tool`/
    /// `replace_tool_kind`/`insert_tool_record_at` use). There's no
    /// conversion between the two, so this method's error type has to
    /// match what it actually propagates.
    pub fn do_add_end_line(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        base_point: ObjectId,
        angle_formula: impl Into<String>,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_end_line(name, base_point, angle_formula, length_formula)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Adds a line between two existing points to `doc` and records the
    /// edit.
    ///
    /// Same rationale as [`Self::do_add_end_line`] for why this returns
    /// `crate::PatternError`, not `DocumentError`.
    pub fn do_add_line(
        &mut self,
        doc: &mut Document,
        p1: ObjectId,
        p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_line(p1, p2)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Adds an `AlongLine` point to `doc` and records the edit.
    ///
    /// Same rationale as [`Self::do_add_end_line`] for why this returns
    /// `crate::PatternError`, not `DocumentError`.
    pub fn do_add_along_line(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_along_line(name, p1, p2, length_formula)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Adds a `Normal` point to `doc` and records the edit.
    ///
    /// Same rationale as [`Self::do_add_end_line`] for why this returns
    /// `crate::PatternError`, not `DocumentError`.
    pub fn do_add_normal(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
        length_formula: impl Into<String>,
        angle_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_normal(name, p1, p2, length_formula, angle_formula)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Adds a `Bisector` point to `doc` and records the edit.
    ///
    /// Same rationale as [`Self::do_add_end_line`] for why this returns
    /// `crate::PatternError`, not `DocumentError`.
    pub fn do_add_bisector(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
        p3: ObjectId,
        length_formula: impl Into<String>,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_bisector(name, p1, p2, p3, length_formula)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Adds a `Height` point to `doc` and records the edit.
    ///
    /// Same rationale as [`Self::do_add_end_line`] for why this returns
    /// `crate::PatternError`, not `DocumentError`.
    pub fn do_add_height(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        point: ObjectId,
        line_p1: ObjectId,
        line_p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_height(name, point, line_p1, line_p2)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Adds a `Midpoint` point to `doc` and records the edit.
    ///
    /// Same rationale as [`Self::do_add_end_line`] for why this returns
    /// `crate::PatternError`, not `DocumentError`.
    pub fn do_add_midpoint(
        &mut self,
        doc: &mut Document,
        name: impl Into<String>,
        p1: ObjectId,
        p2: ObjectId,
    ) -> Result<ObjectId, crate::PatternError> {
        let id = doc.add_midpoint(name, p1, p2)?; // perform the mutation first; propagate a validation failure without recording anything
        if let Some(record) = doc.get_tool(id).cloned() {
            // see do_add_base_point's comment above for why this is an `if let`, not an unwrap/expect
            self.record(Edit::AddTool(record)); // record the edit only now that the mutation is known to have succeeded
        }
        Ok(id) // hand back the new id, same as the underlying Document method
    }

    /// Removes a tool from `doc` and records the edit.
    pub fn do_remove_tool(
        &mut self,
        doc: &mut Document,
        id: ObjectId,
    ) -> Result<(), DocumentError> {
        let (record, index) = doc.remove_tool(id)?; // perform the mutation first; propagate failure (e.g. ToolInUse) without recording anything
        self.record(Edit::RemoveTool { record, index }); // record the edit now that removal is known to have succeeded
        Ok(()) // removed and recorded successfully
    }

    /// Replaces a tool's kind (its formulas/literal values) in `doc` and
    /// records the edit.
    pub fn do_modify_formula(
        &mut self,
        doc: &mut Document,
        id: ObjectId,
        new_kind: ToolKind,
    ) -> Result<(), DocumentError> {
        let old_kind = doc.replace_tool_kind(id, new_kind.clone())?; // perform the mutation first; propagate failure without recording anything
        self.record(Edit::ModifyFormula {
            id,
            old_kind,
            new_kind,
        }); // record the edit now that replacement is known to have succeeded
        Ok(()) // replaced and recorded successfully
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recompute::recompute_all;
    use crate::variable::Variable;

    #[test]
    fn undo_all_then_redo_all_round_trips_through_empty_and_back() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let a = stack.do_add_base_point(&mut doc, "A", 0.0, 0.0);
        let a1 = stack.do_add_end_line(&mut doc, "A1", a, "0", "10").unwrap();
        stack.do_add_line(&mut doc, a, a1).unwrap();
        stack
            .do_modify_formula(
                &mut doc,
                a1,
                ToolKind::EndLine {
                    name: "A1".to_string(),
                    base_point: a,
                    angle_formula: "0".to_string(),
                    length_formula: "20".to_string(),
                },
            )
            .unwrap();
        let b = stack.do_add_base_point(&mut doc, "B", 5.0, 5.0);
        stack.do_remove_tool(&mut doc, b).unwrap();

        let edit_count = stack.undo_len();
        // 6 successful edits recorded: AddTool(A), AddTool(A1), AddTool(line),
        // ModifyFormula(A1), AddTool(B), RemoveTool(B) — adding B and then
        // removing it are two separate edits on the stack, not a cancelling pair.
        assert_eq!(edit_count, 6);

        let after_edits = doc.clone(); // deep clone: the state right after all edits, to compare redo against later

        for _ in 0..edit_count {
            let undone = stack.undo(&mut doc).unwrap();
            assert!(undone);
        }
        assert_eq!(stack.undo_len(), 0);
        // "Fully back to empty/initial state" here means empty history and
        // variables, NOT byte-for-byte equality with `Document::default()`:
        // `remove_tool` (and therefore undoing an AddTool, which calls it)
        // deliberately never rolls back `next_id` — see its own doc comment
        // — so `next_id` keeps whatever value it climbed to even once every
        // tool has been undone away. That's by design (it's what makes a
        // later redo of an add collision-free), so exact equality against a
        // freshly-`Default`-constructed Document (next_id back at 0) is
        // never actually achievable once any tool has ever been added.
        assert_eq!(doc.history().len(), 0);
        assert_eq!(doc.base_variables().count(), 0);

        for _ in 0..edit_count {
            let redone = stack.redo(&mut doc).unwrap();
            assert!(redone);
        }
        assert_eq!(stack.redo_len(), 0);
        assert_eq!(doc, after_edits); // fully back to the state captured right after the edits
    }

    #[test]
    fn new_edit_after_undo_discards_the_stale_redo_entry() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let a = stack.do_add_base_point(&mut doc, "A", 0.0, 0.0);
        stack.do_add_base_point(&mut doc, "B", 1.0, 1.0);

        stack.undo(&mut doc).unwrap(); // undo adding B
        assert_eq!(stack.redo_len(), 1); // redoing B is available...

        stack.do_add_base_point(&mut doc, "C", 2.0, 2.0); // ...until a new edit is made
        assert_eq!(stack.redo_len(), 0); // the stale "redo B" entry is discarded

        let redone = stack.redo(&mut doc).unwrap();
        assert!(!redone); // nothing to redo: confirmed by the return value, not just the length

        // Document reflects the new edit (A, C), not the discarded one (A, B).
        let names: Vec<String> = doc
            .history()
            .iter()
            .map(|record| match &record.kind {
                ToolKind::BasePoint { name, .. } => name.clone(),
                _ => unreachable!("this test only adds BasePoints"),
            })
            .collect();
        assert_eq!(names, vec!["A".to_string(), "C".to_string()]);
        let _ = a; // only used to anchor the first BasePoint's id; not asserted on directly
    }

    #[test]
    fn undo_on_empty_stack_is_a_no_op() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let undone = stack.undo(&mut doc).unwrap();
        assert!(!undone);
        assert_eq!(doc, Document::default()); // completely untouched
    }

    #[test]
    fn redo_on_empty_stack_is_a_no_op() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let redone = stack.redo(&mut doc).unwrap();
        assert!(!redone);
        assert_eq!(doc, Document::default()); // completely untouched
    }

    #[test]
    fn failed_do_add_line_does_not_grow_the_undo_stack() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let a = stack.do_add_base_point(&mut doc, "A", 0.0, 0.0);
        let bogus = ObjectId::new(9999); // never produced by this Document

        let len_before = stack.undo_len();
        let result = stack.do_add_line(&mut doc, a, bogus);
        let len_after = stack.undo_len();

        assert!(result.is_err());
        assert_eq!(len_before, len_after); // the failed attempt left no phantom entry
    }

    #[test]
    fn undo_after_modify_formula_restores_prior_recompute_result() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        doc.set_variable("height_scapula", Variable::Measurement { value: 40.0 });
        let a = stack.do_add_base_point(&mut doc, "A", 0.0, 0.0);
        let a1 = stack
            .do_add_end_line(&mut doc, "A1", a, "0", "height_scapula/10")
            .unwrap();

        let result_before = recompute_all(&doc).unwrap();

        stack
            .do_modify_formula(
                &mut doc,
                a1,
                ToolKind::EndLine {
                    name: "A1".to_string(),
                    base_point: a,
                    angle_formula: "0".to_string(),
                    length_formula: "height_scapula/5".to_string(), // a different formula: should change the recomputed geometry
                },
            )
            .unwrap();
        let result_after_modify = recompute_all(&doc).unwrap();
        assert_ne!(result_before, result_after_modify); // the modification actually changed the resolved geometry

        let undone = stack.undo(&mut doc).unwrap();
        assert!(undone);
        let result_after_undo = recompute_all(&doc).unwrap();
        assert_eq!(result_after_undo, result_before); // undo -> recompute reproduces the original geometry exactly
    }

    #[test]
    fn undo_redo_along_line_matches_pre_and_post_add_recompute() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let p1 = stack.do_add_base_point(&mut doc, "P1", 0.0, 0.0);
        let p2 = stack.do_add_base_point(&mut doc, "P2", 3.0, 4.0);
        let before_add = recompute_all(&doc).unwrap(); // resolved geometry with only P1/P2, before AlongLine exists

        let along = stack
            .do_add_along_line(&mut doc, "AL", p1, p2, "10")
            .unwrap();
        let after_add = recompute_all(&doc).unwrap();
        assert!(after_add.get_point(along).is_ok()); // the new point actually resolved

        let undone = stack.undo(&mut doc).unwrap();
        assert!(undone);
        let after_undo = recompute_all(&doc).unwrap();
        assert_eq!(after_undo, before_add); // undo -> recompute matches the pre-add state exactly

        let redone = stack.redo(&mut doc).unwrap();
        assert!(redone);
        let after_redo = recompute_all(&doc).unwrap();
        assert_eq!(after_redo, after_add); // redo -> recompute matches the post-add state exactly
    }

    #[test]
    fn undo_redo_midpoint_matches_pre_and_post_add_recompute() {
        let mut doc = Document::default();
        let mut stack = UndoStack::default();

        let p1 = stack.do_add_base_point(&mut doc, "P1", 1.0, 3.0);
        let p2 = stack.do_add_base_point(&mut doc, "P2", 7.0, 9.0);
        let before_add = recompute_all(&doc).unwrap(); // resolved geometry with only P1/P2, before Midpoint exists

        let mid = stack.do_add_midpoint(&mut doc, "M", p1, p2).unwrap();
        let after_add = recompute_all(&doc).unwrap();
        assert!(after_add.get_point(mid).is_ok()); // the new point actually resolved

        let undone = stack.undo(&mut doc).unwrap();
        assert!(undone);
        let after_undo = recompute_all(&doc).unwrap();
        assert_eq!(after_undo, before_add); // undo -> recompute matches the pre-add state exactly

        let redone = stack.redo(&mut doc).unwrap();
        assert!(redone);
        let after_redo = recompute_all(&doc).unwrap();
        assert_eq!(after_redo, after_add); // redo -> recompute matches the post-add state exactly
    }
}
