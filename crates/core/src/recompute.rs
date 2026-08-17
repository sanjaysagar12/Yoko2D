use thiserror::Error; // brings in the `Error` derive macro used by `PatternError` below

use crate::document::{Document, ToolKind}; // the ordered tool history this module walks
use crate::formula::{eval_formula, flatten_variables}; // Phase 2's formula pipeline, used by EndLine
use crate::{ContainerError, GeoObject, LineData, ObjectId, PatternData, PointData}; // Phase 1's container and geometry types

/// Everything that can go wrong turning a [`Document`] into a resolved
/// [`PatternData`].
#[derive(Debug, Clone, PartialEq, Error)] // Debug/Clone/PartialEq mirror ContainerError/FormulaError so tests can assert_eq! against this too
pub enum PatternError {
    /// A tool referenced an id that hasn't been resolved into geometry yet
    /// at this point in the history walk — either it belongs to a tool
    /// later in `history` (out-of-order dependency) or to no tool at all
    /// (a corrupted document). Distinguished from a generic
    /// `ContainerError::ObjectNotFound` so callers get a precise,
    /// actionable error naming exactly which dependency is missing.
    #[error("missing dependency: object {0:?} has not been resolved yet")]
    MissingDependency(ObjectId), // the id that was looked up and not found among what's been resolved so far

    /// A lookup against the `PatternData` being built failed for a reason
    /// other than "not found yet" (currently only `WrongObjectType`, e.g.
    /// a tool's `p1` names a `Line` instead of a `Point`). Propagated via
    /// `#[from]` so `?` converts it automatically.
    #[error("container error: {0}")]
    Container(#[from] ContainerError), // wraps whichever ContainerError variant caused the failure

    /// One of a tool's formula strings failed to tokenize, parse, or
    /// evaluate. Propagated via `#[from]` so `?` converts it automatically.
    #[error("formula error: {0}")]
    Formula(#[from] crate::formula::FormulaError), // wraps whichever FormulaError variant caused the failure

    /// A tool's inputs are all valid, fully-resolved points (unlike
    /// `MissingDependency`) but the specific geometric configuration they
    /// form has no well-defined answer — e.g. `AlongLine`'s `p1`/`p2`
    /// turned out to coincide, so there's no direction to measure along.
    /// Distinct from `Formula`/`Container`, which are lookup/evaluation
    /// failures rather than "the math itself doesn't have an answer here".
    #[error("degenerate geometry: {0}")]
    DegenerateGeometry(String), // a human-readable description of which geometric configuration was invalid and why
}

/// Looks up `id` as a point in `data`, translating "not found at all" into
/// [`PatternError::MissingDependency`] (a precise, actionable error) while
/// letting any other [`ContainerError`] (e.g. `WrongObjectType`, if `id`
/// exists but names a `Line`) propagate unchanged via the `#[from]`
/// conversion on [`PatternError::Container`].
///
/// Factored out because `ToolKind::EndLine` and `ToolKind::Line` both need
/// exactly this "resolve a required point dependency" check.
fn resolve_point(data: &PatternData, id: ObjectId) -> Result<&PointData, PatternError> {
    match data.get_point(id) {
        // dispatch on how the lookup went
        Ok(point) => Ok(point), // id exists and holds a point: exactly what the caller needs
        Err(ContainerError::ObjectNotFound(_)) => Err(PatternError::MissingDependency(id)), // not resolved yet: report precisely which id
        Err(other) => Err(other.into()), // any other container error (e.g. WrongObjectType) propagates as-is
    }
}

/// Rebuilds a [`PatternData`] from scratch by walking `doc`'s tool history
/// in order and resolving each tool's formulas/literals against the
/// variables and geometry resolved so far.
///
/// Pure: takes `&Document`, never mutates it, and returns a brand-new
/// `PatternData` rather than modifying one in place — `Document` is the
/// persistent source of truth, `PatternData` is a disposable cache that
/// gets thrown away and rebuilt every time this is called.
pub fn recompute_all(doc: &Document) -> Result<PatternData, PatternError> {
    let mut data = PatternData::default(); // start every recompute from a completely empty cache

    for (name, var) in doc.base_variables() {
        // seed the cache with every base variable before resolving any geometry
        data.add_variable(name.clone(), var.clone()); // `data` owns an independent copy, not a reference into `doc`
    }

    for record in doc.history() {
        // walk the tool history in the exact order the tools were added
        match &record.kind {
            // dispatch on which kind of tool this record is
            ToolKind::BasePoint { x, y, .. } => {
                let point = GeoObject::Point(PointData { x: *x, y: *y }); // a literal coordinate needs no formula evaluation
                data.insert_with_id(record.id, point)?; // place it under the id this tool was assigned when added
            }
            ToolKind::EndLine {
                base_point,
                angle_formula,
                length_formula,
                ..
            } => {
                let base = resolve_point(&data, *base_point)?; // resolve the point this one is measured from
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for these formulas to reference
                let angle = eval_formula(angle_formula, &vars)?; // evaluate the angle formula (degrees, Phase 2's convention)
                let length = eval_formula(length_formula, &vars)?; // evaluate the length formula
                let x = base.x + length * angle.to_radians().cos(); // degrees -> radians before cos, matching Phase 2's trig convention
                let y = base.y + length * angle.to_radians().sin(); // degrees -> radians before sin, same convention
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Line { p1, p2 } => {
                resolve_point(&data, *p1)?; // p1 must already be resolved earlier in history
                resolve_point(&data, *p2)?; // p2 must already be resolved earlier in history
                let line = GeoObject::Line(LineData { p1: *p1, p2: *p2 }); // store by id, same as every other geometric reference
                data.insert_with_id(record.id, line)?; // place it under this tool's assigned id
            }
            ToolKind::AlongLine {
                p1,
                p2,
                length_formula,
                ..
            } => {
                let point1 = resolve_point(&data, *p1)?; // resolve the point this one is measured from
                let point2 = resolve_point(&data, *p2)?; // resolve the point that defines the direction to continue in
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for this tool's formula to reference
                let length = eval_formula(length_formula, &vars)?; // evaluate the length formula
                let dx = point2.x - point1.x; // x component of the p1->p2 direction vector
                let dy = point2.y - point1.y; // y component of the p1->p2 direction vector
                let len = (dx * dx + dy * dy).sqrt(); // magnitude of that direction vector
                if len == 0.0 {
                    // p1 and p2 coincide: there is no direction to measure "along", so the result is undefined
                    return Err(PatternError::DegenerateGeometry(
                        "AlongLine: p1 and p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dir_x = dx / len; // normalized x direction from p1 toward p2
                let dir_y = dy / len; // normalized y direction from p1 toward p2
                let x = point1.x + dir_x * length; // move `length` units from p1 along the normalized direction
                let y = point1.y + dir_y * length; // same, for y
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Normal {
                p1,
                p2,
                length_formula,
                angle_formula,
                ..
            } => {
                let point1 = resolve_point(&data, *p1)?; // resolve the point this one is measured from
                let point2 = resolve_point(&data, *p2)?; // resolve the point that, with p1, defines the line to be perpendicular to
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for these formulas to reference
                let length = eval_formula(length_formula, &vars)?; // evaluate the length formula
                let angle_degrees = eval_formula(angle_formula, &vars)?; // evaluate the additional-rotation formula, in DEGREES (Phase 2's trig convention)
                let dx = point2.x - point1.x; // x component of the p1->p2 direction vector
                let dy = point2.y - point1.y; // y component of the p1->p2 direction vector
                let len = (dx * dx + dy * dy).sqrt(); // magnitude of that direction vector
                if len == 0.0 {
                    // p1 and p2 coincide: there is no p1-p2 line to be perpendicular to, so the result is undefined
                    return Err(PatternError::DegenerateGeometry(
                        "Normal: p1 and p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dir_x = dx / len; // normalized x direction from p1 toward p2
                let dir_y = dy / len; // normalized y direction from p1 toward p2
                let perp_x = -dir_y; // rotate the direction 90 degrees counter-clockwise: x component of the perpendicular
                let perp_y = dir_x; // ...and its y component
                let angle_radians = angle_degrees.to_radians(); // degrees -> radians (Phase 2's convention) before use in cos()/sin() below
                let rotated_x = perp_x * angle_radians.cos() - perp_y * angle_radians.sin(); // standard 2D rotation matrix, x component
                let rotated_y = perp_x * angle_radians.sin() + perp_y * angle_radians.cos(); // standard 2D rotation matrix, y component
                let x = point1.x + rotated_x * length; // move `length` units from p1 along the fully-rotated direction
                let y = point1.y + rotated_y * length; // same, for y
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Bisector {
                p1,
                p2,
                p3,
                length_formula,
                ..
            } => {
                let point1 = resolve_point(&data, *p1)?; // resolve the first angle ray's point
                let point2 = resolve_point(&data, *p2)?; // resolve the angle's vertex
                let point3 = resolve_point(&data, *p3)?; // resolve the second angle ray's point
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for this tool's formula to reference
                let length = eval_formula(length_formula, &vars)?; // evaluate the length formula

                let d1x = point1.x - point2.x; // x component of the vertex->p1 direction vector
                let d1y = point1.y - point2.y; // y component of the vertex->p1 direction vector
                let len1 = (d1x * d1x + d1y * d1y).sqrt(); // magnitude of that direction vector
                if len1 == 0.0 {
                    // p1 and p2 (the vertex) coincide: the first angle ray is undefined
                    return Err(PatternError::DegenerateGeometry(
                        "Bisector: p1 and p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dir1_x = d1x / len1; // normalized vertex->p1 direction, x component
                let dir1_y = d1y / len1; // normalized vertex->p1 direction, y component

                let d2x = point3.x - point2.x; // x component of the vertex->p3 direction vector
                let d2y = point3.y - point2.y; // y component of the vertex->p3 direction vector
                let len2 = (d2x * d2x + d2y * d2y).sqrt(); // magnitude of that direction vector
                if len2 == 0.0 {
                    // p3 and p2 (the vertex) coincide: the second angle ray is undefined
                    return Err(PatternError::DegenerateGeometry(
                        "Bisector: p2 and p3 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dir2_x = d2x / len2; // normalized vertex->p3 direction, x component
                let dir2_y = d2y / len2; // normalized vertex->p3 direction, y component

                let sum_x = dir1_x + dir2_x; // x component of the (unnormalized) bisector direction
                let sum_y = dir1_y + dir2_y; // y component of the (unnormalized) bisector direction
                let sum_len = (sum_x * sum_x + sum_y * sum_y).sqrt(); // magnitude of that sum
                if sum_len == 0.0 {
                    // dir1 and dir2 are exact opposites: p1, p2, p3 are collinear with p2 between p1 and p3,
                    // which means every direction bisects the (straight) angle equally — there is no unique answer
                    return Err(PatternError::DegenerateGeometry(
                        "Bisector: p1, p2, p3 are collinear with p2 between p1 and p3".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let bisector_x = sum_x / sum_len; // normalized bisector direction, x component
                let bisector_y = sum_y / sum_len; // normalized bisector direction, y component
                let x = point2.x + bisector_x * length; // move `length` units from the vertex along the bisector direction
                let y = point2.y + bisector_y * length; // same, for y
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Height {
                point,
                line_p1,
                line_p2,
                ..
            } => {
                let target = resolve_point(&data, *point)?; // resolve the point being projected
                let line1 = resolve_point(&data, *line_p1)?; // resolve one point defining the line being projected onto
                let line2 = resolve_point(&data, *line_p2)?; // resolve the other point defining the line being projected onto
                                                             // no formula fields on Height: it's pure vector projection, nothing to evaluate

                let dx = line2.x - line1.x; // x component of the line_p1->line_p2 direction vector
                let dy = line2.y - line1.y; // y component of the line_p1->line_p2 direction vector
                let len = (dx * dx + dy * dy).sqrt(); // magnitude of that direction vector
                if len == 0.0 {
                    // line_p1 and line_p2 coincide: there is no line to project onto
                    return Err(PatternError::DegenerateGeometry(
                        "Height: line_p1 and line_p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dir_x = dx / len; // normalized line direction, x component
                let dir_y = dy / len; // normalized line direction, y component
                let t = (target.x - line1.x) * dir_x + (target.y - line1.y) * dir_y; // dot product: how far along the line direction the projection lands
                let x = line1.x + dir_x * t; // the projected point's x coordinate
                let y = line1.y + dir_y * t; // the projected point's y coordinate
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Midpoint { p1, p2, .. } => {
                let point1 = resolve_point(&data, *p1)?; // resolve the segment's first endpoint
                let point2 = resolve_point(&data, *p2)?; // resolve the segment's second endpoint
                                                         // no formula fields, and no degenerate case possible: the midpoint of any two points (even coincident ones) is always well-defined
                let x = (point1.x + point2.x) / 2.0; // the midpoint's x coordinate
                let y = (point1.y + point2.y) / 2.0; // the midpoint's y coordinate
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
        }
    }

    Ok(data) // every tool resolved without error; hand back the fully-built cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ToolRecord;
    use crate::variable::Variable;

    // Shared setup: BasePoint "A" at (base_x, base_y), an EndLine "A1" at
    // angle 0 and length height_scapula/10 from A, and a Line from A to
    // A1. Returns the Document plus the ids of A, A1, and the Line, so
    // tests can look up specific resolved objects without recomputing the
    // ids by hand.
    fn sample_document(base_x: f64, base_y: f64) -> (Document, ObjectId, ObjectId, ObjectId) {
        let mut doc = Document::default();
        let a = doc.add_tool(ToolKind::BasePoint {
            name: "A".to_string(),
            x: base_x,
            y: base_y,
        });
        doc.set_variable("height_scapula", Variable::Measurement { value: 40.0 });
        let a1 = doc.add_tool(ToolKind::EndLine {
            name: "A1".to_string(),
            base_point: a,
            angle_formula: "0".to_string(),
            length_formula: "height_scapula/10".to_string(),
        });
        let line = doc.add_tool(ToolKind::Line { p1: a, p2: a1 });
        (doc, a, a1, line)
    }

    #[test]
    fn recompute_resolves_end_line_and_line_from_formulas() {
        let (doc, a, a1, line) = sample_document(0.0, 0.0);
        let data = recompute_all(&doc).unwrap();

        let resolved_a1 = data.get_point(a1).unwrap();
        assert!((resolved_a1.x - 4.0).abs() < 1e-9);
        assert!((resolved_a1.y - 0.0).abs() < 1e-9);

        let resolved_line = data.get_line(line).unwrap();
        assert_eq!(resolved_line.p1, a);
        assert_eq!(resolved_line.p2, a1);
    }

    #[test]
    fn recompute_reports_missing_dependency_instead_of_panicking() {
        let mut doc = Document::default();
        let a = doc.add_tool(ToolKind::BasePoint {
            name: "A".to_string(),
            x: 0.0,
            y: 0.0,
        });
        // `phantom` was never produced by any add_tool call on this Document.
        let phantom = ObjectId::new(9999);
        doc.push_record_for_test(ToolRecord {
            id: ObjectId::new(1000),
            kind: ToolKind::Line { p1: a, p2: phantom },
        });

        let err = recompute_all(&doc).unwrap_err();
        assert_eq!(err, PatternError::MissingDependency(phantom));
    }

    #[test]
    fn recompute_is_deterministic() {
        let (doc, ..) = sample_document(0.0, 0.0);
        let first = recompute_all(&doc).unwrap();
        let second = recompute_all(&doc).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn recompute_cascades_base_point_changes_downstream() {
        let (doc_a, _, a1_a, _) = sample_document(0.0, 0.0);
        let (doc_b, _, a1_b, _) = sample_document(10.0, 5.0);

        let data_a = recompute_all(&doc_a).unwrap();
        let data_b = recompute_all(&doc_b).unwrap();

        let point_a = data_a.get_point(a1_a).unwrap();
        let point_b = data_b.get_point(a1_b).unwrap();
        assert_ne!(point_a, point_b);

        // Sanity-check the actual expected values, not just "not equal".
        assert!((point_a.x - 4.0).abs() < 1e-9);
        assert!((point_b.x - 14.0).abs() < 1e-9);
        assert!((point_b.y - 5.0).abs() < 1e-9);
    }

    // Golden hand-calculated tests for the five Phase 10a tools, all using
    // non-axis-aligned inputs (p1/p2/p3 never share an x or y coordinate)
    // so the tests can't pass "by accident" on a degenerate horizontal or
    // vertical special case.

    #[test]
    fn along_line_golden_value_non_axis_aligned() {
        // p1=(1,1), p2=(4,5): a 3-4-5 right triangle offset off the axes,
        // giving an exact, hand-verifiable normalized direction (0.6, 0.8).
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 1.0, 1.0);
        let p2 = doc.add_base_point("P2", 4.0, 5.0);
        let along = doc.add_along_line("AL", p1, p2, "10").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(along).unwrap();
        // Hand-verified: (1,1) + 10*(0.6,0.8) = (1+6, 1+8) = (7,9).
        assert!((point.x - 7.0).abs() < 1e-9);
        assert!((point.y - 9.0).abs() < 1e-9);
    }

    #[test]
    fn along_line_coincident_points_are_degenerate() {
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 3.0, 3.0);
        let p2 = doc.add_base_point("P2", 3.0, 3.0); // exactly coincides with p1
        doc.add_along_line("AL", p1, p2, "10").unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn normal_golden_value_non_axis_aligned() {
        // Same 3-4-5 base as along_line's test: p1=(1,1), p2=(4,5), direction (0.6,0.8).
        // angle=90 degrees rotates the perpendicular by a further 90 degrees,
        // landing on -direction: perp=(-0.8,0.6) rotated 90 more = (-0.6,-0.8).
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 1.0, 1.0);
        let p2 = doc.add_base_point("P2", 4.0, 5.0);
        let normal = doc.add_normal("N", p1, p2, "10", "90").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(normal).unwrap();
        // Hand-verified: (1,1) + 10*(-0.6,-0.8) = (1-6, 1-8) = (-5,-7).
        assert!((point.x - -5.0).abs() < 1e-9);
        assert!((point.y - -7.0).abs() < 1e-9);
    }

    #[test]
    fn normal_coincident_points_are_degenerate() {
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 2.0, 2.0);
        let p2 = doc.add_base_point("P2", 2.0, 2.0); // exactly coincides with p1
        doc.add_normal("N", p1, p2, "10", "0").unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn bisector_golden_value_non_axis_aligned() {
        // Vertex p2=(1,1). p1=(4,5): dir1 = normalize(4-1,5-1) = normalize(3,4) = (0.6,0.8).
        // p3=(5,4): dir2 = normalize(5-1,4-1) = normalize(4,3) = (0.8,0.6).
        // dir1 and dir2 are mirror images across the vertex's 45-degree line,
        // so their bisector direction is the exact diagonal (sqrt(2)/2, sqrt(2)/2).
        let mut doc = Document::default();
        let p2 = doc.add_base_point("P2", 1.0, 1.0); // the angle vertex
        let p1 = doc.add_base_point("P1", 4.0, 5.0);
        let p3 = doc.add_base_point("P3", 5.0, 4.0);
        let bisector = doc.add_bisector("B", p1, p2, p3, "10").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(bisector).unwrap();
        let expected = 1.0 + 5.0 * 2.0_f64.sqrt(); // (1,1) + 10*(sqrt(2)/2, sqrt(2)/2), same value for both coordinates
        assert!((point.x - expected).abs() < 1e-9);
        assert!((point.y - expected).abs() < 1e-9);
    }

    #[test]
    fn bisector_collinear_opposite_points_are_degenerate() {
        // p2=(0,0) is the vertex; p1=(-1,0) and p3=(1,0) are collinear with
        // p2 exactly between them, so dir1 and dir2 are exact opposites.
        let mut doc = Document::default();
        let p2 = doc.add_base_point("P2", 0.0, 0.0);
        let p1 = doc.add_base_point("P1", -1.0, 0.0);
        let p3 = doc.add_base_point("P3", 1.0, 0.0);
        doc.add_bisector("B", p1, p2, p3, "10").unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn height_golden_value_non_axis_aligned() {
        // Line through (0,0) and (4,3): a 3-4-5 triangle, direction (0.8,0.6).
        // Projecting (4,0) onto that line: t = 4*0.8+0*0.6 = 3.2,
        // foot = (0,0) + 3.2*(0.8,0.6) = (2.56,1.92).
        let mut doc = Document::default();
        let line_p1 = doc.add_base_point("L1", 0.0, 0.0);
        let line_p2 = doc.add_base_point("L2", 4.0, 3.0);
        let off_line = doc.add_base_point("P", 4.0, 0.0);
        let height = doc.add_height("H", off_line, line_p1, line_p2).unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(height).unwrap();
        assert!((point.x - 2.56).abs() < 1e-9);
        assert!((point.y - 1.92).abs() < 1e-9);
    }

    #[test]
    fn height_point_already_on_the_line_returns_that_same_point() {
        // Same line as the golden-value test above; (8,6) continues the
        // same (0.8,0.6) direction from (0,0), so it's already exactly on the line.
        let mut doc = Document::default();
        let line_p1 = doc.add_base_point("L1", 0.0, 0.0);
        let line_p2 = doc.add_base_point("L2", 4.0, 3.0);
        let on_line = doc.add_base_point("P", 8.0, 6.0);
        let height = doc.add_height("H", on_line, line_p1, line_p2).unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(height).unwrap();
        assert!((point.x - 8.0).abs() < 1e-9);
        assert!((point.y - 6.0).abs() < 1e-9);
    }

    #[test]
    fn height_coincident_line_points_are_degenerate() {
        let mut doc = Document::default();
        let line_p1 = doc.add_base_point("L1", 1.0, 1.0);
        let line_p2 = doc.add_base_point("L2", 1.0, 1.0); // coincides with line_p1
        let target = doc.add_base_point("P", 5.0, 5.0);
        doc.add_height("H", target, line_p1, line_p2).unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn midpoint_golden_value_non_axis_aligned() {
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 1.0, 3.0);
        let p2 = doc.add_base_point("P2", 7.0, 9.0);
        let mid = doc.add_midpoint("M", p1, p2).unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(mid).unwrap();
        assert!((point.x - 4.0).abs() < 1e-9);
        assert!((point.y - 6.0).abs() < 1e-9);
    }

    #[test]
    fn along_line_length_formula_reacts_to_measurement_changes() {
        // Simple axis-aligned direction here (this test is about formula
        // reactivity, not geometry correctness — that's covered separately
        // by along_line_golden_value_non_axis_aligned above).
        let mut doc = Document::default();
        doc.set_variable("chest_width", Variable::Measurement { value: 5.0 });
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p2 = doc.add_base_point("P2", 1.0, 0.0); // direction (1,0)
        let along = doc.add_along_line("AL", p1, p2, "chest_width").unwrap();

        let before = recompute_all(&doc).unwrap();
        let point_before = before.get_point(along).unwrap();
        assert!((point_before.x - 5.0).abs() < 1e-9);
        assert!((point_before.y - 0.0).abs() < 1e-9);

        // Simulates what Phase 5/6's measurement-reload pipeline would do.
        doc.set_variable("chest_width", Variable::Measurement { value: 8.0 });
        let after = recompute_all(&doc).unwrap();
        let point_after = after.get_point(along).unwrap();
        assert!((point_after.x - 8.0).abs() < 1e-9);
        assert!((point_after.y - 0.0).abs() < 1e-9);
        assert_ne!(point_before, point_after); // the resolved geometry actually changed
    }
}
