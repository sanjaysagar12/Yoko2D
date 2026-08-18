// NO egui/eframe imports in this file, by design: this module is the pure
// tool-selection state machine and hit-testing logic, unit-tested with
// synthetic (pattern_pos, hit_point) inputs without ever opening a window.
// Only `lib.rs`'s UI wiring (buttons, painting, dialog windows) depends on
// egui; that glue code calls into this module rather than duplicating any
// of its logic.
//
// Note: this crate's dependency on the `core` package is aliased to
// `core_lib` (see `Cargo.toml`'s comment on that dependency, and
// `sync.rs`, which already follows the same convention) — every `core_lib::`
// reference below is what the original design's `core::` naming refers to.

/// Which construction tool is currently selected, and how many of its
/// required point-clicks have been collected so far.
#[derive(Debug, Clone, PartialEq)] // Debug: printable in test failures; Clone: handle_click clones the mode to avoid self-borrow conflicts; PartialEq: enables assert_eq! in tests
pub enum ToolMode {
    /// No tool selected: clicks do nothing.
    Idle,

    /// Needs exactly one click, on ANY position (not necessarily an
    /// existing point); commits immediately — no formula dialog, since a
    /// literal starting point has no formulas.
    BasePoint,

    /// Needs exactly two clicks, both on EXISTING points; commits
    /// immediately once both are collected — no formula dialog, since a
    /// straight line has no formulas.
    Line {
        first: Option<core_lib::ObjectId>, // the first endpoint, once clicked; None until then
    },

    /// Needs exactly two clicks, both on EXISTING points; commits
    /// immediately once both are collected — no formula dialog, since a
    /// midpoint has no formulas.
    Midpoint {
        first: Option<core_lib::ObjectId>, // the first endpoint, once clicked; None until then
    },

    /// Needs exactly one click, on an EXISTING point; then opens a dialog
    /// to collect the angle/length formulas before it can commit.
    EndLine {
        base: Option<core_lib::ObjectId>, // the base point, once clicked; None until then
    },

    /// Needs exactly two clicks, both on EXISTING points; then opens a
    /// dialog to collect the length formula before it can commit.
    AlongLine {
        first: Option<core_lib::ObjectId>, // the point measured from, once clicked; None until then
        second: Option<core_lib::ObjectId>, // the point defining the direction, once clicked; None until then
    },

    /// Needs exactly two clicks, both on EXISTING points; then opens a
    /// dialog to collect the length AND angle formulas before it can
    /// commit.
    Normal {
        first: Option<core_lib::ObjectId>, // the point measured from, once clicked; None until then
        second: Option<core_lib::ObjectId>, // the point defining the perpendicular line, once clicked; None until then
    },

    /// Needs exactly three clicks, all on EXISTING points; then opens a
    /// dialog to collect the length formula before it can commit. Only
    /// the first two clicks need a slot here — the third completes the
    /// sequence directly, the same convention `AlongLine`/`Normal` above
    /// already follow.
    Bisector {
        first: Option<core_lib::ObjectId>, // p1, one angle ray, once clicked; None until then
        second: Option<core_lib::ObjectId>, // p2, the angle's vertex, once clicked; None until then
    },

    /// Needs exactly three clicks, all on EXISTING points; commits
    /// immediately once all three are collected — no formula dialog,
    /// since `Height` is pure geometry (a perpendicular projection) with
    /// no formula fields at all.
    Height {
        first: Option<core_lib::ObjectId>, // `point` (the point being projected), once clicked; None until then
        second: Option<core_lib::ObjectId>, // `line_p1` (one point defining the line), once clicked; None until then
    },
}

/// A formula-collecting dialog waiting for the user to fill in name/
/// formula text and press OK (or Cancel).
///
/// Every referenced point id is fixed the moment the dialog opens (it came
/// from a click already made); only the `String` fields are still being
/// edited by the user.
#[derive(Debug, Clone, PartialEq)] // same rationale as ToolMode's derives above
pub enum PendingDialog {
    EndLine {
        base: core_lib::ObjectId, // fixed once the dialog opens
        name: String,             // starts empty; filled in by the user before OK is pressable
        angle_formula: String,    // starts empty; filled in by the user before OK is pressable
        length_formula: String,   // starts empty; filled in by the user before OK is pressable
    },
    AlongLine {
        p1: core_lib::ObjectId, // fixed once the dialog opens
        p2: core_lib::ObjectId, // fixed once the dialog opens
        name: String,           // starts empty
        length_formula: String, // starts empty
    },
    Normal {
        p1: core_lib::ObjectId, // fixed once the dialog opens
        p2: core_lib::ObjectId, // fixed once the dialog opens
        name: String,           // starts empty
        length_formula: String, // starts empty
        angle_formula: String,  // starts empty
    },
    Bisector {
        p1: core_lib::ObjectId, // fixed once the dialog opens: one angle ray
        p2: core_lib::ObjectId, // fixed once the dialog opens: the angle's vertex
        p3: core_lib::ObjectId, // fixed once the dialog opens: the other angle ray
        name: String,           // starts empty
        length_formula: String, // starts empty
    },
}

/// What the UI layer should do in response to one canvas click.
#[derive(Debug, Clone, PartialEq)] // same rationale as ToolMode's derives above
pub enum ClickOutcome {
    /// Stay in the current tool and keep waiting for more clicks.
    NeedMoreInput,
    /// Show this dialog window; nothing is committed to the document yet.
    OpenDialog(PendingDialog),
    /// Commit this `ToolKind` to the document immediately, via
    /// `PatternSync::perform_edit`.
    Complete(core_lib::ToolKind),
    /// The click didn't hit anything usable for the current tool (or no
    /// tool is selected); do nothing.
    Ignored,
}

/// Finds the closest point in `data` to `pattern_pos`, if any point is
/// within `tolerance`.
///
/// `tolerance` is expected to be in PATTERN-SPACE units, not screen
/// pixels: hit-testing has to stay accurate regardless of zoom level, so
/// the caller is responsible for converting a fixed screen-pixel radius
/// into pattern-space (dividing by the current camera zoom) before calling
/// this function — see `lib.rs`'s click-handling code for that conversion.
pub fn hit_test_point(
    data: &core_lib::PatternData, // the resolved geometry to search
    pattern_pos: (f64, f64),      // the click position, already converted to pattern space
    tolerance: f64,               // maximum acceptable distance, in pattern-space units
) -> Option<core_lib::ObjectId> {
    let mut closest: Option<(core_lib::ObjectId, f64)> = None; // tracks the closest point's id and distance seen so far

    for (id, object) in data.objects() {
        // walk every object in this PatternData, in its deterministic (BTreeMap) order
        let core_lib::GeoObject::Point(point) = object else {
            continue; // not a point (e.g. a Line): nothing to hit-test a click against
        };
        let dx = point.x - pattern_pos.0; // x distance from this point to the click
        let dy = point.y - pattern_pos.1; // y distance from this point to the click
        let distance = (dx * dx + dy * dy).sqrt(); // Euclidean distance between the point and the click

        let is_closer = match closest {
            Some((_, best_distance)) => distance < best_distance, // only replace the current best if this one is strictly closer
            None => true, // no candidate yet: this one is automatically the closest so far
        };
        if is_closer {
            closest = Some((*id, distance)); // remember this point as the new closest candidate
        }
    }

    match closest {
        Some((id, distance)) if distance <= tolerance => Some(id), // close enough to count as a hit
        _ => None, // either no points existed at all, or the closest one was still too far away
    }
}

/// Which toolbar button the user pressed.
#[derive(Debug, Clone, Copy, PartialEq)] // Debug: printable; Clone/Copy: a plain tag value, cheap to pass around; PartialEq: enables assert_eq! and match comparisons in tests
pub enum ToolKindSelector {
    BasePoint,
    Line,
    Midpoint,
    EndLine,
    AlongLine,
    Normal,
    Bisector,
    Height,
}

/// The pure tool-selection state machine: which tool is active, and how
/// many of its clicks have been collected so far.
#[derive(Debug, Clone)] // Debug: printable in test failures; Clone: tests/UI code may want to snapshot controller state
pub struct ToolController {
    mode: ToolMode, // which tool is selected and its in-progress click state
}

impl Default for ToolController {
    /// Starts with no tool selected.
    ///
    /// Implemented manually rather than derived: `ToolMode` itself
    /// intentionally doesn't derive `Default` (there's no single
    /// "obviously correct" default among its many in-progress-click
    /// states), so `#[derive(Default)]` isn't available here — this impl
    /// picks `ToolMode::Idle` explicitly instead.
    fn default() -> Self {
        ToolController {
            mode: ToolMode::Idle, // no tool selected initially: handle_click will just return Ignored until select_tool is called
        }
    }
}

impl ToolController {
    /// Selects `tool`, resetting to that tool's own empty initial click
    /// state (discarding whatever was in progress for the previously
    /// selected tool, if anything).
    pub fn select_tool(&mut self, tool: ToolKindSelector) {
        self.mode = match tool {
            // map each toolbar button to that tool's empty starting ToolMode
            ToolKindSelector::BasePoint => ToolMode::BasePoint, // no clicks needed before this tool is ready
            ToolKindSelector::Line => ToolMode::Line { first: None }, // no points collected yet
            ToolKindSelector::Midpoint => ToolMode::Midpoint { first: None }, // no points collected yet
            ToolKindSelector::EndLine => ToolMode::EndLine { base: None }, // no base point collected yet
            ToolKindSelector::AlongLine => ToolMode::AlongLine {
                first: None, // no points collected yet
                second: None,
            },
            ToolKindSelector::Normal => ToolMode::Normal {
                first: None, // no points collected yet
                second: None,
            },
            ToolKindSelector::Bisector => ToolMode::Bisector {
                first: None, // no points collected yet
                second: None,
            },
            ToolKindSelector::Height => ToolMode::Height {
                first: None, // no points collected yet
                second: None,
            },
        };
    }

    /// Resets the current tool's in-progress click state back to its own
    /// empty starting point, WITHOUT deselecting the tool itself.
    ///
    /// Matches Seamly2D's own behavior: a tool stays active across
    /// multiple uses until the user explicitly picks a different one (or
    /// none). Canceling a half-finished `Line`, for instance, should let
    /// the user immediately start a fresh `Line` — not force them to
    /// re-select the Line tool from the toolbar. If no tool is selected
    /// (`Idle`), this is a no-op.
    pub fn cancel(&mut self) {
        self.mode = match &self.mode {
            // reset each tool to its own empty starting state, preserving which tool (if any) is active
            ToolMode::Idle => ToolMode::Idle, // no tool active: nothing to reset
            ToolMode::BasePoint => ToolMode::BasePoint, // this tool has no in-progress click state to reset
            ToolMode::Line { .. } => ToolMode::Line { first: None }, // discard any in-progress first click
            ToolMode::Midpoint { .. } => ToolMode::Midpoint { first: None }, // discard any in-progress first click
            ToolMode::EndLine { .. } => ToolMode::EndLine { base: None }, // discard any in-progress base click
            ToolMode::AlongLine { .. } => ToolMode::AlongLine {
                first: None, // discard any in-progress clicks
                second: None,
            },
            ToolMode::Normal { .. } => ToolMode::Normal {
                first: None, // discard any in-progress clicks
                second: None,
            },
            ToolMode::Bisector { .. } => ToolMode::Bisector {
                first: None, // discard any in-progress clicks
                second: None,
            },
            ToolMode::Height { .. } => ToolMode::Height {
                first: None, // discard any in-progress clicks
                second: None,
            },
        };
    }

    /// Read-only access to the current tool/click-progress state, for the
    /// UI layer to decide what to display or prompt for.
    pub fn current_mode(&self) -> &ToolMode {
        &self.mode // borrow out the current state; mutation only ever happens through select_tool/cancel/handle_click
    }

    /// Interprets one canvas click, advancing (or completing) the current
    /// tool's state machine.
    pub fn handle_click(
        &mut self,
        pattern_pos: (f64, f64), // the click position, already converted to pattern space
        hit_point: Option<core_lib::ObjectId>, // whatever hit_test_point found at that position, if anything
    ) -> ClickOutcome {
        let mode = self.mode.clone(); // work on an owned copy, decoupled from `self`'s borrow, so assigning `self.mode` below is never a borrow-checker conflict
        match mode {
            ToolMode::Idle => ClickOutcome::Ignored, // no tool selected: clicks are meaningless

            ToolMode::BasePoint => {
                // BasePoint needs no existing point: it places a new literal point wherever
                // clicked, regardless of what (if anything) hit_point found. Stays active
                // afterward, ready for the user to place another point immediately.
                self.mode = ToolMode::BasePoint; // remain in BasePoint mode for repeated placement
                ClickOutcome::Complete(core_lib::ToolKind::BasePoint {
                    name: "P".to_string(), // placeholder name; real naming/numbering is a future UI refinement, out of scope here
                    x: pattern_pos.0,      // the exact clicked x position
                    y: pattern_pos.1,      // the exact clicked y position
                })
            }

            ToolMode::Line { first } => {
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // Line requires clicking an EXISTING point, not empty space; state left unchanged
                };
                match first {
                    None => {
                        // first of two required clicks: remember it and wait for the second
                        self.mode = ToolMode::Line { first: Some(hit) };
                        ClickOutcome::NeedMoreInput
                    }
                    Some(p1) => {
                        // second click: both endpoints now known, so the tool is complete
                        let kind = core_lib::ToolKind::Line { p1, p2: hit };
                        self.mode = ToolMode::Line { first: None }; // ready for another line immediately
                        ClickOutcome::Complete(kind)
                    }
                }
            }

            ToolMode::Midpoint { first } => {
                // identical two-click pattern to Line, just producing a different ToolKind
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // Midpoint also requires clicking an EXISTING point
                };
                match first {
                    None => {
                        self.mode = ToolMode::Midpoint { first: Some(hit) }; // remember the first endpoint
                        ClickOutcome::NeedMoreInput
                    }
                    Some(p1) => {
                        let kind = core_lib::ToolKind::Midpoint {
                            name: "M".to_string(), // placeholder name, same rationale as BasePoint's "P"
                            p1,
                            p2: hit,
                        };
                        self.mode = ToolMode::Midpoint { first: None }; // ready for another midpoint immediately
                        ClickOutcome::Complete(kind)
                    }
                }
            }

            ToolMode::EndLine { .. } => {
                // EndLine needs exactly one click (an EXISTING point) before it opens its
                // dialog. Once that dialog opens, this tool instance's click-collection job
                // is done: the dialog's own confirm/cancel handles what happens next, and the
                // controller resets immediately so a fresh EndLine sequence can start right away.
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // EndLine's base point must be an EXISTING point
                };
                self.mode = ToolMode::EndLine { base: None }; // reset right away, per the comment above
                ClickOutcome::OpenDialog(PendingDialog::EndLine {
                    base: hit,                     // fixed now; won't change while the dialog is open
                    name: String::new(), // starts empty; filled in by the user in the dialog
                    angle_formula: String::new(), // starts empty
                    length_formula: String::new(), // starts empty
                })
            }

            ToolMode::AlongLine { first, .. } => {
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // both AlongLine clicks must land on EXISTING points
                };
                match first {
                    None => {
                        // first of two required clicks
                        self.mode = ToolMode::AlongLine {
                            first: Some(hit), // remember the point measured from
                            second: None,
                        };
                        ClickOutcome::NeedMoreInput
                    }
                    Some(p1) => {
                        // second click: both points known, so open the formula dialog and reset immediately
                        self.mode = ToolMode::AlongLine {
                            first: None, // reset right away, same rationale as EndLine above
                            second: None,
                        };
                        ClickOutcome::OpenDialog(PendingDialog::AlongLine {
                            p1,                            // fixed now
                            p2: hit,                       // fixed now
                            name: String::new(),           // starts empty
                            length_formula: String::new(), // starts empty
                        })
                    }
                }
            }

            ToolMode::Normal { first, .. } => {
                // identical three-state shape to AlongLine, just producing a Normal dialog with an extra angle field
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // both Normal clicks must land on EXISTING points
                };
                match first {
                    None => {
                        self.mode = ToolMode::Normal {
                            first: Some(hit), // remember the point measured from
                            second: None,
                        };
                        ClickOutcome::NeedMoreInput
                    }
                    Some(p1) => {
                        self.mode = ToolMode::Normal {
                            first: None, // reset right away, same rationale as EndLine/AlongLine above
                            second: None,
                        };
                        ClickOutcome::OpenDialog(PendingDialog::Normal {
                            p1,                            // fixed now
                            p2: hit,                       // fixed now
                            name: String::new(),           // starts empty
                            length_formula: String::new(), // starts empty
                            angle_formula: String::new(),  // starts empty
                        })
                    }
                }
            }

            ToolMode::Bisector { first, second } => {
                // needs three clicks (unlike the two-click AlongLine/Normal above), so the match below covers three progress states, not two
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // all three Bisector clicks must land on EXISTING points
                };
                match (first, second) {
                    (None, _) => {
                        // first of three required clicks
                        self.mode = ToolMode::Bisector {
                            first: Some(hit), // remember the first angle ray
                            second: None,
                        };
                        ClickOutcome::NeedMoreInput
                    }
                    (Some(p1), None) => {
                        // second of three required clicks
                        self.mode = ToolMode::Bisector {
                            first: Some(p1),   // carry the first click forward unchanged
                            second: Some(hit), // remember the angle's vertex
                        };
                        ClickOutcome::NeedMoreInput
                    }
                    (Some(p1), Some(p2)) => {
                        // third click: all three points known, so open the formula dialog and reset immediately
                        self.mode = ToolMode::Bisector {
                            first: None, // reset right away, same rationale as EndLine/AlongLine/Normal above
                            second: None,
                        };
                        ClickOutcome::OpenDialog(PendingDialog::Bisector {
                            p1,                            // fixed now
                            p2,                            // fixed now
                            p3: hit,                       // fixed now
                            name: String::new(),           // starts empty
                            length_formula: String::new(), // starts empty
                        })
                    }
                }
            }

            ToolMode::Height { first, second } => {
                // also needs three clicks, but — unlike Bisector — completes immediately on the third: Height has no formula fields at all
                let Some(hit) = hit_point else {
                    return ClickOutcome::Ignored; // all three Height clicks must land on EXISTING points
                };
                match (first, second) {
                    (None, _) => {
                        // first of three required clicks
                        self.mode = ToolMode::Height {
                            first: Some(hit), // remember the point being projected
                            second: None,
                        };
                        ClickOutcome::NeedMoreInput
                    }
                    (Some(point), None) => {
                        // second of three required clicks
                        self.mode = ToolMode::Height {
                            first: Some(point), // carry the first click forward unchanged
                            second: Some(hit),  // remember the line's first defining point
                        };
                        ClickOutcome::NeedMoreInput
                    }
                    (Some(point), Some(line_p1)) => {
                        // third click: all three points known, so the tool is complete — no dialog, since Height has no formulas
                        let kind = core_lib::ToolKind::Height {
                            name: "H".to_string(), // placeholder name; real naming/numbering is a future UI refinement, out of scope here
                            point,
                            line_p1,
                            line_p2: hit,
                        };
                        self.mode = ToolMode::Height {
                            first: None, // ready for another Height immediately
                            second: None,
                        };
                        ClickOutcome::Complete(kind)
                    }
                }
            }
        }
    }

    /// Converts a completed `PendingDialog` (after the user filled in
    /// name/formula fields and pressed OK) into the `ToolKind` it
    /// describes.
    ///
    /// Only checks that `name` is non-empty — basic, structural validation
    /// that belongs here since it needs no outside data. Formula
    /// *validity* (whether a formula string actually evaluates) is a
    /// separate concern the UI layer checks itself, every frame, via
    /// `core::formula::eval_formula` against the live variable table; this
    /// function has no access to that variable table, so it assumes the
    /// formula strings are merely syntactically present, not necessarily
    /// already validated as evaluable.
    pub fn finish_dialog(&self, dialog: PendingDialog) -> Result<core_lib::ToolKind, String> {
        match dialog {
            // dispatch on which dialog variant this is, mapping fields one-to-one onto the matching ToolKind
            PendingDialog::EndLine {
                base,
                name,
                angle_formula,
                length_formula,
            } => {
                if name.is_empty() {
                    return Err("name must not be empty".to_string()); // structural validation: every tool needs a label
                }
                Ok(core_lib::ToolKind::EndLine {
                    name,             // carried through unchanged
                    base_point: base, // PendingDialog's `base` maps to ToolKind::EndLine's `base_point`
                    angle_formula,    // carried through unchanged
                    length_formula,   // carried through unchanged
                })
            }
            PendingDialog::AlongLine {
                p1,
                p2,
                name,
                length_formula,
            } => {
                if name.is_empty() {
                    return Err("name must not be empty".to_string());
                }
                Ok(core_lib::ToolKind::AlongLine {
                    name,           // carried through unchanged
                    p1,             // carried through unchanged
                    p2,             // carried through unchanged
                    length_formula, // carried through unchanged
                })
            }
            PendingDialog::Normal {
                p1,
                p2,
                name,
                length_formula,
                angle_formula,
            } => {
                if name.is_empty() {
                    return Err("name must not be empty".to_string());
                }
                Ok(core_lib::ToolKind::Normal {
                    name,           // carried through unchanged
                    p1,             // carried through unchanged
                    p2,             // carried through unchanged
                    length_formula, // carried through unchanged
                    angle_formula,  // carried through unchanged
                })
            }
            PendingDialog::Bisector {
                p1,
                p2,
                p3,
                name,
                length_formula,
            } => {
                if name.is_empty() {
                    return Err("name must not be empty".to_string());
                }
                Ok(core_lib::ToolKind::Bisector {
                    name,           // carried through unchanged
                    p1,             // carried through unchanged
                    p2,             // carried through unchanged
                    p3,             // carried through unchanged
                    length_formula, // carried through unchanged
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> (
        core_lib::PatternData,
        core_lib::ObjectId,
        core_lib::ObjectId,
        core_lib::ObjectId,
    ) {
        let mut data = core_lib::PatternData::default();
        let a = data.add_object(core_lib::GeoObject::Point(core_lib::PointData {
            x: 0.0,
            y: 0.0,
        }));
        let b = data.add_object(core_lib::GeoObject::Point(core_lib::PointData {
            x: 10.0,
            y: 0.0,
        }));
        let c = data.add_object(core_lib::GeoObject::Point(core_lib::PointData {
            x: 20.0,
            y: 0.0,
        }));
        (data, a, b, c)
    }

    #[test]
    fn hit_test_finds_the_closest_point_within_tolerance() {
        let (data, _a, b, _c) = sample_data();
        let hit = hit_test_point(&data, (10.5, 0.2), 2.0);
        assert_eq!(hit, Some(b)); // (10.5, 0.2) is closest to B at (10.0, 0.0)
    }

    #[test]
    fn hit_test_returns_none_when_every_point_is_too_far() {
        let (data, ..) = sample_data();
        let hit = hit_test_point(&data, (1000.0, 1000.0), 2.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn hit_test_on_empty_pattern_data_returns_none_without_panicking() {
        let data = core_lib::PatternData::default();
        let hit = hit_test_point(&data, (0.0, 0.0), 100.0);
        assert_eq!(hit, None);
    }

    #[test]
    fn line_ignores_a_click_that_hits_nothing() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::Line);

        let outcome = controller.handle_click((5.0, 5.0), None);
        assert_eq!(outcome, ClickOutcome::Ignored);
        assert_eq!(controller.current_mode(), &ToolMode::Line { first: None }); // unchanged by an ignored click
    }

    #[test]
    fn line_two_valid_clicks_complete_in_correct_order() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::Line);
        let p1 = core_lib::ObjectId::from_raw(1);
        let p2 = core_lib::ObjectId::from_raw(2);

        let first_outcome = controller.handle_click((0.0, 0.0), Some(p1));
        assert_eq!(first_outcome, ClickOutcome::NeedMoreInput);
        assert_eq!(
            controller.current_mode(),
            &ToolMode::Line { first: Some(p1) }
        );

        let second_outcome = controller.handle_click((10.0, 0.0), Some(p2));
        assert_eq!(
            second_outcome,
            ClickOutcome::Complete(core_lib::ToolKind::Line { p1, p2 })
        );
        assert_eq!(controller.current_mode(), &ToolMode::Line { first: None }); // ready for another line
    }

    #[test]
    fn end_line_one_click_opens_dialog_with_empty_fields() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::EndLine);
        let base = core_lib::ObjectId::from_raw(1);

        let outcome = controller.handle_click((0.0, 0.0), Some(base));
        assert_eq!(
            outcome,
            ClickOutcome::OpenDialog(PendingDialog::EndLine {
                base,
                name: String::new(),
                angle_formula: String::new(),
                length_formula: String::new(),
            })
        );
        assert_eq!(controller.current_mode(), &ToolMode::EndLine { base: None });
        // reset immediately
    }

    #[test]
    fn base_point_always_completes_with_the_clicked_position_regardless_of_hit_point() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::BasePoint);
        let bogus_hit = core_lib::ObjectId::from_raw(42); // an arbitrary id; BasePoint must ignore it entirely

        let outcome_with_hit = controller.handle_click((3.0, 4.0), Some(bogus_hit));
        assert_eq!(
            outcome_with_hit,
            ClickOutcome::Complete(core_lib::ToolKind::BasePoint {
                name: "P".to_string(),
                x: 3.0,
                y: 4.0,
            })
        );

        let outcome_without_hit = controller.handle_click((7.0, 8.0), None);
        assert_eq!(
            outcome_without_hit,
            ClickOutcome::Complete(core_lib::ToolKind::BasePoint {
                name: "P".to_string(),
                x: 7.0,
                y: 8.0,
            })
        );
    }

    #[test]
    fn cancel_mid_line_resets_to_the_tools_own_empty_state_not_idle() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::Line);
        let p1 = core_lib::ObjectId::from_raw(1);
        controller.handle_click((0.0, 0.0), Some(p1)); // first click of two

        controller.cancel();
        assert_eq!(controller.current_mode(), &ToolMode::Line { first: None }); // NOT Idle

        // Confirm the tool still works correctly for a fresh two-click sequence afterward.
        let p3 = core_lib::ObjectId::from_raw(3);
        let p4 = core_lib::ObjectId::from_raw(4);
        let first = controller.handle_click((1.0, 1.0), Some(p3));
        assert_eq!(first, ClickOutcome::NeedMoreInput);
        let second = controller.handle_click((2.0, 2.0), Some(p4));
        assert_eq!(
            second,
            ClickOutcome::Complete(core_lib::ToolKind::Line { p1: p3, p2: p4 })
        );
    }

    #[test]
    fn finish_dialog_rejects_empty_name() {
        let controller = ToolController::default();
        let dialog = PendingDialog::EndLine {
            base: core_lib::ObjectId::from_raw(1),
            name: String::new(), // empty: should be rejected
            angle_formula: "0".to_string(),
            length_formula: "10".to_string(),
        };
        let result = controller.finish_dialog(dialog);
        assert!(result.is_err());
    }

    #[test]
    fn finish_dialog_accepts_non_empty_name_without_validating_formulas() {
        let controller = ToolController::default();
        let base = core_lib::ObjectId::from_raw(1);
        let dialog = PendingDialog::EndLine {
            base,
            name: "A1".to_string(),
            angle_formula: "not a real formula at all".to_string(), // finish_dialog doesn't check evaluability
            length_formula: "10".to_string(),
        };
        let kind = controller.finish_dialog(dialog).unwrap();
        assert_eq!(
            kind,
            core_lib::ToolKind::EndLine {
                name: "A1".to_string(),
                base_point: base,
                angle_formula: "not a real formula at all".to_string(),
                length_formula: "10".to_string(),
            }
        );
    }

    #[test]
    fn bisector_three_valid_clicks_open_dialog_with_empty_fields() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::Bisector);
        let p1 = core_lib::ObjectId::from_raw(1);
        let p2 = core_lib::ObjectId::from_raw(2);
        let p3 = core_lib::ObjectId::from_raw(3);

        let first = controller.handle_click((0.0, 0.0), Some(p1));
        assert_eq!(first, ClickOutcome::NeedMoreInput);
        let second = controller.handle_click((1.0, 1.0), Some(p2));
        assert_eq!(second, ClickOutcome::NeedMoreInput);

        let third = controller.handle_click((2.0, 2.0), Some(p3));
        assert_eq!(
            third,
            ClickOutcome::OpenDialog(PendingDialog::Bisector {
                p1,
                p2,
                p3,
                name: String::new(),
                length_formula: String::new(),
            })
        );
        assert_eq!(
            controller.current_mode(),
            &ToolMode::Bisector {
                first: None,
                second: None
            }
        ); // reset immediately, ready for another Bisector sequence
    }

    #[test]
    fn height_three_valid_clicks_complete_immediately_with_no_dialog() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::Height);
        let point = core_lib::ObjectId::from_raw(1);
        let line_p1 = core_lib::ObjectId::from_raw(2);
        let line_p2 = core_lib::ObjectId::from_raw(3);

        let first = controller.handle_click((0.0, 0.0), Some(point));
        assert_eq!(first, ClickOutcome::NeedMoreInput);
        let second = controller.handle_click((1.0, 1.0), Some(line_p1));
        assert_eq!(second, ClickOutcome::NeedMoreInput);

        let third = controller.handle_click((2.0, 2.0), Some(line_p2));
        assert_eq!(
            third,
            ClickOutcome::Complete(core_lib::ToolKind::Height {
                name: "H".to_string(),
                point,
                line_p1,
                line_p2,
            })
        );
        assert_eq!(
            controller.current_mode(),
            &ToolMode::Height {
                first: None,
                second: None
            }
        ); // ready for another Height immediately
    }

    #[test]
    fn bisector_ignores_a_click_that_hits_nothing() {
        let mut controller = ToolController::default();
        controller.select_tool(ToolKindSelector::Bisector);

        let outcome = controller.handle_click((5.0, 5.0), None);
        assert_eq!(outcome, ClickOutcome::Ignored);
        assert_eq!(
            controller.current_mode(),
            &ToolMode::Bisector {
                first: None,
                second: None
            }
        ); // unchanged by an ignored click
    }
}
