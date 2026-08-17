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
}

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
        }
    }

    Ok(commands) // every object translated successfully (or a dangling reference already returned above)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_lib::{GeoObject, LineData, PatternData, PointData};

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
}
