// must-fail: loadgen User-Agent detection
fn fast_path(user_agent: &str) -> Option<&'static [u8]> {
    if user_agent.contains("wrk") {
        return Some(b"fast_body");
    }
    None
}
