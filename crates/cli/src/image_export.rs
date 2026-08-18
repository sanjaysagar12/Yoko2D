use std::collections::HashMap; // maps a resolved point's ObjectId back to its user-facing name, for labeling
use std::path::Path; // the parameter type for export_pattern_image's own `path`, matching this binary's other file-path parameters

use plotters::prelude::*; // BitMapBackend, DrawingArea, Circle/PathElement/Text elements, colors, IntoFont — this module's whole drawing toolkit
use thiserror::Error; // brings in the `Error` derive macro used by `ImageExportError` below

/// One resolved `Piece`'s contour, plus its seam allowance if it has one —
/// factored out purely to keep `export_pattern_image`'s own local variable
/// declarations readable (clippy's `type_complexity` lint otherwise flags
/// the inline nested-generic spelling of this exact shape).
type ResolvedPiece = (Vec<(f64, f64)>, Option<Vec<(f64, f64)>>);

/// How many straight-line segments an `Arc`/`Spline` curve is tessellated
/// into for drawing. Mirrors `render::CURVE_SAMPLE_SEGMENTS` — this module
/// draws directly via `plotters` rather than consuming `render::DrawCommand`,
/// so it keeps its own copy of the same named constant instead of a magic
/// number, exactly matching that crate's own reasoning for why the sample
/// count deserves a name.
const CURVE_SAMPLE_SEGMENTS: usize = 32;

/// Everything that can go wrong turning a resolved [`core_lib::PatternData`]
/// into a PNG file.
#[derive(Debug, Error)] // Debug: printable in test failures/CLI error messages; Error: implements std::error::Error via thiserror
pub enum ImageExportError {
    /// A `Line` (or a `Piece` node) referenced a point id that isn't
    /// actually a point in the given `PatternData` — the same dangling-
    /// reference scenario `render::RenderError::DanglingReference` guards
    /// against, propagated here via `core_lib`'s own `ContainerError`
    /// instead of `render`'s type, since this module resolves points
    /// itself (to keep each id available for name-label lookup) rather
    /// than going through `render::render`.
    #[error("pattern geometry error: {0}")]
    Pattern(#[from] core_lib::ContainerError), // wraps whichever ContainerError variant caused the failure

    /// Plotters' own bitmap-backend drawing/encoding failed. Stored as a
    /// formatted message rather than the original error value: plotters'
    /// real error type here is `DrawingAreaErrorKind<BitMapBackendError>`,
    /// generic over a backend error type this crate has no direct
    /// dependency on naming (only `plotters` itself is a dependency, not
    /// the `plotters-bitmap` crate it pulls in internally) — `.to_string()`
    /// captures the same diagnostic information without that coupling.
    #[error("failed to draw pattern image: {0}")]
    Drawing(String), // a human-readable description of the underlying plotters failure

    /// Writing the finished image file to disk failed at the OS level
    /// (e.g. the destination path's parent directory doesn't exist).
    /// Wrapped via `#[from]` so `?` converts a `std::io::Error`
    /// automatically, matching this project's established error-handling
    /// conventions everywhere else (e.g. `action_script::ActionScriptError::Io`).
    #[error("failed to write image file: {0}")]
    Io(#[from] std::io::Error), // the underlying I/O failure
}

/// Renders every resolved point (as a marker + name label), line, and piece
/// contour/seam-allowance in `data` to a PNG file at `path`.
///
/// `document` is needed purely to recover each point's user-facing NAME:
/// `core_lib::PointData` itself stores only `x`/`y`, never a name — names
/// live one layer up, as a field inside whichever `ToolKind` produced that
/// point (see `action_script::tool_name`'s own doc comment for the same
/// observation) — so labeling points by name requires walking `document`'s
/// history alongside `data`'s resolved coordinates.
pub fn export_pattern_image(
    data: &core_lib::PatternData,  // the resolved pattern geometry to draw
    document: &core_lib::Document, // supplies each point's name, via its tool history
    path: &Path,                   // where to write the PNG file
    width: u32,                    // the output image's pixel width
    height: u32,                   // the output image's pixel height
) -> Result<(), ImageExportError> {
    // Builds the id -> name lookup table this function needs for labels,
    // by walking `document`'s history once and keeping only the named
    // tools (mirrors `main.rs`'s own "Tools:" listing, which does the same
    // walk for the same reason).
    let mut point_names: HashMap<core_lib::ObjectId, &str> = HashMap::new(); // accumulates id -> name pairs as history is walked below
    for record in document.history() {
        // every ToolRecord in this document's history, in the order tools were originally added
        if let Some(name) = crate::action_script::tool_name(&record.kind) {
            // only named tools (i.e. every point-producing kind) have anything to add here
            point_names.insert(record.id, name); // record this tool's id -> name mapping for the label pass below
        }
    }

    // Resolves every drawable object in `data` into plain (already-
    // labeled, already-coordinate) values ONE time, up front: the same
    // resolved lists are then walked twice below (once to compute the
    // bounding box, once to actually draw), so lookups against `data`
    // itself only ever happen in this one pass.
    let mut resolved_points: Vec<(core_lib::ObjectId, f64, f64)> = Vec::new(); // every Point object's id and coordinates
    let mut resolved_lines: Vec<((f64, f64), (f64, f64))> = Vec::new(); // every Line's two resolved endpoints
    let mut resolved_pieces: Vec<ResolvedPiece> = Vec::new(); // every Piece's contour, plus its seam allowance if any
    let mut resolved_polylines: Vec<Vec<(f64, f64)>> = Vec::new(); // every Arc/Spline, tessellated into an open sample-point chain

    for (id, object) in data.objects() {
        // walk every object in `data`, in the same deterministic ascending-id order `render::render` relies on
        match object {
            core_lib::GeoObject::Point(point) => {
                resolved_points.push((*id, point.x, point.y)); // keep this point's own id (for its label) alongside its coordinates
            }
            core_lib::GeoObject::Line(line) => {
                let p1 = data.get_point(line.p1)?; // resolve the first endpoint; propagates ContainerError via ImageExportError::Pattern on a dangling reference
                let p2 = data.get_point(line.p2)?; // resolve the second endpoint, same rationale
                resolved_lines.push(((p1.x, p1.y), (p2.x, p2.y))); // store both endpoints' already-resolved coordinates
            }
            core_lib::GeoObject::Piece(piece) => {
                // both already resolved coordinate lists; no further lookups needed
                resolved_pieces.push((piece.contour.clone(), piece.seam_allowance.clone()));
            }
            core_lib::GeoObject::Arc(arc) => {
                let mut points = Vec::with_capacity(CURVE_SAMPLE_SEGMENTS + 1); // one sample per segment boundary, including both ends
                for i in 0..=CURVE_SAMPLE_SEGMENTS {
                    let t = i as f64 / CURVE_SAMPLE_SEGMENTS as f64; // 0.0..=1.0, evenly spaced across the sweep
                    let theta_deg =
                        arc.start_angle_deg + t * (arc.end_angle_deg - arc.start_angle_deg); // linearly interpolate the angle across the sweep
                    let theta_rad = theta_deg.to_radians(); // degrees -> radians before cos/sin, same convention as core_lib::geo::ArcData's own doc comment
                    let x = arc.center.0 + arc.radius * theta_rad.cos(); // center + radius*cos(theta)
                    let y = arc.center.1 + arc.radius * theta_rad.sin(); // center + radius*sin(theta)
                    points.push((x, y));
                }
                resolved_polylines.push(points);
            }
            core_lib::GeoObject::Spline(spline) => {
                let mut points = Vec::with_capacity(CURVE_SAMPLE_SEGMENTS + 1); // one sample per segment boundary, including both ends
                for i in 0..=CURVE_SAMPLE_SEGMENTS {
                    let t = i as f64 / CURVE_SAMPLE_SEGMENTS as f64; // 0.0..=1.0, evenly spaced along the curve
                    points.push(core_lib::geometry::cubic_bezier_point(
                        spline.p1, spline.p2, spline.p3, spline.p4, t,
                    )); // reuses core_lib's own Bezier formula rather than reimplementing it here
                }
                resolved_polylines.push(points);
            }
        }
    }

    // Computes the bounding box over EVERY coordinate this image will
    // actually draw — not just Points: a Piece's seam-allowance offset
    // introduces coordinates that don't correspond to any standalone Point
    // object, so leaving them out here would silently clip the seam
    // allowance at the image edge.
    let mut all_coords: Vec<(f64, f64)> = resolved_points
        .iter()
        .map(|&(_, x, y)| (x, y)) // strip the id: only the coordinate matters for the bounding box itself
        .collect();
    for (p1, p2) in &resolved_lines {
        // Line endpoints are already covered by resolved_points above
        // (every Line's endpoints are themselves named Point objects in
        // `data`), but included again here anyway: cheap, and keeps this
        // bounding-box pass correct even if that invariant ever changes.
        all_coords.push(*p1);
        all_coords.push(*p2);
    }
    for (contour, seam_allowance) in &resolved_pieces {
        all_coords.extend(contour.iter().copied()); // every contour vertex contributes its own coordinate
        if let Some(sa) = seam_allowance {
            all_coords.extend(sa.iter().copied()); // the offset seam-allowance boundary, genuinely new coordinates not present anywhere else
        }
    }
    for points in &resolved_polylines {
        // Arc/Spline sample points: genuinely new coordinates (interior
        // control points/arc sweep extremes) that don't necessarily
        // coincide with any standalone Point object, so — same rationale as
        // the seam-allowance loop above — leaving these out would risk
        // silently clipping a curve at the image edge.
        all_coords.extend(points.iter().copied());
    }

    // Seeds the running bounding box from the first coordinate, then widens
    // it to cover every remaining one — the same pattern `app::camera::
    // fit_to_points` itself already uses internally for the same purpose.
    let (mut min_x, mut min_y, mut max_x, mut max_y) = match all_coords.first() {
        Some(&(x, y)) => (x, y, x, y), // start every bound at the first coordinate
        None => (0.0, 0.0, 0.0, 0.0), // an entirely empty pattern: no geometry to bound, but this function must still produce a valid (blank) image rather than panicking
    };
    for &(x, y) in &all_coords {
        // widen the running bounding box to cover every remaining coordinate
        min_x = min_x.min(x); // extend leftward if this coordinate is further left than anything seen so far
        max_x = max_x.max(x); // extend rightward, symmetrically
        min_y = min_y.min(y); // extend downward (pattern-space), symmetrically
        max_y = max_y.max(y); // extend upward (pattern-space), symmetrically
    }

    // Margin: 10% of the bounding box's LARGER dimension, added on every
    // side, so nothing touches the image's edge. A degenerate bounding box
    // (every coordinate sharing an x and/or y — e.g. a single point, or a
    // perfectly horizontal/vertical arrangement) would make this margin
    // 0.0 too if computed naively from a zero-sized box, so a minimum
    // fallback span stands in for the larger dimension whenever the real
    // one collapses to zero, guaranteeing a strictly positive margin (and
    // therefore a strictly positive expanded width/height) in every case.
    const MIN_FALLBACK_SPAN: f64 = 10.0; // an arbitrary but reasonable minimum pattern-space span, in the same units this project's measurements already use (centimeters)
    let larger_dimension = (max_x - min_x).max(max_y - min_y); // the bounding box's own larger extent, before any margin is applied
    let effective_larger_dimension = if larger_dimension > 0.0 {
        larger_dimension // a real, non-degenerate extent: use it directly
    } else {
        MIN_FALLBACK_SPAN // every coordinate coincided on both axes: fall back to a sane fixed span instead of a zero margin
    };
    let margin = effective_larger_dimension * 0.1; // 10% of the (possibly-fallback) larger dimension, per this function's own contract

    // The margin-expanded bounding box's own two opposite corners — handed
    // to `fit_to_points` below as the ONLY points it needs to see: since
    // every real coordinate already lies within [min_x,max_x]x[min_y,max_y]
    // (by construction, from the widening loop above), it's also within
    // this strictly larger, margin-expanded box, so these two corners alone
    // are sufficient to make `fit_to_points` compute a camera that fits
    // everything with the desired pattern-space margin baked in.
    let expanded_min = (min_x - margin, min_y - margin); // the padded box's bottom-left corner, in pattern space
    let expanded_max = (max_x + margin, max_y + margin); // the padded box's top-right corner, in pattern space

    // Reuses `app::camera::fit_to_points` rather than re-deriving the
    // scale/offset/y-flip math independently: this is the exact function
    // (and therefore the exact y-negation convention, documented on
    // `Camera::to_screen`) the GUI itself uses to fit freshly-opened
    // geometry into its own canvas, so an exported image and the GUI's own
    // view of the same pattern are guaranteed to read "the same way up"
    // without this module needing to duplicate (and risk diverging from)
    // that convention. `app` is already a dependency of this crate for
    // `app::run_with_document`; `camera` carries no egui/eframe types of
    // its own (`Camera::to_screen` returns a plain `(f32, f32)`), so using
    // it here doesn't pull any windowing/GPU dependency into this
    // otherwise headless PNG-export path.
    let camera = app::camera::fit_to_points(
        &[expanded_min, expanded_max], // the padded bounding box's two corners are sufficient; see the comment above
        width as f32,                  // the target image's pixel width
        height as f32,                 // the target image's pixel height
    );

    // Opens the actual bitmap canvas this function draws onto, sized
    // exactly `width` x `height`, and paints a plain white background — a
    // static exported image, not a themed live GUI, so a fixed white
    // background with dark strokes is the right, unambiguous choice here
    // (unlike the `app` crate's dark-theme colors, chosen specifically for
    // contrast against THAT crate's own dark GUI background, not this one).
    let root = BitMapBackend::new(path, (width, height)).into_drawing_area(); // the drawing surface this whole function paints onto
    root.fill(&WHITE)
        .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // clears the canvas to solid white before anything else is drawn

    // Draws every construction line first, so points/labels painted
    // afterward land visually on top of (not underneath) any line they
    // happen to touch.
    for (p1, p2) in &resolved_lines {
        let (x1, y1) = camera.to_screen(p1.0, p1.1); // pattern space -> screen pixels, first endpoint
        let (x2, y2) = camera.to_screen(p2.0, p2.1); // pattern space -> screen pixels, second endpoint
        root.draw(&PathElement::new(
            vec![(x1 as i32, y1 as i32), (x2 as i32, y2 as i32)], // BitMapBackend's own pixel coordinates are i32, not the f32 Camera::to_screen returns
            ShapeStyle::from(&BLACK).stroke_width(1), // dark, clearly visible against the white background, per this function's own contract
        ))
        .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // propagate a drawing failure rather than silently skipping this line
    }

    // Draws every piece's contour (and seam allowance, if present) as a
    // closed polygon outline, in a distinct color from plain construction
    // lines so pieces are visually distinguishable at a glance.
    for (contour, seam_allowance) in &resolved_pieces {
        draw_closed_polygon(&root, &camera, contour, &RGBColor(30, 90, 200))?; // the piece's own boundary, in a mid-blue distinct from black construction lines
        if let Some(sa) = seam_allowance {
            draw_closed_polygon(&root, &camera, sa, &RGBColor(30, 90, 200))?; // the offset seam-allowance boundary, same distinguishing color as its own contour
        }
    }

    // Draws every Arc/Spline's tessellated sample chain as an OPEN polyline
    // (no wrap-around edge, unlike a piece's closed contour) — in a distinct
    // green so curve tessellation reads as its own visual category next to
    // the black construction lines and blue piece outlines.
    for points in &resolved_polylines {
        draw_open_polyline(&root, &camera, points, &RGBColor(40, 160, 80))?;
    }

    // Draws every point LAST: a small filled marker plus its name label,
    // offset a few pixels up-and-right so the text doesn't sit exactly on
    // top of the marker itself.
    for (id, x, y) in &resolved_points {
        let (screen_x, screen_y) = camera.to_screen(*x, *y); // pattern space -> screen pixels, this point's own position
        let (px, py) = (screen_x as i32, screen_y as i32); // BitMapBackend's own pixel coordinates are i32, not the f32 Camera::to_screen returns
        root.draw(&Circle::new(
            (px, py),                          // the marker's screen-pixel center
            3, // a small fixed radius, in pixels, regardless of the image's own scale
            ShapeStyle::from(&BLACK).filled(), // a solid dark dot, clearly visible against the white background
        ))
        .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // propagate a drawing failure rather than silently skipping this marker
        if let Some(name) = point_names.get(id) {
            // only Points that resolve back to a named tool get a label; every Point this codebase produces today always does, but this stays defensive rather than assuming it
            root.draw(&Text::new(
                name.to_string(), // this point's user-facing name, e.g. "A1", exactly as given in the action script/pattern file
                (px + 6, py - 6), // a few pixels up-and-right of the marker, so the label doesn't sit exactly on top of the point itself
                ("sans-serif", 14).into_font().color(&BLACK), // a plain, clearly legible label style against the white background
            ))
            .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // propagate a drawing failure rather than silently skipping this label
        }
    }

    root.present()
        .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // flushes every draw call above and actually encodes/writes the PNG file to `path`
    Ok(()) // every object drawn and the file written successfully
}

/// Draws `points` (already in pattern-space coordinates) as a CLOSED
/// polygon outline in `color`: every consecutive pair of vertices, plus the
/// wrap-around edge from the last vertex back to the first.
///
/// Factored out because [`export_pattern_image`] needs this exact same
/// "close the loop" loop for both a piece's contour and its (optional)
/// seam-allowance boundary.
fn draw_closed_polygon(
    root: &DrawingArea<BitMapBackend, plotters::coord::Shift>, // the same bitmap canvas export_pattern_image itself draws onto
    camera: &app::camera::Camera, // the same pattern-space -> screen-pixel conversion export_pattern_image itself uses
    points: &[(f64, f64)],        // this polygon's vertices, in order, in pattern-space coordinates
    color: &RGBColor,             // this polygon's stroke color
) -> Result<(), ImageExportError> {
    if points.len() < 2 {
        // fewer than 2 points can't form any edge at all: nothing to draw, and no wrap-around edge would make sense either
        return Ok(());
    }
    let screen_points: Vec<(i32, i32)> = points
        .iter()
        .map(|&(x, y)| camera.to_screen(x, y)) // pattern space -> screen pixels, one vertex at a time
        .map(|(sx, sy)| (sx as i32, sy as i32)) // BitMapBackend's own pixel coordinates are i32, not the f32 Camera::to_screen returns
        .collect();
    let vertex_count = screen_points.len(); // needed to compute the wrap-around edge back to the first vertex
    for i in 0..vertex_count {
        // draw every edge, including the wrap-around edge from the last vertex back to the first, closing the loop
        let next_i = (i + 1) % vertex_count; // the next vertex's index, wrapping to 0 after the last
        root.draw(&PathElement::new(
            vec![screen_points[i], screen_points[next_i]], // this edge's two screen-pixel endpoints
            ShapeStyle::from(color).stroke_width(2), // this polygon's own distinguishing color, slightly thicker than a plain construction line
        ))
        .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // propagate a drawing failure rather than silently skipping this edge
    }
    Ok(()) // every edge of this polygon drawn successfully
}

/// Draws `points` (already in pattern-space coordinates) as an OPEN
/// sequence of connected line segments in `color`: every consecutive pair
/// of vertices, with NO wrap-around edge back to the first vertex — the
/// distinction from [`draw_closed_polygon`] above, which a piece's own
/// closed contour needs but a curve's open tessellation must not have.
fn draw_open_polyline(
    root: &DrawingArea<BitMapBackend, plotters::coord::Shift>, // the same bitmap canvas export_pattern_image itself draws onto
    camera: &app::camera::Camera, // the same pattern-space -> screen-pixel conversion export_pattern_image itself uses
    points: &[(f64, f64)], // this curve's sample points, in order, in pattern-space coordinates
    color: &RGBColor,      // this curve's stroke color
) -> Result<(), ImageExportError> {
    if points.len() < 2 {
        // fewer than 2 points can't form any edge at all: nothing to draw
        return Ok(());
    }
    let screen_points: Vec<(i32, i32)> = points
        .iter()
        .map(|&(x, y)| camera.to_screen(x, y)) // pattern space -> screen pixels, one sample at a time
        .map(|(sx, sy)| (sx as i32, sy as i32)) // BitMapBackend's own pixel coordinates are i32, not the f32 Camera::to_screen returns
        .collect();
    for pair in screen_points.windows(2) {
        // draw every consecutive sample pair as one segment; windows(2) naturally stops short of any wrap-around edge
        root.draw(&PathElement::new(
            vec![pair[0], pair[1]], // this segment's two screen-pixel endpoints
            ShapeStyle::from(color).stroke_width(2), // this curve's own distinguishing color, matching draw_closed_polygon's own stroke width
        ))
        .map_err(|err| ImageExportError::Drawing(err.to_string()))?; // propagate a drawing failure rather than silently skipping this segment
    }
    Ok(()) // every segment of this polyline drawn successfully
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a small triangle of three named points and two lines (an "A",
    /// "B", "C" hand-built Document/PatternData pair), the exact fixture
    /// shape this module's own tests need repeatedly.
    fn triangle_document_and_data() -> (core_lib::Document, core_lib::PatternData) {
        let mut document = core_lib::Document::default();
        let a = document.add_base_point("A", 0.0, 0.0);
        let b = document.add_base_point("B", 10.0, 0.0);
        let c = document.add_base_point("C", 0.0, 10.0);
        document.add_line(a, b).unwrap();
        document.add_line(b, c).unwrap();
        let data = core_lib::recompute_all(&document).unwrap();
        (document, data)
    }

    #[test]
    fn export_pattern_image_produces_a_valid_nonempty_png_of_the_requested_size() {
        let (document, data) = triangle_document_and_data();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("triangle.png");

        export_pattern_image(&data, &document, &path, 400, 300).unwrap();

        let metadata = std::fs::metadata(&path).unwrap();
        assert!(metadata.len() > 0); // a real PNG was actually written, not an empty file

        // Independently decodes the file via the `image` crate (a dev-only
        // verification dependency, never used by the export path itself)
        // rather than asserting on exact pixel content, which would be
        // fragile against harmless rendering-library changes.
        let decoded = image::open(&path).unwrap();
        assert_eq!(decoded.width(), 400);
        assert_eq!(decoded.height(), 300);
    }

    #[test]
    fn export_pattern_image_on_a_single_point_does_not_panic_and_produces_a_valid_image() {
        let mut document = core_lib::Document::default();
        document.add_base_point("Solo", 5.0, 5.0); // exactly one point: a zero-width, zero-height bounding box
        let data = core_lib::recompute_all(&document).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("solo.png");

        export_pattern_image(&data, &document, &path, 200, 200).unwrap(); // must not panic/divide by zero on a degenerate bounding box

        let decoded = image::open(&path).unwrap();
        assert_eq!(decoded.width(), 200);
        assert_eq!(decoded.height(), 200);
    }
}
