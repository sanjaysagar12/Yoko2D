use std::collections::HashMap;

use crate::error::ContainerError;
use crate::geo::{GeoObject, LineData, PointData};
use crate::id::ObjectId;
use crate::variable::{Variable, VariableKind};

/// The equivalent of Seamly2D's `VContainer`: a store mapping numeric ids to
/// geometric objects and string names to variables (measurements, custom
/// variables, derived lengths/angles).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatternData {
    objects: HashMap<ObjectId, GeoObject>,
    variables: HashMap<String, Variable>,
    next_id: u32,
}

impl PatternData {
    pub fn add_object(&mut self, obj: GeoObject) -> ObjectId {
        let id = ObjectId::new(self.next_id);
        self.next_id += 1;
        self.objects.insert(id, obj);
        id
    }

    pub fn get_point(&self, id: ObjectId) -> Result<&PointData, ContainerError> {
        match self.objects.get(&id) {
            Some(GeoObject::Point(point)) => Ok(point),
            Some(_) => Err(ContainerError::WrongObjectType {
                id,
                expected: "Point",
            }),
            None => Err(ContainerError::ObjectNotFound(id)),
        }
    }

    pub fn get_line(&self, id: ObjectId) -> Result<&LineData, ContainerError> {
        match self.objects.get(&id) {
            Some(GeoObject::Line(line)) => Ok(line),
            Some(_) => Err(ContainerError::WrongObjectType {
                id,
                expected: "Line",
            }),
            None => Err(ContainerError::ObjectNotFound(id)),
        }
    }

    pub fn remove_object(&mut self, id: ObjectId) -> Option<GeoObject> {
        self.objects.remove(&id)
    }

    /// Inserts `var` under `name`, overwriting any existing variable with
    /// that name.
    ///
    /// This is a deliberate policy, not an oversight: it mirrors how
    /// reloading a measurement file (or recomputing a custom/derived
    /// variable) is expected to replace the previous value under the same
    /// name rather than error out or accumulate stale duplicates. A
    /// variable's name is its identity, so "same name" means "same
    /// variable, new value".
    pub fn add_variable(&mut self, name: impl Into<String>, var: Variable) {
        self.variables.insert(name.into(), var);
    }

    pub fn get_variable(&self, name: &str) -> Result<&Variable, ContainerError> {
        self.variables
            .get(name)
            .ok_or_else(|| ContainerError::VariableNotFound(name.to_string()))
    }

    pub fn clear_variables(&mut self, kind: VariableKind) {
        self.variables.retain(|_, var| var.kind() != kind);
    }

    pub fn clear_objects(&mut self) {
        self.objects.clear();
        // 0 matches the `next_id` a derived `Default::default()` produces,
        // so a cleared container behaves like a brand-new one for id allocation.
        self.next_id = 0;
    }

    pub fn clear_all(&mut self) {
        self.clear_objects();
        self.variables.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_get_point_and_line() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 1.0, y: 2.0 }));
        let p2 = data.add_object(GeoObject::Point(PointData { x: 3.0, y: 4.0 }));
        let line = data.add_object(GeoObject::Line(LineData { p1, p2 }));

        let point = data.get_point(p1).unwrap();
        assert_eq!(point.x, 1.0);
        assert_eq!(point.y, 2.0);

        let line_data = data.get_line(line).unwrap();
        assert_eq!(line_data.p1, p1);
        assert_eq!(line_data.p2, p2);
    }

    #[test]
    fn get_line_on_point_id_returns_wrong_object_type() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));

        let err = data.get_line(p1).unwrap_err();
        assert!(matches!(
            err,
            ContainerError::WrongObjectType { id, .. } if id == p1
        ));
    }

    #[test]
    fn get_point_on_line_id_returns_wrong_object_type() {
        let mut data = PatternData::default();
        let p1 = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        let p2 = data.add_object(GeoObject::Point(PointData { x: 1.0, y: 1.0 }));
        let line = data.add_object(GeoObject::Line(LineData { p1, p2 }));

        let err = data.get_point(line).unwrap_err();
        assert!(matches!(
            err,
            ContainerError::WrongObjectType { id, .. } if id == line
        ));
    }

    #[test]
    fn get_point_on_nonexistent_id_returns_object_not_found() {
        let mut data = PatternData::default();
        let real = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        data.remove_object(real);

        let err = data.get_point(real).unwrap_err();
        assert_eq!(err, ContainerError::ObjectNotFound(real));
    }

    #[test]
    fn get_line_on_nonexistent_id_returns_object_not_found() {
        let mut data = PatternData::default();
        let real = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        data.remove_object(real);

        let err = data.get_line(real).unwrap_err();
        assert_eq!(err, ContainerError::ObjectNotFound(real));
    }

    #[test]
    fn next_id_increases_strictly_across_add_object_calls() {
        let mut data = PatternData::default();
        let ids: Vec<ObjectId> = (0..5)
            .map(|i| {
                data.add_object(GeoObject::Point(PointData {
                    x: i as f64,
                    y: 0.0,
                }))
            })
            .collect();

        for pair in ids.windows(2) {
            assert!(pair[0] < pair[1]);
        }

        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn clear_objects_resets_id_counter_to_initial_value() {
        let mut fresh = PatternData::default();
        let fresh_first_id = fresh.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));

        let mut data = PatternData::default();
        data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        data.clear_objects();

        let id_after_clear = data.add_object(GeoObject::Point(PointData { x: 0.0, y: 0.0 }));
        assert_eq!(id_after_clear, fresh_first_id);
    }

    #[test]
    fn add_variable_twice_keeps_most_recent_value() {
        let mut data = PatternData::default();
        data.add_variable("waist", Variable::Measurement { value: 70.0 });
        data.add_variable("waist", Variable::Measurement { value: 72.5 });

        let var = data.get_variable("waist").unwrap();
        assert_eq!(var, &Variable::Measurement { value: 72.5 });
    }

    #[test]
    fn clear_variables_removes_only_matching_kind() {
        let mut data = PatternData::default();
        data.add_variable("waist", Variable::Measurement { value: 70.0 });
        data.add_variable("hip", Variable::Measurement { value: 90.0 });
        data.add_variable(
            "half_waist",
            Variable::Custom {
                formula: "waist / 2".to_string(),
                cached_value: 35.0,
            },
        );
        data.add_variable("seam_len", Variable::LineLength { value: 12.0 });

        data.clear_variables(VariableKind::Measurement);

        assert_eq!(
            data.get_variable("waist").unwrap_err(),
            ContainerError::VariableNotFound("waist".to_string())
        );
        assert_eq!(
            data.get_variable("hip").unwrap_err(),
            ContainerError::VariableNotFound("hip".to_string())
        );
        assert_eq!(
            data.get_variable("half_waist").unwrap(),
            &Variable::Custom {
                formula: "waist / 2".to_string(),
                cached_value: 35.0,
            }
        );
        assert_eq!(
            data.get_variable("seam_len").unwrap(),
            &Variable::LineLength { value: 12.0 }
        );
    }

    #[test]
    fn cloning_pattern_data_is_deep_and_independent() {
        let original = PatternData::default();
        let mut clone = original.clone();

        let new_id = clone.add_object(GeoObject::Point(PointData { x: 5.0, y: 5.0 }));

        assert_eq!(
            original.get_point(new_id).unwrap_err(),
            ContainerError::ObjectNotFound(new_id)
        );
        assert!(clone.get_point(new_id).is_ok());
    }

    #[test]
    fn get_variable_on_missing_name_returns_variable_not_found() {
        let data = PatternData::default();
        let err = data.get_variable("does_not_exist").unwrap_err();
        assert_eq!(
            err,
            ContainerError::VariableNotFound("does_not_exist".to_string())
        );
    }
}
