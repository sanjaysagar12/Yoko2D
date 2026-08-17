use crate::id::ObjectId;

#[derive(Debug, Clone, PartialEq)]
pub struct PointData {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineData {
    pub p1: ObjectId,
    pub p2: ObjectId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GeoObject {
    Point(PointData),
    Line(LineData),
    // TODO(phase 10): Arc, Spline, SplinePath, EllipticalArc
}
