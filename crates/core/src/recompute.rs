use thiserror::Error; // brings in the `Error` derive macro used by `PatternError` below

use crate::document::{Document, ToolKind}; // the ordered tool history this module walks
use crate::formula::{eval_formula, flatten_variables}; // Phase 2's formula pipeline, used by EndLine
use crate::{ContainerError, GeoObject, LineData, ObjectId, PatternData, PointData}; // Phase 1's container and geometry types

/// Everything that can go wrong turning a [`Document`] into a resolved
/// [`PatternData`].
#[derive(Debug, Clone, PartialEq, Error)] // Debug/Clone/PartialEq mirror ContainerError/FormulaError so tests can assert_eq! against this too
pub enum PatternError {
    /// A tool referenced an id that hasn't been resolved into geometry yet
    /// at this point in the history walk — either it belongs to a tool
    /// later in `history` (out-of-order dependency) or to no tool at all
    /// (a corrupted document). Distinguished from a generic
    /// `ContainerError::ObjectNotFound` so callers get a precise,
    /// actionable error naming exactly which dependency is missing.
    #[error("missing dependency: object {0:?} has not been resolved yet")]
    MissingDependency(ObjectId), // the id that was looked up and not found among what's been resolved so far

    /// A lookup against the `PatternData` being built failed for a reason
    /// other than "not found yet" (currently only `WrongObjectType`, e.g.
    /// a tool's `p1` names a `Line` instead of a `Point`). Propagated via
    /// `#[from]` so `?` converts it automatically.
    #[error("container error: {0}")]
    Container(#[from] ContainerError), // wraps whichever ContainerError variant caused the failure

    /// One of a tool's formula strings failed to tokenize, parse, or
    /// evaluate. Propagated via `#[from]` so `?` converts it automatically.
    #[error("formula error: {0}")]
    Formula(#[from] crate::formula::FormulaError), // wraps whichever FormulaError variant caused the failure
}

/// Looks up `id` as a point in `data`, translating "not found at all" into
/// [`PatternError::MissingDependency`] (a precise, actionable error) while
/// letting any other [`ContainerError`] (e.g. `WrongObjectType`, if `id`
/// exists but names a `Line`) propagate unchanged via the `#[from]`
/// conversion on [`PatternError::Container`].
///
/// Factored out because `ToolKind::EndLine` and `ToolKind::Line` both need
/// exactly this "resolve a required point dependency" check.
fn resolve_point(data: &PatternData, id: ObjectId) -> Result<&PointData, PatternError> {
    match data.get_point(id) {
        // dispatch on how the lookup went
        Ok(point) => Ok(point), // id exists and holds a point: exactly what the caller needs
        Err(ContainerError::ObjectNotFound(_)) => Err(PatternError::MissingDependency(id)), // not resolved yet: report precisely which id
        Err(other) => Err(other.into()), // any other container error (e.g. WrongObjectType) propagates as-is
    }
}

/// Rebuilds a [`PatternData`] from scratch by walking `doc`'s tool history
/// in order and resolving each tool's formulas/literals against the
/// variables and geometry resolved so far.
///
/// Pure: takes `&Document`, never mutates it, and returns a brand-new
/// `PatternData` rather than modifying one in place — `Document` is the
/// persistent source of truth, `PatternData` is a disposable cache that
/// gets thrown away and rebuilt every time this is called.
pub fn recompute_all(doc: &Document) -> Result<PatternData, PatternError> {
    let mut data = PatternData::default(); // start every recompute from a completely empty cache

    for (name, var) in doc.base_variables() {
        // seed the cache with every base variable before resolving any geometry
        data.add_variable(name.clone(), var.clone()); // `data` owns an independent copy, not a reference into `doc`
    }

    for record in doc.history() {
        // walk the tool history in the exact order the tools were added
        match &record.kind {
            // dispatch on which kind of tool this record is
            ToolKind::BasePoint { x, y, .. } => {
                let point = GeoObject::Point(PointData { x: *x, y: *y }); // a literal coordinate needs no formula evaluation
                data.insert_with_id(record.id, point)?; // place it under the id this tool was assigned when added
            }
            ToolKind::EndLine {
                base_point,
                angle_formula,
                length_formula,
                ..
            } => {
                let base = resolve_point(&data, *base_point)?; // resolve the point this one is measured from
                let vars = flatten_variables(&data); // snapshot every variable resolved so far, for these formulas to reference
                let angle = eval_formula(angle_formula, &vars)?; // evaluate the angle formula (degrees, Phase 2's convention)
                let length = eval_formula(length_formula, &vars)?; // evaluate the length formula
                let x = base.x + length * angle.to_radians().cos(); // degrees -> radians before cos, matching Phase 2's trig convention
                let y = base.y + length * angle.to_radians().sin(); // degrees -> radians before sin, same convention
                let point = GeoObject::Point(PointData { x, y }); // the resolved point for this tool
                data.insert_with_id(record.id, point)?; // place it under this tool's assigned id
            }
            ToolKind::Line { p1, p2 } => {
                resolve_point(&data, *p1)?; // p1 must already be resolved earlier in history
                resolve_point(&data, *p2)?; // p2 must already be resolved earlier in history
                let line = GeoObject::Line(LineData { p1: *p1, p2: *p2 }); // store by id, same as every other geometric reference
                data.insert_with_id(record.id, line)?; // place it under this tool's assigned id
            }
        }
    }

    Ok(data) // every tool resolved without error; hand back the fully-built cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::ToolRecord;
    use crate::variable::Variable;

    // Shared setup: BasePoint "A" at (base_x, base_y), an EndLine "A1" at
    // angle 0 and length height_scapula/10 from A, and a Line from A to
    // A1. Returns the Document plus the ids of A, A1, and the Line, so
    // tests can look up specific resolved objects without recomputing the
    // ids by hand.
    fn sample_document(base_x: f64, base_y: f64) -> (Document, ObjectId, ObjectId, ObjectId) {
        let mut doc = Document::default();
        let a = doc.add_tool(ToolKind::BasePoint {
            name: "A".to_string(),
            x: base_x,
            y: base_y,
        });
        doc.set_variable("height_scapula", Variable::Measurement { value: 40.0 });
        let a1 = doc.add_tool(ToolKind::EndLine {
            name: "A1".to_string(),
            base_point: a,
            angle_formula: "0".to_string(),
            length_formula: "height_scapula/10".to_string(),
        });
        let line = doc.add_tool(ToolKind::Line { p1: a, p2: a1 });
        (doc, a, a1, line)
    }

    #[test]
    fn recompute_resolves_end_line_and_line_from_formulas() {
        let (doc, a, a1, line) = sample_document(0.0, 0.0);
        let data = recompute_all(&doc).unwrap();

        let resolved_a1 = data.get_point(a1).unwrap();
        assert!((resolved_a1.x - 4.0).abs() < 1e-9);
        assert!((resolved_a1.y - 0.0).abs() < 1e-9);

        let resolved_line = data.get_line(line).unwrap();
        assert_eq!(resolved_line.p1, a);
        assert_eq!(resolved_line.p2, a1);
    }

    #[test]
    fn recompute_reports_missing_dependency_instead_of_panicking() {
        let mut doc = Document::default();
        let a = doc.add_tool(ToolKind::BasePoint {
            name: "A".to_string(),
            x: 0.0,
            y: 0.0,
        });
        // `phantom` was never produced by any add_tool call on this Document.
        let phantom = ObjectId::new(9999);
        doc.push_record_for_test(ToolRecord {
            id: ObjectId::new(1000),
            kind: ToolKind::Line { p1: a, p2: phantom },
        });

        let err = recompute_all(&doc).unwrap_err();
        assert_eq!(err, PatternError::MissingDependency(phantom));
    }

    #[test]
    fn recompute_is_deterministic() {
        let (doc, ..) = sample_document(0.0, 0.0);
        let first = recompute_all(&doc).unwrap();
        let second = recompute_all(&doc).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn recompute_cascades_base_point_changes_downstream() {
        let (doc_a, _, a1_a, _) = sample_document(0.0, 0.0);
        let (doc_b, _, a1_b, _) = sample_document(10.0, 5.0);

        let data_a = recompute_all(&doc_a).unwrap();
        let data_b = recompute_all(&doc_b).unwrap();

        let point_a = data_a.get_point(a1_a).unwrap();
        let point_b = data_b.get_point(a1_b).unwrap();
        assert_ne!(point_a, point_b);

        // Sanity-check the actual expected values, not just "not equal".
        assert!((point_a.x - 4.0).abs() < 1e-9);
        assert!((point_b.x - 14.0).abs() < 1e-9);
        assert!((point_b.y - 5.0).abs() < 1e-9);
    }
}
