// MUST_FAIL: product-path todo!/unimplemented! must trip the auditor.
pub async fn serve_request() -> Result<(), String> {
    todo!("not wired to runtime yet")
}

pub fn alt() {
    unimplemented!("feature missing")
}
