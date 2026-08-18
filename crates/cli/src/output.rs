use crate::action_script::ActionScriptError; // write_output_to_file's error type, so its I/O/JSON failures unify with the rest of this binary's error handling

/// One resolved point, ready to draw without needing to re-run
/// `recompute_all`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)] // Debug/Clone/PartialEq: printable/comparable in tests; Serialize: this is the whole point of this type; Deserialize: lets tests round-trip it back for comparison
pub struct OutputPoint {
    pub id: u32, // the point's ObjectId, as a plain integer (ObjectId::raw())
    pub x: f64,  // resolved pattern-space x coordinate
    pub y: f64,  // resolved pattern-space y coordinate
}

/// One resolved line, carrying BOTH its endpoints' ids AND their already-
/// resolved coordinates — so a consumer of this output (e.g. the GUI) can
/// draw it directly without looking anything back up.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)] // same derive rationale as OutputPoint above
pub struct OutputLine {
    pub id: u32, // the line's own ObjectId
    pub p1: u32, // the first endpoint's ObjectId
    pub p2: u32, // the second endpoint's ObjectId
    pub x1: f64, // the first endpoint's resolved x coordinate
    pub y1: f64, // the first endpoint's resolved y coordinate
    pub x2: f64, // the second endpoint's resolved x coordinate
    pub y2: f64, // the second endpoint's resolved y coordinate
}

/// A portable, already-resolved snapshot of a pattern's geometry: every
/// point and line, self-contained, needing no further lookups against a
/// `core_lib::PatternData` to draw.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)] // Default: build_output starts from an empty snapshot and fills it in
pub struct PatternOutput {
    pub points: Vec<OutputPoint>, // every resolved point, in the same ascending-id order PatternData::objects() yields
    pub lines: Vec<OutputLine>,   // every resolved line, same ordering guarantee
}

/// Builds a [`PatternOutput`] snapshot from `data`.
///
/// Returns `core_lib::ContainerError` (rather than introducing a parallel
/// error type) if a `Line` references a point id that isn't actually a
/// point in `data` — the same dangling-reference scenario
/// `render::render` guards against, just propagated here via the existing
/// `ContainerError` `render` itself resolves through internally, instead
/// of wrapping it in a second, render-specific error type.
pub fn build_output(
    data: &core_lib::PatternData,
) -> Result<PatternOutput, core_lib::ContainerError> {
    let mut output = PatternOutput::default(); // accumulates points/lines in the order data.objects() visits them

    for (id, object) in data.objects() {
        // walk every object in this PatternData, in its deterministic (BTreeMap) ascending-id order
        match object {
            // dispatch on which kind of geometry this object is
            core_lib::GeoObject::Point(point) => {
                output.points.push(OutputPoint {
                    id: id.raw(), // this object's own id, as a plain integer
                    x: point.x,   // carry the resolved x coordinate through unchanged
                    y: point.y,   // carry the resolved y coordinate through unchanged
                });
            }
            core_lib::GeoObject::Line(line) => {
                let p1 = data.get_point(line.p1)?; // resolve the first endpoint; propagate ObjectNotFound/WrongObjectType if it's dangling
                let p2 = data.get_point(line.p2)?; // resolve the second endpoint, same rationale
                output.lines.push(OutputLine {
                    id: id.raw(),      // this line's own id
                    p1: line.p1.raw(), // the first endpoint's id, as a plain integer
                    p2: line.p2.raw(), // the second endpoint's id, as a plain integer
                    x1: p1.x,          // the first endpoint's resolved x coordinate
                    y1: p1.y,          // the first endpoint's resolved y coordinate
                    x2: p2.x,          // the second endpoint's resolved x coordinate
                    y2: p2.y,          // the second endpoint's resolved y coordinate
                });
            }
            core_lib::GeoObject::Piece(_) => {
                // Pieces are explicitly out of scope for this task's action-script
                // format (no `add_piece` operation exists in `Action`), so no
                // action script this CLI executes can ever produce one — but
                // GeoObject is a shared type with more variants than this task
                // covers, so this arm exists to keep the match exhaustive rather
                // than leaving Piece output support silently unimplemented via a
                // compile error. Skipped rather than erroring, since a Piece
                // present in `data` (e.g. loaded from an unrelated pattern file)
                // is a normal, valid object this snapshot format just doesn't
                // represent yet.
            }
        }
    }

    Ok(output) // every object translated successfully (or a dangling reference already returned above)
}

/// Serializes `output` as pretty-printed JSON and writes it to `path`.
pub fn write_output_to_file(
    output: &PatternOutput,
    path: &std::path::Path,
) -> Result<(), ActionScriptError> {
    let json = serde_json::to_string_pretty(output)?; // pretty-print for human readability; propagate a (practically unreachable, since PatternOutput is plain data) serialization failure via ActionScriptError::Json
    std::fs::write(path, json)?; // write the whole JSON text to disk in one call; propagate any I/O failure via ActionScriptError::Io
    Ok(()) // written successfully
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_output_produces_matching_point_and_line_coordinates() {
        let mut data = core_lib::PatternData::default();
        let p1 = data.add_object(core_lib::GeoObject::Point(core_lib::PointData {
            x: 1.0,
            y: 2.0,
        }));
        let p2 = data.add_object(core_lib::GeoObject::Point(core_lib::PointData {
            x: 3.0,
            y: 4.0,
        }));
        let line = data.add_object(core_lib::GeoObject::Line(core_lib::LineData { p1, p2 }));

        let output = build_output(&data).unwrap();
        assert_eq!(
            output.points,
            vec![
                OutputPoint {
                    id: p1.raw(),
                    x: 1.0,
                    y: 2.0
                },
                OutputPoint {
                    id: p2.raw(),
                    x: 3.0,
                    y: 4.0
                },
            ]
        );
        assert_eq!(
            output.lines,
            vec![OutputLine {
                id: line.raw(),
                p1: p1.raw(),
                p2: p2.raw(),
                x1: 1.0,
                y1: 2.0,
                x2: 3.0,
                y2: 4.0,
            }]
        );
    }

    #[test]
    fn build_output_serializes_to_json_with_expected_keys() {
        let mut data = core_lib::PatternData::default();
        data.add_object(core_lib::GeoObject::Point(core_lib::PointData {
            x: 5.0,
            y: 6.0,
        }));

        let output = build_output(&data).unwrap();
        let json = serde_json::to_string(&output).unwrap();

        assert!(json.contains("\"points\""));
        assert!(json.contains("\"lines\""));
        assert!(json.contains("\"x\":5.0"));
        assert!(json.contains("\"y\":6.0"));

        // Round-trip through deserialization confirms the shape is fully self-consistent, not just substring-matched.
        let round_tripped: PatternOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, output);
    }
}
