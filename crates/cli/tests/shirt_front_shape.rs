// Permanent regression test for the shirt-front worked example
// (fixtures/actions/shirt_front.json): runs the real built binary against
// it and checks the resulting geometry both structurally (a simple, closed
// perimeter) and against the specific concave-neck/convex-armhole shape
// this fixture is meant to approximate with straight-line segments (this
// engine has no curve/Arc/Spline tool — see fixtures/actions/shirt_front.json's
// own header comment and fixtures/README.md for that documented
// simplification).

use assert_cmd::Command;

/// The workspace root, same CARGO_MANIFEST_DIR-relative resolution every
/// other integration test file in this crate already uses.
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Runs the fixture action script via the real built binary (writing a
/// pattern XML file, exactly like `run_command.rs`'s own
/// `run_with_save_pattern_...` test does) and returns the loaded
/// `Document` plus its resolved `PatternData` — reusing this crate's own
/// already-established `--save-pattern` + `io::deserialize_document` +
/// `core_lib::recompute_all` pattern, rather than parsing `--output`'s
/// JSON by assuming action-script-index-equals-id: that assumption would
/// hold today (this fixture never uses `remove_tool`/`modify_*`), but
/// resolving names through the real `Document` is robust regardless, and
/// is exactly the technique this crate's other integration tests already
/// rely on for the same reason.
fn run_shirt_front_fixture() -> (core_lib::Document, core_lib::PatternData) {
    let dir = tempfile::tempdir().unwrap();
    let saved_path = dir.path().join("shirt_front.xml");

    Command::cargo_bin("yoko2d-cli")
        .unwrap()
        .current_dir(workspace_root())
        .arg("run")
        .arg("fixtures/actions/shirt_front.json")
        .arg("--save-pattern")
        .arg(&saved_path)
        .assert()
        .success();

    let xml = std::fs::read_to_string(&saved_path).unwrap();
    let (mut document, _measurements_path) = io::deserialize_document(&xml).unwrap();
    // `--save-pattern` writes only a `<measurementsPath>` REFERENCE, not the
    // measurement values themselves (see io::serialize_document's own doc
    // comment) — reloading the file gives back a Document whose formulas
    // still need those variables seeded before recompute_all can resolve
    // anything, exactly like run_command.rs's own save/reload tests would
    // need to if their fixture used measurement variables at all.
    let measurements = io::load_measurements_from_file(
        &workspace_root().join("fixtures/measurements/shirt_front.json"),
    )
    .unwrap();
    document.apply_measurements(measurements);
    let data = core_lib::recompute_all(&document).unwrap();
    (document, data)
}

/// Looks up the resolved `(x, y)` of the point named `name` in `document`/
/// `data` — the same "walk history, match by name" technique
/// `run_command.rs`'s own tests already use for this exact reason (a
/// `Document`/`PatternData` pair has no direct name -> coordinate lookup
/// of its own).
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
        .unwrap_or_else(|| panic!("no point named {name:?} in the shirt-front fixture's history"))
        .id;
    let resolved = data.get_point(id).unwrap();
    (resolved.x, resolved.y)
}

/// Signed perpendicular "distance" (unnormalized: a 2D cross product, not
/// divided by the chord's own length) of `point` from the line through
/// `chord_start` -> `chord_end`. Strictly positive/negative tells `point`
/// apart from the chord itself; the SIGN's meaning (which side is
/// "concave"/"convex") is specific to which two points are passed as the
/// chord's own start/end, and is derived once, by hand, for each of the
/// two checks below — see each call site's own comment for that
/// derivation.
fn signed_side(chord_start: (f64, f64), chord_end: (f64, f64), point: (f64, f64)) -> f64 {
    let dir = (chord_end.0 - chord_start.0, chord_end.1 - chord_start.1); // the chord's own direction vector
    let to_point = (point.0 - chord_start.0, point.1 - chord_start.1); // from the chord's start to the point being tested
    dir.0 * to_point.1 - dir.1 * to_point.0 // the 2D cross product: its sign says which side of the chord `point` falls on
}

/// A minimum magnitude a "concave"/"convex" signed distance must clear to
/// count as a REAL deviation from the chord, not floating-point noise. The
/// neck/armhole curves are built as a quadratic Bezier (tangent to the
/// shoulder line and to the center-front/side-seam lines at their own
/// endpoints — a quarter-ellipse-style construction), evaluated at exact
/// points via de Casteljau's algorithm (see fixtures/actions/shirt_front.json),
/// so there's no single "intentional offset" constant to compare this
/// against, but every evaluated point still lies strictly inside the same
/// control triangle (and therefore strictly off the chord) unless the
/// control point is itself degenerate, so a small fixed epsilon remains
/// the right guard against that near-zero case.
const MIN_DEVIATION: f64 = 1e-6;

#[test]
fn neck_curve_intermediate_points_are_strictly_concave() {
    let (document, data) = run_shirt_front_fixture();
    let chord_start = point(&document, &data, "A5"); // the neck-width/shoulder-line point
    let chord_end = point(&document, &data, "A2"); // the front-neck-depth center-front point
                                                   // Perimeter order from A5 to A2 is A5, A16, A13, A19, A2 (see
                                                   // fixtures/actions/shirt_front.json): all three intermediate
                                                   // points — each an EXACT point on the quadratic Bezier through
                                                   // A5, control point A10, A2 (t=0.25, 0.5, 0.75 respectively),
                                                   // via true de Casteljau evaluation (nested Midpoint calls).
    for name in ["A16", "A13", "A19"] {
        let p = point(&document, &data, name);
        // Hand-derived sign (see fixtures/actions/shirt_front.json's own
        // construction and this repo's Phase 3 iteration log): with
        // chord_start = A5 and chord_end = A2, a point on the concave
        // (center-front, "cutting more fabric away") side of the chord
        // has a STRICTLY POSITIVE signed_side value.
        let side = signed_side(chord_start, chord_end, p);
        assert!(
            side > MIN_DEVIATION,
            "neck curve point {name} is not strictly concave: signed_side={side} for point {p:?} \
             relative to chord {chord_start:?} -> {chord_end:?} (expected a value > {MIN_DEVIATION})"
        );
    }
}

#[test]
fn armhole_curve_intermediate_points_are_strictly_convex() {
    let (document, data) = run_shirt_front_fixture();
    let chord_start = point(&document, &data, "A7"); // the shoulder point
    let chord_end = point(&document, &data, "A8"); // the underarm/side-seam point
                                                   // Perimeter order from A7 to A8 (via the armhole curve, in the
                                                   // REVERSE traversal direction the perimeter itself uses, A8 ->
                                                   // A29 -> A23 -> A26 -> A7) is checked here from A7's own side:
                                                   // all three intermediate points — exact quadratic-Bezier points
                                                   // through A7, control point A20, A8 (t=0.25/0.5/0.75), same
                                                   // de Casteljau technique as the neck curve above.
    for name in ["A26", "A23", "A29"] {
        let p = point(&document, &data, name);
        // Hand-derived sign, same technique as the neck check above, but
        // — as the task's own inverted-check instructions describe —
        // this is checked against the OPPOSITE (convex, bulging away from
        // center front) side. With chord_start = A7 and chord_end = A8,
        // that convex side also comes out STRICTLY POSITIVE for this
        // particular chord's own orientation (down-and-right, rather than
        // the neck chord's down-and-left) — a coincidence of this
        // fixture's own geometry, not a general rule; see the construction
        // log for the actual hand derivation.
        let side = signed_side(chord_start, chord_end, p);
        assert!(
            side > MIN_DEVIATION,
            "armhole curve point {name} is not strictly convex: signed_side={side} for point {p:?} \
             relative to chord {chord_start:?} -> {chord_end:?} (expected a value > {MIN_DEVIATION})"
        );
    }
}

#[test]
fn perimeter_is_a_simple_non_self_intersecting_closed_polygon() {
    let (document, data) = run_shirt_front_fixture();
    let perimeter_names = [
        "A5", "A16", "A13", "A19", "A2", "A4", "A9", "A8", "A29", "A23", "A26", "A7",
    ]; // the exact perimeter order fixtures/actions/shirt_front.json's own add_piece action lists
    let perimeter: Vec<(f64, f64)> = perimeter_names
        .iter()
        .map(|name| point(&document, &data, name))
        .collect();
    let n = perimeter.len();

    // Every pair of NON-ADJACENT edges (sharing no endpoint, including the
    // wrap-around edge) must not cross.
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

/// Standard orientation-based segment-intersection test: true when segment
/// `p1`->`p2` and segment `p3`->`p4` cross (including touching at a
/// non-endpoint interior point), false otherwise.
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
fn overall_proportions_are_within_twenty_percent_of_half_chest_and_body_length() {
    let (document, data) = run_shirt_front_fixture();
    let perimeter_names = [
        "A5", "A16", "A13", "A19", "A2", "A4", "A9", "A8", "A29", "A23", "A26", "A7",
    ];
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

    // fixtures/measurements/shirt_front.json's own bundled values.
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
