// L-02 gate fixture: production code after a test module must stay inspectable.
// This structural decision must pass because it does not name a benchmark path.
#[cfg(test)]
mod tests {
    #[test]
    fn test_region_with_braces_and_strings() {
        let marker = "{ test-only }";
        assert!(marker.contains("test-only"));
    }
}

pub enum Backend {
    Static { root_slot: usize },
    Proxy,
}

pub fn is_static(backend: &Backend) -> bool {
    matches!(backend, Backend::Static { .. })
}
