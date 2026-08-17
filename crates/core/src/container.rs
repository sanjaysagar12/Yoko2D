use std::collections::HashMap;

use crate::error::ContainerError;
use crate::geo::{GeoObject, LineData, PointData};
use crate::id::ObjectId;
use crate::variable::{Variable, VariableKind};

/// The equivalent of Seamly2D's `VContainer`: a store mapping numeric ids to
/// geometric objects and string names to variables (measurements, custom
/// variables, derived lengths/angles).
///
/// Fields are private on purpose — `objects`/`variables`/`next_id` are
/// implementation details of "how the container tracks things", not part of
/// its API. All access goes through the methods below so invariants like
/// "ids are never reused" and "every stored object is reachable by exactly
/// one id" stay enforced in one place instead of being the caller's
/// responsibility.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PatternData {
    objects: HashMap<ObjectId, GeoObject>,
    variables: HashMap<String, Variable>,
    next_id: u32,
}

impl PatternData {
    /// Stores `obj` under a freshly allocated id and returns that id.
    ///
    /// Ids are handed out from a strictly increasing counter, so every id
    /// this returns is guaranteed unused — nothing is ever overwritten by
    /// accident, and (short of `clear_objects`/`clear_all`) no id is ever
    /// reused even after its object is removed.
    pub fn add_object(&mut self, obj: GeoObject) -> ObjectId {
        let id = ObjectId::new(self.next_id);
        self.next_id += 1;
        self.objects.insert(id, obj);
        id
    }

    /// Looks up `id` expecting it to hold a [`PointData`].
    ///
    /// Distinguishes the two ways this can fail: `id` isn't in the
    /// container at all (`ObjectNotFound`), versus `id` is valid but names
    /// something that isn't a point, e.g. a line (`WrongObjectType`). Both
    /// are returned as `Err` rather than panicking — see the crate-level
    /// `#![deny(clippy::unwrap_used, ...)]` for why that matters here.
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

    /// Looks up `id` expecting it to hold a [`LineData`].
    ///
    /// Same not-found/wrong-type distinction as [`Self::get_point`], mirrored
    /// for the `Line` variant.
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

    /// Removes and returns the object stored at `id`, if any.
    ///
    /// Note this does *not* roll back the id counter — the freed id is
    /// simply gone, never reissued by a later `add_object`. That's what
    /// keeps an `ObjectId` a caller is still holding onto from ever silently
    /// starting to point at a different, unrelated object.
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

    /// Looks up the variable registered under `name`.
    ///
    /// Returns `VariableNotFound` rather than `Option::None` (unlike
    /// `remove_object`) because a missing variable during formula evaluation
    /// is meant to surface as a proper error the caller can report, not be
    /// silently treated as "no value" — there is no sensible default for a
    /// measurement or custom variable that was never defined.
    pub fn get_variable(&self, name: &str) -> Result<&Variable, ContainerError> {
        self.variables
            .get(name)
            .ok_or_else(|| ContainerError::VariableNotFound(name.to_string()))
    }

    /// Iterates every `(name, Variable)` pair currently stored, in
    /// unspecified order (the same order `HashMap::iter` yields).
    ///
    /// Exists so callers like the formula engine's `flatten_variables` can
    /// build their own view of the whole variable table (e.g. a flat
    /// `name -> f64` map) without `PatternData` needing to know anything
    /// about what they'll do with it.
    pub fn variables(&self) -> impl Iterator<Item = (&String, &Variable)> {
        self.variables.iter()
    }

    /// Removes every variable whose [`Variable::kind`] equals `kind`,
    /// leaving all others untouched.
    ///
    /// Modeled on the original app's measurement-reload flow: re-importing a
    /// `.vit`/`.smis` file needs to drop *only* the old `Measurement`
    /// variables and replace them, without disturbing `Custom` variables or
    /// derived lengths/angles that the user's formulas still depend on.
    pub fn clear_variables(&mut self, kind: VariableKind) {
        self.variables.retain(|_, var| var.kind() != kind);
    }

    /// Empties the object map and resets the id counter.
    ///
    /// Resetting `next_id` (not just clearing `objects`) is what makes this
    /// different from removing every object one at a time: afterward, the
    /// container hands out ids starting from the same point a brand-new
    /// `PatternData` would, rather than continuing to climb from wherever it
    /// left off.
    pub fn clear_objects(&mut self) {
        self.objects.clear();
        // 0 matches the `next_id` a derived `Default::default()` produces,
        // so a cleared container behaves like a brand-new one for id allocation.
        self.next_id = 0;
    }

    /// Resets the container to its just-created state: clears every object
    /// (and the id counter with it, via [`Self::clear_objects`]) and every
    /// variable, regardless of kind.
    ///
    /// Deliberately clears variables directly rather than calling
    /// `clear_variables` once per `VariableKind` — "all kinds" isn't really
    /// a kind-based filter, it's the absence of one.
    pub fn clear_all(&mut self) {
        self.clear_objects();
        self.variables.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic round-trip: insert a Point and a Line, read them back through
    // the typed getters, and check the values match exactly.
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

    // A valid id that names the *other* variant must be reported as
    // WrongObjectType, not ObjectNotFound — the id isn't missing, it's just
    // pointing at the wrong kind of thing.
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

    // `real` is removed after being allocated so the id is guaranteed to
    // have never existed in the container by the time it's looked up,
    // exercising ObjectNotFound rather than an id that was simply never
    // issued in the first place.
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

    // Checks both that ids strictly increase (windows(2) pairwise) and that
    // none repeat (sort + dedup shouldn't drop anything) — increasing alone
    // wouldn't catch a counter that stalled or wrapped.
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

    // Compares against a brand-new PatternData's first id rather than a
    // hardcoded literal, so this test keeps working even if the counter's
    // starting value ever changes.
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

    // Mixes Measurement, Custom, and LineLength variables so clearing one
    // kind can be checked from both directions: the targeted kind is gone,
    // and the untargeted kinds survive completely unchanged.
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

    // If `Clone` were shallow (e.g. an Rc-shared map), adding to the clone
    // would be visible from `original` too. Asserting the original still
    // can't see the new id proves the clone owns an independent copy of the
    // container's state.
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
    fn variables_iterates_every_stored_pair() {
        let mut data = PatternData::default();
        data.add_variable("waist", Variable::Measurement { value: 70.0 });
        data.add_variable("seam_len", Variable::LineLength { value: 12.0 });

        let mut names: Vec<&String> = data.variables().map(|(name, _)| name).collect();
        names.sort();
        assert_eq!(names, vec!["seam_len", "waist"]);
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
