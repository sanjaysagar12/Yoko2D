/// Stands in for this crate's eventual file format read/write support
/// (`.val`/`.vit`/`.smis`, etc.).
///
/// Takes `&core::PatternData` purely to prove the dependency wiring from
/// Phase 0 works end-to-end — `io` can see and reference `core`'s public
/// types — without touching the container's fields, which are private by
/// design (see `core::container::PatternData`).
pub fn placeholder(_data: &core::PatternData) -> &'static str {
    "io placeholder"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_uses_core_pattern_data() {
        let data = core::PatternData::default();
        assert!(placeholder(&data).contains("io placeholder"));
    }
}
