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
    // TODO(phase 10): Arc, Spline, SplinePath, EllipticalArc
}
