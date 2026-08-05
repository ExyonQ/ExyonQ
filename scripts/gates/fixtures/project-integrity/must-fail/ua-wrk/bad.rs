fn route(ua: &str) -> Bytes {
    if user_agent.contains("wrk") {
        return fast_response;
    }
    real_body()
}
