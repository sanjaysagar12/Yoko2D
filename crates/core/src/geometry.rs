use crate::recompute::PatternError; // reuses the existing DegenerateGeometry variant (Phase 10a) rather than introducing a parallel error type

/// The two endpoints of one offset polygon edge, as returned by
/// [`offset_edge`]. Named purely to keep that function's signature
/// readable — clippy flags the equivalent nested tuple type as overly
/// complex.
type OffsetEdge = ((f64, f64), (f64, f64));

/// Computes the shoelace-formula signed area of a closed polygon.
///
/// A positive result means `points` are wound counter-clockwise; negative
/// means clockwise. [`offset_polygon`] uses this to decide which
/// perpendicular direction counts as "outward" for a given input, so
/// seam-allowance offsetting always expands outward regardless of which
/// way the caller happened to order the contour's points.
fn signed_area(points: &[(f64, f64)]) -> f64 {
    let n = points.len(); // vertex count, needed to wrap the last edge back to the first point
    let mut sum = 0.0; // accumulates the shoelace sum across every edge
    for i in 0..n {
        // walk every edge, including the wrap-around edge from the last point back to the first
        let (x_i, y_i) = points[i]; // this edge's starting vertex
        let (x_next, y_next) = points[(i + 1) % n]; // this edge's ending vertex, wrapping to index 0 after the last
        sum += x_i * y_next - x_next * y_i; // one shoelace term for this edge
    }
    sum / 2.0 // the shoelace formula's final division by 2
}

/// Computes the parallel offset of one polygon edge, pushed outward by
/// `width`.
///
/// `outward_sign` is +1.0 or -1.0, set by the caller ([`offset_polygon`])
/// from [`signed_area`]'s sign — it flips which of the two perpendicular
/// directions to `p1`->`p2` counts as "outward" for this particular
/// contour's winding.
fn offset_edge(
    p1: (f64, f64),    // this edge's starting point
    p2: (f64, f64),    // this edge's ending point
    width: f64,        // how far to push the edge outward
    outward_sign: f64, // +1.0 or -1.0, flips which perpendicular direction is "outward"
) -> Result<OffsetEdge, PatternError> {
    let dx = p2.0 - p1.0; // x component of the edge's direction vector
    let dy = p2.1 - p1.1; // y component of the edge's direction vector
    let len = (dx * dx + dy * dy).sqrt(); // the edge's length
    if len == 0.0 {
        // p1 and p2 are identical: this edge has no direction, so no offset can be computed
        return Err(PatternError::DegenerateGeometry(format!(
            "piece contour has a zero-length edge between {p1:?} and {p2:?}" // names exactly which edge failed
        )));
    }
    let ux = dx / len; // normalized edge direction, x component
    let uy = dy / len; // normalized edge direction, y component
    let perp_x = -uy; // perpendicular to the direction, rotated 90 degrees counter-clockwise; x component
    let perp_y = ux; // ...and its y component
    let offset_x = perp_x * outward_sign * width; // scale the perpendicular by outward_sign and width to get the actual offset vector, x component
    let offset_y = perp_y * outward_sign * width; // ...and its y component
    let offset_p1 = (p1.0 + offset_x, p1.1 + offset_y); // push the edge's start point by the offset vector
    let offset_p2 = (p2.0 + offset_x, p2.1 + offset_y); // push the edge's end point by the same offset vector, keeping the offset edge parallel to the original
    Ok((offset_p1, offset_p2)) // the two points defining the offset edge
}

/// Computes the intersection of two infinite lines, each defined by a point
/// and a direction vector.
///
/// Solves the standard 2x2 linear system `p1 + t*d1 = p2 + s*d2` for `t`
/// via Cramer's rule, then evaluates `p1 + t*d1` to get the intersection
/// point (solving for `s` too would give the same point, so it's skipped).
///
/// `pub(crate)`, not private: originally built only for [`offset_polygon`]'s
/// own miter-join math, but Phase 10b's `LineIntersect`/`Triangle` tools in
/// `recompute.rs` need the exact same infinite-line-line intersection, so
/// this is reused directly rather than reimplementing it a second time.
pub(crate) fn line_intersection(
    p1: (f64, f64), // a point on the first line
    d1: (f64, f64), // the first line's direction vector
    p2: (f64, f64), // a point on the second line
    d2: (f64, f64), // the second line's direction vector
) -> Result<(f64, f64), PatternError> {
    let cross = d1.0 * d2.1 - d1.1 * d2.0; // the 2D cross product of the two directions; the linear system's determinant
    if cross.abs() < 1e-9 {
        // the two directions are parallel (or anti-parallel): no unique
        // intersection exists. The message stays generic (not
        // piece-offsetting-specific wording) now that LineIntersect/Triangle
        // also call this helper, not just offset_polygon.
        return Err(PatternError::DegenerateGeometry(format!(
            "two lines are parallel/collinear: no unique intersection exists (first line through {p1:?})" // names roughly where the failure is
        )));
    }
    // Rearranging `p1 + t*d1 = p2 + s*d2` into `t*d1 - s*d2 = (p2 - p1)`
    // gives a 2x2 linear system in (t, s); Cramer's rule solves for t
    // using the determinant of the matrix with the right-hand side
    // substituted into the first column, divided by `cross` (the full
    // matrix's determinant, computed above).
    let rhs_x = p2.0 - p1.0; // x component of the right-hand side vector (p2 - p1)
    let rhs_y = p2.1 - p1.1; // y component of the right-hand side vector (p2 - p1)
    let t = (rhs_x * d2.1 - rhs_y * d2.0) / cross; // Cramer's rule: solve for t
    let intersection_x = p1.0 + t * d1.0; // walk `t` units along the first line's direction from p1
    let intersection_y = p1.1 + t * d1.1; // same, for y
    Ok((intersection_x, intersection_y)) // the point where the two infinite lines cross
}

/// Computes where an infinite line intersects a circle: zero points (the
/// line misses the circle), one point (the line is exactly tangent), or two
/// points.
///
/// Mirrors Seamly2D's `VGObject::LineIntersectCircle`: project `circle_center`
/// onto the line to get the foot of the perpendicular, then walk
/// `±sqrt(radius^2 - d^2)` units from that foot along the line's own
/// direction, where `d` is the distance from `circle_center` to the foot.
///
/// `line_unit_dir` must already be normalized to unit length — the caller
/// (every Phase 10b tool using this) already computes and validates a unit
/// direction for its own degenerate-input checks (e.g. "are these two
/// points coincident"), so this helper stays a pure, always-succeeds
/// function rather than re-deriving/re-validating a direction itself.
pub(crate) fn line_circle_intersection(
    line_point: (f64, f64),    // any point on the line
    line_unit_dir: (f64, f64), // the line's direction, already unit length
    circle_center: (f64, f64), // the circle's center
    radius: f64,               // the circle's radius; must be >= 0.0
) -> Vec<(f64, f64)> {
    let to_center_x = circle_center.0 - line_point.0; // x component of the vector from line_point to circle_center
    let to_center_y = circle_center.1 - line_point.1; // y component
    let t = to_center_x * line_unit_dir.0 + to_center_y * line_unit_dir.1; // dot product: how far along the line direction the foot of the perpendicular lands
    let foot_x = line_point.0 + t * line_unit_dir.0; // the foot's x coordinate: the closest point on the line to circle_center
    let foot_y = line_point.1 + t * line_unit_dir.1; // the foot's y coordinate
    let dx = circle_center.0 - foot_x; // x component of the foot-to-center vector
    let dy = circle_center.1 - foot_y; // y component
    let d = (dx * dx + dy * dy).sqrt(); // the line's distance from the circle's center

    if d > radius + 1e-9 {
        // the line passes too far from the center to reach the circle at all
        return Vec::new();
    }

    let k = (radius * radius - d * d).max(0.0).sqrt(); // half-chord length; clamped at 0.0 so float error when d is extremely close to radius never produces a tiny negative operand to sqrt
    if k < 1e-9 {
        vec![(foot_x, foot_y)] // tangent: the line touches the circle at exactly one point, the foot itself
    } else {
        vec![
            (foot_x + k * line_unit_dir.0, foot_y + k * line_unit_dir.1), // one intersection, k units forward from the foot
            (foot_x - k * line_unit_dir.0, foot_y - k * line_unit_dir.1), // the other, k units backward from the foot
        ]
    }
}

/// Computes a simplified miter-join outward offset of a closed, straight-
/// edged polygon contour by `width`.
///
/// A straight-edges-only, simple-miter-join subset of Seamly2D's full
/// `VAbstractPiece::EkvPoint` seam-allowance algorithm: for each vertex,
/// this offsets its two adjacent edges outward by `width` and intersects
/// the offset lines to find the new vertex position. Curved boundary
/// segments, bevel/round join styles, notch generation, and concave-corner
/// self-intersection cleanup are all out of scope — see this phase's own
/// scope note.
pub fn offset_polygon(
    points: &[(f64, f64)], // the contour's vertices, in order, assumed to form a simple (non-self-intersecting) closed polygon
    width: f64,            // how far to push the contour outward; 0.0 means "no offset"
) -> Result<Vec<(f64, f64)>, PatternError> {
    if points.len() < 3 {
        // fewer than 3 points can't form a closed polygon at all
        return Err(PatternError::DegenerateGeometry(
            "piece contour needs at least 3 points".to_string(), // names exactly why this input is invalid
        ));
    }
    if width == 0.0 {
        // a zero-width seam allowance is just the original contour: no offsetting needed
        return Ok(points.to_vec());
    }

    let area = signed_area(points); // determines the contour's winding direction
                                    // Chosen (and verified against this module's own golden square test) so
                                    // the RESULT always expands the polygon outward regardless of which
                                    // direction the input happens to be wound in: a negative (clockwise)
                                    // area needs outward_sign +1.0, a positive (counter-clockwise) area
                                    // needs -1.0, given how offset_edge's perpendicular is defined (a
                                    // 90-degree counter-clockwise rotation of the edge direction).
    let outward_sign = if area < 0.0 { 1.0 } else { -1.0 };

    let n = points.len(); // vertex count, needed for wrap-around indexing below
    let mut result = Vec::with_capacity(n); // accumulates one offset vertex per input vertex, in the same order
    for i in 0..n {
        // process every vertex, offsetting its two adjacent edges and intersecting them
        let prev_index = (i + n - 1) % n; // the previous vertex's index, wrapping from 0 back to the last
        let next_index = (i + 1) % n; // the next vertex's index, wrapping from the last back to 0
        let prev_point = points[prev_index]; // the vertex before this one
        let current_point = points[i]; // this vertex itself
        let next_point = points[next_index]; // the vertex after this one

        let (prev_offset_a, prev_offset_b) =
            offset_edge(prev_point, current_point, width, outward_sign)?; // offset the incoming edge (prev -> current)
        let (next_offset_a, next_offset_b) =
            offset_edge(current_point, next_point, width, outward_sign)?; // offset the outgoing edge (current -> next)

        let prev_dir = (
            prev_offset_b.0 - prev_offset_a.0, // x component of the offset incoming edge's direction
            prev_offset_b.1 - prev_offset_a.1, // y component
        );
        let next_dir = (
            next_offset_b.0 - next_offset_a.0, // x component of the offset outgoing edge's direction
            next_offset_b.1 - next_offset_a.1, // y component
        );

        let vertex = line_intersection(prev_offset_a, prev_dir, next_offset_a, next_dir)?; // the new vertex: where the two offset edges meet
        result.push(vertex); // collect this vertex's offset position, in the same order as the input
    }

    Ok(result) // every vertex successfully offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_polygon_square_matches_hand_calculated_golden_values() {
        let square = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]; // counter-clockwise winding
        let offset = offset_polygon(&square, 1.0).unwrap();
        let expected = [(-1.0, -1.0), (11.0, -1.0), (11.0, 11.0), (-1.0, 11.0)];

        assert_eq!(offset.len(), expected.len());
        for (got, want) in offset.iter().zip(expected.iter()) {
            assert!((got.0 - want.0).abs() < 1e-9);
            assert!((got.1 - want.1).abs() < 1e-9);
        }
    }

    #[test]
    fn offset_polygon_with_reversed_winding_still_expands_outward() {
        let square_reversed = vec![(0.0, 10.0), (10.0, 10.0), (10.0, 0.0), (0.0, 0.0)]; // same square, opposite (clockwise) order
        let offset = offset_polygon(&square_reversed, 1.0).unwrap();
        let expected = [(-1.0, 11.0), (11.0, 11.0), (11.0, -1.0), (-1.0, -1.0)];

        assert_eq!(offset.len(), expected.len());
        for (got, want) in offset.iter().zip(expected.iter()) {
            assert!((got.0 - want.0).abs() < 1e-9);
            assert!((got.1 - want.1).abs() < 1e-9);
        }
    }

    #[test]
    fn offset_polygon_with_fewer_than_three_points_is_degenerate() {
        let err = offset_polygon(&[(0.0, 0.0), (1.0, 1.0)], 1.0).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn offset_polygon_with_a_zero_length_edge_is_degenerate() {
        let points = vec![(0.0, 0.0), (5.0, 5.0), (5.0, 5.0)]; // last two points coincide: a zero-length edge
        let err = offset_polygon(&points, 1.0).unwrap_err();
        assert!(matches!(err, PatternError::DegenerateGeometry(_)));
    }

    #[test]
    fn offset_polygon_with_zero_width_returns_the_original_points_unchanged() {
        let points = vec![(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)];
        let offset = offset_polygon(&points, 0.0).unwrap();
        assert_eq!(offset, points);
    }

    #[test]
    fn signed_area_distinguishes_winding_direction() {
        let ccw_triangle = [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]; // right, then up-and-left: counter-clockwise
        let cw_triangle = [(0.0, 0.0), (0.0, 10.0), (10.0, 0.0)]; // same points, opposite order: clockwise
        assert!(signed_area(&ccw_triangle) > 0.0);
        assert!(signed_area(&cw_triangle) < 0.0);
    }

    #[test]
    fn line_circle_intersection_two_points_matches_hand_calculation() {
        // Line: horizontal, through (0,0), direction (1,0). Circle: center
        // (15,8), radius 10. Hand-calculated: the perpendicular foot of
        // (15,8) onto y=0 is (15,0); distance from center to foot is 8;
        // half-chord k = sqrt(10^2 - 8^2) = sqrt(36) = 6; intersections are
        // (15,0) +/- 6*(1,0) = (21,0) and (9,0).
        let points = line_circle_intersection((0.0, 0.0), (1.0, 0.0), (15.0, 8.0), 10.0);
        assert_eq!(points.len(), 2);
        let contains_near = |target: (f64, f64)| {
            points
                .iter()
                .any(|p| (p.0 - target.0).abs() < 1e-9 && (p.1 - target.1).abs() < 1e-9)
        };
        assert!(contains_near((21.0, 0.0)));
        assert!(contains_near((9.0, 0.0)));
    }

    #[test]
    fn line_circle_intersection_tangent_line_produces_exactly_one_point() {
        // Circle center (0,5), radius 5: the x-axis (y=0) touches it at
        // exactly one point, (0,0) — the closest point on the line to the
        // center is exactly `radius` away.
        let points = line_circle_intersection((-10.0, 0.0), (1.0, 0.0), (0.0, 5.0), 5.0);
        assert_eq!(points.len(), 1);
        assert!((points[0].0 - 0.0).abs() < 1e-9);
        assert!((points[0].1 - 0.0).abs() < 1e-9);
    }

    #[test]
    fn line_circle_intersection_too_far_returns_no_points() {
        // Circle center (0,100), radius 1: nowhere near the x-axis.
        let points = line_circle_intersection((-10.0, 0.0), (1.0, 0.0), (0.0, 100.0), 1.0);
        assert!(points.is_empty());
    }
}
