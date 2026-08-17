/// Identifies a [`crate::GeoObject`] stored in a [`crate::PatternData`].
///
/// This is a newtype around `u32` rather than a plain type alias so the
/// compiler treats it as a distinct type: it can't be accidentally passed
/// where a raw index, count, or other `u32` was expected, and vice versa.
///
/// Ids are normally only ever handed out by `PatternData::add_object`
/// during a live session, so an `ObjectId` obtained that way is always
/// traceable back to a real insertion. The one deliberate exception is
/// deserializing a `Document` from a saved pattern file (Phase 8's
/// `io::deserialize_document`): a file's `id="..."` attributes are, by
/// construction, exactly the ids a real session once allocated, so
/// reconstructing `ObjectId`s from them via [`Self::from_raw`] is
/// restoring history, not fabricating it. `Document::from_parts` is what
/// still guards against a corrupted/hand-edited file claiming two records
/// under the same id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(u32);

impl ObjectId {
    /// Wraps a raw id value.
    ///
    /// `pub(crate)`: within this crate, `PatternData`'s own id counter in
    /// `container.rs` is the only legitimate source of a *freshly
    /// allocated* id. External crates that need to reconstruct a
    /// previously-allocated id (currently: `io`, deserializing a saved
    /// pattern file) should use the public [`Self::from_raw`] instead,
    /// which exists specifically for that narrower purpose.
    pub(crate) fn new(id: u32) -> Self {
        ObjectId(id)
    }

    /// Reconstructs an `ObjectId` from a raw value previously obtained via
    /// [`Self::raw`] — in practice, a `id="..."` attribute read back out of
    /// a saved pattern file by `io::deserialize_document`.
    ///
    /// Public, unlike `new`: loading a file inherently means reconstituting
    /// ids a real session already allocated at save time, not fabricating
    /// arbitrary new ones, so this doesn't undermine the "ids are
    /// traceable to a real insertion" property `new` being `pub(crate)`
    /// protects during a live session — it extends that property across a
    /// save/load round-trip instead. Nothing here validates that `raw`
    /// actually corresponds to anything; [`crate::Document::from_parts`]
    /// is what still rejects a corrupted/hand-edited file with colliding
    /// ids.
    pub fn from_raw(raw: u32) -> Self {
        ObjectId(raw)
    }

    /// Returns the wrapped raw value.
    ///
    /// Public, unlike before Phase 8: originally only used internally by
    /// `PatternData::insert_with_id`, `io::serialize_document` now also
    /// needs to read this out to write a `ToolRecord`'s id back as an
    /// `id="..."` attribute when saving a pattern file.
    pub fn raw(self) -> u32 {
        self.0
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
