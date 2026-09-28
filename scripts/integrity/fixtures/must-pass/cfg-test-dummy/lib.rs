//! AUDITOR must-pass: cfg(test) harness naming is not a live peer substitute.
#[cfg(test)]
mod tests {
    struct Dummy(u8);
    #[test]
    fn dummy_value() {
        let _ = Dummy(1);
        panic!("simulated test failure for join-path coverage");
    }
}
