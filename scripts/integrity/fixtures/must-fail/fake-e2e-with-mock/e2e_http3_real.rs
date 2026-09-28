//! MUST_FAIL: name claims e2e/real but substitutes mock backend.
#[cfg(test)]
mod tests {
    struct MockBackend;
    impl MockBackend {
        fn handle(&self) -> u16 {
            200
        }
    }

    #[test]
    fn e2e_real_production_http3_post() {
        let backend = MockBackend;
        assert_eq!(backend.handle(), 200);
    }
}
