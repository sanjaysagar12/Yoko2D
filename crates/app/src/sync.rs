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
    // `None` means this PatternSync has no measurement file to track at
    // all (see `Self::new_without_measurements`) — `resync` then simply
    // recomputes against whatever variables `document` already carries,
    // rather than trying to read a file that doesn't exist for this
    // instance.
    measurement_path: Option<PathBuf>, // where resync reads measurements from each time it's called, if anywhere
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
            measurement_path: Some(measurement_path), // the caller-supplied measurement file path, tracked from now on
        };
        sync.resync()?; // populate current_data (and re-apply measurements to document) right away; propagate any failure
        Ok(sync) // resync succeeded: the returned PatternSync is fully in sync with the file
    }

    /// Builds a `PatternSync` for `document` with NO measurement file to
    /// track — `document`'s variables (e.g. baked in by an action-script
    /// executor via `apply_measurements`, before this is ever called) are
    /// used exactly as given, and `resync`/the file watcher have nothing
    /// to reload from for the lifetime of this instance.
    ///
    /// Exists for `app::run_with_document`'s "open a Document that has no
    /// associated measurement file" case — the CLI-generated-pattern path
    /// this crate's `run_with_document` supports, where an action script
    /// may legitimately have no `measurements_path` at all.
    pub fn new_without_measurements(document: core_lib::Document) -> Result<Self, SyncError> {
        let mut sync = PatternSync {
            document, // the caller-supplied starting document, variables already however the caller wants them
            current_data: core_lib::PatternData::default(), // placeholder: replaced by the resync call below before this returns
            measurement_path: None,                         // no file to track for this instance
        };
        sync.resync()?; // populate current_data via a single recompute; no measurement file to reload from, so this can only fail if `document` itself is invalid
        Ok(sync) // resync succeeded: the returned PatternSync reflects `document` exactly as given
    }

    /// Reloads measurements from `measurement_path` (if this `PatternSync`
    /// has one) and recomputes the pattern's geometry, replacing
    /// `document`/`current_data` only if every step succeeds.
    pub fn resync(&mut self) -> Result<(), SyncError> {
        // Work on a clone of the document, not `self.document` directly, so
        // that if a later step (recompute) fails, `self.document` is never
        // left partially updated — `self` is only touched once every
        // fallible step below has already succeeded.
        let mut candidate_document = self.document.clone(); // clone: self.document stays untouched until the very end

        if let Some(path) = &self.measurement_path {
            // Load first, before mutating the candidate at all: if this
            // fails, `self` must remain exactly as it was — an
            // unreadable/malformed file on a later resync should never
            // corrupt a previously-good in-memory state.
            let measurements = io::load_measurements_from_file(path)?;
            candidate_document.apply_measurements(measurements); // infallible: just replaces/sets variables on the clone
        }
        // else: no measurement file tracked for this instance; recompute
        // below uses whatever variables `candidate_document` already has.

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
    fn new_without_measurements_resolves_geometry_from_the_documents_own_variables() {
        let mut doc = core_lib::Document::default();
        doc.set_variable(
            "height_scapula",
            core_lib::Variable::Measurement { value: 40.0 },
        ); // baked directly into the Document, as an action-script executor would do, with no measurement file involved at all
        let a = doc.add_base_point("A", 0.0, 0.0);
        let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();

        let sync = PatternSync::new_without_measurements(doc).unwrap();
        let point = sync.current_data().get_point(a1).unwrap();
        assert!((point.x - 4.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn resync_on_a_sync_without_measurements_is_a_no_op_reload_but_still_recomputes() {
        let mut doc = core_lib::Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 1.0, 1.0);
        let mut sync = PatternSync::new_without_measurements(doc).unwrap();

        // No measurement file to fail to read, and no variables changed:
        // resync should simply succeed and leave the same two points resolved.
        sync.resync().unwrap();
        assert!(sync.current_data().get_point(a).is_ok());
        assert!(sync.current_data().get_point(b).is_ok());
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
}
