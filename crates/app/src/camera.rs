/// Converts pattern-space coordinates (what [`render::DrawCommand`]s carry)
/// into screen-pixel coordinates for egui's painter.
///
/// Deliberately lives in `app`, not `render`: pan/zoom is a per-window,
/// per-session UI concern, not something a pure `PatternData -> draw
/// commands` translation layer should know about.
#[derive(Debug, Clone, Copy)] // Debug: printable; Clone/Copy: this is a small plain-data value, cheap to pass/store by value
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
}
