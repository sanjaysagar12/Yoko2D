// Permanent regression test for the shirt-back worked example
// (fixtures/actions/shirt_back.json): runs the real built binary against
// it and checks the resulting geometry both structurally (a simple, closed
// perimeter) and against the specific concave-neck/concave-armhole shape.
//
// Mirrors crates/cli/tests/shirt_front_shape.rs's own structure and
// technique exactly (same helper functions, same signed-side concavity
// check), applied to the back piece: back neck/armhole are each a single
// core_lib::ToolKind::Spline, built the same way as the front's — via
// exact quadratic-to-cubic Bezier degree elevation from a
// point-of-intersection control point, sampled at the curve's own exact
// t=0.5 point through a short Midpoint chain. The back shoulder point (B7)
// resolves to the exact same (shoulder_width, -shoulder_drop) coordinate
// as the front's shoulder point (A7), since both are built from the same
// shared measurements (fixtures/measurements/shirt_front.json) via the
// identical BasePoint/EndLine/ShoulderPoint construction — so the back
// armhole curve's angle/length formulas are the front ArmholeCurve's own
// formulas, unchanged, and produce the identical (mirrored, concave)
// shape.

use assert_cmd::Command;

/// The workspace root, same CARGO_MANIFEST_DIR-relative resolution every
/// other integration test file in this crate already uses.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs the fixture action script via the real built binary and returns the
/// loaded `Document` plus its resolved `PatternData` — same technique as
/// shirt_front_shape.rs's own `run_shirt_front_fixture`.
fn run_shirt_back_fixture() -> (core_lib::Document, core_lib::PatternData) {
    let dir = tempfile::tempdir().unwrap();
    let saved_path = dir.path().join("shirt_back.xml");

    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("run")
        .arg("fixtures/actions/shirt_back.json")
        .arg("--save-pattern")
        .arg(&saved_path)
        .assert()
        .success();

    let xml = std::fs::read_to_string(&saved_path).unwrap();
    let (mut document, _measurements_path) = io::deserialize_document(&xml).unwrap();
    // Same shared measurements file the front fixture uses (see
    // fixtures/actions/shirt_back.json's own measurements_path):
    // back_neck_depth lives alongside the other shared body measurements.
    let measurements = io::load_measurements_from_file(
        &workspace_root().join("fixtures/measurements/shirt_front.json"),
    )
    .unwrap();
    document.apply_measurements(measurements);
    let data = core_lib::recompute_all(&document).unwrap();
    (document, data)
}

/// Looks up the resolved `(x, y)` of the point named `name` in `document`/
/// `data` — identical technique to shirt_front_shape.rs's own `point`.
fn point(document: &core_lib::Document, data: &core_lib::PatternData, name: &str) -> (f64, f64) {
    let id = document
        .history()
        .iter()
        .find(|r| match &r.kind {
            core_lib::ToolKind::BasePoint { name: n, .. } => n == name,
            core_lib::ToolKind::EndLine { name: n, .. } => n == name,
            core_lib::ToolKind::AlongLine { name: n, .. } => n == name,
            core_lib::ToolKind::Normal { name: n, .. } => n == name,
            core_lib::ToolKind::ShoulderPoint { name: n, .. } => n == name,
            core_lib::ToolKind::PointOfIntersection { name: n, .. } => n == name,
            core_lib::ToolKind::Midpoint { name: n, .. } => n == name,
            _ => false,
        })
        .unwrap_or_else(|| panic!("no point named {name:?} in the shirt-back fixture's history"))
        .id;
    let resolved = data.get_point(id).unwrap();
    (resolved.x, resolved.y)
}

/// Signed perpendicular "distance" of `point` from the line through
/// `chord_start` -> `chord_end` — identical to shirt_front_shape.rs's own
/// `signed_side`.
fn signed_side(chord_start: (f64, f64), chord_end: (f64, f64), point: (f64, f64)) -> f64 {
    let dir = (chord_end.0 - chord_start.0, chord_end.1 - chord_start.1); // the chord's own direction vector
    let to_point = (point.0 - chord_start.0, point.1 - chord_start.1); // from the chord's start to the point being tested
    dir.0 * to_point.1 - dir.1 * to_point.0 // the 2D cross product: its sign says which side of the chord `point` falls on
}

/// Same rationale as shirt_front_shape.rs's own `MIN_DEVIATION`.
const MIN_DEVIATION: f64 = 1e-6;

#[test]
fn back_neck_curve_intermediate_point_is_strictly_concave() {
    let (document, data) = run_shirt_back_fixture();
    let chord_start = point(&document, &data, "B5"); // the neck-width/shoulder-line point
    let chord_end = point(&document, &data, "B2"); // the back-neck-depth center-back point
                                                   // Hand-calculated: B5=(8,0), B2=(0,-2), control B10=(8,-2), giving
                                                   // B13 (the exact t=0.5 point on BackNeckCurve) = (6,-1.5); the
                                                   // concave (center-back) side comes out STRICTLY POSITIVE for
                                                   // this chord, same sign as the front neck curve's own
                                                   // down-and-left chord.
    let p = point(&document, &data, "B13");
    let side = signed_side(chord_start, chord_end, p);
    assert!(
        side > MIN_DEVIATION,
        "back neck curve point B13 is not strictly concave: signed_side={side} for point {p:?} \
         relative to chord {chord_start:?} -> {chord_end:?} (expected a value > {MIN_DEVIATION})"
    );
}

#[test]
fn back_armhole_curve_intermediate_point_is_strictly_concave() {
    let (document, data) = run_shirt_back_fixture();
    let chord_start = point(&document, &data, "B7"); // the shoulder point
    let chord_end = point(&document, &data, "B8"); // the underarm/side-seam point
                                                   // B7/B8 resolve to the exact same coordinates as the front's
                                                   // A7/A8 (both built from the same shared measurements), and
                                                   // BackArmholeCurve uses the front ArmholeCurve's own formulas
                                                   // unchanged, so this must land on the same STRICTLY NEGATIVE
                                                   // side as the front's own (mirrored) armhole check.
    let p = point(&document, &data, "B23");
    let side = signed_side(chord_start, chord_end, p);
    assert!(
        side < -MIN_DEVIATION,
        "back armhole curve point B23 is not strictly concave: signed_side={side} for point {p:?} \
         relative to chord {chord_start:?} -> {chord_end:?} (expected a value < {})",
        -MIN_DEVIATION
    );
}

#[test]
fn back_perimeter_is_a_simple_non_self_intersecting_closed_polygon() {
    let (document, data) = run_shirt_back_fixture();
    let perimeter_names = ["B5", "B13", "B2", "B4", "B9", "B8", "B23", "B7"]; // the exact perimeter order fixtures/actions/shirt_back.json's own add_piece action lists
    let perimeter: Vec<(f64, f64)> = perimeter_names
        .iter()
        .map(|name| point(&document, &data, name))
        .collect();
    let n = perimeter.len();

    // Every pair of NON-ADJACENT edges (sharing no endpoint, including the
    // wrap-around edge) must not cross — identical technique to
    // shirt_front_shape.rs's own perimeter test.
    for i in 0..n {
        let a1 = perimeter[i];
        let a2 = perimeter[(i + 1) % n];
        for j in (i + 1)..n {
            let shares_endpoint = j == i || j == (i + 1) % n || (j + 1) % n == i; // adjacent edges (including the wrap-around pair) legitimately share exactly one endpoint
            if shares_endpoint {
                continue; // adjacent edges touching at their shared vertex is expected, not a crossing
            }
            let b1 = perimeter[j];
            let b2 = perimeter[(j + 1) % n];
            assert!(
                !segments_intersect(a1, a2, b1, b2),
                "perimeter edges {i} ({a1:?}->{a2:?}) and {j} ({b1:?}->{b2:?}) cross: the \
                 perimeter is not a simple polygon"
            );
        }
    }
}

/// Standard orientation-based segment-intersection test — identical to
/// shirt_front_shape.rs's own `segments_intersect`.
fn segments_intersect(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), p4: (f64, f64)) -> bool {
    let orientation = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| -> f64 {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0) // 2D cross product of (b-a) and (c-a): its sign says which way c turns relative to a->b
    };
    let d1 = orientation(p3, p4, p1);
    let d2 = orientation(p3, p4, p2);
    let d3 = orientation(p1, p2, p3);
    let d4 = orientation(p1, p2, p4);
    // A genuine crossing: p1/p2 fall on opposite sides of line p3-p4, AND
    // p3/p4 fall on opposite sides of line p1-p2.
    ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0))
}

#[test]
fn back_overall_proportions_are_within_twenty_percent_of_half_chest_and_body_length() {
    let (document, data) = run_shirt_back_fixture();
    let perimeter_names = ["B5", "B13", "B2", "B4", "B9", "B8", "B23", "B7"];
    let perimeter: Vec<(f64, f64)> = perimeter_names
        .iter()
        .map(|name| point(&document, &data, name))
        .collect();

    let min_x = perimeter
        .iter()
        .fold(f64::INFINITY, |acc, &(x, _)| acc.min(x));
    let max_x = perimeter
        .iter()
        .fold(f64::NEG_INFINITY, |acc, &(x, _)| acc.max(x));
    let min_y = perimeter
        .iter()
        .fold(f64::INFINITY, |acc, &(_, y)| acc.min(y));
    let max_y = perimeter
        .iter()
        .fold(f64::NEG_INFINITY, |acc, &(_, y)| acc.max(y));
    let width = max_x - min_x;
    let height = max_y - min_y;

    // fixtures/measurements/shirt_front.json's own bundled values (shared by the back fixture).
    let half_chest = 50.0_f64;
    let body_length = 65.0_f64;

    let width_ratio = width / half_chest;
    assert!(
        (0.8..=1.2).contains(&width_ratio),
        "bounding box width {width} is not within +/-20% of half_chest {half_chest} (ratio {width_ratio})"
    );
    let height_ratio = height / body_length;
    assert!(
        (0.8..=1.2).contains(&height_ratio),
        "bounding box height {height} is not within +/-20% of body_length {body_length} (ratio {height_ratio})"
    );
}
