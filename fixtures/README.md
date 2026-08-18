# Fixtures

This directory holds sample pattern files, action scripts, and measurement
files used as test fixtures throughout this project.

## `patterns/shirt_front_reference.png`

The exported PNG (`yoko2d-cli run fixtures/actions/shirt_front.json
--export-image ...`) of `fixtures/actions/shirt_front.json`'s resolved
geometry, at the point its automated shape checks
(`crates/cli/tests/shirt_front_shape.rs`) first passed. Committed as a
visual reference for that fixture — if the underlying tools' math ever
changes in a way that alters this shape but somehow still satisfies the
automated checks, a diff against this image is a fast way to notice.
It is NOT itself re-verified by any test (images aren't diffed
pixel-for-pixel anywhere in this project); `shirt_front_shape.rs` is the
actual regression guard.
