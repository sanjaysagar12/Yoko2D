pub mod camera; // Camera, pattern-space -> screen-pixel conversion
pub mod sync; // PatternSync, SyncError
pub mod tool_controller; // ToolController, ToolMode, PendingDialog, ClickOutcome, hit_test_point — pure, no egui
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
    tool_controller: tool_controller::ToolController, // which construction tool is selected and its in-progress clicks (Phase 11)
    active_dialog: Option<tool_controller::PendingDialog>, // a formula dialog currently open, if any (Phase 11)
}

/// Commits `kind` to `sync` via the matching `UndoStack::do_add_*` method,
/// dispatching on which `ToolKind` variant it is.
///
/// Shared by both the immediate-completion click path (`Complete(kind)`)
/// and the dialog-confirm path (`finish_dialog` succeeding), since both
/// eventually need to turn a resolved `ToolKind` into an actual `Document`
/// edit the same way.
fn commit_tool_kind(sync: &mut sync::PatternSync, kind: core_lib::ToolKind) {
    let result = sync.perform_edit(|doc, undo_stack| {
        // clone-then-commit: perform_edit only applies this closure's edits to `sync` if recompute succeeds afterward
        match kind {
            // dispatch on which kind of tool this is, calling the matching UndoStack::do_add_* method
            core_lib::ToolKind::BasePoint { name, x, y } => {
                undo_stack.do_add_base_point(doc, name, x, y); // infallible: BasePoint has no references to validate
                Ok(())
            }
            core_lib::ToolKind::Line { p1, p2 } => {
                undo_stack.do_add_line(doc, p1, p2)?; // propagate a validation failure, if any, via ?
                Ok(())
            }
            core_lib::ToolKind::Midpoint { name, p1, p2 } => {
                undo_stack.do_add_midpoint(doc, name, p1, p2)?; // propagate a validation failure, if any
                Ok(())
            }
            core_lib::ToolKind::EndLine {
                name,
                base_point,
                angle_formula,
                length_formula,
            } => {
                undo_stack.do_add_end_line(doc, name, base_point, angle_formula, length_formula)?; // propagate a validation failure, if any
                Ok(())
            }
            core_lib::ToolKind::AlongLine {
                name,
                p1,
                p2,
                length_formula,
            } => {
                undo_stack.do_add_along_line(doc, name, p1, p2, length_formula)?; // propagate a validation failure, if any
                Ok(())
            }
            core_lib::ToolKind::Normal {
                name,
                p1,
                p2,
                length_formula,
                angle_formula,
            } => {
                undo_stack.do_add_normal(doc, name, p1, p2, length_formula, angle_formula)?; // propagate a validation failure, if any
                Ok(())
            }
            // Bisector/Height aren't reachable through this phase's toolbar (no
            // ToolKindSelector variant creates them), but ToolKind is a shared
            // type with more variants than this phase's UI produces — handled
            // here anyway rather than leaving this match non-exhaustive.
            core_lib::ToolKind::Bisector {
                name,
                p1,
                p2,
                p3,
                length_formula,
            } => {
                undo_stack.do_add_bisector(doc, name, p1, p2, p3, length_formula)?; // propagate a validation failure, if any
                Ok(())
            }
            core_lib::ToolKind::Height {
                name,
                point,
                line_p1,
                line_p2,
            } => {
                undo_stack.do_add_height(doc, name, point, line_p1, line_p2)?; // propagate a validation failure, if any
                Ok(())
            }
            // Piece (Phase 12) has no toolbar entry point in this phase — no
            // ToolKindSelector variant, and no click-to-draw state machine
            // support — and `UndoStack` has no `do_add_piece` yet either;
            // building pieces interactively is explicitly out of scope for
            // Phase 12 (see its own scope note). Handled here anyway, as a
            // no-op, purely so this match stays exhaustive against the
            // shared `ToolKind` type rather than silently failing to build.
            core_lib::ToolKind::Piece { .. } => Ok(()),
        }
    });
    if let Err(err) = result {
        // A failed edit at this point should not be possible if hit-testing/
        // state-machine logic above is correct, but defensive error handling
        // here prevents a UI crash if it somehow is.
        eprintln!("yoko2d: failed to commit tool: {err}");
    }
}

/// Draws one live-validated formula text field with a colored ok/error
/// indicator, returning whether the current text evaluates successfully
/// against `vars`.
///
/// Shared by every formula field in the dialog UI, so the "type text,
/// evaluate every frame, show green/red" behavior — matching the original
/// app's `EditFormulaDialog` — isn't duplicated per field.
fn formula_field(
    ui: &mut egui::Ui, // the dialog window's Ui to draw into
    label: &str,       // this field's caption, e.g. "Length:"
    text: &mut String, // the editable formula text itself
    vars: &std::collections::HashMap<String, f64>, // the live variable table to validate against, this frame
) -> bool {
    let is_ok = core_lib::formula::eval_formula(text, vars).is_ok(); // live-evaluate the current text every frame
    ui.horizontal(|ui| {
        ui.label(label); // the field's caption
        ui.text_edit_singleline(text); // the editable formula text
        if is_ok {
            ui.colored_label(egui::Color32::GREEN, "\u{2713}"); // green checkmark: currently evaluates successfully
        } else {
            ui.colored_label(egui::Color32::RED, "\u{2717}"); // red X: does not currently evaluate
        }
    });
    is_ok // hand back whether this field currently validates, so the caller can gate the OK button on ALL fields validating
}

/// Draws the name field plus every formula field for whichever
/// `PendingDialog` variant `dialog` currently is, returning whether the
/// dialog is ready to commit (a non-empty name AND every formula field
/// currently valid).
fn render_dialog_fields(
    ui: &mut egui::Ui,
    dialog: &mut tool_controller::PendingDialog,
    vars: &std::collections::HashMap<String, f64>,
) -> bool {
    match dialog {
        // dispatch on which dialog variant this is, drawing exactly the fields it has
        tool_controller::PendingDialog::EndLine {
            name,
            angle_formula,
            length_formula,
            ..
        } => {
            ui.horizontal(|ui| {
                ui.label("Name:"); // caption for the name field
                ui.text_edit_singleline(name); // editable name text
            });
            let angle_ok = formula_field(ui, "Angle:", angle_formula, vars); // draws the field, returns whether it currently validates
            let length_ok = formula_field(ui, "Length:", length_formula, vars); // same, for the length field
            !name.is_empty() && angle_ok && length_ok // ready to commit only if every check passes
        }
        tool_controller::PendingDialog::AlongLine {
            name,
            length_formula,
            ..
        } => {
            ui.horizontal(|ui| {
                ui.label("Name:"); // caption for the name field
                ui.text_edit_singleline(name); // editable name text
            });
            let length_ok = formula_field(ui, "Length:", length_formula, vars); // draws the field, returns whether it currently validates
            !name.is_empty() && length_ok // ready to commit only if every check passes
        }
        tool_controller::PendingDialog::Normal {
            name,
            length_formula,
            angle_formula,
            ..
        } => {
            ui.horizontal(|ui| {
                ui.label("Name:"); // caption for the name field
                ui.text_edit_singleline(name); // editable name text
            });
            let length_ok = formula_field(ui, "Length:", length_formula, vars); // draws the field, returns whether it currently validates
            let angle_ok = formula_field(ui, "Angle:", angle_formula, vars); // same, for the angle field
            !name.is_empty() && length_ok && angle_ok // ready to commit only if every check passes
        }
    }
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

        // Read every keyboard shortcut once per frame, via egui's
        // closure-based input API (the InputState is behind a lock).
        let (escape_pressed, undo_pressed, redo_pressed) = ctx.input(|i| {
            let escape = i.key_pressed(egui::Key::Escape); // cancels the current tool / closes any open dialog
            let undo = i.modifiers.ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z); // Ctrl+Z, without Shift (Shift+Ctrl+Z means redo instead)
            let redo = (i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) // Ctrl+Y ...
                || (i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::Z)); // ...or Ctrl+Shift+Z
            (escape, undo, redo) // hand all three booleans back out of the locked closure at once
        });

        if escape_pressed {
            self.tool_controller.cancel(); // reset the current tool's in-progress clicks (does NOT deselect the tool)
            self.active_dialog = None; // also close any open formula dialog, discarding it without committing
        }
        if undo_pressed {
            if let Err(err) = self.sync.undo() {
                // self.sync.undo() above already reverted the last recorded edit, if any existed
                eprintln!("yoko2d: undo failed: {err}"); // don't crash the UI over a failed undo
            }
        }
        if redo_pressed {
            if let Err(err) = self.sync.redo() {
                // self.sync.redo() above already re-applied the last undone edit, if any existed
                eprintln!("yoko2d: redo failed: {err}"); // don't crash the UI over a failed redo
            }
        }

        // Toolbar: one button per ToolKindSelector variant, selecting that tool on click.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Base Point").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::BasePoint);
                }
                if ui.button("Line").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::Line);
                }
                if ui.button("Midpoint").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::Midpoint);
                }
                if ui.button("End Line").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::EndLine);
                }
                if ui.button("Along Line").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::AlongLine);
                }
                if ui.button("Normal").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::Normal);
                }
            });
        });

        // Translate the current resolved geometry into draw commands once per frame,
        // outside the closure below so a render failure can be handled before any painting starts.
        let draw_result = render::render(self.sync.current_data());

        egui::CentralPanel::default().show(ctx, |ui| {
            // Reserve the whole remaining area as both a click target and a paint surface.
            let (response, painter) =
                ui.allocate_painter(ui.available_size(), egui::Sense::click());

            if response.clicked() {
                if let Some(screen_pos) = response.interact_pointer_pos() {
                    let pattern_pos = self.camera.to_pattern(screen_pos.x, screen_pos.y); // screen pixels -> pattern space
                                                                                          // Convert a fixed 8px screen radius into pattern-space units via the
                                                                                          // current zoom, so hit-testing stays accurate regardless of zoom level.
                    let tolerance = 8.0 / self.camera.zoom;
                    let hit = tool_controller::hit_test_point(
                        self.sync.current_data(),
                        pattern_pos,
                        tolerance,
                    ); // find the closest existing point, if any, within tolerance
                    let outcome = self.tool_controller.handle_click(pattern_pos, hit); // advance the tool state machine
                    match outcome {
                        tool_controller::ClickOutcome::NeedMoreInput
                        | tool_controller::ClickOutcome::Ignored => {
                            // stay in the current tool and wait; nothing further to do this frame
                        }
                        tool_controller::ClickOutcome::Complete(kind) => {
                            commit_tool_kind(&mut self.sync, kind); // commit this tool to the document immediately
                        }
                        tool_controller::ClickOutcome::OpenDialog(pending) => {
                            self.active_dialog = Some(pending); // show the formula dialog next frame
                        }
                    }
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
                            render::DrawCommand::Polygon { points, .. } => {
                                // `filled` is ignored here: this phase always draws an outline only,
                                // matching Part F's own doc comment that this crate makes no styling decisions
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

            // Live "rubber band" preview: while a multi-click tool has its
            // first point already collected, draw a faint line from that
            // point's current screen position to the mouse cursor — purely
            // visual, matching the original app's VisLine-style live
            // preview, and never committed to the Document.
            let in_progress_first = match self.tool_controller.current_mode() {
                tool_controller::ToolMode::Line { first: Some(id) } => Some(*id),
                tool_controller::ToolMode::Midpoint { first: Some(id) } => Some(*id),
                tool_controller::ToolMode::AlongLine {
                    first: Some(id), ..
                } => Some(*id),
                tool_controller::ToolMode::Normal {
                    first: Some(id), ..
                } => Some(*id),
                _ => None, // no tool, or a tool with no first click collected yet: nothing to preview
            };
            if let Some(first_id) = in_progress_first {
                if let Ok(point) = self.sync.current_data().get_point(first_id) {
                    if let Some(cursor_pos) = ui.input(|i| i.pointer.hover_pos()) {
                        let (start_x, start_y) = self.camera.to_screen(point.x, point.y); // the collected first point's screen position
                        painter.line_segment(
                            [egui::pos2(start_x, start_y), cursor_pos], // from the first point to wherever the mouse currently is
                            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(128)), // a faint gray preview stroke, distinct from committed geometry's white
                        );
                    }
                }
            }
        });

        // Formula dialog, if one is open. `.take()`s the Option out of
        // self first, so the rest of this block is fully decoupled from
        // self.active_dialog's borrow and can freely touch self.sync/
        // self.tool_controller without any borrow-checker conflict.
        if let Some(mut dialog) = self.active_dialog.take() {
            let vars = core_lib::formula::flatten_variables(self.sync.current_data()); // live variable table for validating formula fields this frame
            let mut keep_open = true; // whether the dialog should remain open after this frame
            let mut confirmed_kind: Option<core_lib::ToolKind> = None; // set below if OK is pressed and finish_dialog succeeds

            egui::Window::new("Tool Formula")
                .collapsible(false) // a small fixed dialog, not a general-purpose panel
                .resizable(false)
                .show(ctx, |ui| {
                    let all_valid = render_dialog_fields(ui, &mut dialog, &vars); // draw the fields, get whether they're all currently valid

                    ui.horizontal(|ui| {
                        if ui.add_enabled(all_valid, egui::Button::new("OK")).clicked() {
                            match self.tool_controller.finish_dialog(dialog.clone()) {
                                Ok(kind) => {
                                    confirmed_kind = Some(kind); // commit after this closure returns, not from inside it
                                    keep_open = false;
                                }
                                Err(err) => {
                                    // Shouldn't happen given `all_valid` already gates OK on a
                                    // non-empty name, but handled rather than silently ignored.
                                    eprintln!("yoko2d: {err}");
                                }
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            keep_open = false; // discard without committing anything
                        }
                    });
                });

            if let Some(kind) = confirmed_kind {
                commit_tool_kind(&mut self.sync, kind); // safe now: the window closure above (which borrowed self.tool_controller) has already returned
            }
            if keep_open {
                self.active_dialog = Some(dialog); // put it back so the same dialog keeps showing next frame
            }
            // else: leave self.active_dialog as None (already set by .take() above), closing the dialog
        }

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
        sync,                                                        // the constructed PatternSync
        _watcher_handle: watcher_handle, // held only to keep the watcher thread alive; see the field's own comment
        events,                          // the channel Yoko2DApp::update polls each frame
        camera: camera::Camera::default(), // a sensible starting pan/zoom (see Camera::default's doc comment)
        tool_controller: tool_controller::ToolController::default(), // no tool selected initially
        active_dialog: None,               // no formula dialog open initially
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
