#[derive(Debug, Clone, PartialEq)]
pub enum Variable {
    Measurement { value: f64 },
    Custom { formula: String, cached_value: f64 },
    LineLength { value: f64 },
    LineAngle { value: f64 },
    CurveLength { value: f64 },
    ArcRadius { value: f64 },
}

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
