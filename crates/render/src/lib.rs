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
