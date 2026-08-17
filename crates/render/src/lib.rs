/// Stands in for this crate's eventual `PatternData -> draw commands`
/// translation.
///
/// Takes `&core::PatternData` purely to prove the dependency wiring from
/// Phase 0 works end-to-end — same rationale as `io::placeholder`.
pub fn placeholder(_data: &core::PatternData) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_returns_empty_vec() {
        let data = core::PatternData::default();
        assert!(placeholder(&data).is_empty());
    }
}
