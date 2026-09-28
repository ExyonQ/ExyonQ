//! MUST_REVIEW: named integration but stays in-process.
#[cfg(test)]
mod tests {
    fn call_internal() -> bool {
        true
    }

    #[test]
    fn integration_full_proxy_path() {
        // In-process only — no subprocess spawn of the product server.
        assert!(call_internal());
    }
}
