use crate::id::ObjectId;

/// A point in the pattern's 2D coordinate space.
///
/// Just the coordinates — no id, no name, no styling. Those live one layer
/// up: a point only becomes addressable once it's stored in a
/// `PatternData` under an [`ObjectId`], via `GeoObject::Point`.
#[derive(Debug, Clone, PartialEq)]
pub struct PointData {
    pub x: f64,
    pub y: f64,
}

/// A straight line between two existing points.
///
/// Stores the endpoints by [`ObjectId`] rather than by value, so a line
/// always reflects wherever its endpoint points currently are — the same
/// relationship the original Seamly2D tool graph relies on (move a point,
/// every line built from it moves too). Nothing here validates that `p1`
/// and `p2` actually point at `Point` objects in the same container; that's
/// the container's job when the line is looked up (see
/// `PatternData::get_line`).
#[derive(Debug, Clone, PartialEq)]
pub struct LineData {
    pub p1: ObjectId,
    pub p2: ObjectId,
}

/// A closed straight-line polygon boundary (a pattern "piece"), plus its
/// optional seam-allowance offset.
///
/// `contour` is the resolved boundary point sequence, IN ORDER — the same
/// "store resolved geometry, not formulas" contract every other
/// `GeoObject` variant follows (the formula/reference inputs live on
/// [`crate::document::ToolKind::Piece`] instead). `seam_allowance` is
/// `None` when the width formula evaluates to `0.0` (no seam allowance
/// requested), or `Some(offset points)` otherwise, computed by
/// [`crate::geometry::offset_polygon`].
#[derive(Debug, Clone, PartialEq)]
pub struct PieceData {
    pub contour: Vec<(f64, f64)>, // the boundary's resolved points, in order
    pub seam_allowance: Option<Vec<(f64, f64)>>, // None if the seam-allowance width was 0.0, else Some(the offset boundary)
}

/// A circular arc: a center point, a radius, and a start/end angle sweep.
///
/// Mirrors Seamly2D's `VArc`: `VArc::GetP1()`/`GetP2()` build each of the
/// arc's own endpoints by starting from a horizontal radius line and
/// rotating it via `QLineF::setAngle` — the exact same angle convention
/// already verified sign-correct for this crate's y-up coordinates by the
/// `EndLine`/`Normal` parity tests in `crate::recompute`'s test module (Qt's
/// y-down `setAngle` washes out to plain `base + length*(cos,sin)` with no
/// extra sign flip needed here). A point on the arc at `theta` degrees is
/// therefore `center + radius*(cos(theta_rad), sin(theta_rad))`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArcData {
    pub center: (f64, f64),   // the arc's own center, already resolved
    pub radius: f64, // the arc's radius; always > 0.0 once resolved (see recompute's own guard)
    pub start_angle_deg: f64, // the sweep's starting angle, in degrees
    pub end_angle_deg: f64, // the sweep's ending angle, in degrees
}

/// A cubic Bezier curve through two existing endpoints (`p1`/`p4`), with two
/// derived interior control points (`p2`/`p3`).
///
/// `p2`/`p3` are stored ALREADY RESOLVED (plain coordinates), not as
/// formulas — every `GeoObject` is always a derived/recomputed cache
/// rebuilt from scratch on each `recompute_all` call, never a place formula
/// strings live. The FORMULAS that produced `p2`/`p3` (an angle + a
/// handle-length at each endpoint) live only on
/// `crate::document::ToolKind::Spline`, exactly matching how every other
/// tool in this crate keeps its formula-bearing fields on `ToolKind` and
/// only its resolved fields on the matching `GeoObject` variant.
#[derive(Debug, Clone, PartialEq)]
pub struct SplineData {
    pub p1: (f64, f64), // the curve's first endpoint, already resolved
    pub p2: (f64, f64), // the first interior control point, derived from p1's own angle/length formulas
    pub p3: (f64, f64), // the second interior control point, derived from p4's own angle/length formulas
    pub p4: (f64, f64), // the curve's second endpoint, already resolved
}

/// Any geometric object a [`crate::PatternData`] can store under an
/// [`ObjectId`].
///
/// This is the single type behind the container's `objects` map, so adding
/// a new kind of geometry later means adding a variant here (and a matching
/// typed getter on `PatternData`, the way `get_point`/`get_line` work now).
#[derive(Debug, Clone, PartialEq)]
pub enum GeoObject {
    Point(PointData),
    Line(LineData),
    Piece(PieceData),
    Arc(ArcData),
    Spline(SplineData),
    // TODO(future work): SplinePath, EllipticalArc
}
