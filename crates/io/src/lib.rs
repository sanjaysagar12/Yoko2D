pub fn placeholder(data: &core::PatternData) -> String {
    format!("io placeholder — note: {:?}", data.note)
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
