use std::path::PathBuf; // the measurement file path type stored on PatternSync

use thiserror::Error; // brings in the Error derive macro used below

/// Everything that can go wrong keeping a [`PatternSync`] up to date.
#[derive(Debug, Error)] // Debug: printable in test failures; Error: implements std::error::Error via thiserror
pub enum SyncError {
    /// Reading/parsing/validating the measurement file failed. Wrapped via
    /// `#[from]` so `?` converts a `io::MeasurementError` automatically.
    #[error("failed to load measurements: {0}")]
    Measurement(#[from] io::MeasurementError), // the underlying measurement-file failure

    /// Recomputing the pattern's geometry from the (possibly updated)
    /// `Document` failed. Wrapped via `#[from]` so `?` converts a
    /// `core_lib::PatternError` automatically.
    #[error("failed to recompute pattern: {0}")]
    Pattern(#[from] core_lib::PatternError), // the underlying recompute failure

    /// An undo/redo step (or another direct `Document` mutation) failed.
    /// Wrapped via `#[from]` so `?` converts a `core_lib::DocumentError`
    /// automatically.
    #[error("failed to apply document edit: {0}")]
    Document(#[from] core_lib::DocumentError), // the underlying Document-mutation failure
}

/// Keeps a resolved [`core_lib::PatternData`] in sync with a
/// [`core_lib::Document`] and a measurement file on disk.
///
/// Fields are private: `document` and `current_data` must only ever change
/// together, through [`Self::resync`], so an external caller can never see
/// them in a state where one reflects a newer measurement load than the
/// other.
pub struct PatternSync {
    document: core_lib::Document, // the persistent tool history (Phases 3/4's source of truth)
    current_data: core_lib::PatternData, // the most recently resolved geometry cache
    measurement_path: PathBuf,    // where resync reads measurements from each time it's called
    undo_stack: core_lib::UndoStack, // the undo/redo history of edits made to `document` via `perform_edit`
}

impl PatternSync {
    /// Builds a `PatternSync` for `document`, reading `measurement_path`
    /// immediately so the returned value is never out of sync with the
    /// file's current contents — there is no "construct now, sync later"
    /// window.
    pub fn new(document: core_lib::Document, measurement_path: PathBuf) -> Result<Self, SyncError> {
        let mut sync = PatternSync {
            document,                                       // the caller-supplied starting document
            current_data: core_lib::PatternData::default(), // placeholder: replaced by the resync call below before this returns
            measurement_path, // the caller-supplied measurement file path
            undo_stack: core_lib::UndoStack::default(), // no edits recorded yet
        };
        sync.resync()?; // populate current_data (and re-apply measurements to document) right away; propagate any failure
        Ok(sync) // resync succeeded: the returned PatternSync is fully in sync with the file
    }

    /// Reloads measurements from `measurement_path` and recomputes the
    /// pattern's geometry, replacing `document`/`current_data` only if
    /// every step succeeds.
    pub fn resync(&mut self) -> Result<(), SyncError> {
        // Load first, before touching `self` at all: if this fails, `self`
        // must remain exactly as it was — an unreadable/malformed file on a
        // later resync should never corrupt a previously-good in-memory state.
        let measurements = io::load_measurements_from_file(&self.measurement_path)?;

        // Work on a clone of the document, not `self.document` directly, so
        // that if a later step (recompute) fails, `self.document` is never
        // left partially updated — `self` is only touched once every
        // fallible step below has already succeeded.
        let mut candidate_document = self.document.clone(); // clone: self.document stays untouched until the very end
        candidate_document.apply_measurements(measurements); // infallible: just replaces/sets variables on the clone

        // Recompute against the candidate, still without touching `self`:
        // if this fails, `self.document`/`self.current_data` must remain
        // exactly as they were before this call.
        let candidate_data = core_lib::recompute_all(&candidate_document)?; // propagate a recompute failure before committing anything

        // Both fallible steps above succeeded: only now is it safe to commit.
        self.document = candidate_document; // adopt the newly-measured document
        self.current_data = candidate_data; // adopt the freshly recomputed geometry
        Ok(()) // resync completed successfully
    }

    /// Returns the most recently resolved geometry.
    pub fn current_data(&self) -> &core_lib::PatternData {
        &self.current_data // read-only borrow; mutation only happens through resync
    }

    /// Returns the current tool history / variable table.
    pub fn document(&self) -> &core_lib::Document {
        &self.document // read-only borrow; mutation only happens through resync
    }

    /// Mutable access to the tool history, for later phases (interactive
    /// editing) to add/modify tools. Not exercised by this phase's tests,
    /// but exposed now so those later phases don't require another API
    /// change to `PatternSync` itself.
    pub fn document_mut(&mut self) -> &mut core_lib::Document {
        &mut self.document // caller is responsible for calling resync afterward if geometry needs to reflect the edit
    }

    /// Applies one interactive edit to the document via `edit_fn`,
    /// recomputing geometry and recording the edit for undo only if
    /// everything succeeds.
    ///
    /// `edit_fn` is expected to call one of `UndoStack`'s `do_add_*`
    /// methods (Phase 9/10a) on the given `Document`/`UndoStack` pair —
    /// this is the single entry point Phase 11's click-handling code uses
    /// to commit a completed tool.
    pub fn perform_edit<F>(&mut self, edit_fn: F) -> Result<(), SyncError>
    where
        F: FnOnce(
            &mut core_lib::Document,
            &mut core_lib::UndoStack,
        ) -> Result<(), core_lib::PatternError>,
    {
        // Work on clones of both the document and the undo stack, exactly
        // as `resync` does: if any later step fails, `self` must remain
        // completely untouched, never left with a half-applied edit.
        let mut candidate_document = self.document.clone(); // clone: self.document stays untouched until the very end
        let mut candidate_undo_stack = self.undo_stack.clone(); // clone: self.undo_stack stays untouched until the very end

        edit_fn(&mut candidate_document, &mut candidate_undo_stack)?; // perform the caller's edit on the clones; propagate any validation failure before touching self

        // Even if the edit closure itself succeeded (e.g. because it
        // validated every referenced id correctly), recompute could still
        // theoretically fail for an unrelated reason — a formula now
        // evaluating to a degenerate result, for instance — so this must
        // also be checked before committing anything to self.
        let candidate_data = core_lib::recompute_all(&candidate_document)?; // propagate a recompute failure before touching self

        // Both fallible steps above succeeded: only now is it safe to commit.
        self.document = candidate_document; // adopt the edited document
        self.undo_stack = candidate_undo_stack; // adopt the updated undo history
        self.current_data = candidate_data; // adopt the freshly recomputed geometry
        Ok(()) // the edit was applied, recomputed, and recorded successfully
    }

    /// Undoes the most recent edit, if any.
    ///
    /// Returns `Ok(true)` if something was undone, `Ok(false)` if the undo
    /// history was empty — mirrors `core_lib::UndoStack::undo`'s own
    /// "empty is normal, not an error" contract.
    pub fn undo(&mut self) -> Result<bool, SyncError> {
        let mut candidate_document = self.document.clone(); // clone: self.document stays untouched unless the undo actually changes something
        let mut candidate_undo_stack = self.undo_stack.clone(); // clone: self.undo_stack stays untouched unless the undo actually changes something

        let did_undo = candidate_undo_stack.undo(&mut candidate_document)?; // propagate a DocumentError via #[from] before touching self

        if !did_undo {
            // Nothing was actually undone (the undo history was empty): no
            // state changed, so there's nothing to recompute or commit —
            // this also avoids unnecessary recompute work on a no-op call.
            return Ok(false);
        }

        let candidate_data = core_lib::recompute_all(&candidate_document)?; // propagate a recompute failure before touching self

        self.document = candidate_document; // adopt the reverted document
        self.undo_stack = candidate_undo_stack; // adopt the updated undo/redo history
        self.current_data = candidate_data; // adopt the freshly recomputed geometry
        Ok(true) // something was undone
    }

    /// Redoes the most recently undone edit, if any.
    ///
    /// Exact mirror of [`Self::undo`]: `Ok(true)` if something was redone,
    /// `Ok(false)` if the redo history was empty.
    pub fn redo(&mut self) -> Result<bool, SyncError> {
        let mut candidate_document = self.document.clone(); // clone: self.document stays untouched unless the redo actually changes something
        let mut candidate_undo_stack = self.undo_stack.clone(); // clone: self.undo_stack stays untouched unless the redo actually changes something

        let did_redo = candidate_undo_stack.redo(&mut candidate_document)?; // propagate a DocumentError via #[from] before touching self

        if !did_redo {
            // Nothing was actually redone (the redo history was empty): same rationale as undo() above.
            return Ok(false);
        }

        let candidate_data = core_lib::recompute_all(&candidate_document)?; // propagate a recompute failure before touching self

        self.document = candidate_document; // adopt the reapplied document
        self.undo_stack = candidate_undo_stack; // adopt the updated undo/redo history
        self.current_data = candidate_data; // adopt the freshly recomputed geometry
        Ok(true) // something was redone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_measurements(path: &std::path::Path, height_scapula: f64) {
        let json =
            format!(r#"{{"measurements":[{{"name":"height_scapula","value":{height_scapula}}}]}}"#);
        std::fs::write(path, json).unwrap();
    }

    fn build_document() -> (core_lib::Document, core_lib::ObjectId) {
        let mut doc = core_lib::Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();
        (doc, a1)
    }

    #[test]
    fn new_immediately_resolves_geometry_from_the_measurement_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("measurements.json");
        write_measurements(&path, 40.0);

        let (doc, a1) = build_document();
        let sync = PatternSync::new(doc, path).unwrap();

        let point = sync.current_data().get_point(a1).unwrap();
        assert!((point.x - 4.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn resync_reflects_new_values_written_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("measurements.json");
        write_measurements(&path, 40.0);

        let (doc, a1) = build_document();
        let mut sync = PatternSync::new(doc, path.clone()).unwrap();

        write_measurements(&path, 80.0);
        sync.resync().unwrap();

        let point = sync.current_data().get_point(a1).unwrap();
        assert!((point.x - 8.0).abs() < 1e-9);
    }

    #[test]
    fn resync_leaves_state_untouched_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("measurements.json");
        write_measurements(&path, 40.0);

        let (doc, _a1) = build_document();
        let mut sync = PatternSync::new(doc, path.clone()).unwrap();

        let data_before = sync.current_data().clone();
        let doc_before = sync.document().clone();

        std::fs::write(&path, "{ not valid json").unwrap();
        let result = sync.resync();
        assert!(result.is_err());

        assert_eq!(sync.current_data(), &data_before);
        assert_eq!(sync.document(), &doc_before);
    }

    // A PatternSync pointed at an empty (no measurements) but validly-formed file,
    // for tests that only care about perform_edit/undo/redo, not measurement loading.
    fn empty_sync() -> (tempfile::TempDir, PatternSync) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("measurements.json");
        write_measurements(&path, 0.0); // content doesn't matter for these tests, just needs to parse
        let sync = PatternSync::new(core_lib::Document::default(), path).unwrap();
        (dir, sync) // keep `dir` alive alongside `sync`, since dropping it would delete the file sync's measurement_path points at
    }

    #[test]
    fn perform_edit_with_infallible_closure_updates_current_data() {
        let (_dir, mut sync) = empty_sync();

        sync.perform_edit(|doc, undo_stack| {
            undo_stack.do_add_base_point(doc, "A", 1.0, 2.0); // cannot fail
            Ok(())
        })
        .unwrap();

        let ids: Vec<core_lib::ObjectId> = sync
            .document()
            .history()
            .iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids.len(), 1);
        let point = sync.current_data().get_point(ids[0]).unwrap();
        assert!((point.x - 1.0).abs() < 1e-9);
        assert!((point.y - 2.0).abs() < 1e-9);
    }

    #[test]
    fn perform_edit_with_failing_closure_leaves_everything_untouched() {
        let (_dir, mut sync) = empty_sync();
        sync.perform_edit(|doc, undo_stack| {
            undo_stack.do_add_base_point(doc, "A", 0.0, 0.0);
            Ok(())
        })
        .unwrap();

        let data_before = sync.current_data().clone();
        let doc_before = sync.document().clone();

        let valid_id = doc_before.history()[0].id; // the id from the successful edit above
        let bogus_id = core_lib::ObjectId::from_raw(9999); // never produced by this Document

        let result = sync.perform_edit(|doc, undo_stack| {
            undo_stack.do_add_line(doc, valid_id, bogus_id)?; // should fail: bogus_id doesn't exist
            Ok(())
        });
        assert!(result.is_err());

        assert_eq!(sync.current_data(), &data_before);
        assert_eq!(sync.document(), &doc_before);
    }

    #[test]
    fn undo_then_redo_round_trips_a_successful_perform_edit() {
        let (_dir, mut sync) = empty_sync();
        sync.perform_edit(|doc, undo_stack| {
            undo_stack.do_add_base_point(doc, "A", 5.0, 5.0);
            Ok(())
        })
        .unwrap();
        let id = sync.document().history()[0].id;
        assert!(sync.current_data().get_point(id).is_ok()); // present right after the edit

        let undone = sync.undo().unwrap();
        assert!(undone);
        assert!(sync.current_data().get_point(id).is_err()); // gone after undo

        let redone = sync.redo().unwrap();
        assert!(redone);
        assert!(sync.current_data().get_point(id).is_ok()); // back after redo
    }

    #[test]
    fn undo_on_empty_history_is_a_no_op() {
        let (_dir, mut sync) = empty_sync();
        let data_before = sync.current_data().clone();

        let undone = sync.undo().unwrap();
        assert!(!undone);
        assert_eq!(sync.current_data(), &data_before); // completely untouched
    }

    // Simulates the full interactive click -> tool_controller -> perform_edit
    // pipeline the UI layer drives, but with no egui involved at all: two
    // BasePoint placements, then a Line between them, each committed via
    // perform_edit, followed by undoing both edits in turn.
    #[test]
    fn simulated_interactive_sequence_matches_ui_layer_behavior() {
        let (_dir, mut sync) = empty_sync();
        let mut controller = crate::tool_controller::ToolController::default();

        controller.select_tool(crate::tool_controller::ToolKindSelector::BasePoint);
        let first_click = controller.handle_click((0.0, 0.0), None);
        match first_click {
            crate::tool_controller::ClickOutcome::Complete(kind) => {
                sync.perform_edit(|doc, undo_stack| {
                    let core_lib::ToolKind::BasePoint { name, x, y } = kind else {
                        unreachable!("BasePoint tool always completes with a BasePoint kind")
                    };
                    undo_stack.do_add_base_point(doc, name, x, y);
                    Ok(())
                })
                .unwrap();
            }
            other => panic!("expected Complete, got {other:?}"),
        }

        let second_click = controller.handle_click((10.0, 0.0), None);
        match second_click {
            crate::tool_controller::ClickOutcome::Complete(kind) => {
                sync.perform_edit(|doc, undo_stack| {
                    let core_lib::ToolKind::BasePoint { name, x, y } = kind else {
                        unreachable!("BasePoint tool always completes with a BasePoint kind")
                    };
                    undo_stack.do_add_base_point(doc, name, x, y);
                    Ok(())
                })
                .unwrap();
            }
            other => panic!("expected Complete, got {other:?}"),
        }

        let point_ids: Vec<core_lib::ObjectId> = sync
            .document()
            .history()
            .iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(point_ids.len(), 2); // both points committed

        controller.select_tool(crate::tool_controller::ToolKindSelector::Line);
        let hit_a = crate::tool_controller::hit_test_point(sync.current_data(), (0.0, 0.0), 0.1);
        let line_first = controller.handle_click((0.0, 0.0), hit_a);
        assert_eq!(
            line_first,
            crate::tool_controller::ClickOutcome::NeedMoreInput
        );

        let hit_b = crate::tool_controller::hit_test_point(sync.current_data(), (10.0, 0.0), 0.1);
        let line_second = controller.handle_click((10.0, 0.0), hit_b);
        let line_id = match line_second {
            crate::tool_controller::ClickOutcome::Complete(kind) => {
                sync.perform_edit(|doc, undo_stack| {
                    let core_lib::ToolKind::Line { p1, p2 } = kind else {
                        unreachable!("Line tool always completes with a Line kind")
                    };
                    undo_stack.do_add_line(doc, p1, p2)?;
                    Ok(())
                })
                .unwrap();
                sync.document().history().last().unwrap().id
            }
            other => panic!("expected Complete, got {other:?}"),
        };

        assert!(sync.current_data().get_line(line_id).is_ok()); // the line resolved
        assert!(sync.current_data().get_point(point_ids[0]).is_ok());
        assert!(sync.current_data().get_point(point_ids[1]).is_ok());

        // Undo the line: both points remain, the line is gone.
        assert!(sync.undo().unwrap());
        assert!(sync.current_data().get_line(line_id).is_err());
        assert!(sync.current_data().get_point(point_ids[0]).is_ok());
        assert!(sync.current_data().get_point(point_ids[1]).is_ok());

        // Undo the second point: only the first point remains.
        assert!(sync.undo().unwrap());
        assert!(sync.current_data().get_point(point_ids[0]).is_ok());
        assert!(sync.current_data().get_point(point_ids[1]).is_err());
    }
}
