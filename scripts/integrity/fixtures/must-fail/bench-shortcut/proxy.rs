// MUST_FAIL: benchmark cache shortcut on product path.
const BENCH_API_CACHE_PATHS: &[&str] = ["/api/", "/api/health"];

pub fn maybe_cache(path: &str) -> bool {
    BENCH_API_CACHE_PATHS.iter().any(|p| path.starts_with(p))
}
