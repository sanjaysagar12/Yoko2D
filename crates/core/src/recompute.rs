use thiserror::Error; // brings in the `Error` derive macro used by `PatternError` below

use crate::document::{Document, ToolKind}; // the ordered tool history this module walks
use crate::formula::{eval_formula, flatten_variables}; // Phase 2's formula pipeline, used by EndLine
use crate::{
    ArcData, ContainerError, GeoObject, LineData, ObjectId, PatternData, PieceData, PointData,
    SplineData,
}; // Phase 1's container and geometry types, plus Part A/B's new curve GeoObject payloads

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
                let dir = crate::geometry::direction_from_angle_deg(angle); // shared angle-to-unit-direction helper, also used by Part A/B's Arc/Spline arms below
                let x = base.x + length * dir.0; // base + length*cos(angle)
                let y = base.y + length * dir.1; // base + length*sin(angle)
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
            ToolKind::Piece {
                nodes,
                seam_allowance_formula,
                ..
            } => {
                let mut contour = Vec::with_capacity(nodes.len()); // accumulates each node's resolved (x, y) coordinates, in node order
                for node in nodes {
                    // resolve every boundary vertex's point, in node order
                    let resolved = resolve_point(&data, node.point)?; // reuses the same missing-dependency mapping every other arm relies on
                    contour.push((resolved.x, resolved.y)); // collect just the coordinates; the node's id/exclusion flag aren't part of the resolved contour itself
                }
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for the seam-allowance formula to reference
                let width = eval_formula(seam_allowance_formula, &vars)?; // evaluate the seam-allowance width formula
                                                                          // NOTE: `node.excluded_from_seam_allowance` is not consulted here — every
                                                                          // edge is always offset regardless of the flag. Properly respecting
                                                                          // per-edge exclusion would require `offset_polygon` to accept a
                                                                          // per-edge inclusion mask rather than always offsetting every edge,
                                                                          // which is out of scope for this phase (see the module's scope note);
                                                                          // this is a TODO for a future refinement.
                let seam_allowance = if width == 0.0 {
                    None // a zero-width seam allowance means no offset polygon to compute
                } else {
                    Some(crate::geometry::offset_polygon(&contour, width)?) // propagate any degenerate-geometry failure via ?
                };
                let piece = GeoObject::Piece(PieceData {
                    contour,
                    seam_allowance,
                }); // the resolved piece for this tool
                data.insert_with_id(record.id, piece)?; // place it under this tool's assigned id
            }
            ToolKind::ShoulderPoint {
                p1_line,
                p2_line,
                shoulder,
                length_formula,
                ..
            } => {
                let point1 = resolve_point(&data, *p1_line)?; // resolve the ray's starting point
                let point2 = resolve_point(&data, *p2_line)?; // resolve the point defining the ray's direction
                let shoulder_point = resolve_point(&data, *shoulder)?; // resolve the circle's center
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for this tool's formula to reference
                let radius = eval_formula(length_formula, &vars)?; // evaluate the circle-radius formula
                if radius <= 0.0 {
                    // a non-positive radius has no sensible "point on the circle" answer for this tool
                    return Err(PatternError::DegenerateGeometry(
                        "ShoulderPoint: length must evaluate to a positive radius".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dx = point2.x - point1.x; // x component of the p1_line->p2_line direction vector
                let dy = point2.y - point1.y; // y component of the p1_line->p2_line direction vector
                let base_length = (dx * dx + dy * dy).sqrt(); // the distance from p1_line to p2_line
                if base_length == 0.0 {
                    // p1_line and p2_line coincide: there is no ray to intersect the circle with
                    return Err(PatternError::DegenerateGeometry(
                        "ShoulderPoint: p1_line and p2_line are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dir_x = dx / base_length; // normalized ray direction, x component
                let dir_y = dy / base_length; // normalized ray direction, y component
                let candidates = crate::geometry::line_circle_intersection(
                    (point1.x, point1.y),
                    (dir_x, dir_y),
                    (shoulder_point.x, shoulder_point.y),
                    radius,
                ); // 0, 1, or 2 points where the ray's infinite line crosses the circle
                   // Mirrors VToolShoulderPoint::FindPoint's own selection: among the
                   // circle-line intersection candidates (checked in the same order
                   // LineIntersectCircle itself returns them), pick the first one that
                   // lies FARTHER than base_length from p1_line, in the ray's own
                   // forward direction — i.e. genuinely past p2_line, not behind it.
                let chosen = candidates.into_iter().find(|&(cx, cy)| {
                    let forward = (cx - point1.x) * dir_x + (cy - point1.y) * dir_y; // signed distance from p1_line along the ray direction
                    forward > base_length // strictly farther along the ray than p2_line itself
                });
                let Some((x, y)) = chosen else {
                    // Deliberate deviation from Seamly2D, which silently falls back to
                    // p2_line's own position in this case rather than reporting a
                    // failure — this codebase always reports an ill-defined
                    // configuration as a typed error instead of substituting a
                    // plausible-looking but wrong value.
                    return Err(PatternError::DegenerateGeometry(
                        "ShoulderPoint: the circle does not reach far enough along the ray past p2_line".to_string(),
                    ));
                };
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::LineIntersect {
                p1_line1,
                p2_line1,
                p1_line2,
                p2_line2,
                ..
            } => {
                let a1 = resolve_point(&data, *p1_line1)?; // resolve the first line's first defining point
                let a2 = resolve_point(&data, *p2_line1)?; // resolve the first line's second defining point
                let b1 = resolve_point(&data, *p1_line2)?; // resolve the second line's first defining point
                let b2 = resolve_point(&data, *p2_line2)?; // resolve the second line's second defining point
                let d1 = (a2.x - a1.x, a2.y - a1.y); // the first line's direction vector
                let d2 = (b2.x - b1.x, b2.y - b1.y); // the second line's direction vector
                let (x, y) =
                    crate::geometry::line_intersection((a1.x, a1.y), d1, (b1.x, b1.y), d2)?; // propagate a parallel/collinear failure via ? (also covers a coincident-point degenerate direction, since that yields a zero cross-product too)
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::PointOfIntersection { p1, p2, .. } => {
                let point1 = resolve_point(&data, *p1)?; // resolve the point this one takes its x coordinate from
                let point2 = resolve_point(&data, *p2)?; // resolve the point this one takes its y coordinate from
                                                         // no formula fields, and no degenerate case possible: combining any two points' x/y coordinates (even the same point twice) is always well-defined
                let point = GeoObject::Point(PointData {
                    x: point1.x, // literally p1's x coordinate
                    y: point2.y, // literally p2's y coordinate
                });
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Triangle {
                axis_p1,
                axis_p2,
                hypotenuse_p1,
                hypotenuse_p2,
                ..
            } => {
                let ap1 = resolve_point(&data, *axis_p1)?; // resolve the reference line's first defining point
                let ap2 = resolve_point(&data, *axis_p2)?; // resolve the reference line's second defining point
                let hp1 = resolve_point(&data, *hypotenuse_p1)?; // resolve one endpoint of the segment forming the right angle
                let hp2 = resolve_point(&data, *hypotenuse_p2)?; // resolve the other endpoint of that segment

                let axis_dx = ap2.x - ap1.x; // x component of the axis_p1->axis_p2 direction vector
                let axis_dy = ap2.y - ap1.y; // y component of the axis_p1->axis_p2 direction vector
                let axis_len = (axis_dx * axis_dx + axis_dy * axis_dy).sqrt(); // the axis line's own length
                if axis_len == 0.0 {
                    // axis_p1 and axis_p2 coincide: there is no reference line at all
                    return Err(PatternError::DegenerateGeometry(
                        "Triangle: axis_p1 and axis_p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }

                let hyp_dx = hp2.x - hp1.x; // x component of the hypotenuse_p1->hypotenuse_p2 direction vector
                let hyp_dy = hp2.y - hp1.y; // y component of the hypotenuse_p1->hypotenuse_p2 direction vector
                let hyp_len = (hyp_dx * hyp_dx + hyp_dy * hyp_dy).sqrt(); // the hypotenuse segment's own length
                if hyp_len == 0.0 {
                    // hypotenuse_p1 and hypotenuse_p2 coincide: there is no segment to form a right angle with
                    return Err(PatternError::DegenerateGeometry(
                        "Triangle: hypotenuse_p1 and hypotenuse_p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }

                let cross = axis_dx * hyp_dy - axis_dy * hyp_dx; // the 2D cross product of the two directions; zero exactly when they're parallel
                if cross.abs() < 1e-9 {
                    // the axis and the hypotenuse never cross at a unique point, so there's no startPoint to search forward from
                    return Err(PatternError::DegenerateGeometry(
                        "Triangle: axis and hypotenuse are parallel".to_string(), // names exactly which geometric configuration failed
                    ));
                }

                let (start_x, start_y) = crate::geometry::line_intersection(
                    (ap1.x, ap1.y),
                    (axis_dx, axis_dy),
                    (hp1.x, hp1.y),
                    (hyp_dx, hyp_dy),
                )?; // where the axis crosses the hypotenuse; propagate the (already-guarded-against-above) parallel failure via ? defensively

                // Seamly2D's VToolTriangle::FindPoint numerically searches
                // outward from `startPoint` along the axis, 1 pixel at a
                // time, for the first point where the law-of-cosines angle
                // at that point (opposite the fixed hypotenuse length) drops
                // to <=90 degrees — by Thales' theorem, that condition holds
                // exactly on the circle whose diameter is the hypotenuse
                // segment, so this computes that crossing directly instead
                // of searching for it.
                let center_x = (hp1.x + hp2.x) / 2.0; // the Thales circle's center: the hypotenuse segment's own midpoint
                let center_y = (hp1.y + hp2.y) / 2.0;
                let radius = hyp_len / 2.0; // the Thales circle's radius: half the hypotenuse segment's length

                let axis_unit = (axis_dx / axis_len, axis_dy / axis_len); // normalized axis direction, for the circle-intersection helper
                let candidates = crate::geometry::line_circle_intersection(
                    (ap1.x, ap1.y),
                    axis_unit,
                    (center_x, center_y),
                    radius,
                ); // 0, 1, or 2 points where the axis crosses the Thales circle

                // Seamly2D's search only ever steps FORWARD from startPoint
                // (in the axis_p1->axis_p2 direction), so among the circle
                // candidates, pick whichever one lies strictly ahead of
                // startPoint in that same direction.
                let chosen = candidates.into_iter().find(|&(cx, cy)| {
                    let forward = (cx - start_x) * axis_dx + (cy - start_y) * axis_dy; // signed projection of (candidate - startPoint) onto the (non-unit, but sign-preserving) axis direction
                    forward > 1e-9
                });
                let Some((x, y)) = chosen else {
                    // Seamly2D's own numeric search has no bound on this
                    // case (it would loop forever rather than terminate) —
                    // reported here as a typed error instead.
                    return Err(PatternError::DegenerateGeometry(
                        "Triangle: axis never reaches the right-angle point in the forward direction".to_string(),
                    ));
                };
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::PointOfContact {
                center,
                p1,
                p2,
                radius_formula,
                ..
            } => {
                let center_point = resolve_point(&data, *center)?; // resolve the circle's center
                let point1 = resolve_point(&data, *p1)?; // resolve the segment's first endpoint
                let point2 = resolve_point(&data, *p2)?; // resolve the segment's second endpoint
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for this tool's formula to reference
                let radius = eval_formula(radius_formula, &vars)?; // evaluate the circle-radius formula
                if radius < 0.0 {
                    // a negative radius has no sensible geometric meaning for a circle
                    return Err(PatternError::DegenerateGeometry(
                        "PointOfContact: radius must evaluate to a non-negative value".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let dx = point2.x - point1.x; // x component of the p1->p2 direction vector
                let dy = point2.y - point1.y; // y component of the p1->p2 direction vector
                let seg_len = (dx * dx + dy * dy).sqrt(); // the segment's own length
                if seg_len == 0.0 {
                    // p1 and p2 coincide: there is no segment for the circle to cross
                    return Err(PatternError::DegenerateGeometry(
                        "PointOfContact: p1 and p2 are coincident".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let unit = (dx / seg_len, dy / seg_len); // normalized p1->p2 direction, for the circle-intersection helper
                let candidates = crate::geometry::line_circle_intersection(
                    (point1.x, point1.y),
                    unit,
                    (center_point.x, center_point.y),
                    radius,
                ); // 0, 1, or 2 points where the circle crosses the infinite line through p1/p2
                if candidates.is_empty() {
                    // the circle never reaches the line at all
                    return Err(PatternError::DegenerateGeometry(
                        "PointOfContact: the circle does not intersect the line through p1 and p2"
                            .to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let (x, y) = if candidates.len() == 1 {
                    candidates[0] // tangent: only one candidate exists at all
                } else {
                    // Mirrors VToolPointOfContact::FindPoint's own
                    // disambiguation: prefer whichever candidate actually
                    // lies within the finite segment [p1, p2] (not just
                    // somewhere on the infinite line through it); if both
                    // or neither qualify, prefer whichever is closer to p1.
                    let param = |candidate: (f64, f64)| {
                        (candidate.0 - point1.x) * unit.0 + (candidate.1 - point1.y) * unit.1
                        // signed distance from p1 along the p1->p2 direction; since `candidate` is already known to lie on this exact line, this equals its true Euclidean distance from p1 whenever it's positive
                    };
                    let on_segment = |t: f64| (-1e-9..=seg_len + 1e-9).contains(&t); // within [0, seg_len], with a small tolerance for float error at the endpoints
                    let t0 = param(candidates[0]);
                    let t1 = param(candidates[1]);
                    let on0 = on_segment(t0);
                    let on1 = on_segment(t1);
                    if on0 == on1 {
                        // both candidates are on the segment, or neither is: fall back to whichever is closer to p1
                        if t0.abs() <= t1.abs() {
                            candidates[0]
                        } else {
                            candidates[1]
                        }
                    } else if on0 {
                        candidates[0] // exactly one candidate is truly on the segment: prefer it
                    } else {
                        candidates[1]
                    }
                };
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Arc {
                center,
                radius_formula,
                start_angle_formula,
                end_angle_formula,
                ..
            } => {
                let center_point = resolve_point(&data, *center)?; // resolve the circle's center
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for these formulas to reference
                let radius = eval_formula(radius_formula, &vars)?; // evaluate the radius formula
                let start_angle_deg = eval_formula(start_angle_formula, &vars)?; // evaluate the sweep's starting-angle formula, in degrees
                let end_angle_deg = eval_formula(end_angle_formula, &vars)?; // evaluate the sweep's ending-angle formula, in degrees
                if radius <= 0.0 {
                    // mirrors ShoulderPoint's own guard above: a non-positive radius has no sensible arc
                    return Err(PatternError::DegenerateGeometry(
                        "Arc: radius must evaluate to a positive value".to_string(), // names exactly which geometric configuration failed
                    ));
                }
                let arc = GeoObject::Arc(ArcData {
                    center: (center_point.x, center_point.y), // the arc's own center, already resolved
                    radius,                                   // validated positive above
                    start_angle_deg, // the sweep's starting angle, in degrees
                    end_angle_deg,   // the sweep's ending angle, in degrees
                });
                data.insert_with_id(record.id, arc)?; // place it under this tool's assigned id
            }
            ToolKind::Spline {
                p1,
                p4,
                angle1_formula,
                length1_formula,
                angle2_formula,
                length2_formula,
                ..
            } => {
                let point1 = resolve_point(&data, *p1)?; // resolve the curve's first endpoint
                let point4 = resolve_point(&data, *p4)?; // resolve the curve's second endpoint
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for these formulas to reference
                let angle1 = eval_formula(angle1_formula, &vars)?; // evaluate p1's own tangent-angle formula, in degrees
                let length1 = eval_formula(length1_formula, &vars)?; // evaluate p1's own control-handle length formula
                let angle2 = eval_formula(angle2_formula, &vars)?; // evaluate p4's own tangent-angle formula, in degrees
                let length2 = eval_formula(length2_formula, &vars)?; // evaluate p4's own control-handle length formula
                let dir1 = crate::geometry::direction_from_angle_deg(angle1); // p1->p2 unit direction, mirrors VSpline::GetP2's own QLineF::setAngle convention
                let dir2 = crate::geometry::direction_from_angle_deg(angle2); // p4->p3 unit direction, mirrors VSpline::GetP3's own convention
                let p2 = (point1.x + length1 * dir1.0, point1.y + length1 * dir1.1); // P2 = P1 + length1*(cos(angle1),sin(angle1))
                let p3 = (point4.x + length2 * dir2.0, point4.y + length2 * dir2.1); // P3 = P4 + length2*(cos(angle2),sin(angle2))
                let spline = GeoObject::Spline(SplineData {
                    p1: (point1.x, point1.y), // the curve's first endpoint, already resolved
                    p2,                       // derived interior control point, computed above
                    p3,                       // derived interior control point, computed above
                    p4: (point4.x, point4.y), // the curve's second endpoint, already resolved
                });
                data.insert_with_id(record.id, spline)?; // place it under this tool's assigned id
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

    // Cross-checks the two code paths against each other: the Piece tool's
    // resolved contour/seam_allowance must match calling
    // crate::geometry::offset_polygon directly on the same input points.
    #[test]
    fn piece_contour_and_seam_allowance_match_offset_polygon_directly() {
        use crate::document::PieceNode;

        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 10.0, 0.0);
        let c = doc.add_base_point("C", 0.0, 10.0);
        let piece = doc
            .add_piece(
                "Piece1",
                vec![
                    PieceNode {
                        point: a,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: b,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: c,
                        excluded_from_seam_allowance: false,
                    },
                ],
                "1.0",
            )
            .unwrap();

        let data = recompute_all(&doc).unwrap();
        let resolved = data.get_piece(piece).unwrap();

        let expected_contour = vec![(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)];
        assert_eq!(resolved.contour, expected_contour);

        let expected_offset = crate::geometry::offset_polygon(&expected_contour, 1.0).unwrap();
        assert_eq!(resolved.seam_allowance, Some(expected_offset));
    }

    #[test]
    fn piece_seam_allowance_formula_reacts_to_measurement_changes() {
        use crate::document::PieceNode;

        let mut doc = Document::default();
        doc.set_variable("seam_width", Variable::Measurement { value: 1.0 });
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 10.0, 0.0);
        let c = doc.add_base_point("C", 0.0, 10.0);
        let piece = doc
            .add_piece(
                "Piece1",
                vec![
                    PieceNode {
                        point: a,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: b,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: c,
                        excluded_from_seam_allowance: false,
                    },
                ],
                "seam_width",
            )
            .unwrap();

        let before = recompute_all(&doc).unwrap();
        let sa_before = before
            .get_piece(piece)
            .unwrap()
            .seam_allowance
            .clone()
            .unwrap();

        doc.set_variable("seam_width", Variable::Measurement { value: 2.0 });
        let after = recompute_all(&doc).unwrap();
        let sa_after = after
            .get_piece(piece)
            .unwrap()
            .seam_allowance
            .clone()
            .unwrap();

        assert_ne!(sa_before, sa_after);
    }

    // ===================================================================
    // Part B: hand-verified parity checks against Seamly2D's actual C++
    // source, for the six tools this project already implements. Each
    // test's comment shows the hand calculation performed against the
    // cited Seamly2D function, independently of this crate's own
    // implementation.
    // ===================================================================

    #[test]
    fn along_line_parity_extrapolates_and_shortens_like_qlinef_setlength() {
        // Seamly2D: VToolAlongLine::Create builds `QLineF line(p1, p2);
        // line.setLength(result);` — Qt's QLineF::setLength keeps the
        // line's existing ANGLE (the p1->p2 direction) and simply rescales
        // p2 to the new length; it never clamps to the original p1-p2
        // distance, so a length longer than the original extrapolates past
        // p2, and a length shorter lands short of it.
        //
        // p1=(2,3), p2=(6,6): dx=4, dy=3, distance=5, unit direction=(0.8,0.6).
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 2.0, 3.0);
        let p2 = doc.add_base_point("P2", 6.0, 6.0);

        // length=20, LONGER than the original 5-unit p1-p2 distance:
        // hand-calculated (2,3) + 20*(0.8,0.6) = (2+16, 3+12) = (18,15).
        let longer = doc.add_along_line("Longer", p1, p2, "20").unwrap();
        // length=2, SHORTER than the original 5-unit distance:
        // hand-calculated (2,3) + 2*(0.8,0.6) = (2+1.6, 3+1.2) = (3.6,4.2).
        let shorter = doc.add_along_line("Shorter", p1, p2, "2").unwrap();

        let data = recompute_all(&doc).unwrap();
        let longer_point = data.get_point(longer).unwrap();
        assert!((longer_point.x - 18.0).abs() < 1e-9);
        assert!((longer_point.y - 15.0).abs() < 1e-9);
        let shorter_point = data.get_point(shorter).unwrap();
        assert!((shorter_point.x - 3.6).abs() < 1e-9);
        assert!((shorter_point.y - 4.2).abs() < 1e-9);
    }

    #[test]
    fn normal_parity_matches_qlinef_normalvector_convention() {
        // Seamly2D: VToolNormal::FindPoint does
        // `QLineF normal = line.normalVector(); normal.setAngle(normal.angle()+angle);
        // normal.setLength(length); return normal.p2();` where
        // `line = QLineF(firstPoint, secondPoint)`.
        //
        // Qt's own docs: QLineF::normalVector() "Returns a line that is
        // perpendicular to this line, with the same starting point and
        // length, obtained by rotating this line 90 degrees counterclockwise";
        // QLineF::angle()/setAngle() work in Qt's own SCREEN-Y-DOWN raw
        // coordinates via `atan2(-dy, dx)` (a deliberate negation Qt applies
        // specifically so its angle values already read as a conventional,
        // "up is positive" measure despite the underlying y-down pixels).
        // Converting that 90-degree-CCW rotation into THIS crate's plain
        // y-up (x,y) convention (Yoko2D never negates y until camera.rs's
        // final screen-paint step) shows it lands on the SAME 90-degree
        // counter-clockwise rotation Yoko2D's own Normal arm already uses
        // (perp_x=-dir_y, perp_y=dir_x) — Qt's angle-based construction and
        // Yoko2D's plain trig turn out to agree exactly, with no extra sign
        // flip needed anywhere in the chain.
        //
        // Chosen test case: firstPoint=(0,0), secondPoint=(10,0), angle=0,
        // length=5 — deliberately asymmetric (a sign error here would flip
        // the result to (0,-5) instead of the correct (0,5), a clearly
        // different, visibly mirrored point).
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p2 = doc.add_base_point("P2", 10.0, 0.0);
        let normal = doc.add_normal("N", p1, p2, "5", "0").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(normal).unwrap();
        assert!((point.x - 0.0).abs() < 1e-9);
        assert!((point.y - 5.0).abs() < 1e-9); // NOT -5.0: confirms the rotation direction matches Seamly2D's actual behavior
    }

    #[test]
    fn end_line_parity_matches_qlinef_setangle_convention_no_sign_flip() {
        // Seamly2D: VToolEndLine::Create builds `QLineF line; line.setAngle(angle);
        // line.setLength(length);` starting from `basePoint`. Per Qt's own docs,
        // `QLineF::setAngle` measures its `angle` counter-clockwise in a
        // conventional (mathematical, "up is positive") sense despite Qt's
        // underlying pixel coordinates being y-down internally — the same
        // `atan2(-dy, dx)` deliberate negation already cited in the Normal
        // parity test above. Converting that into Yoko2D's plain y-up (x,y)
        // convention (never negated until camera.rs's final screen-paint
        // step) means `dx = length*cos(angle)`, `dy = length*sin(angle)`
        // with NO extra sign flip — exactly the plain trig formula
        // `recompute_all`'s `ToolKind::EndLine` arm already uses.
        //
        // The ONLY existing EndLine test before this one
        // (`recompute_resolves_end_line_and_line_from_formulas`, using
        // angle=0) cannot actually catch a sign error in the sine term:
        // sin(0)=0 regardless of which sign convention is used. This test
        // picks two non-trivial angles specifically to discriminate a
        // sign-flipped or swapped-axis implementation from the correct one.
        let mut doc = Document::default();

        // angle=90 degrees, length=5, base=(0,0): a pure-y case that would
        // visibly mirror to (0,-5) if the y-component's sign were flipped
        // (the exact same discriminating shape as the Normal parity test).
        let base1 = doc.add_base_point("Base1", 0.0, 0.0);
        let ninety = doc.add_end_line("E90", base1, "90", "5").unwrap();

        // angle=30 degrees, length=10, base=(2,3): both x and y components
        // are nonzero and unequal, so this also catches a cos/sin axis swap,
        // not just a sign flip.
        let base2 = doc.add_base_point("Base2", 2.0, 3.0);
        let thirty = doc.add_end_line("E30", base2, "30", "10").unwrap();

        let data = recompute_all(&doc).unwrap();

        let point90 = data.get_point(ninety).unwrap();
        // Hand-verified: (0,0) + 5*(cos(90deg),sin(90deg)) = (0,0)+5*(0,1) = (0,5).
        assert!((point90.x - 0.0).abs() < 1e-9);
        assert!((point90.y - 5.0).abs() < 1e-9); // NOT -5.0: confirms no sign flip vs. Seamly2D's setAngle convention

        let point30 = data.get_point(thirty).unwrap();
        // Hand-verified: (2,3) + 10*(cos(30deg),sin(30deg))
        // = (2,3) + 10*(sqrt(3)/2, 0.5) = (2 + 5*sqrt(3), 3 + 5).
        let expected_x = 2.0 + 5.0 * 3.0_f64.sqrt();
        let expected_y = 8.0;
        assert!((point30.x - expected_x).abs() < 1e-9);
        assert!((point30.y - expected_y).abs() < 1e-9);
    }

    /// Independently verifies that `candidate` truly bisects the angle
    /// p1-p2-p3 (p2 the vertex) at the given `length` from p2, WITHOUT
    /// relying on either Seamly2D's QLineF-angle formula or this crate's
    /// own vector-sum implementation: checks the definition directly — the
    /// angle between (p1-p2) and (candidate-p2) equals the angle between
    /// (candidate-p2) and (p3-p2), via the dot-product angle-between-vectors
    /// formula, and that `candidate` is exactly `length` from p2.
    fn assert_is_a_true_angle_bisector(
        p1: (f64, f64),
        p2: (f64, f64),
        p3: (f64, f64),
        length: f64,
        candidate: (f64, f64),
    ) {
        let angle_between = |a: (f64, f64), b: (f64, f64)| {
            let dot = a.0 * b.0 + a.1 * b.1; // dot product
            let len_a = (a.0 * a.0 + a.1 * a.1).sqrt();
            let len_b = (b.0 * b.0 + b.1 * b.1).sqrt();
            (dot / (len_a * len_b)).clamp(-1.0, 1.0).acos() // the angle between the two vectors, in radians
        };
        let to_p1 = (p1.0 - p2.0, p1.1 - p2.1);
        let to_p3 = (p3.0 - p2.0, p3.1 - p2.1);
        let to_candidate = (candidate.0 - p2.0, candidate.1 - p2.1);

        let angle_to_p1 = angle_between(to_p1, to_candidate);
        let angle_to_p3 = angle_between(to_candidate, to_p3);
        assert!(
            (angle_to_p1 - angle_to_p3).abs() < 1e-9,
            "not a true bisector: angle to p1 = {angle_to_p1}, angle to p3 = {angle_to_p3}"
        );

        let candidate_distance =
            ((candidate.0 - p2.0).powi(2) + (candidate.1 - p2.1).powi(2)).sqrt();
        assert!((candidate_distance - length).abs() < 1e-9);
    }

    #[test]
    fn bisector_parity_matches_seamly2ds_selected_bisector_across_angle_ranges() {
        // Seamly2D: VToolBisector::BisectorAngle computes the true angle
        // bisector via QLineF::angleTo with explicit reflex-angle (>180
        // degrees) handling, always landing on the INTERIOR (<=180 degree)
        // bisector between the two rays. Yoko2D's own Bisector arm instead
        // normalizes and sums the two ray direction vectors. Rather than
        // re-deriving Seamly2D's own trig formula (which would just prove
        // the two formulas are algebraically equal, not that the actual
        // CODE behaves correctly), this verifies Yoko2D's real output
        // satisfies the bisector's actual geometric DEFINITION directly, at
        // a right angle (~90 degrees), a very acute angle (~20 degrees),
        // and a very obtuse angle (~160 degrees) between the two rays.
        let mut doc = Document::default();
        let p2 = doc.add_base_point("P2", 0.0, 0.0); // shared vertex for every case below
        let p1 = doc.add_base_point("P1", 10.0, 0.0); // ray at angle 0 degrees, shared by every case

        // ~90 degrees: p3 at angle 90.
        let p3_right = doc.add_base_point("P3Right", 0.0, 10.0);
        let bisector_right = doc.add_bisector("BRight", p1, p2, p3_right, "10").unwrap();

        // ~20 degrees: p3 at angle 20.
        let angle20 = 20.0_f64.to_radians();
        let p3_acute = doc.add_base_point("P3Acute", 10.0 * angle20.cos(), 10.0 * angle20.sin());
        let bisector_acute = doc.add_bisector("BAcute", p1, p2, p3_acute, "10").unwrap();

        // ~160 degrees: p3 at angle 160.
        let angle160 = 160.0_f64.to_radians();
        let p3_obtuse =
            doc.add_base_point("P3Obtuse", 10.0 * angle160.cos(), 10.0 * angle160.sin());
        let bisector_obtuse = doc
            .add_bisector("BObtuse", p1, p2, p3_obtuse, "10")
            .unwrap();

        let data = recompute_all(&doc).unwrap();

        let right_point = data.get_point(bisector_right).unwrap();
        assert_is_a_true_angle_bisector(
            (10.0, 0.0),
            (0.0, 0.0),
            (0.0, 10.0),
            10.0,
            (right_point.x, right_point.y),
        );

        let acute_point = data.get_point(bisector_acute).unwrap();
        assert_is_a_true_angle_bisector(
            (10.0, 0.0),
            (0.0, 0.0),
            (10.0 * angle20.cos(), 10.0 * angle20.sin()),
            10.0,
            (acute_point.x, acute_point.y),
        );

        let obtuse_point = data.get_point(bisector_obtuse).unwrap();
        assert_is_a_true_angle_bisector(
            (10.0, 0.0),
            (0.0, 0.0),
            (10.0 * angle160.cos(), 10.0 * angle160.sin()),
            10.0,
            (obtuse_point.x, obtuse_point.y),
        );
    }

    #[test]
    fn height_parity_never_clamps_to_the_line_segment() {
        // Seamly2D: VToolHeight::FindPoint calls VGObject::ClosestPoint,
        // which builds the actual perpendicular line through `point` and
        // intersects it with `line` via `line.intersects(lin, &p)`,
        // returning `p` whether Qt classifies the intersection as
        // BoundedIntersection OR UnboundedIntersection — i.e. the
        // projected foot is never clamped to fall between line_p1/line_p2;
        // it can land anywhere on the INFINITE line through them.
        //
        // Line through (0,0) and (4,3): a 3-4-5 triangle, direction
        // (0.8,0.6), segment length 5. Target point (20,0), far off the
        // line. Hand-calculated: t = (20-0)*0.8 + (0-0)*0.6 = 16, which is
        // far beyond the 5-unit segment; foot = (0,0) + 16*(0.8,0.6) =
        // (12.8, 9.6) — well past line_p2, confirming no clamping.
        let mut doc = Document::default();
        let line_p1 = doc.add_base_point("L1", 0.0, 0.0);
        let line_p2 = doc.add_base_point("L2", 4.0, 3.0);
        let off_line = doc.add_base_point("P", 20.0, 0.0);
        let height = doc.add_height("H", off_line, line_p1, line_p2).unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(height).unwrap();
        assert!((point.x - 12.8).abs() < 1e-9);
        assert!((point.y - 9.6).abs() < 1e-9);
    }

    #[test]
    fn midpoint_parity_matches_along_line_at_half_the_distance() {
        // Seamly2D has no single dedicated "midpoint" tool function —
        // searching src/libs/vtools for "Midpoint" finds only
        // VToolMidpoint-adjacent naming that itself reduces to the same
        // AlongLine-at-50%-length construction Seamly2D's own UI exposes
        // (an AlongLine tool with its length formula set to half the
        // segment's own length). This is therefore a consistency check
        // between Yoko2D's own two implementations, not a Seamly2D source
        // citation: Midpoint(p1,p2) must equal AlongLine(p1,p2,dist/2).
        //
        // p1=(1,2), p2=(7,10): dx=6, dy=8, distance=10, half=5.
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 1.0, 2.0);
        let p2 = doc.add_base_point("P2", 7.0, 10.0);
        let midpoint = doc.add_midpoint("M", p1, p2).unwrap();
        let along = doc.add_along_line("AL", p1, p2, "5").unwrap();

        let data = recompute_all(&doc).unwrap();
        let midpoint_resolved = data.get_point(midpoint).unwrap();
        let along_resolved = data.get_point(along).unwrap();
        assert!((midpoint_resolved.x - along_resolved.x).abs() < 1e-9);
        assert!((midpoint_resolved.y - along_resolved.y).abs() < 1e-9);
    }

    #[test]
    fn piece_seam_allowance_parity_matches_ekvpoint_for_a_simple_convex_triangle() {
        use crate::document::PieceNode;

        // Seamly2D: VAbstractPiece::EkvPoint's full miter-join algorithm has
        // many branches (darts, acute/obtuse special cases, a hard-coded
        // miter-limit constant `maxL = 2.4` that bevels corners whose miter
        // point would otherwise land farther than `width*maxL` from the
        // original vertex) — but its own `AngleByLength` function shows
        // that whenever the miter point's distance from the original vertex
        // is <= width*maxL (an ordinary, non-extreme convex corner), EkvPoint
        // returns EXACTLY the plain intersection of the two offset edges
        // (its own `CrosPoint`), with no further adjustment — the same
        // value Yoko2D's own (documented-simplified) `offset_polygon`
        // always computes for every corner. An equilateral triangle (all
        // 60-degree corners) with width=1 keeps every corner's miter
        // distance (width/sin(30 degrees) = 2.0) safely under maxL=2.4, so
        // this is exactly the "ordinary corner" case EkvPoint's own code
        // reduces to plain-intersection for, not one of its documented
        // special-case simplifications.
        //
        // A=(0,0), B=(10,0), C=(5, 5*sqrt(3)): an equilateral triangle,
        // counter-clockwise. Offsetting each edge outward by 1.0 and
        // intersecting the offset lines by hand (same method
        // offset_polygon itself uses) gives exactly:
        // A' = (-sqrt(3), -1), B' = (10+sqrt(3), -1), C' = (5, 2+5*sqrt(3)).
        let sqrt3 = 3.0_f64.sqrt();
        let mut doc = Document::default();
        let a = doc.add_base_point("A", 0.0, 0.0);
        let b = doc.add_base_point("B", 10.0, 0.0);
        let c = doc.add_base_point("C", 5.0, 5.0 * sqrt3);
        let piece = doc
            .add_piece(
                "Piece1",
                vec![
                    PieceNode {
                        point: a,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: b,
                        excluded_from_seam_allowance: false,
                    },
                    PieceNode {
                        point: c,
                        excluded_from_seam_allowance: false,
                    },
                ],
                "1.0",
            )
            .unwrap();

        let data = recompute_all(&doc).unwrap();
        let resolved = data.get_piece(piece).unwrap();
        let seam_allowance = resolved.seam_allowance.as_ref().unwrap();

        let expected = [
            (-sqrt3, -1.0),
            (10.0 + sqrt3, -1.0),
            (5.0, 2.0 + 5.0 * sqrt3),
        ];
        assert_eq!(seam_allowance.len(), expected.len());
        for (got, want) in seam_allowance.iter().zip(expected.iter()) {
            assert!((got.0 - want.0).abs() < 1e-9);
            assert!((got.1 - want.1).abs() < 1e-9);
        }
    }

    // ===================================================================
    // Part C: golden and degenerate-input tests for the five newly added
    // tools (ShoulderPoint, LineIntersect, PointOfIntersection, Triangle,
    // PointOfContact).
    // ===================================================================

    #[test]
    fn shoulder_point_golden_value_matches_hand_calculated_circle_intersection() {
        // p1_line=(0,0), p2_line=(10,0): a horizontal ray, base_length=10.
        // shoulder=(15,8), length=10 (circle radius). Hand-calculated: the
        // perpendicular foot of (15,8) onto the ray's line (y=0) is (15,0);
        // distance from shoulder to that foot is 8; half-chord
        // k=sqrt(10^2-8^2)=sqrt(36)=6; candidates are (15,0)+-6*(1,0) =
        // (21,0) and (9,0). Only (21,0) is both farther than base_length=10
        // from p1_line AND in the ray's forward direction, so it's selected.
        let mut doc = Document::default();
        let p1_line = doc.add_base_point("P1Line", 0.0, 0.0);
        let p2_line = doc.add_base_point("P2Line", 10.0, 0.0);
        let shoulder = doc.add_base_point("Shoulder", 15.0, 8.0);
        let result = doc
            .add_shoulder_point("S", p1_line, p2_line, shoulder, "10")
            .unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(result).unwrap();
        assert!((point.x - 21.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn shoulder_point_circle_too_far_from_ray_is_degenerate() {
        let mut doc = Document::default();
        let p1_line = doc.add_base_point("P1Line", 0.0, 0.0);
        let p2_line = doc.add_base_point("P2Line", 10.0, 0.0);
        let shoulder = doc.add_base_point("Shoulder", 15.0, 20.0); // far above the ray
        doc.add_shoulder_point("S", p1_line, p2_line, shoulder, "5") // radius 5, distance to ray is 20: never reaches
            .unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn line_intersect_golden_value_non_axis_aligned() {
        // Line 1 through (0,0) and (3,3): the line y=x.
        // Line 2 through (0,4) and (4,0): the line y=4-x.
        // Hand-calculated intersection: x=4-x => x=2, y=2.
        let mut doc = Document::default();
        let p1_line1 = doc.add_base_point("P1L1", 0.0, 0.0);
        let p2_line1 = doc.add_base_point("P2L1", 3.0, 3.0);
        let p1_line2 = doc.add_base_point("P1L2", 0.0, 4.0);
        let p2_line2 = doc.add_base_point("P2L2", 4.0, 0.0);
        let result = doc
            .add_line_intersect("X", p1_line1, p2_line1, p1_line2, p2_line2)
            .unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(result).unwrap();
        assert!((point.x - 2.0).abs() < 1e-9);
        assert!((point.y - 2.0).abs() < 1e-9);
    }

    #[test]
    fn line_intersect_parallel_lines_are_degenerate() {
        let mut doc = Document::default();
        let p1_line1 = doc.add_base_point("P1L1", 0.0, 0.0);
        let p2_line1 = doc.add_base_point("P2L1", 2.0, 2.0);
        let p1_line2 = doc.add_base_point("P1L2", 0.0, 1.0); // same (1,1) direction as line 1, offset by 1
        let p2_line2 = doc.add_base_point("P2L2", 2.0, 3.0);
        doc.add_line_intersect("X", p1_line1, p2_line1, p1_line2, p2_line2)
            .unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn point_of_intersection_golden_value_combines_x_and_y_from_each_point() {
        // No degenerate-input test exists for this tool: combining any two
        // points' x/y coordinates (even the same point given twice) is
        // always well-defined — see this crate's own document.rs doc
        // comment on ToolKind::PointOfIntersection for the same note,
        // mirroring Midpoint's identical "no degenerate case possible"
        // precedent elsewhere in this file.
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 3.0, 7.0);
        let p2 = doc.add_base_point("P2", 9.0, -2.0);
        let result = doc.add_point_of_intersection("X", p1, p2).unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(result).unwrap();
        assert!((point.x - 3.0).abs() < 1e-9); // p1's x
        assert!((point.y - -2.0).abs() < 1e-9); // p2's y
    }

    #[test]
    fn triangle_golden_value_matches_hand_calculated_thales_circle_intersection() {
        // axis: (0,0)->(20,0), the x-axis. hypotenuse: (8,-3)->(14,9).
        // Hand-calculated: hypotenuse crosses the x-axis (axis line y=0) at
        // startPoint=(9.5,0) (parametrize hypotenuse (8+6s,-3+12s), y=0 at
        // s=0.25, x=8+6*0.25=9.5). Thales circle: center = hypotenuse
        // midpoint = (11,3), radius = |hypotenuse|/2 = sqrt(6^2+12^2)/2 =
        // sqrt(180)/2 = 3*sqrt(5). Intersecting the x-axis with that circle:
        // foot of (11,3) onto y=0 is (11,0), distance 3 from center;
        // k=sqrt(45-9)=6; candidates (17,0) and (5,0). Only (17,0) is
        // forward of startPoint=(9.5,0) in the axis_p1->axis_p2 (+x)
        // direction, so it's selected. Cross-check: distance((17,0),(8,-3))
        // = sqrt(81+9) = sqrt(90); distance((17,0),(14,9)) = sqrt(9+81) =
        // sqrt(90); 90+90=180 = the hypotenuse's own squared length
        // (6*sqrt(5))^2=180 exactly, confirming a true right angle at (17,0).
        let mut doc = Document::default();
        let axis_p1 = doc.add_base_point("AxisP1", 0.0, 0.0);
        let axis_p2 = doc.add_base_point("AxisP2", 20.0, 0.0);
        let hyp_p1 = doc.add_base_point("HypP1", 8.0, -3.0);
        let hyp_p2 = doc.add_base_point("HypP2", 14.0, 9.0);
        let result = doc
            .add_triangle("T", axis_p1, axis_p2, hyp_p1, hyp_p2)
            .unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(result).unwrap();
        assert!((point.x - 17.0).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn triangle_parallel_axis_and_hypotenuse_are_degenerate() {
        let mut doc = Document::default();
        let axis_p1 = doc.add_base_point("AxisP1", 0.0, 0.0);
        let axis_p2 = doc.add_base_point("AxisP2", 20.0, 0.0); // horizontal axis
        let hyp_p1 = doc.add_base_point("HypP1", 0.0, 5.0);
        let hyp_p2 = doc.add_base_point("HypP2", 10.0, 5.0); // also horizontal: parallel to the axis
        doc.add_triangle("T", axis_p1, axis_p2, hyp_p1, hyp_p2)
            .unwrap();

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn point_of_contact_golden_value_ambiguous_case_prefers_closer_to_p1() {
        // center=(5,3), radius=4, p1=(0,0), p2=(10,0): circle meets y=0 at
        // x=5+-sqrt(7) (from (x-5)^2+9=16 => (x-5)^2=7). Both candidates lie
        // within the segment [0,10], so Seamly2D's "both/neither on
        // segment: prefer closer to p1" tiebreak applies, selecting the
        // smaller-distance one: 5-sqrt(7).
        let sqrt7 = 7.0_f64.sqrt();
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 5.0, 3.0);
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p2 = doc.add_base_point("P2", 10.0, 0.0);
        let result = doc.add_point_of_contact("X", center, p1, p2, "4").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(result).unwrap();
        assert!((point.x - (5.0 - sqrt7)).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn point_of_contact_golden_value_prefers_the_candidate_actually_on_the_segment() {
        // Same circle as the ambiguous-case test above (center=(5,3),
        // radius=4), but p1=(4,0), p2=(10,0): the segment is now [4,10].
        // Candidate 5-sqrt(7)=~2.354 falls OUTSIDE [4,10]; candidate
        // 5+sqrt(7)=~7.646 falls INSIDE it — exactly one candidate is truly
        // on the segment, so it's selected regardless of which is closer
        // to p1 (proving segment membership takes precedence over
        // proximity, not just "always pick the nearer one").
        let sqrt7 = 7.0_f64.sqrt();
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 5.0, 3.0);
        let p1 = doc.add_base_point("P1", 4.0, 0.0);
        let p2 = doc.add_base_point("P2", 10.0, 0.0);
        let result = doc.add_point_of_contact("X", center, p1, p2, "4").unwrap();

        let data = recompute_all(&doc).unwrap();
        let point = data.get_point(result).unwrap();
        assert!((point.x - (5.0 + sqrt7)).abs() < 1e-9);
        assert!((point.y - 0.0).abs() < 1e-9);
    }

    #[test]
    fn point_of_contact_circle_never_reaches_the_line_is_degenerate() {
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 5.0, 100.0); // far above the line
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p2 = doc.add_base_point("P2", 10.0, 0.0);
        doc.add_point_of_contact("X", center, p1, p2, "1").unwrap(); // radius 1, distance to line is 100

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    // ===================================================================
    // This task's Part A/B: golden parity tests for the two new curve
    // tools, Arc and Spline, grounded in Seamly2D's actual VArc/VSpline
    // source (VArc::GetP1/GetP2 and VSpline::GetP2/GetP3, both built via
    // QLineF::setAngle) — the same angle convention already proven to need
    // no sign flip in this crate's y-up coordinates by the EndLine/Normal
    // parity tests above.
    // ===================================================================

    #[test]
    fn arc_golden_value_matches_hand_calculated_point_at_forty_five_degrees() {
        // center=(5,3), radius=10, start_angle=0, end_angle=90.
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 5.0, 3.0);
        let arc = doc.add_arc("A", center, "10", "0", "90").unwrap();

        let data = recompute_all(&doc).unwrap();
        let resolved = data.get_arc(arc).unwrap();
        assert!((resolved.center.0 - 5.0).abs() < 1e-9);
        assert!((resolved.center.1 - 3.0).abs() < 1e-9);
        assert!((resolved.radius - 10.0).abs() < 1e-9);
        assert!((resolved.start_angle_deg - 0.0).abs() < 1e-9);
        assert!((resolved.end_angle_deg - 90.0).abs() < 1e-9);

        // A point on the arc at 45 degrees: center + radius*(cos(45deg),sin(45deg)).
        // Hand-verified: (5,3) + 10*(sqrt(2)/2, sqrt(2)/2) = (5 + 5*sqrt(2), 3 + 5*sqrt(2)).
        let theta = 45.0_f64.to_radians();
        let expected_x = resolved.center.0 + resolved.radius * theta.cos();
        let expected_y = resolved.center.1 + resolved.radius * theta.sin();
        let hand_x = 5.0 + 5.0 * 2.0_f64.sqrt();
        let hand_y = 3.0 + 5.0 * 2.0_f64.sqrt();
        assert!((expected_x - hand_x).abs() < 1e-9);
        assert!((expected_y - hand_y).abs() < 1e-9);
        // NOT (5 - 5*sqrt(2), 3 - 5*sqrt(2)): confirms the angle sweeps in the
        // standard mathematical (counter-clockwise, y-up) direction, matching
        // VArc::GetP1/GetP2's own QLineF::setAngle convention with no sign flip
        // — the exact same convention already verified for EndLine/Normal above.
    }

    #[test]
    fn arc_non_positive_radius_is_degenerate() {
        let mut doc = Document::default();
        let center = doc.add_base_point("Center", 0.0, 0.0);
        doc.add_arc("A", center, "0", "0", "90").unwrap(); // radius formula evaluates to 0.0

        let err = recompute_all(&doc).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn spline_golden_value_bows_away_from_the_straight_p1_p4_line() {
        // p1=(0,0), p4=(10,0). angle1=90 (straight up from p1), length1=3.
        // angle2=90 — per VSpline::GetP3's own QLineF(p4, p4+(c2Length,0))
        // .setAngle(angle2) construction (verified by reading the actual
        // Seamly2D source), angle2 is measured as the p4->p3 DIRECTION
        // itself, not the tangent-of-travel-at-p4 (which would point the
        // opposite way) — so angle2=90 also points straight up, giving
        // P3 directly above p4, not below it.
        let mut doc = Document::default();
        let p1 = doc.add_base_point("P1", 0.0, 0.0);
        let p4 = doc.add_base_point("P4", 10.0, 0.0);
        let spline = doc.add_spline("S", p1, p4, "90", "3", "90", "3").unwrap();

        let data = recompute_all(&doc).unwrap();
        let resolved = data.get_spline(spline).unwrap();

        // Hand-calculated: P2 = (0,0) + 3*(cos(90deg),sin(90deg)) = (0,3).
        assert!((resolved.p2.0 - 0.0).abs() < 1e-9);
        assert!((resolved.p2.1 - 3.0).abs() < 1e-9);
        // Hand-calculated: P3 = (10,0) + 3*(cos(90deg),sin(90deg)) = (10,3).
        assert!((resolved.p3.0 - 10.0).abs() < 1e-9);
        assert!((resolved.p3.1 - 3.0).abs() < 1e-9);

        // Hand-calculated point at t=0.5, via the cubic Bezier Bernstein
        // coefficients (mt3=0.125, 3*mt2*t=0.375, 3*mt*t2=0.375, t3=0.125):
        // x = 0.375*p2.x + 0.375*p3.x = 0.375*0 + 0.375*10 = 3.75... plus
        // 0.125*p1.x + 0.125*p4.x = 0 + 1.25 => x = 5.0.
        // y = 0.375*p2.y + 0.375*p3.y = 0.375*3 + 0.375*3 = 2.25 (p1.y/p4.y are both 0).
        let midpoint = core_lib_cubic_bezier_point(&data, spline, 0.5);
        assert!((midpoint.0 - 5.0).abs() < 1e-9);
        assert!((midpoint.1 - 2.25).abs() < 1e-9);
        // The curve genuinely bows away from the straight p1-p4 line (y=0):
        // its t=0.5 point sits meaningfully above it, not on it.
        assert!(midpoint.1 > 1.0);
    }

    /// Small local wrapper around `crate::geometry::cubic_bezier_point`,
    /// applied to an already-resolved `Spline` object's own four control
    /// points — kept local to this test module rather than adding a
    /// `PatternData`-level convenience method for a single test's sake.
    fn core_lib_cubic_bezier_point(data: &PatternData, spline: ObjectId, t: f64) -> (f64, f64) {
        let resolved = data.get_spline(spline).unwrap();
        crate::geometry::cubic_bezier_point(resolved.p1, resolved.p2, resolved.p3, resolved.p4, t)
    }
}
