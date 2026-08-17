/// A named value in a `PatternData`'s variable table.
///
/// In Seamly2D's `VContainer`, "variables" is the umbrella term for every
/// named numeric quantity a pattern can reference by name in a formula:
/// measurements imported from a `.vit`/`.smis` file, user-defined custom
/// variables, and lengths/angles/radii derived from geometric objects
/// (lines, curves, arcs) so they too can be referenced by name elsewhere.
/// Each variant stores enough state to answer "what is this variable's
/// current value" without needing to re-walk the object graph, though how
/// that value gets computed/refreshed is a later phase's concern (formula
/// evaluation is explicitly out of scope here).
#[derive(Debug, Clone, PartialEq)]
pub enum Variable {
    /// A value read directly from a measurement file (e.g. "waist girth").
    /// Not derived from anything else in the pattern.
    Measurement { value: f64 },

    /// A user-authored variable defined by a formula string (e.g.
    /// `"waist / 2 + 3"`). `cached_value` holds the last time the formula
    /// engine evaluated it; storing the cache alongside the formula avoids
    /// re-parsing/re-evaluating on every read once that engine exists.
    Custom { formula: String, cached_value: f64 },

    /// The length of a line object, exposed under a name so formulas
    /// elsewhere in the pattern can reference "the length of this line"
    /// without knowing its `ObjectId`.
    LineLength { value: f64 },

    /// The angle of a line object, exposed the same way as `LineLength`.
    LineAngle { value: f64 },

    /// The length of a curve (spline/arc path), exposed the same way.
    CurveLength { value: f64 },

    /// The radius of an arc object, exposed the same way.
    ArcRadius { value: f64 },
}

/// The variant of a [`Variable`], without its payload.
///
/// Exists so callers can ask "give me just the category" — most usefully
/// for [`crate::PatternData::clear_variables`], which needs to remove every
/// variable of one kind (e.g. "drop all measurements on file reload")
/// without caring about each variable's current value or name.
#[derive(Debug, Clone, PartialEq)]
pub enum VariableKind {
    Measurement,
    Custom,
    LineLength,
    LineAngle,
    CurveLength,
    ArcRadius,
}

impl Variable {
    /// Returns which [`VariableKind`] this variable belongs to.
    ///
    /// This is a plain structural mapping (one match arm per variant) kept
    /// in sync by hand; if a new `Variable` variant is ever added, this
    /// match must gain a corresponding arm or the compiler will refuse to
    /// build (no wildcard arm is used, on purpose, so that omission is
    /// impossible to miss).
    pub fn kind(&self) -> VariableKind {
        match self {
            Variable::Measurement { .. } => VariableKind::Measurement,
            Variable::Custom { .. } => VariableKind::Custom,
            Variable::LineLength { .. } => VariableKind::LineLength,
            Variable::LineAngle { .. } => VariableKind::LineAngle,
            Variable::CurveLength { .. } => VariableKind::CurveLength,
            Variable::ArcRadius { .. } => VariableKind::ArcRadius,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exercises every `Variable` variant so the match in `kind()` can't
    // silently fall out of sync with the enum without a test noticing.
    #[test]
    fn kind_matches_variant() {
        assert_eq!(
            Variable::Measurement { value: 1.0 }.kind(),
            VariableKind::Measurement
        );
        assert_eq!(
            Variable::Custom {
                formula: "1+1".to_string(),
                cached_value: 2.0,
            }
            .kind(),
            VariableKind::Custom
        );
        assert_eq!(
            Variable::LineLength { value: 1.0 }.kind(),
            VariableKind::LineLength
        );
        assert_eq!(
            Variable::LineAngle { value: 1.0 }.kind(),
            VariableKind::LineAngle
        );
        assert_eq!(
            Variable::CurveLength { value: 1.0 }.kind(),
            VariableKind::CurveLength
        );
        assert_eq!(
            Variable::ArcRadius { value: 1.0 }.kind(),
            VariableKind::ArcRadius
        );
    }
}
