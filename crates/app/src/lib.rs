pub mod camera; // Camera, pattern-space -> screen-pixel conversion
pub mod sync; // PatternSync, SyncError
pub mod watch; // WatchEvent, WatchError, WatcherHandle, spawn_watcher

/// The root egui application state.
struct Yoko2DApp {
    sync: sync::PatternSync, // the current Document + its resolved PatternData, kept in sync with the measurement file (if any)
    // `None` when this app was opened via `run_with_document` with no
    // measurement path (see that function) — there is then no file being
    // watched at all, and `events` is correspondingly `None` too. Kept
    // alive only for its Drop impl (see WatcherHandle's own doc comment):
    // dropping this would silently stop the background watcher thread, so
    // it must live exactly as long as `events` is expected to keep
    // receiving anything. Never read otherwise, hence the leading `_`.
    _watcher_handle: Option<watch::WatcherHandle>,
    events: Option<std::sync::mpsc::Receiver<watch::WatchEvent>>, // where debounced "measurement file changed" notifications arrive, if this instance is watching a file at all
    camera: camera::Camera, // pattern-space -> screen-pixel conversion state for this window
    // Starts `false` so the very first frame that has a real canvas size
    // available auto-fits `camera` to whatever geometry was opened (see
    // `camera::fit_to_points`), rather than leaving the fixed
    // `Camera::default()` in place — which only happens to be visible for
    // one narrow range of pattern scales, and was the actual root cause of
    // freshly-opened patterns rendering completely off-screen. Set to
    // `true` right after that one-time fit, so a user's later pan/zoom
    // (once implemented) isn't silently overwritten on every frame.
    camera_fitted: bool,
}

impl eframe::App for Yoko2DApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain every pending watch event non-blockingly. `try_recv()`
        // returns immediately whether or not an event is waiting; the
        // blocking `recv()` used by `run_sync_loop` would freeze this paint
        // callback (and therefore the whole UI thread) until the next file
        // change, which is unacceptable inside `update()`.
        if let Some(events) = &self.events {
            // only an app opened with a real measurement path (see run_with_document) has anything to drain here
            while let Ok(_event) = events.try_recv() {
                if let Err(err) = self.sync.resync() {
                    // A bad measurement-file edit (e.g. caught mid-save with
                    // malformed JSON) must not crash the running app — this
                    // mirrors Seamly2D's own qCWarning-and-continue behavior on
                    // a sync failure: log it, keep the last-good state, move on.
                    eprintln!("yoko2d: resync failed: {err}");
                }
            }
        }

        // Translate the current resolved geometry into draw commands once per frame,
        // outside the closure below so a render failure can be handled before any painting starts.
        let draw_result = render::render(self.sync.current_data());

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_size = ui.available_size(); // captured once, BEFORE allocate_painter reserves it below — needed both for the painter itself and for the camera auto-fit that follows
                                                      // Reserve the whole remaining area as a paint surface only: this view is read-only, so it no longer needs to detect clicks at all — Sense::hover() is the minimal sense needed for the painter to still exist and be paintable into, without listening for interaction this view no longer responds to.
            let (_response, painter) = ui.allocate_painter(available_size, egui::Sense::hover());

            if !self.camera_fitted {
                // One-time auto-fit: collect every currently-resolved point's
                // coordinates (Lines/Pieces reference points that are
                // already counted here, so nothing else needs including)
                // and, if there's anything to fit to yet, replace the
                // placeholder Camera::default() with one that actually
                // centers this pattern in the real, now-known canvas size —
                // this is what fixes freshly-opened geometry rendering
                // completely off-screen. Deliberately re-checked every
                // frame until it succeeds once (rather than only on frame
                // one): a document opened with zero points yet (e.g. a
                // blank pattern) has nothing to fit to until the user
                // places a first point, at which point this still fires.
                let points: Vec<(f64, f64)> = self
                    .sync
                    .current_data()
                    .objects()
                    .filter_map(|(_, object)| match object {
                        core_lib::GeoObject::Point(point) => Some((point.x, point.y)), // only Points carry standalone coordinates; Lines/Pieces reference them, not duplicate them
                        _ => None, // Lines and Pieces contribute no NEW coordinates beyond the Points they reference
                    })
                    .collect(); // materialize before borrowing self mutably below, since current_data() borrows self.sync immutably
                if !points.is_empty() {
                    // only fit (and lock in) once there's actually something real to fit to
                    self.camera =
                        camera::fit_to_points(&points, available_size.x, available_size.y); // center and scale the camera to this pattern's actual bounding box
                    self.camera_fitted = true; // don't keep re-fitting every frame once this has succeeded, so later manual pan/zoom (once implemented) won't be silently overwritten
                }
            }

            match draw_result {
                Ok(commands) => {
                    for command in commands {
                        // walk every draw command in the deterministic order render() produced
                        match command {
                            render::DrawCommand::Point { x, y, .. } => {
                                let (screen_x, screen_y) = self.camera.to_screen(x, y); // pattern space -> screen pixels
                                painter.circle_filled(
                                    egui::pos2(screen_x, screen_y), // the point's screen position
                                    4.0_f32, // a small fixed radius, in pixels, regardless of zoom
                                    // Warm amber/gold rather than plain white: chosen to read clearly
                                    // against the dark theme this window now always forces (see
                                    // run_with_document's set_visuals call), and to be visually
                                    // distinct from the light-gray line color just below, so points
                                    // are easy to pick out at a glance — which also directly helps
                                    // click-to-draw hit-testing feedback (Phase 11), since the user
                                    // needs to clearly see which points exist to click on them.
                                    egui::Color32::from_rgb(255, 200, 0),
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
                                    egui::Stroke::new(
                                        2.0_f32,
                                        // A light, near-white gray rather than exact white: this keeps
                                        // the clean, high-contrast construction-line look against the
                                        // now-guaranteed-dark background, while staying visually
                                        // distinguishable from the amber point markers above. Also
                                        // deliberate: relying on the plain all-caps "pure white" egui
                                        // color constant right next to an equally pure-white point/panel
                                        // color was part of what made this bug's contrast collision easy
                                        // to introduce unnoticed — any near-white color reads fine here
                                        // without repeating that trap.
                                        egui::Color32::from_rgb(220, 220, 220),
                                    ),
                                );
                            }
                            render::DrawCommand::Polyline { points } => {
                                // Curve tessellation (Arc/Spline sampled into straight
                                // segments): drawn as a connected OPEN chain, with no
                                // wrap-around edge back to the first vertex — the
                                // Polygon arm just below is the one that closes the loop.
                                let screen_points: Vec<egui::Pos2> = points
                                    .iter()
                                    .map(|(x, y)| {
                                        let (screen_x, screen_y) = self.camera.to_screen(*x, *y); // pattern space -> screen pixels, per sample
                                        egui::pos2(screen_x, screen_y)
                                    })
                                    .collect();
                                for pair in screen_points.windows(2) {
                                    // draw every consecutive sample pair as one segment; windows(2) naturally skips the (absent) wrap-around edge
                                    painter.line_segment(
                                        [pair[0], pair[1]],
                                        // A distinct green from both the amber points and the
                                        // light-blue piece outlines, so curve tessellation reads
                                        // as its own visual category at a glance.
                                        egui::Stroke::new(
                                            1.5_f32,
                                            egui::Color32::from_rgb(120, 220, 140),
                                        ),
                                    );
                                }
                            }
                            render::DrawCommand::Polygon { points, .. } => {
                                // `filled` is ignored here: this phase always draws an outline only,
                                // matching Part F's own doc comment that this crate makes no styling decisions.
                                // Its existing egui::Color32::LIGHT_BLUE stroke (below) was already
                                // clearly visible before this fix and needs no change here — this fix
                                // targets only the two colors (Point/Line) that actually caused the bug.
                                let screen_points: Vec<egui::Pos2> = points
                                    .iter()
                                    .map(|(x, y)| {
                                        let (screen_x, screen_y) = self.camera.to_screen(*x, *y); // pattern space -> screen pixels, per vertex
                                        egui::pos2(screen_x, screen_y)
                                    })
                                    .collect(); // every vertex converted, in the same order as the polygon's points
                                let vertex_count = screen_points.len(); // needed to wrap the closing edge back to the first vertex
                                for i in 0..vertex_count {
                                    // draw every edge, including the wrap-around edge from the last vertex back to the first
                                    let next_i = (i + 1) % vertex_count; // the next vertex's index, wrapping to 0 after the last
                                    painter.line_segment(
                                        [screen_points[i], screen_points[next_i]], // this edge's two screen endpoints
                                        egui::Stroke::new(1.5_f32, egui::Color32::LIGHT_BLUE), // a distinct stroke from plain construction lines, so piece outlines are visually distinguishable
                                    );
                                }
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

/// Builds the small demo [`core_lib::Document`] this crate's `run()`
/// displays: a `BasePoint` "A" at the origin and an `EndLine` "A1" whose
/// length comes from the bundled `height_scapula` measurement, so editing
/// that measurement's value on disk visibly moves the drawn line while the
/// app is running.
///
/// TODO(later phase): this bakes in a demo document — real "open a
/// pattern" UI (out of scope for this phase) will replace this with a
/// user-driven flow. Splitting this out from the old `build_demo_sync`
/// (which also built the `PatternSync`/watcher) is what lets `run()` and
/// `run_with_document` below share the exact same window/watcher setup
/// code, rather than duplicating it — this function only builds the
/// `Document` itself.
fn build_demo_document() -> core_lib::Document {
    let mut document = core_lib::Document::default(); // start from an empty pattern document
    let base = document.add_base_point("A", 0.0, 0.0); // a literal starting point at the pattern's origin
    let end = document
        .add_end_line("A1", base, "0", "height_scapula/10") // a point measured along angle 0, at length height_scapula/10, from A
        .expect("base point A was just created above, so this reference is always valid"); // infallible given the line above; documented, not swallowed
    document
        .add_line(base, end) // a visible line segment from A to A1, so there's something to see and watch change on screen
        .expect("both A and A1 were just created above, so these references are always valid"); // infallible given the lines above; documented, not swallowed
    document // hand back the fully-built demo document
}

/// The bundled demo measurement fixture's path, resolved relative to this
/// crate's own manifest directory so it works regardless of the process's
/// current working directory.
fn demo_measurements_path() -> std::path::PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR"); // crates/app at build time; the fixture lives two levels up from there
    std::path::Path::new(manifest_dir).join("../../fixtures/measurements/sample.json")
    // the bundled demo measurement file
}

/// Launches the egui application showing this crate's own hardcoded demo
/// pattern.
///
/// Moved here from `main.rs` (now just a thin wrapper calling this
/// function) so this crate's logic lives in a library target and is
/// testable — a `main.rs` binary can't be unit tested directly, but code
/// in `lib.rs` can be exercised by `#[cfg(test)]` modules throughout this
/// crate the same way `core`/`io` already are. A thin wrapper itself now,
/// over [`run_with_document`] — see that function for the actual window/
/// event-loop setup, shared with the CLI's "generate then open in the GUI"
/// flow.
pub fn run() -> eframe::Result<()> {
    run_with_document(build_demo_document(), Some(demo_measurements_path())) // the demo document, watching its bundled measurement fixture
}

/// Launches the egui application showing `document`.
///
/// `measurements_path`, if given, is both applied to `document` (via
/// [`sync::PatternSync::new`]) and watched for live changes for the
/// lifetime of the window, exactly like [`run`]'s own demo pattern. If
/// `None` (e.g. a CLI-generated pattern whose action script had no
/// `measurements_path`), the opened window has no measurement file to
/// watch at all — `document`'s variables are used exactly as the caller
/// already set them up (see [`sync::PatternSync::new_without_measurements`]).
///
/// This is the shared entry point both [`run`] (the bundled demo) and the
/// `cli` crate's `run --open` flow use, so the actual window/event-loop
/// setup exists in exactly one place rather than being duplicated between
/// them.
pub fn run_with_document(
    document: core_lib::Document, // the pattern to open, already fully built (e.g. by an action-script executor, or build_demo_document above)
    measurements_path: Option<std::path::PathBuf>, // an optional measurement file to apply and keep watching
) -> eframe::Result<()> {
    let options = eframe::NativeOptions::default(); // default window/backend configuration, unchanged from Phase 0
    eframe::run_native(
        "Yoko2D",
        options,
        // The actual PatternSync/watcher setup happens INSIDE this
        // closure, not before calling run_native: eframe's AppCreator
        // returns `Result<Box<dyn App>, Box<dyn Error + Send + Sync>>`,
        // so any setup failure here (a malformed measurement file, a
        // watcher that fails to register) can be reported through
        // eframe's own native error path — surfacing as this function's
        // own `Err(eframe::Error::AppCreation(..))` — instead of needing
        // to panic or invent a separate error-reporting mechanism.
        Box::new(move |cc| {
            // Force a known, consistent theme rather than inheriting
            // whatever the OS/system theme happens to be: egui's
            // `theme_preference` defaults to `ThemePreference::System`, and
            // this is exactly the class of bug that caused geometry to
            // render invisibly — a color hardcoded to look good on one
            // theme silently vanishes the moment the app runs on a system
            // set to the opposite theme. Dark mode is chosen specifically
            // because it gives the most reliable contrast against the
            // light/bright geometry colors below, and matches the visual
            // style most CAD-style tools use.
            //
            // `Context::set_visuals` alone (an earlier attempt at this same
            // fix) does NOT actually force the theme: it only mutates the
            // `Style` for whichever theme slot `Context::theme()` resolves
            // to AT THE MOMENT IT'S CALLED. Since `theme_preference` is
            // still `System` at that point, `theme()` still tracks the
            // real OS theme on every later frame (once winit reports it),
            // so a `set_visuals` call made against, say, the dark slot has
            // no visible effect once the context switches to displaying
            // the light slot instead — which is exactly why the previous
            // attempt at this fix had no visible effect on a light-themed
            // system. `Context::set_theme` is the actual fix: it sets
            // `theme_preference` itself, so `theme()` always resolves to
            // `Dark` regardless of what the OS reports, and egui's own
            // built-in dark `Style` (already sensible) is what gets used.
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            let (sync, watcher_handle, events): (
                sync::PatternSync,
                Option<watch::WatcherHandle>,
                Option<std::sync::mpsc::Receiver<watch::WatchEvent>>,
            ) =
                match measurements_path {
                    Some(path) => {
                        let sync = sync::PatternSync::new(document, path.clone()) // build the sync state, resolving against the given file immediately
                            .map_err(|err| {
                                Box::new(err) as Box<dyn std::error::Error + Send + Sync>
                            })?; // lift SyncError into the boxed error AppCreator expects
                        let (watcher_handle, events) =
                            watch::spawn_watcher(path, std::time::Duration::from_millis(300)) // watch that same file for edits, 300ms debounce
                                .map_err(|err| {
                                    Box::new(err) as Box<dyn std::error::Error + Send + Sync>
                                })?; // lift WatchError the same way
                        (sync, Some(watcher_handle), Some(events)) // this instance IS watching a file
                    }
                    None => {
                        let sync = sync::PatternSync::new_without_measurements(document) // no file to apply/watch; use document's variables exactly as given
                            .map_err(|err| {
                                Box::new(err) as Box<dyn std::error::Error + Send + Sync>
                            })?; // lift SyncError the same way
                        (sync, None, None) // this instance has nothing to watch
                    }
                };
            let app = Yoko2DApp {
                sync, // the constructed PatternSync, watching a file or not per the match above
                _watcher_handle: watcher_handle, // held only to keep the watcher thread alive, if there is one; see the field's own comment
                events, // the channel Yoko2DApp::update polls each frame, if there is one
                camera: camera::Camera::default(), // a placeholder starting pan/zoom, immediately replaced by an auto-fit on the first frame with a real canvas size (see camera_fitted's own comment)
                camera_fitted: false, // triggers the one-time auto-fit-to-geometry in update()'s first frame
            };
            Ok(Box::new(app) as Box<dyn eframe::App>) // hand the constructed app back to eframe's event loop
        }),
    )
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

    // This project has no pixel-level GUI testing infrastructure — nothing
    // here can render a frame and inspect actual pixel colors — so a real
    // "is this point visible against the background" test isn't possible.
    // Instead, this asserts at the SOURCE level that the exact regression
    // that caused this bug (Point/Line geometry hardcoded to a plain-white
    // egui color constant, invisible against a light theme) can't be
    // silently reintroduced: it reads this very file's own source text and
    // checks that the offending constant's full name is absent. This is a
    // narrow, pragmatic guard, not a general-purpose styling test — it
    // exists specifically because plain white was what broke visibility
    // here, not because white is inherently disallowed. A genuinely new,
    // different reason to draw something in white in the future (a
    // deliberate design choice, not a hardcoded-color-vs-theme mistake)
    // would need to update this test alongside the code — that update
    // friction is the intended point, forcing a conscious decision rather
    // than an accidental regression.
    //
    // The needle below is deliberately built from two concatenated halves,
    // and this explanatory comment deliberately never spells it out as one
    // contiguous token: `include_str!` below reads this ENTIRE file,
    // including this test's own source, so writing the offending name as
    // one unbroken literal anywhere in this file — even here, in a comment
    // about it — would make the file always contain it and this assertion
    // would trivially (and uselessly) always fail.
    #[test]
    fn point_and_line_rendering_never_hardcodes_the_plain_white_color_constant_again() {
        let source = include_str!("lib.rs"); // this file's own source text, read at test time (not the compiled binary)
        let forbidden_color_constant = format!("Color32::{}", "WHITE"); // built from two pieces so this file never contains the full name as one contiguous literal (see the comment above)
        assert!(
            !source.contains(&forbidden_color_constant), // the exact constant that caused points/lines to vanish against a light theme
            "found a hardcoded plain-white egui color constant in lib.rs — this is the exact \
             contrast bug that made pattern geometry invisible on light-themed systems; use an \
             explicit, theme-independent color instead (see the Point/Line draw calls in update())"
        );
    }

    // This project's design is now CLI-only construction/editing (via the
    // CLI's own action-script executor and pattern-XML load/modify
    // support) with this crate's GUI reduced to a READ-ONLY VIEWER — no
    // toolbar, no click handling, no tool selection, no formula dialogs,
    // no undo/redo. Same source-level-guard technique as the color-
    // constant regression test above, for the same reason (no pixel-level
    // GUI testing infrastructure exists here): this reads lib.rs's own
    // source and asserts neither the removed tool-selection module's name
    // nor its former toolbar-button-selection enum name has been
    // reintroduced, catching an accidental regression back toward
    // interactive editing in the GUI layer before it ships.
    //
    // Deliberately never spells out either forbidden name anywhere in this
    // comment or the assertions below, including in the panic messages:
    // `include_str!` reads this ENTIRE file, including this test's own
    // source, so writing either name as one unbroken literal ANYWHERE in
    // this file — even in a comment or a failure message — would make the
    // file always contain it and these assertions would trivially (and
    // uselessly) always fail. Each needle is instead built at runtime from
    // two concatenated halves, same trick as the color-constant guard.
    #[test]
    fn interactive_editing_is_never_reintroduced_into_the_gui_layer() {
        let source = include_str!("lib.rs"); // this file's own source text, read at test time (not the compiled binary)
        let forbidden_module = format!("tool_{}", "controller"); // the removed pure tool-selection state-machine module's name, built from two pieces so this file never contains it as one contiguous literal
        let forbidden_selector = format!("ToolKind{}", "Selector"); // the removed toolbar-button-selection enum's name, same rationale
        assert!(
            !source.contains(&forbidden_module),
            "found a reference to a removed GUI editing module in lib.rs — this project's GUI \
             is a read-only viewer; interactive click-to-draw editing must not be reintroduced \
             here (construction/editing belongs exclusively in the CLI's action-script executor)"
        );
        assert!(
            !source.contains(&forbidden_selector),
            "found a reference to a removed GUI tool-selection type in lib.rs — this project's \
             GUI is a read-only viewer; interactive click-to-draw editing must not be \
             reintroduced here (construction/editing belongs exclusively in the CLI's \
             action-script executor)"
        );
    }
}
