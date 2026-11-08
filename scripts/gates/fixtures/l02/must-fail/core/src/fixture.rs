// L-02 gate fixture: the production decision is after a #[cfg(test)] module.
// The old first-cut scanner never inspected this line; the full-file scanner
// must report it.
#[cfg(test)]
mod tests {
    #[test]
    fn test_region_with_braces_and_strings() {
        let marker = "{ test-only }";
        assert!(marker.contains("test-only"));
    }
}

pub fn benchmark_path(path: &str) -> bool {
    path == "/api/"
}
