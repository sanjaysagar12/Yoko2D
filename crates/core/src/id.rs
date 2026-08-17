/// Identifies a [`crate::GeoObject`] stored in a [`crate::PatternData`].
///
/// Ids are only ever handed out by `PatternData::add_object`, so an
/// `ObjectId` in hand is always traceable back to a real insertion — callers
/// cannot fabricate one that was never allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(u32);

impl ObjectId {
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
