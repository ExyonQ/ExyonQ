// must-fail: path + benchmark implicit cache
fn handle(path: &str, benchmark: bool, cached_response: Bytes) -> Bytes {
    if path == "/api/" && benchmark {
        return cached_response;
    }
    todo!()
}
