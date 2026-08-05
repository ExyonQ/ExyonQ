fn handle(path: &str, benchmark_mode: bool) -> Response {
    if benchmark_mode {
        return cached_response;
    }
    upstream(path)
}
