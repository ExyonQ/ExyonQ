// MUST_FAIL: hard-coded success without doing work.
pub fn apply_policy() -> Result<(), ()> {
    // HARDCODED_SUCCESS: always ok
    let _ = fake_success();
    Ok(())
}

fn fake_success() -> bool {
    true
}
