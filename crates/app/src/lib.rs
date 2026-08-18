pub mod camera; // Camera, pattern-space -> screen-pixel conversion
pub mod sync; // PatternSync, SyncError
pub mod tool_controller; // ToolController, ToolMode, PendingDialog, ClickOutcome, hit_test_point — pure, no egui
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
        tool_controller::PendingDialog::Bisector {
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
    }
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
                if ui.button("Bisector").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::Bisector);
                }
                if ui.button("Height").clicked() {
                    self.tool_controller
                        .select_tool(tool_controller::ToolKindSelector::Height);
                }
            });
        });

        // Translate the current resolved geometry into draw commands once per frame,
        // outside the closure below so a render failure can be handled before any painting starts.
        let draw_result = render::render(self.sync.current_data());

        egui::CentralPanel::default().show(ctx, |ui| {
            let available_size = ui.available_size(); // captured once, BEFORE allocate_painter reserves it below — needed both for the painter itself and for the camera auto-fit that follows
                                                      // Reserve the whole remaining area as both a click target and a paint surface.
            let (response, painter) = ui.allocate_painter(available_size, egui::Sense::click());

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
                tool_controller::ToolMode::Bisector {
                    first: Some(id), ..
                } => Some(*id),
                tool_controller::ToolMode::Height {
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
                tool_controller: tool_controller::ToolController::default(), // no tool selected initially
                active_dialog: None, // no formula dialog open initially
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
}
