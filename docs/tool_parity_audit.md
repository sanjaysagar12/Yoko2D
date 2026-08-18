# Tool parity audit: Seamly2D vs. Yoko2D

This document compares every entry in Seamly2D's `enum class Tool`
(`src/libs/vmisc/def.h`) against Yoko2D's current `core_lib::ToolKind`
(`crates/core/src/document.rs`). It was produced by cloning
[FashionFreedom/Seamly2D](https://github.com/FashionFreedom/Seamly2D) fresh
and reading the cited source files directly — not from memory of either
codebase.

Status legend:
- **Implemented-Correct** — Yoko2D has a matching `ToolKind` variant, and
  its geometry has been verified (a hand-calculated golden test, an
  independent-derivation check, or a direct reading of the Seamly2D source
  showing the math is trivial/definitional) to match Seamly2D's actual
  behavior. See `crates/core/src/recompute.rs`'s test module for the
  verification tests themselves.
- **Missing** — no `ToolKind` variant exists yet in Yoko2D.
- **N/A** — an abstract/infrastructure category with no geometric meaning
  of its own, not a drawable tool.

| Seamly2D Tool | Seamly2D source file(s) | Yoko2D ToolKind variant | Status | Notes |
|---|---|---|---|---|
| `Arrow` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category, not a drawable tool. |
| `SinglePoint` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category, not a drawable tool. |
| `DoublePoint` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category, not a drawable tool. |
| `LinePoint` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category, not a drawable tool. |
| `AbstractSpline` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category, not a drawable tool. |
| `Cut` | `tools/drawTools/toolpoint/toolsinglepoint/toolcut/vtoolcut.cpp` | `MISSING` | Missing | Base class for `CutArc`/`CutSpline`/`CutSplinePath`; splits a curve at a point along its length. Requires curve/arc `GeoObject` support, out of scope for this task. |
| `BasePoint` | `tools/drawTools/toolpoint/toolsinglepoint/vtoolbasepoint.cpp` | `ToolKind::BasePoint` | Implemented-Correct | A literal coordinate; no geometry to verify beyond storing x/y as given. |
| `EndLine` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolendline.cpp` | `ToolKind::EndLine` | Implemented-Correct | Source confirmed: `line.setAngle(angle); line.setLength(length);`. Verified by its own dedicated golden test at two non-trivial angles (90 and 30 degrees), `end_line_parity_matches_qlinef_setangle_convention_no_sign_flip` — Qt's angle-based line construction maps directly onto Yoko2D's plain `base + length*(cos,sin)` formula with no sign difference. **Phase 0 audit correction (this task):** the only test that previously existed for this tool (`recompute_resolves_end_line_and_line_from_formulas`) used angle=0, which cannot discriminate a sign-flipped or axis-swapped implementation since `sin(0)=0` either way; the audit's prior "verified" claim leaned on the analysis written for `Normal`'s test rather than an EndLine-specific test exercising a non-trivial angle. No bug was found — the existing formula was already correct — but the gap in verification rigor is fixed now. |
| `Line` | (no dedicated math file; a straight reference between two existing points) | `ToolKind::Line` | Implemented-Correct | No resolvable geometry beyond storing two endpoint ids — nothing for Seamly2D's math to diverge from. |
| `AlongLine` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolalongline.cpp` | `ToolKind::AlongLine` | Implemented-Correct | Part B #1. `QLineF::setLength` keeps direction, rescales p2 with no clamping; verified with both a longer- and a shorter-than-original length. See `along_line_parity_extrapolates_and_shortens_like_qlinef_setlength`. |
| `ShoulderPoint` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolshoulderpoint.cpp` | `ToolKind::ShoulderPoint` | Implemented-Correct | **New in this task (Part C).** Circle-ray intersection via a new `geometry::line_circle_intersection` helper; deliberately diverges from Seamly2D by returning `DegenerateGeometry` instead of silently falling back to `p2_line` when no candidate qualifies — see the variant's own doc comment. |
| `Normal` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolnormal.cpp` | `ToolKind::Normal` | Implemented-Correct | Part B #2. `QLineF::normalVector()`'s 90-degree rotation, converted through Qt's y-down-vs-Yoko2D's-y-up coordinate difference, lands on the SAME counter-clockwise rotation Yoko2D's own code already used — no sign bug found. Verified with an asymmetric case ((0,0)->(10,0), angle=0, length=5) that would visibly mirror to (0,-5) if the sign were wrong; it correctly resolves to (0,5). |
| `Bisector` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolbisector.cpp` | `ToolKind::Bisector` | Implemented-Correct | Part B #3. Verified across a ~90, ~20, and ~160 degree case via an independent definitional check (equal angle-to-each-ray via the dot-product formula), not by re-deriving Seamly2D's own `QLineF::angleTo` formula — the vector-sum approach and Seamly2D's own selected bisector agree in every case tested. |
| `LineIntersect` | `tools/drawTools/toolpoint/toolsinglepoint/vtoollineintersect.cpp` | `ToolKind::LineIntersect` | Implemented-Correct | **New in this task (Part C).** Plain infinite-line intersection; reuses `geometry::line_intersection` (now `pub(crate)`), the same helper `offset_polygon`'s own miter-join math already used. |
| `Spline` | `tools/drawTools/toolcurve/vtoolspline.cpp`, `src/libs/vgeometry/vspline.cpp` | `ToolKind::Spline` | Implemented-Correct | **New in this task.** Verified against `VToolSpline::Create`/`VSpline::GetP2()`/`VSpline::GetP3()`, read directly from a fresh Seamly2D clone: both control points are built via `QLineF(p, p+(length,0)).setAngle(angle)` — the SAME `QLineF::setAngle` convention already verified sign-correct (no extra flip needed in this crate's y-up coordinates) by the `EndLine`/`Normal` parity tests, now factored into a single shared `geometry::direction_from_angle_deg` helper reused by `EndLine`/`Arc`/`Spline` alike. `angle2` is confirmed (by reading `GetP3` itself) to be the literal p4->p3 direction, not a tangent-of-travel angle. See `spline_golden_value_bows_away_from_the_straight_p1_p4_line`. |
| `CubicBezier` | `tools/drawTools/toolcurve/vtoolcubicbezier.cpp` | `MISSING` | Missing | A cubic Bezier with EXPLICIT (not angle/length-derived) control points; distinct from `Spline` above, still out of scope for this task. |
| `CutSpline` | `tools/drawTools/toolpoint/toolsinglepoint/toolcut/vtoolcutspline.cpp` | `MISSING` | Missing | A curve-splitting tool; `Spline` itself now exists, but splitting one at a point along its length remains future work. |
| `CutArc` | `tools/drawTools/toolpoint/toolsinglepoint/toolcut/vtoolcutarc.cpp` | `MISSING` | Missing | Same note as `CutSpline`: `Arc` now exists, but splitting one remains future work. |
| `Arc` | `tools/drawTools/toolcurve/vtoolarc.cpp`, `src/libs/vgeometry/varc.cpp` | `ToolKind::Arc` | Implemented-Correct | **New in this task.** Verified against `VToolArc::Create`/`VArc::GetP1()`/`VArc::GetP2()`, read directly from a fresh Seamly2D clone: each endpoint is built via `QLineF(center, center+(radius,0)).setAngle(angle)`, the same `setAngle` convention as `Spline` above — so a point on the arc at `theta` degrees is plain `center + radius*(cos(theta),sin(theta))` with no sign flip. See `arc_golden_value_matches_hand_calculated_point_at_forty_five_degrees`. |
| `ArcWithLength` | `tools/drawTools/toolcurve/vtoolarcwithlength.cpp` | `MISSING` | Missing | Curve tool, same scope note as `Spline`. |
| `SplinePath` | `tools/drawTools/toolcurve/vtoolsplinepath.cpp` | `MISSING` | Missing | Curve tool, same scope note as `Spline`. |
| `CubicBezierPath` | `tools/drawTools/toolcurve/vtoolcubicbezierpath.cpp` | `MISSING` | Missing | Curve tool, same scope note as `Spline`. |
| `CutSplinePath` | `tools/drawTools/toolpoint/toolsinglepoint/toolcut/vtoolcutsplinepath.cpp` | `MISSING` | Missing | Depends on `SplinePath` existing first. |
| `PointOfContact` | `tools/drawTools/toolpoint/toolsinglepoint/vtoolpointofcontact.cpp` | `ToolKind::PointOfContact` | Implemented-Correct | **New in this task (Part C).** Circle-segment intersection via `geometry::line_circle_intersection`, with Seamly2D's exact "prefer the candidate truly on the finite segment, else prefer the one closer to p1" disambiguation, verified with both branches exercised by separate golden tests. |
| `Piece` | seam allowance math: `src/libs/vlayout/vabstractpiece.cpp` (`EkvPoint`) | `ToolKind::Piece` | Implemented-Correct | Part B #6. `offset_polygon` is a documented simplification of `EkvPoint`'s full miter/dart/bevel algorithm; confirmed for a simple convex (equilateral triangle) corner that stays under `EkvPoint`'s own `maxL=2.4` miter-limit constant, EkvPoint's `AngleByLength` reduces to exactly the same plain offset-edge intersection `offset_polygon` always computes. Divergence for concave corners or corners needing `EkvPoint`'s miter-limit bevel remains a documented, deliberate simplification, not a bug — see `crates/core/src/geometry.rs`'s own `offset_polygon` doc comment. |
| `InternalPath` | `tools/nodeDetails/internal_path_tool.cpp` | `MISSING` | Missing | A piece-path-editing feature (an internal reference path within a piece), not a point-construction tool. |
| `NodePoint` | `tools/nodeDetails/vnodepoint.cpp` | `MISSING` | Missing | References an existing point as a piece-boundary node; Yoko2D's `PieceNode` already covers this need directly (a `Piece`'s `nodes` list references points by id), so no separate tool is needed to reach parity in practice. |
| `NodeArc` | `tools/nodeDetails/vnodearc.cpp` | `MISSING` | Missing | Depends on `Arc` existing first. |
| `NodeElArc` | `tools/nodeDetails/vnodeellipticalarc.cpp` | `MISSING` | Missing | Depends on `EllipticalArc` existing first. |
| `NodeSpline` | `tools/nodeDetails/vnodespline.cpp` | `MISSING` | Missing | Depends on `Spline` existing first. |
| `NodeSplinePath` | `tools/nodeDetails/vnodesplinepath.cpp` | `MISSING` | Missing | Depends on `SplinePath` existing first. |
| `Height` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolheight.cpp` | `ToolKind::Height` | Implemented-Correct | Part B #4. `VToolHeight::FindPoint` calls `VGObject::ClosestPoint`, confirmed (by reading its actual implementation) to project onto the INFINITE line with no clamping to the `line_p1`/`line_p2` segment; verified with a target point whose projection lands well outside that segment. |
| `Triangle` | `tools/drawTools/toolpoint/toolsinglepoint/vtooltriangle.cpp` | `ToolKind::Triangle` | Implemented-Correct | **New in this task (Part C).** Note: this file lives under `toolsinglepoint/`, not `tooldoublepoint/` as originally guessed. Seamly2D's actual `FindPoint` is a fragile 1-pixel-per-step unbounded numeric search for the first point where a law-of-cosines angle check passes; by Thales' theorem this is exactly equivalent to intersecting the axis with the circle whose diameter is the hypotenuse segment, computed here directly via `geometry::line_circle_intersection` instead of searched for. |
| `LineIntersectAxis` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoollineintersectaxis.cpp` | `MISSING` | Missing | Intersects a line with an axis at a fixed angle from a point; a variant of `LineIntersect`/`PointOfIntersection`, natural follow-up work. |
| `PointOfIntersectionArcs` | `tools/drawTools/toolpoint/toolsinglepoint/vtoolpointofintersectionarcs.cpp` | `MISSING` | Missing | Requires `Arc` `GeoObject` support first. |
| `PointOfIntersectionCircles` | `tools/drawTools/toolpoint/toolsinglepoint/intersect_circles_tool.cpp` | `MISSING` | Missing | Circle-circle intersection; would reuse a future `circle_circle_intersection` helper alongside this task's new `line_circle_intersection`. |
| `PointOfIntersectionCurves` | `tools/drawTools/toolpoint/toolsinglepoint/vtoolpointofintersectioncurves.cpp` | `MISSING` | Missing | Requires `Spline`/curve `GeoObject` support first. |
| `CurveIntersectAxis` | `tools/drawTools/toolpoint/toolsinglepoint/toollinepoint/vtoolcurveintersectaxis.cpp` | `MISSING` | Missing | Requires curve `GeoObject` support first. |
| `ArcIntersectAxis` | `src/app/seamly2d/xml/vpattern.cpp` (referenced only) | `MISSING` | Missing | Confirmed by reading Seamly2D's own source: every reference to this enum value is annotated `// Same as Tool::CurveIntersectAxis, but tool will never has such type` (e.g. `vpattern.cpp:4168`, `groups_widget.cpp:764`, `history_dialog.cpp:373`) — Seamly2D itself never actually instantiates this as a distinct tool. Listed here per this audit's instructions rather than silently omitted, but there is no real, separate tool behavior to port. |
| `PointOfIntersection` | `tools/drawTools/toolpoint/toolsinglepoint/point_intersectxy_tool.cpp` (internal type `"intersectXY"`) | `ToolKind::PointOfIntersection` | Implemented-Correct | **New in this task (Part C).** Confirmed via source reading to be the plain, axis-aligned member of the `PointOfIntersection*` family — literally `QPointF(firstPoint->x(), secondPoint->y())` — distinct from the Arcs/Circles/Curves variants above, which remain out of scope. Never degenerate. |
| `PointFromCircleAndTangent` | `tools/drawTools/toolpoint/toolsinglepoint/intersect_circletangent_tool.cpp` | `MISSING` | Missing | A tangent-line construction; natural follow-up alongside `ShoulderPoint`/`PointOfContact`. |
| `PointFromArcAndTangent` | `tools/drawTools/toolpoint/toolsinglepoint/vtoolpointfromarcandtangent.cpp` | `MISSING` | Missing | Requires `Arc` `GeoObject` support first. |
| `TrueDarts` | `tools/drawTools/toolpoint/tooldoublepoint/vtooltruedarts.cpp` | `MISSING` | Missing | A two-point-output dart-construction tool; natural follow-up using this task's exact seven-step pattern. |
| `Union` | `tools/union_tool.cpp` | `MISSING` | Missing | Merges two pieces into one; a piece-level operation, not a point-construction tool. |
| `Group` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category (a UI grouping/visibility concept), not a drawable tool. |
| `Rotation` | `tools/drawTools/operation/vtoolrotation.cpp` | `MISSING` | Missing | A transform operation (out of scope per this task's explicit exclusions). |
| `MirrorByLine` | `tools/drawTools/operation/mirror/vtoolmirrorbyline.cpp` | `MISSING` | Missing | A transform operation (out of scope per this task's explicit exclusions). |
| `MirrorByAxis` | `tools/drawTools/operation/mirror/vtoolmirrorbyaxis.cpp` | `MISSING` | Missing | A transform operation (out of scope per this task's explicit exclusions). |
| `Move` | `tools/drawTools/operation/vtoolmove.cpp` | `MISSING` | Missing | A transform operation (out of scope per this task's explicit exclusions). |
| `Midpoint` | (no dedicated Seamly2D source — see Notes) | `ToolKind::Midpoint` | Implemented-Correct | Part B #5. Searched `src/libs/vtools` for "Midpoint": no dedicated tool function exists; Seamly2D achieves this purely via an `AlongLine` at exactly 50% of the segment's own length. Verified as a consistency check between Yoko2D's own two implementations: `Midpoint(p1,p2)` produces the identical resolved point as `AlongLine(p1,p2,dist(p1,p2)/2)`. |
| `EllipticalArc` | `tools/drawTools/toolcurve/vtoolellipticalarc.cpp` | `MISSING` | Missing | Curve tool, same scope note as `Spline`. |
| `AnchorPoint` | `tools/nodeDetails/anchorpoint_tool.cpp` | `MISSING` | Missing | A label/detail-placement anchor, not a pattern-construction point. |
| `InsertNodes` | referenced only via `Tool::InsertNodes` in `src/app/seamly2d/mainwindow.cpp` | `MISSING` | Missing | No dedicated standalone tool class was found (unlike every other entry above); it appears to be a piece-path-editing UI operation implemented inline, not a point/curve-construction tool in the same sense as the others. |
| `BackgroundImage` | `src/libs/tools/images/image_item.h` | `MISSING` | Missing | A reference-image graphics item for tracing over, not a geometric construction tool. |
| `LAST_ONE_DO_NOT_USE` | `src/libs/vmisc/def.h` | — | N/A | Abstract base category: a sentinel marking the end of the enum, explicitly documented as never to be used as a real value. |

## Summary

- **Implemented-Correct: 16** — `BasePoint`, `EndLine`, `Line`, `AlongLine`,
  `Normal`, `Bisector`, `Height`, `Midpoint`, `Piece` (verified in Part B,
  no fixes needed), `ShoulderPoint`, `LineIntersect`,
  `PointOfIntersection`, `Triangle`, `PointOfContact` (implemented in
  Part C), plus `Arc` and `Spline` (newly implemented in this task — the
  first two curve/`GeoObject`-producing tools in this codebase, as opposed
  to every earlier tool producing only a `Point`).
- **Missing: 32**
- **N/A: 7** — `Arrow`, `SinglePoint`, `DoublePoint`, `LinePoint`,
  `AbstractSpline`, `Group`, `LAST_ONE_DO_NOT_USE`.

No Part B parity check required a code fix: every already-implemented
tool's geometry already matched Seamly2D's actual behavior once the exact
source was read and hand-verified. The five Part C tools were implemented
end-to-end (through the CLI's action-script layer only — the GUI remains
read-only per this project's current design) following the identical
seven-step pattern used for the original five construction tools. This
task's `Arc`/`Spline` addition extends that same pattern to curve geometry:
a new `GeoObject` variant each (`ArcData`/`SplineData`), a `ToolKind`
variant, a `Document::add_*` constructor, a `recompute_all` arm, XML
read/write support under their own `<arc>`/`<spline>` tags, an
action-script `add_arc`/`add_spline` op, and tessellation into
`render::DrawCommand::Polyline`/`image_export`'s own open-polyline drawing
for both the GUI and PNG-export rendering paths. `fixtures/actions/
shirt_front.json`'s neck and armhole curves were refactored from a ~10-step
point-of-intersection/midpoint chain each into a single `add_spline` action
each (see the fixture's own comments and `crates/cli/tests/
shirt_front_shape.rs`'s header comment for how the chosen angle/length
formulas reproduce, via exact quadratic-to-cubic Bezier degree elevation,
the identical curve shape the old chain traced by hand).
