//! MUST_FAIL: mock in unit test forbidden under ZERO_FAKE.
#[cfg(test)]
mod tests {
    struct MockClock;
    impl MockClock {
        fn now(&self) -> u64 {
            1
        }
    }

    #[test]
    fn unit_elapsed_math() {
        let clock = MockClock;
        assert_eq!(clock.now(), 1);
    }
}
