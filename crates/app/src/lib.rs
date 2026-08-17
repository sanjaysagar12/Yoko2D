pub mod camera; // Camera, pattern-space -> screen-pixel conversion
pub mod sync; // PatternSync, SyncError
pub mod watch; // WatchEvent, WatchError, WatcherHandle, spawn_watcher

/// The root egui application state.
struct Yoko2DApp {
    sync: sync::PatternSync, // the current Document + its resolved PatternData, kept in sync with the measurement file
    // Kept alive only for its Drop impl (see WatcherHandle's own doc
    // comment): dropping this would silently stop the background watcher
    // thread, so it must live exactly as long as `events` is expected to
    // keep receiving anything. Never read otherwise, hence the leading `_`.
    _watcher_handle: watch::WatcherHandle,
    events: std::sync::mpsc::Receiver<watch::WatchEvent>, // where debounced "measurement file changed" notifications arrive
    camera: camera::Camera, // pattern-space -> screen-pixel conversion state for this window
}

impl eframe::App for Yoko2DApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain every pending watch event non-blockingly. `try_recv()`
        // returns immediately whether or not an event is waiting; the
        // blocking `recv()` used by `run_sync_loop` would freeze this paint
        // callback (and therefore the whole UI thread) until the next file
        // change, which is unacceptable inside `update()`.
        while let Ok(_event) = self.events.try_recv() {
            if let Err(err) = self.sync.resync() {
                // A bad measurement-file edit (e.g. caught mid-save with
                // malformed JSON) must not crash the running app — this
                // mirrors Seamly2D's own qCWarning-and-continue behavior on
                // a sync failure: log it, keep the last-good state, move on.
                eprintln!("yoko2d: resync failed: {err}");
            }
        }

        // Translate the current resolved geometry into draw commands once per frame,
        // outside the closure below so a render failure can be handled before any painting starts.
        let draw_result = render::render(self.sync.current_data());

        egui::CentralPanel::default().show(ctx, |ui| match draw_result {
            Ok(commands) => {
                let painter = ui.painter(); // this frame's immediate-mode drawing handle
                for command in commands {
                    // walk every draw command in the deterministic order render() produced
                    match command {
                        render::DrawCommand::Point { x, y, .. } => {
                            let (screen_x, screen_y) = self.camera.to_screen(x, y); // pattern space -> screen pixels
                            painter.circle_filled(
                                egui::pos2(screen_x, screen_y), // the point's screen position
                                4.0_f32, // a small fixed radius, in pixels, regardless of zoom
                                egui::Color32::WHITE, // plain white fill; styling is out of scope for this phase
                            );
                        }
                        render::DrawCommand::Line { x1, y1, x2, y2 } => {
                            let (screen_x1, screen_y1) = self.camera.to_screen(x1, y1); // first endpoint, converted
                            let (screen_x2, screen_y2) = self.camera.to_screen(x2, y2); // second endpoint, converted
                            painter.line_segment(
                                [
                                    egui::pos2(screen_x1, screen_y1),
                                    egui::pos2(screen_x2, screen_y2),
                                ], // the segment's two screen endpoints
                                egui::Stroke::new(2.0_f32, egui::Color32::WHITE), // a plain 2px white stroke; styling is out of scope for this phase
                            );
                        }
                    }
                }
            }
            Err(err) => {
                // A malformed PatternData (e.g. a dangling Line reference)
                // must not crash the app either: log it and skip drawing
                // this frame, leaving the previous frame's content on
                // screen rather than tearing down the window.
                eprintln!("yoko2d: render failed: {err}");
            }
        });

        // Request another repaint shortly, even with no user input: egui
        // otherwise only repaints in response to input events, so the
        // try_recv() poll above wouldn't run again until the user moved
        // the mouse or typed something. This periodic-poll approach is the
        // deliberately simpler alternative to wiring a cross-thread
        // repaint-request callback from the watcher thread directly (a
        // possible future improvement); 200ms keeps file-edit-to-screen
        // latency low without busy-looping the UI thread.
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// Builds the small demo [`sync::PatternSync`] + watcher this phase's
/// `run()` displays: a `BasePoint` "A" at the origin and an `EndLine` "A1"
/// whose length comes from the bundled `height_scapula` measurement, so
/// editing that measurement's value on disk visibly moves the drawn line
/// while the app is running.
///
/// TODO(later phase): this bakes in a demo document and the bundled fixture
/// file's path — real "open a pattern"/"open a measurement file" UI (out
/// of scope for this phase) will replace this with a user-driven flow.
fn build_demo_sync() -> (
    sync::PatternSync,                            // the constructed sync state
    watch::WatcherHandle, // its accompanying watcher handle, to keep alive alongside it
    std::sync::mpsc::Receiver<watch::WatchEvent>, // the channel that handle's watcher delivers events on
) {
    let mut document = core_lib::Document::default(); // start from an empty pattern document
    let base = document.add_base_point("A", 0.0, 0.0); // a literal starting point at the pattern's origin
    let end = document
        .add_end_line("A1", base, "0", "height_scapula/10") // a point measured along angle 0, at length height_scapula/10, from A
        .expect("base point A was just created above, so this reference is always valid"); // infallible given the line above; documented, not swallowed
    document
        .add_line(base, end) // a visible line segment from A to A1, so there's something to see and watch change on screen
        .expect("both A and A1 were just created above, so these references are always valid"); // infallible given the lines above; documented, not swallowed

    let manifest_dir = env!("CARGO_MANIFEST_DIR"); // crates/app at build time; the fixture lives two levels up from there
    let measurement_path =
        std::path::Path::new(manifest_dir).join("../../fixtures/measurements/sample.json"); // the bundled demo measurement file

    let sync =
        sync::PatternSync::new(document, measurement_path.clone()) // build the sync state, resolving against the fixture immediately
            .expect("bundled demo measurement fixture should always be present and valid"); // a missing/broken bundled fixture is a build-time setup bug, not a runtime condition to recover from

    let (watcher_handle, events) = watch::spawn_watcher(
        measurement_path,
        std::time::Duration::from_millis(300),
    ) // watch that same file for edits, 300ms debounce
    .expect("failed to start file watcher for the bundled demo measurement fixture"); // setup failure here also means the environment is fundamentally broken, not something to gracefully degrade from

    (sync, watcher_handle, events) // hand back everything Yoko2DApp needs to hold onto
}

/// Launches the egui application.
///
/// Moved here from `main.rs` (now just a thin wrapper calling this
/// function) so this crate's logic lives in a library target and is
/// testable — a `main.rs` binary can't be unit tested directly, but code
/// in `lib.rs` can be exercised by `#[cfg(test)]` modules throughout this
/// crate the same way `core`/`io` already are.
pub fn run() -> eframe::Result<()> {
    let (sync, watcher_handle, events) = build_demo_sync(); // set up the demo document, its resolved geometry, and its file watcher
    let app = Yoko2DApp {
        sync,                              // the constructed PatternSync
        _watcher_handle: watcher_handle, // held only to keep the watcher thread alive; see the field's own comment
        events,                          // the channel Yoko2DApp::update polls each frame
        camera: camera::Camera::default(), // a sensible starting pan/zoom (see Camera::default's doc comment)
    };
    let options = eframe::NativeOptions::default(); // default window/backend configuration, unchanged from Phase 0
    eframe::run_native("Yoko2D", options, Box::new(|_cc| Ok(Box::new(app)))) // hand the constructed app to eframe's event loop
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
