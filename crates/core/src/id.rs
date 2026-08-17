/// Identifies a [`crate::GeoObject`] stored in a [`crate::PatternData`].
///
/// This is a newtype around `u32` rather than a plain type alias so the
/// compiler treats it as a distinct type: it can't be accidentally passed
/// where a raw index, count, or other `u32` was expected, and vice versa.
///
/// Ids are only ever handed out by `PatternData::add_object`, so an
/// `ObjectId` in hand is always traceable back to a real insertion — callers
/// cannot fabricate one that was never allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(u32);

impl ObjectId {
    /// Wraps a raw id value.
    ///
    /// Deliberately `pub(crate)`, not `pub`: the only legitimate source of an
    /// `ObjectId` is `PatternData`'s own id counter in `container.rs`. If
    /// this were public, external code could construct ids that were never
    /// actually allocated (or belong to a *different* `PatternData`), which
    /// would let `get_point`/`get_line` be called with ids that look valid
    /// but aren't — defeating the point of a typed id in the first place.
    pub(crate) fn new(id: u32) -> Self {
        ObjectId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_with_same_value_are_equal() {
        assert_eq!(ObjectId::new(3), ObjectId::new(3));
    }

    #[test]
    fn ids_order_by_value() {
        assert!(ObjectId::new(1) < ObjectId::new(2));
    }
}
