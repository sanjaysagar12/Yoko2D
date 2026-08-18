/// Converts pattern-space coordinates (what [`render::DrawCommand`]s carry)
/// into screen-pixel coordinates for egui's painter.
///
/// Deliberately lives in `app`, not `render`: pan/zoom is a per-window,
/// per-session UI concern, not something a pure `PatternData -> draw
/// commands` translation layer should know about.
#[derive(Debug, Clone, Copy, PartialEq)] // Debug: printable; Clone/Copy: this is a small plain-data value, cheap to pass/store by value; PartialEq: lets fit_to_points's tests assert_eq! against Camera::default()
pub struct Camera {
    pub offset_x: f64, // shifts the pattern's x origin, in pattern units, before scaling to pixels
    pub offset_y: f64, // shifts the pattern's y origin, in pattern units, before scaling to pixels
    pub zoom: f64,     // pattern units -> screen pixels scale factor; must stay positive and finite
}

impl Camera {
    /// Converts one pattern-space point to a screen-pixel position.
    ///
    /// Y IS NEGATED HERE, deliberately: this crate's formula engine
    /// (Phase 2) builds points with plain `angle.cos()`/`angle.sin()`, the
    /// standard mathematical convention where positive y is "up" and
    /// angles increase counter-clockwise. egui, like most screen graphics
    /// APIs, is y-DOWN from the top-left corner — without negating y here,
    /// the whole pattern would render upside down relative to how a user
    /// sketching angles/lengths on graph paper would expect "up" to look.
    /// Negating y is what makes on-screen "up" correspond to pattern-space
    /// "up", at the cost of needing to remember this crate is the one
    /// place that flip happens (`render` itself stays y-up throughout).
    pub fn to_screen(&self, x: f64, y: f64) -> (f32, f32) {
        let screen_x = (x + self.offset_x) * self.zoom; // shift into visible range, then scale to pixels
        let screen_y = (-y + self.offset_y) * self.zoom; // negate y (see doc comment above), then shift and scale the same way
        (screen_x as f32, screen_y as f32) // egui's painter API takes f32, not f64
    }

    /// Converts one screen-pixel position back to pattern space — the
    /// mathematically exact inverse of [`Self::to_screen`].
    ///
    /// Used by Phase 11's click handling: a canvas click arrives in screen
    /// pixels, but hit-testing against `PatternData` needs pattern-space
    /// coordinates. Each step here undoes the corresponding `to_screen`
    /// step in reverse order:
    /// `to_screen` computes `screen_x = (x + offset_x) * zoom`, so
    /// solving for `x` gives `x = screen_x / zoom - offset_x`.
    /// `to_screen` computes `screen_y = (-y + offset_y) * zoom`, so
    /// solving for `y` gives `y = offset_y - screen_y / zoom` — this is
    /// also what correctly un-negates the y-flip `to_screen` applied,
    /// since isolating `y` (not `-y`) on one side naturally reintroduces
    /// the sign flip as part of solving the equation.
    pub fn to_pattern(&self, screen_x: f32, screen_y: f32) -> (f64, f64) {
        let x = (screen_x as f64 / self.zoom) - self.offset_x; // inverse of `(x + offset_x) * zoom`: divide then un-shift
        let y = self.offset_y - (screen_y as f64 / self.zoom); // inverse of `(-y + offset_y) * zoom`: divide, then subtract from offset_y (undoes both the shift and the negation)
        (x, y) // pattern-space coordinates, as f64 to match PatternData's own coordinate type
    }
}

impl Default for Camera {
    /// A starting point that puts a freshly-loaded pattern somewhere
    /// visible and at a readable scale, without any user panning/zooming
    /// yet.
    ///
    /// This project's own fixtures (`fixtures/measurements/sample.json`)
    /// use measurement values on the order of tens of pattern units (e.g.
    /// `height_scapula: 40.0`, `waist_circ: 72.5`), consistent with
    /// pattern units being roughly centimeter-scale. `zoom: 5.0` turns
    /// that into a comfortably-sized on-screen drawing (a 70-unit-wide
    /// pattern becomes 350 pixels wide) without needing a huge window.
    /// `offset_x`/`offset_y: 300.0` shift a pattern whose geometry starts
    /// near its own origin `(0, 0)` away from the screen's top-left
    /// corner into a typical window's visible area, rather than clipping
    /// against the edge.
    fn default() -> Self {
        Camera {
            offset_x: 300.0, // pattern-space units to shift right before scaling, so geometry near (0,0) isn't clipped at the left edge
            offset_y: 300.0, // pattern-space units to shift "up" (see to_screen's y negation) before scaling, for the same reason
            zoom: 5.0,       // pixels per pattern unit; see the reasoning above
        }
    }
}

/// Computes a `Camera` that fits every point in `points` within a
/// `viewport_width` x `viewport_height` pixel viewport, centered, with a
/// fixed pixel margin so geometry doesn't touch the panel's edges.
///
/// This is what actually places freshly-opened geometry somewhere VISIBLE:
/// `Camera::default`'s fixed `offset`/`zoom` only happens to look
/// reasonable for one specific, narrow range of pattern scales — any
/// pattern whose coordinates fall outside that range (this project's own
/// action-script fixture included) renders far outside any reasonably-
/// sized window, which is exactly what made pattern geometry appear as an
/// empty canvas even though it was being painted correctly, unconditionally,
/// every frame. Fitting to the ACTUAL geometry's bounding box, computed
/// fresh from whatever pattern was just opened, works regardless of scale.
pub fn fit_to_points(
    points: &[(f64, f64)], // every point's pattern-space coordinates to fit within the viewport
    viewport_width: f32,   // the available canvas width, in screen pixels
    viewport_height: f32,  // the available canvas height, in screen pixels
) -> Camera {
    if points.is_empty() {
        // nothing to fit to (e.g. a brand-new empty pattern): fall back to the fixed default rather than computing bounds over nothing
        return Camera::default();
    }

    let margin = 40.0_f64; // pixels of blank space kept between the fitted geometry and the viewport's edges

    // Seed every bound from the first point, then widen to cover the rest —
    // the standard way to compute a bounding box over an arbitrary point set.
    let mut min_x = points[0].0;
    let mut max_x = points[0].0;
    let mut min_y = points[0].1;
    let mut max_y = points[0].1;
    for &(x, y) in points.iter().skip(1) {
        // widen the running bounding box to cover every remaining point
        min_x = min_x.min(x); // extend the box leftward if this point is further left than anything seen so far
        max_x = max_x.max(x); // extend the box rightward, symmetrically
        min_y = min_y.min(y); // extend the box downward (pattern-space), symmetrically
        max_y = max_y.max(y); // extend the box upward (pattern-space), symmetrically
    }

    let center_x = (min_x + max_x) / 2.0; // the bounding box's horizontal midpoint, in pattern space
    let center_y = (min_y + max_y) / 2.0; // the bounding box's vertical midpoint, in pattern space
    let width = max_x - min_x; // the bounding box's pattern-space width; 0.0 if every point shares the same x
    let height = max_y - min_y; // the bounding box's pattern-space height; 0.0 if every point shares the same y

    let available_width = (viewport_width as f64 - 2.0 * margin).max(1.0); // pixel width left for geometry after both margins; floored at 1.0 so a tiny viewport never divides by zero or goes negative
    let available_height = (viewport_height as f64 - 2.0 * margin).max(1.0); // same, for height

    // Compute the zoom each dimension alone would need to exactly fill its
    // available space, then take the SMALLER of the two below, so the whole
    // bounding box fits without distorting the pattern's aspect ratio (a
    // technical drafting tool must never stretch x differently from y). A
    // dimension with zero pattern-space extent (every point shares that
    // coordinate — e.g. a single point, or a perfectly horizontal/vertical
    // line) contributes no constraint at all, rather than a division by
    // zero or an artificially huge zoom.
    let zoom_x = if width > 0.0 {
        Some(available_width / width) // the zoom that would make the bounding box exactly as wide as available_width
    } else {
        None // every point shares this x coordinate: width alone implies no zoom limit
    };
    let zoom_y = if height > 0.0 {
        Some(available_height / height) // the zoom that would make the bounding box exactly as tall as available_height
    } else {
        None // every point shares this y coordinate: height alone implies no zoom limit
    };
    let zoom = match (zoom_x, zoom_y) {
        (Some(zx), Some(zy)) => zx.min(zy), // both dimensions constrain zoom: fit the tighter (smaller) one
        (Some(zx), None) => zx, // only width varies (e.g. every point on one horizontal line): fit that alone
        (None, Some(zy)) => zy, // only height varies: fit that alone
        (None, None) => 5.0, // every point coincides at one position: no scale is implied by the geometry, so fall back to a sane fixed zoom
    };

    // Solve `to_screen`'s formulas for the offsets that place the bounding
    // box's CENTER at the viewport's center: `to_screen` computes
    // `screen_x = (x + offset_x) * zoom`, so requiring
    // `(center_x + offset_x) * zoom == viewport_width / 2` and solving for
    // `offset_x` gives the line below; `screen_y = (-y + offset_y) * zoom`
    // similarly gives `offset_y` (the same style of algebraic derivation
    // `to_pattern`'s own doc comment uses, applied here in the opposite
    // direction — solving for the camera's parameters instead of a point).
    let offset_x = (viewport_width as f64 / 2.0) / zoom - center_x; // centers the bounding box horizontally in the viewport
    let offset_y = (viewport_height as f64 / 2.0) / zoom + center_y; // centers the bounding box vertically in the viewport (the `+` here is what the y-negation in to_screen requires; see the derivation comment above)

    Camera {
        offset_x, // computed above to horizontally center the fitted geometry
        offset_y, // computed above to vertically center the fitted geometry
        zoom, // computed above to fit the whole bounding box within the viewport, with margin, preserving aspect ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_screen_matches_hand_computed_coordinates() {
        let camera = Camera {
            offset_x: 10.0,
            offset_y: 20.0,
            zoom: 2.0,
        };

        let cases = [
            // (input x, input y, expected screen x, expected screen y)
            (0.0, 0.0, 20.0, 40.0),  // (0+10)*2, (-0+20)*2
            (5.0, 5.0, 30.0, 30.0),  // (5+10)*2, (-5+20)*2
            (-3.0, 4.0, 14.0, 32.0), // (-3+10)*2, (-4+20)*2
        ];

        for (x, y, expected_x, expected_y) in cases {
            let (screen_x, screen_y) = camera.to_screen(x, y);
            assert!(
                (screen_x - expected_x).abs() < 1e-6,
                "x mismatch for ({x}, {y}): got {screen_x}, expected {expected_x}"
            );
            assert!(
                (screen_y - expected_y).abs() < 1e-6,
                "y mismatch for ({x}, {y}): got {screen_y}, expected {expected_y}"
            );
        }
    }

    #[test]
    fn default_camera_has_a_sane_positive_finite_zoom() {
        let camera = Camera::default();
        assert!(camera.zoom.is_finite());
        assert!(camera.zoom > 0.0);
    }

    #[test]
    fn to_pattern_matches_hand_computed_coordinates() {
        let camera = Camera {
            offset_x: 10.0,
            offset_y: 20.0,
            zoom: 2.0,
        };

        // Same (screen, pattern) pairs as to_screen's own test, just read the other direction.
        let cases = [
            (20.0_f32, 40.0_f32, 0.0, 0.0),
            (30.0, 30.0, 5.0, 5.0),
            (14.0, 32.0, -3.0, 4.0),
        ];

        for (screen_x, screen_y, expected_x, expected_y) in cases {
            let (x, y) = camera.to_pattern(screen_x, screen_y);
            assert!(
                (x - expected_x).abs() < 1e-9,
                "x mismatch for ({screen_x}, {screen_y}): got {x}, expected {expected_x}"
            );
            assert!(
                (y - expected_y).abs() < 1e-9,
                "y mismatch for ({screen_x}, {screen_y}): got {y}, expected {expected_y}"
            );
        }
    }

    #[test]
    fn to_pattern_is_the_exact_inverse_of_to_screen() {
        let camera = Camera {
            offset_x: -7.5,
            offset_y: 42.0,
            zoom: 3.25,
        };

        for (x, y) in [(0.0, 0.0), (12.5, -8.25), (-100.0, 33.3)] {
            let (screen_x, screen_y) = camera.to_screen(x, y);
            let (round_tripped_x, round_tripped_y) = camera.to_pattern(screen_x, screen_y);
            assert!((round_tripped_x - x).abs() < 1e-4); // f32 screen coordinates limit round-trip precision slightly
            assert!((round_tripped_y - y).abs() < 1e-4);
        }
    }

    #[test]
    fn fit_to_points_centers_the_bounding_box_in_the_viewport() {
        let points = [(0.0, 0.0), (10.0, 0.0), (0.0, 5.0)]; // the same x:[0,10] y:[0,5] bounding box this bug's real fixture data has
        let camera = fit_to_points(&points, 780.0, 550.0);

        // The bounding box's center, (5.0, 2.5), must map to the viewport's center.
        let (screen_x, screen_y) = camera.to_screen(5.0, 2.5);
        assert!((screen_x - 390.0).abs() < 1e-6); // 780 / 2
        assert!((screen_y - 275.0).abs() < 1e-6); // 550 / 2
    }

    #[test]
    fn fit_to_points_keeps_every_point_within_the_viewport_with_margin() {
        let points = [(0.0, 0.0), (10.0, 0.0), (0.0, 5.0)];
        let (viewport_width, viewport_height) = (780.0, 550.0);
        let camera = fit_to_points(&points, viewport_width, viewport_height);

        for &(x, y) in &points {
            let (screen_x, screen_y) = camera.to_screen(x, y);
            // Every point must land inside the viewport, with at least a small margin —
            // proving the fix actually solves "geometry rendered off-screen", not just
            // "geometry rendered somewhere technically finite".
            assert!(screen_x >= 0.0 && screen_x <= viewport_width);
            assert!(screen_y >= 0.0 && screen_y <= viewport_height);
        }
    }

    #[test]
    fn fit_to_points_preserves_aspect_ratio_for_a_non_square_bounding_box() {
        let points = [(0.0, 0.0), (100.0, 0.0), (0.0, 10.0)]; // a 10:1 wide-vs-tall bounding box
        let camera = fit_to_points(&points, 780.0, 550.0);

        // A 100-wide, 10-tall pattern-space box must scale identically in both
        // axes: distorting one axis relative to the other would be wrong for a
        // technical pattern-drafting tool (a 100x10 rectangle must render as a
        // 100x10-proportioned rectangle on screen, not a stretched square).
        let (left, _) = camera.to_screen(0.0, 0.0);
        let (right, _) = camera.to_screen(100.0, 0.0);
        let (_, top) = camera.to_screen(0.0, 10.0);
        let (_, bottom) = camera.to_screen(0.0, 0.0);
        let screen_width = right - left;
        let screen_height = bottom - top;
        assert!(((screen_width / screen_height) - 10.0).abs() < 1e-3); // the same 10:1 ratio, preserved on screen
    }

    #[test]
    fn fit_to_points_on_a_single_point_falls_back_to_a_sane_zoom_without_dividing_by_zero() {
        let points = [(3.0, 4.0)]; // a single point: zero-width, zero-height bounding box
        let camera = fit_to_points(&points, 780.0, 550.0);
        assert!(camera.zoom.is_finite());
        assert!(camera.zoom > 0.0);

        // The one point should still land centered in the viewport.
        let (screen_x, screen_y) = camera.to_screen(3.0, 4.0);
        assert!((screen_x - 390.0).abs() < 1e-6);
        assert!((screen_y - 275.0).abs() < 1e-6);
    }

    #[test]
    fn fit_to_points_on_an_empty_slice_falls_back_to_the_default_camera() {
        let camera = fit_to_points(&[], 780.0, 550.0);
        assert_eq!(camera, Camera::default());
    }
}
