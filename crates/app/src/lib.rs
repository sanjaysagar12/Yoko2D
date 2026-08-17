pub mod sync; // PatternSync, SyncError
pub mod watch; // WatchEvent, WatchError, WatcherHandle, spawn_watcher

/// The root egui application state. Holds nothing yet beyond what's needed
/// to draw the Phase 0 placeholder window; a later phase will grow this to
/// hold a [`sync::PatternSync`] and drive the canvas from it.
struct Yoko2DApp;

impl eframe::App for Yoko2DApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Hello Yoko2D — Phase 0 scaffold"); // unchanged placeholder text carried over from Phase 0
        });
    }
}

/// Launches the egui application.
///
/// Moved here from `main.rs` (now just a thin wrapper calling this
/// function) so this crate's logic lives in a library target and is
/// testable — a `main.rs` binary can't be unit tested directly, but code
/// in `lib.rs` can be exercised by `#[cfg(test)]` modules throughout this
/// crate the same way `core`/`io` already are.
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default(); // default window/backend configuration, unchanged from Phase 0
    eframe::run_native("Yoko2D", options, Box::new(|_cc| Ok(Box::new(Yoko2DApp))))
    // hand the app struct to eframe's event loop
}

/// Blocks on `events`, calling `sync.resync()` once per received
/// [`watch::WatchEvent`], until the channel closes (every `Sender`
/// dropped), then returns `Ok(())`.
///
/// TODO(later phase): a successful resync doesn't request an egui repaint
/// here — wiring that up needs an `egui::Context`/frame handle threaded
/// through, which belongs with the actual canvas-drawing work this phase
/// explicitly excludes.
///
/// "Stop on first error" below is a deliberate placeholder policy, not the
/// final behavior: real UI wiring in a later phase will likely want to log
/// a failed resync (e.g. "measurement file has invalid JSON right now")
/// and keep watching, rather than tear down the whole sync loop over one
/// bad edit a user might fix seconds later. That policy call belongs with
/// the UI/error-reporting work, not here.
pub fn run_sync_loop(
    sync: &mut sync::PatternSync, // the PatternSync each received event triggers a resync on
    events: std::sync::mpsc::Receiver<watch::WatchEvent>, // the channel watch events arrive on
) -> Result<(), sync::SyncError> {
    for _event in events {
        // iterating a Receiver blocks on recv() each pass and ends the loop once the channel closes
        sync.resync()?; // propagate the first resync failure; see the TODO above re: this placeholder policy
    }
    Ok(()) // the channel closed (every Sender dropped) with no error: a clean, expected shutdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn run_sync_loop_resyncs_on_each_event_and_exits_when_channel_closes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("measurements.json");
        std::fs::write(
            &path,
            r#"{"measurements":[{"name":"height_scapula","value":40.0}]}"#,
        )
        .unwrap();

        let mut doc = core_lib::Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let a1 = doc.add_end_line("A1", a, "0", "height_scapula/10").unwrap();

        let mut sync = sync::PatternSync::new(doc, path.clone()).unwrap();

        // Write the file's final state BEFORE sending any events: resync
        // reads whatever is on disk right now, not anything carried in the
        // event payload itself.
        std::fs::write(
            &path,
            r#"{"measurements":[{"name":"height_scapula","value":80.0}]}"#,
        )
        .unwrap();

        let (sender, receiver) = channel();
        sender
            .send(watch::WatchEvent { path: path.clone() })
            .unwrap();
        sender
            .send(watch::WatchEvent { path: path.clone() })
            .unwrap();
        drop(sender); // close the channel so run_sync_loop's for-loop ends

        let result = run_sync_loop(&mut sync, receiver);
        assert!(result.is_ok());

        let point = sync.current_data().get_point(a1).unwrap();
        assert!((point.x - 8.0).abs() < 1e-9);
    }
}
