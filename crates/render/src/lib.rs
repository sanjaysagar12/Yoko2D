use thiserror::Error; // brings in the Error derive macro used below

/// One primitive drawing instruction produced from a [`core_lib::PatternData`].
///
/// Coordinates here are in PATTERN SPACE — the same coordinate system
/// [`core_lib::PatternData`] stores its objects in (arbitrary pattern
/// units, y-up, origin wherever the pattern's first `BasePoint` put it) —
/// NOT screen pixels. Converting pattern-space coordinates to actual
/// screen pixels (pan/zoom/y-flip) is deliberately left to the caller
/// (the `app` crate's `camera` module), so this crate stays a pure,
/// swappable translation layer with no knowledge of any particular
/// windowing/graphics backend.
#[derive(Debug, Clone, PartialEq)] // Debug: printable in test failures; Clone: callers may want to keep a copy; PartialEq: enables assert_eq! in tests
pub enum DrawCommand {
    Point {
        x: f64,                // pattern-space x coordinate
        y: f64,                // pattern-space y coordinate
        label: Option<String>, // TODO(phase 10+): populate from the point's tool-assigned name once that's available at this layer; always None for now
    },
    Line {
        x1: f64, // pattern-space x coordinate of the first endpoint
        y1: f64, // pattern-space y coordinate of the first endpoint
        x2: f64, // pattern-space x coordinate of the second endpoint
        y2: f64, // pattern-space y coordinate of the second endpoint
    },
    /// A closed polygon outline — used for piece contours and their seam
    /// allowances.
    Polygon {
        points: Vec<(f64, f64)>, // the polygon's vertices, in order, in pattern-space coordinates
        // A rendering-style hint for the UI layer (the `app` crate) — e.g.
        // whether to draw a filled shape versus an outline only. This
        // crate makes no rendering-style decisions itself; it only carries
        // the hint through from the source geometry.
        filled: bool,
    },
    /// An OPEN (not closed/wrap-around) sequence of connected line
    /// segments — used for `Arc`/`Spline` curves, tessellated into a
    /// straight-line approximation for backends with no native curve
    /// primitive of their own.
    Polyline {
        points: Vec<(f64, f64)>, // the sampled curve points, in order, in pattern-space coordinates
    },
}

/// How many straight-line segments an `Arc`/`Spline` curve is sampled into
/// for `Polyline` tessellation. A named constant, not a magic number
/// repeated at each call site below: both curve kinds share the same
/// sampling density, since neither benefits from a finer or coarser count
/// than the other at this crate's typical pattern scale.
const CURVE_SAMPLE_SEGMENTS: usize = 32;

/// Everything that can go wrong turning a [`core_lib::PatternData`] into
/// draw commands.
#[derive(Debug, Clone, PartialEq, Error)] // Debug/Clone/PartialEq mirror core_lib's own error types; Error implements std::error::Error via thiserror
pub enum RenderError {
    /// A `Line` object referenced a point id that isn't present (as a
    /// `Point`) in the same `PatternData`. This shouldn't normally happen
    /// after a correct `recompute_all`, but `render` operates on whatever
    /// `PatternData` it's handed and must not assume it's always
    /// well-formed — see the comment in `render()` for why this is
    /// surfaced as an error rather than silently skipped.
    #[error("line references object {0:?}, which is not a point in this pattern")]
    DanglingReference(core_lib::ObjectId), // the missing/mistyped id a Line pointed at
}

/// Translates every object in `data` into an ordered list of
/// [`DrawCommand`]s.
///
/// Iterates `data.objects()` in ascending `ObjectId` order — guaranteed by
/// `PatternData` storing objects in a `BTreeMap` (Phase 7's Part A change)
/// — so that calling this repeatedly on unchanged data always produces the
/// exact same `Vec` in the exact same order. That determinism is
/// load-bearing for the `app` crate: redrawing every frame from a
/// differently-ordered command list would flicker even when nothing in
/// the pattern actually changed.
pub fn render(data: &core_lib::PatternData) -> Result<Vec<DrawCommand>, RenderError> {
    let mut commands = Vec::new(); // accumulates the draw commands in the same order objects are visited below

    for (_id, object) in data.objects() {
        // the object's own id isn't needed here: a dangling Line reference is named by the id it *points at*
        // (line.p1/line.p2 below), not by this object's own id, so the loop variable itself stays unused
        match object {
            // dispatch on which kind of geometry this object is
            core_lib::GeoObject::Point(point) => {
                commands.push(DrawCommand::Point {
                    x: point.x,  // carry the point's pattern-space x through unchanged
                    y: point.y,  // carry the point's pattern-space y through unchanged
                    label: None, // no name available at this layer yet; see the TODO on DrawCommand::Point
                });
            }
            core_lib::GeoObject::Line(line) => {
                let p1 = data.get_point(line.p1).map_err(|_| {
                    // any lookup failure here (not-found or wrong-type) means p1 isn't a real point in `data`
                    RenderError::DanglingReference(line.p1) // name exactly which id is dangling
                })?; // stop immediately: don't silently drop this line, since that would hide an upstream bug
                let p2 = data.get_point(line.p2).map_err(|_| {
                    // same check for the line's second endpoint
                    RenderError::DanglingReference(line.p2) // name exactly which id is dangling
                })?; // stop immediately, same rationale as the p1 lookup above
                commands.push(DrawCommand::Line {
                    x1: p1.x, // resolved coordinates of the first endpoint
                    y1: p1.y,
                    x2: p2.x, // resolved coordinates of the second endpoint
                    y2: p2.y,
                });
            }
            core_lib::GeoObject::Piece(piece) => {
                commands.push(DrawCommand::Polygon {
                    points: piece.contour.clone(), // the piece's boundary, in order
                    filled: false, // pieces are drawn as outlines in this phase; styling decisions belong to the UI layer
                });
                if let Some(sa_points) = &piece.seam_allowance {
                    // Pushed immediately after the contour, in that fixed
                    // order, so a UI layer consuming this list in order can
                    // naturally style the seam allowance as an outer/
                    // secondary outline without needing to inspect which
                    // GeoObject each command came from.
                    commands.push(DrawCommand::Polygon {
                        points: sa_points.clone(), // the seam-allowance offset boundary, in order
                        filled: false,
                    });
                }
            }
            core_lib::GeoObject::Arc(arc) => {
                let mut points = Vec::with_capacity(CURVE_SAMPLE_SEGMENTS + 1); // one sample per segment boundary, including both ends
                for i in 0..=CURVE_SAMPLE_SEGMENTS {
                    let t = i as f64 / CURVE_SAMPLE_SEGMENTS as f64; // 0.0..=1.0, evenly spaced across the sweep
                    let theta_deg =
                        arc.start_angle_deg + t * (arc.end_angle_deg - arc.start_angle_deg); // linearly interpolate the angle across the sweep
                    let theta_rad = theta_deg.to_radians(); // degrees -> radians before cos/sin, same convention as core_lib::geo::ArcData's own doc comment
                    let x = arc.center.0 + arc.radius * theta_rad.cos(); // center + radius*cos(theta)
                    let y = arc.center.1 + arc.radius * theta_rad.sin(); // center + radius*sin(theta)
                    points.push((x, y));
                }
                commands.push(DrawCommand::Polyline { points });
            }
            core_lib::GeoObject::Spline(spline) => {
                let mut points = Vec::with_capacity(CURVE_SAMPLE_SEGMENTS + 1); // one sample per segment boundary, including both ends
                for i in 0..=CURVE_SAMPLE_SEGMENTS {
                    let t = i as f64 / CURVE_SAMPLE_SEGMENTS as f64; // 0.0..=1.0, evenly spaced along the curve
                    points.push(core_lib::geometry::cubic_bezier_point(
                        spline.p1, spline.p2, spline.p3, spline.p4, t,
                    )); // reuses core_lib's own Bezier formula rather than reimplementing it here
                }
                commands.push(DrawCommand::Polyline { points });
            }
        }
    }

    Ok(commands) // every object translated successfully (or a dangling reference already returned above)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::{GeoObject, LineData, PatternData, PieceData, PointData};

    #[test]
    fn renders_a_point_and_a_line_with_correct_coordinates() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 1.0, y: 2.0 }));
        let p2 = data.add_object(GeoObject::Point(PointData { x: 3.0, y: 4.0 }));
        data.add_object(GeoObject::Line(LineData { p1, p2 }));

        let commands = render(&data).unwrap();
        assert_eq!(
            commands,
            vec![
                DrawCommand::Point {
                    x: 1.0,
                    y: 2.0,
                    label: None
                },
                DrawCommand::Point {
                    x: 3.0,
                    y: 4.0,
                    label: None
                },
                DrawCommand::Line {
                    x1: 1.0,
                    y1: 2.0,
                    x2: 3.0,
                    y2: 4.0
                },
            ]
        );
    }

    #[test]
    fn render_is_deterministic() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 1.0, y: 2.0 }));
        let p2 = data.add_object(GeoObject::Point(PointData { x: 3.0, y: 4.0 }));
        data.add_object(GeoObject::Line(LineData { p1, p2 }));

        let first = render(&data).unwrap();
        let second = render(&data).unwrap();
        assert_eq!(first, second);
    }

    // `add_object` always hands out strictly increasing ids in call order
    // (Phase 1), so the public API alone can't diverge insertion order
    // from id order — that requires core's pub(crate) `insert_with_id`,
    // which is why the "insertion order differs from id order, iteration
    // stays ascending" guarantee itself is proven directly against
    // `PatternData::objects()` in core's own test suite
    // (`objects_iterates_in_ascending_id_order_regardless_of_insertion_order`
    // in `crates/core/src/container.rs`). What this test covers instead:
    // that `render()` doesn't reorder or reshuffle whatever order
    // `data.objects()` gives it — objects added in a given call order
    // (here, deliberately Line before one of its own endpoints, in terms
    // of "the arguments describe a later point" though not in terms of
    // *insertion* order, since a Line's endpoints must already exist to
    // reference them) come out in ascending id order.
    #[test]
    fn render_output_is_ordered_by_ascending_object_id() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 })); // id 0
        let p2 = data.add_object(GeoObject::Point(PointData { x: 5.0, y: 5.0 })); // id 1
        let line = data.add_object(GeoObject::Line(LineData { p1, p2 })); // id 2

        let commands = render(&data).unwrap();
        // Three objects went in (ids 0, 1, 2); three commands must come out, in that ascending order.
        assert_eq!(commands.len(), 3);
        assert!(matches!(
            commands[0],
            DrawCommand::Point { x: 0.0, y: 0.0, .. }
        ));
        assert!(matches!(
            commands[1],
            DrawCommand::Point { x: 5.0, y: 5.0, .. }
        ));
        assert!(matches!(commands[2], DrawCommand::Line { .. }));
        let _ = line; // only its position in the output (last) matters for this test
    }

    #[test]
    fn dangling_line_reference_is_reported_not_panicked() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        let p2 = data.add_object(GeoObject::Point(PointData { x: 1.0, y: 1.0 }));
        data.add_object(GeoObject::Line(LineData { p1, p2 }));

        // Remove p1 after the Line was created, so the Line now references
        // an id that used to be valid but no longer resolves to a Point —
        // exactly the "dangling reference" scenario render() must handle
        // gracefully.
        data.remove_object(p1);

        let err = render(&data).unwrap_err();
        assert_eq!(err, RenderError::DanglingReference(p1));
    }

    #[test]
    fn render_of_a_piece_with_contour_and_seam_allowance_produces_two_polygons_in_order() {
        let mut data = PatternData::default();
        data.add_object(GeoObject::Piece(PieceData {
            contour: vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)],
            seam_allowance: Some(vec![(-1.0, -1.0), (11.0, -1.0), (-1.0, 11.0)]),
        }));

        let commands = render(&data).unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0],
            DrawCommand::Polygon {
                points: vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)],
                filled: false,
            }
        );
        assert_eq!(
            commands[1],
            DrawCommand::Polygon {
                points: vec![(-1.0, -1.0), (11.0, -1.0), (-1.0, 11.0)],
                filled: false,
            }
        );
    }

    #[test]
    fn render_of_a_spline_produces_a_polyline_whose_ends_match_p1_and_p4() {
        let mut data = PatternData::default();
        data.add_object(GeoObject::Spline(core_lib::SplineData {
            p1: (0.0, 0.0),
            p2: (0.0, 3.0),
            p3: (10.0, 3.0),
            p4: (10.0, 0.0),
        }));

        let commands = render(&data).unwrap();
        assert_eq!(commands.len(), 1);
        let DrawCommand::Polyline { points } = &commands[0] else {
            panic!("expected a single Polyline command, got {:?}", commands[0]);
        };
        // A meaningful number of intermediate samples were actually produced,
        // not just the two endpoints.
        assert!(points.len() > 2);
        let first = points.first().unwrap();
        let last = points.last().unwrap();
        assert!((first.0 - 0.0).abs() < 1e-9);
        assert!((first.1 - 0.0).abs() < 1e-9);
        assert!((last.0 - 10.0).abs() < 1e-9);
        assert!((last.1 - 0.0).abs() < 1e-9);
    }

    #[test]
    fn render_of_an_arc_produces_a_polyline_sampled_across_the_sweep() {
        let mut data = PatternData::default();
        data.add_object(GeoObject::Arc(core_lib::ArcData {
            center: (5.0, 3.0),
            radius: 10.0,
            start_angle_deg: 0.0,
            end_angle_deg: 90.0,
        }));

        let commands = render(&data).unwrap();
        assert_eq!(commands.len(), 1);
        let DrawCommand::Polyline { points } = &commands[0] else {
            panic!("expected a single Polyline command, got {:?}", commands[0]);
        };
        assert!(points.len() > 2);
        let first = points.first().unwrap();
        let last = points.last().unwrap();
        // theta=0: center + radius*(cos(0),sin(0)) = (15,3).
        assert!((first.0 - 15.0).abs() < 1e-9);
        assert!((first.1 - 3.0).abs() < 1e-9);
        // theta=90: center + radius*(cos(90),sin(90)) = (5,13).
        assert!((last.0 - 5.0).abs() < 1e-9);
        assert!((last.1 - 13.0).abs() < 1e-9);
    }

    #[test]
    fn render_of_a_piece_with_no_seam_allowance_produces_one_polygon() {
        let mut data = PatternData::default();
        data.add_object(GeoObject::Piece(PieceData {
            contour: vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)],
            seam_allowance: None,
        }));

        let commands = render(&data).unwrap();
        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], DrawCommand::Polygon { .. }));
    }
}
