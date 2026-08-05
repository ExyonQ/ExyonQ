// must-pass: observational profiling only
#[cfg(feature = "profiling")]
fn on_request() {
    record_timing();
}
