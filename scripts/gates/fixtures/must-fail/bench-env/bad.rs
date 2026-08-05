// must-fail: BENCHMARK env skips upstream
fn maybe_proxy() {
    if env::var("BENCHMARK").is_ok() {
        skip_upstream();
    }
}
