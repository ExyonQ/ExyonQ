//! Plan 12 v0 — neutral response-cache contract (compile-time policy, request-time lookup).

use std::time::Duration;

/// Global store limits (v0 defaults).
pub const CACHE_MAX_ENTRIES: usize = 10_000;
pub const CACHE_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
pub const CACHE_DEFAULT_TTL_SECS: u64 = 30;
pub const CACHE_DEFAULT_MAX_OBJECT_BYTES: usize = 1024 * 1024;

/// Compiled cache policy consumed by the dataplane (no IR on hot path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledCachePolicy {
    pub name: String,
    pub ttl: Duration,
    pub max_object_bytes: usize,
    /// Bumped when policy fields change at compile time.
    pub policy_generation: u64,
}

/// Safe response headers stored in the cache (hop-by-hop excluded).
pub const CACHE_STORE_HEADER_NAMES: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "etag",
    "last-modified",
    "cache-control",
];

/// Reason a response was not stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRejection {
    Method,
    Status,
    RequestAuthorization,
    RequestCookie,
    RequestRange,
    RequestUpgrade,
    RequestCacheControl,
    RequestPragma,
    ResponseSetCookie,
    ResponseCacheControl,
    ResponseVary,
    ResponseContentRange,
    ResponseStreaming,
    ResponseHopByHop,
    /// Cap056: Content-Encoding other than identity is incompatible with identity key.
    ResponseContentEncoding,
    BodyTooLarge,
    BodyNotMaterialized,
    PolicyDisabled,
}

/// Bounded reasons for FPC lookup bypass (WC1/WC2B). Plan 12
/// [`request_eligible_for_cache`] remains separate (cookie-any).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BypassReason {
    Method,
    Authorization,
    CookieDenied,
    CookiePresent,
    Range,
    /// Cap020: If-None-Match / If-Modified-Since must hit origin Cap020 path.
    Conditional,
    Upgrade,
    RequestCacheControl,
    RequestPragma,
    WpAdmin,
    WpLogin,
    WpCron,
    WpJson,
    Preview,
    Customize,
    CommercePath,
    UnsafeQuery,
    UnsafePath,
    MissingIdentity,
    Unknown,
}

/// Cookie name prefixes / exact names that always bypass FPC (WC1 hard deny).
pub const FPC_COOKIE_HARD_DENY_PREFIXES: &[&str] = &[
    "wordpress_logged_in_",
    "wordpress_sec_",
    "wordpress_",
    "wp-settings-",
    "wp-postpass_",
    "comment_author",
    "woocommerce_cart_hash",
    "woocommerce_items_in_cart",
    "wp_woocommerce_session_",
    "PHPSESSID",
];

/// Whether a single cookie name matches the WC1 hard-deny set.
pub fn fpc_cookie_name_hard_denied(name: &str) -> bool {
    let n = name.trim();
    if n.is_empty() {
        return false;
    }
    if n.eq_ignore_ascii_case("PHPSESSID")
        || n.eq_ignore_ascii_case("woocommerce_cart_hash")
        || n.eq_ignore_ascii_case("woocommerce_items_in_cart")
    {
        return true;
    }
    let lower = n.to_ascii_lowercase();
    FPC_COOKIE_HARD_DENY_PREFIXES.iter().any(|p| {
        let pl = p.to_ascii_lowercase();
        lower.starts_with(&pl)
    })
}

/// Parse `Cookie` header value into names (values ignored).
pub fn cookie_header_names(cookie_header: &str) -> Vec<String> {
    cookie_header
        .split(';')
        .filter_map(|part| {
            let name = part.split_once('=').map(|(n, _)| n).unwrap_or(part).trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        })
        .collect()
}

/// Join **all** `Cookie` header values (security gate F3 — do not use first-only).
pub fn aggregate_cookie_headers(request_headers: &[(String, String)]) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for (name, value) in request_headers {
        if name.eq_ignore_ascii_case("cookie") && !value.is_empty() {
            parts.push(value.as_str());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

/// Cookie policy for FPC (WC1): absent → ok; hard-deny → CookieDenied; else require every
/// name on `safe_allowlist` (empty allowlist → any Cookie → CookiePresent).
pub fn fpc_cookie_bypass(
    request_headers: &[(String, String)],
    safe_allowlist: &[&str],
) -> Option<BypassReason> {
    let raw = aggregate_cookie_headers(request_headers)?;
    let names = cookie_header_names(&raw);
    if names.is_empty() {
        return None;
    }
    for name in &names {
        if fpc_cookie_name_hard_denied(name) {
            return Some(BypassReason::CookieDenied);
        }
    }
    let allow: std::collections::HashSet<&str> = safe_allowlist.iter().copied().collect();
    if names.iter().all(|n| allow.contains(n.as_str())) {
        return None;
    }
    Some(BypassReason::CookiePresent)
}

/// Canonicalize path for FPC policy. `None` → treat as unsafe (`BypassReason::UnsafePath`).
///
/// Rejects `..` segments and percent-encoded path separators / dots that could hide
/// `/wp-admin` style prefixes (WC2B security gate F4).
pub fn fpc_canonicalize_path(path: &str) -> Option<String> {
    let path = if path.is_empty() { "/" } else { path };
    let lower = path.to_ascii_lowercase();
    if lower.contains("%2f")
        || lower.contains("%2e")
        || lower.contains("%5c")
        || lower.contains('\\')
    {
        return None;
    }
    let mut out = String::new();
    for seg in path.split('/') {
        if seg.is_empty() {
            continue;
        }
        if seg == "." {
            continue;
        }
        if seg == ".." || seg.eq_ignore_ascii_case("%2e%2e") {
            return None;
        }
        out.push('/');
        out.push_str(seg);
    }
    if out.is_empty() {
        Some("/".into())
    } else {
        Some(out)
    }
}

/// Query policy decision for FPC keying (WC2B).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FpcQueryDecision {
    /// Use this canonical query string in the cache key (often empty).
    Eligible {
        canonical: String,
    },
    Bypass(BypassReason),
}

/// Empty query → eligible. Tracking-only params may be stripped. Any remaining
/// (functional) params with empty allowlist → bypass. Non-empty allowlist keeps
/// only listed names (sorted); leftover functional names → bypass.
pub fn fpc_query_decision(raw: &str, functional_allowlist: &[&str]) -> FpcQueryDecision {
    if raw.is_empty() {
        return FpcQueryDecision::Eligible {
            canonical: String::new(),
        };
    }
    let mut functional: Vec<(String, String)> = Vec::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        if name.is_empty() {
            continue;
        }
        if is_tracking_query_param_local(name) {
            continue;
        }
        functional.push((name.to_string(), value.to_string()));
    }
    if functional.is_empty() {
        return FpcQueryDecision::Eligible {
            canonical: String::new(),
        };
    }
    if functional_allowlist.is_empty() {
        return FpcQueryDecision::Bypass(BypassReason::UnsafeQuery);
    }
    let allow: std::collections::HashSet<&str> = functional_allowlist.iter().copied().collect();
    if functional.iter().any(|(n, _)| !allow.contains(n.as_str())) {
        return FpcQueryDecision::Bypass(BypassReason::UnsafeQuery);
    }
    functional.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical = functional
        .into_iter()
        .map(|(k, v)| if v.is_empty() { k } else { format!("{k}={v}") })
        .collect::<Vec<_>>()
        .join("&");
    FpcQueryDecision::Eligible { canonical }
}

fn is_tracking_query_param_local(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "utm_source"
            | "utm_medium"
            | "utm_campaign"
            | "utm_term"
            | "utm_content"
            | "utm_id"
            | "gclid"
            | "fbclid"
            | "msclkid"
            | "mc_cid"
            | "mc_eid"
            | "_ga"
            | "_gl"
    )
}

/// Path / query bypass rules for WordPress-oriented FPC (WC1 §2.2). Does not inspect method.
pub fn fpc_path_query_bypass(path: &str, query: &str) -> Option<BypassReason> {
    let Some(canon) = fpc_canonicalize_path(path) else {
        return Some(BypassReason::UnsafePath);
    };
    let lower_path = canon.to_ascii_lowercase();
    if lower_path.starts_with("/wp-admin/") || lower_path == "/wp-admin" {
        return Some(BypassReason::WpAdmin);
    }
    if lower_path == "/wp-login.php" || lower_path.ends_with("/wp-login.php") {
        return Some(BypassReason::WpLogin);
    }
    if lower_path == "/wp-cron.php" || lower_path.ends_with("/wp-cron.php") {
        return Some(BypassReason::WpCron);
    }
    if lower_path.starts_with("/wp-json/") || lower_path == "/wp-json" {
        return Some(BypassReason::WpJson);
    }
    for prefix in ["/cart", "/checkout", "/my-account"] {
        if lower_path == prefix || lower_path.starts_with(&format!("{prefix}/")) {
            return Some(BypassReason::CommercePath);
        }
    }
    let q = query.to_ascii_lowercase();
    if query_param_truthy(&q, "preview") {
        return Some(BypassReason::Preview);
    }
    if query_has_name(&q, "customize_changeset_uuid") || query_has_name(&q, "wp_customize") {
        return Some(BypassReason::Customize);
    }
    None
}

/// Combined FPC eligibility (WC2B). Returns bypass reason or `Ok((canon_path, canon_query))`.
pub fn fpc_request_evaluate(
    method: &str,
    path: &str,
    query: &str,
    request_headers: &[(String, String)],
    safe_cookie_allowlist: &[&str],
    functional_query_allowlist: &[&str],
) -> Result<(String, String), BypassReason> {
    let method = method.to_ascii_uppercase();
    if method != "GET" && method != "HEAD" {
        return Err(BypassReason::Method);
    }
    if header_present_ignore_case(request_headers, "authorization") {
        return Err(BypassReason::Authorization);
    }
    if header_present_ignore_case(request_headers, "range") {
        return Err(BypassReason::Range);
    }
    if header_present_ignore_case(request_headers, "if-none-match")
        || header_present_ignore_case(request_headers, "if-modified-since")
    {
        return Err(BypassReason::Conditional);
    }
    if header_present_ignore_case(request_headers, "upgrade")
        || connection_requests_upgrade(request_headers)
    {
        return Err(BypassReason::Upgrade);
    }
    if request_cache_control_blocks_lookup(request_headers) {
        return Err(BypassReason::RequestCacheControl);
    }
    if request_pragma_blocks_lookup(request_headers) {
        return Err(BypassReason::RequestPragma);
    }
    if let Some(reason) = fpc_cookie_bypass(request_headers, safe_cookie_allowlist) {
        return Err(reason);
    }
    let Some(canon_path) = fpc_canonicalize_path(path) else {
        return Err(BypassReason::UnsafePath);
    };
    if let Some(reason) = fpc_path_query_bypass(&canon_path, query) {
        return Err(reason);
    }
    match fpc_query_decision(query, functional_query_allowlist) {
        FpcQueryDecision::Eligible { canonical } => Ok((canon_path, canonical)),
        FpcQueryDecision::Bypass(reason) => Err(reason),
    }
}

/// Combined FPC eligibility decision (library API). Leaves Plan 12 helpers unchanged.
pub fn fpc_request_bypass(
    method: &str,
    path: &str,
    query: &str,
    request_headers: &[(String, String)],
    safe_cookie_allowlist: &[&str],
) -> Option<BypassReason> {
    fpc_request_evaluate(
        method,
        path,
        query,
        request_headers,
        safe_cookie_allowlist,
        &[],
    )
    .err()
}

fn query_has_name(query_lower: &str, name: &str) -> bool {
    for pair in query_lower.split('&') {
        if pair.is_empty() {
            continue;
        }
        let n = pair.split_once('=').map(|(n, _)| n).unwrap_or(pair);
        if n == name {
            return true;
        }
    }
    false
}

fn query_param_truthy(query_lower: &str, name: &str) -> bool {
    for pair in query_lower.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (n, v) = pair.split_once('=').unwrap_or((pair, ""));
        if n == name {
            return v.is_empty() || v == "1" || v == "true" || v == "yes";
        }
    }
    false
}

use crate::hop_by_hop::FIXED_HOP_BY_HOP_HEADERS as HOP_BY_HOP_RESPONSE;

/// Whether a request may participate in cache lookup / singleflight (fail closed).
pub fn request_eligible_for_cache(method: &str, request_headers: &[(String, String)]) -> bool {
    let method = method.to_ascii_uppercase();
    if method != "GET" && method != "HEAD" {
        return false;
    }
    if header_present_ignore_case(request_headers, "authorization") {
        return false;
    }
    if header_present_ignore_case(request_headers, "cookie") {
        return false;
    }
    if header_present_ignore_case(request_headers, "range") {
        return false;
    }
    // Cap020: conditional validators must evaluate at origin (not cache HIT as full 200).
    if header_present_ignore_case(request_headers, "if-none-match")
        || header_present_ignore_case(request_headers, "if-modified-since")
    {
        return false;
    }
    if header_present_ignore_case(request_headers, "upgrade") {
        return false;
    }
    if request_cache_control_blocks_lookup(request_headers) {
        return false;
    }
    if request_pragma_blocks_lookup(request_headers) {
        return false;
    }
    if connection_requests_upgrade(request_headers) {
        return false;
    }
    true
}

/// Evaluate whether a response may be stored (fail closed).
pub fn assess_cacheability(
    method: &str,
    request_headers: &[(String, String)],
    status: u16,
    response_headers: &[(String, String)],
    body_len: usize,
    max_object_bytes: usize,
) -> Result<(), CacheRejection> {
    let method = method.to_ascii_uppercase();
    if method != "GET" && method != "HEAD" {
        return Err(CacheRejection::Method);
    }
    if status != 200 {
        return Err(CacheRejection::Status);
    }
    if header_present_ignore_case(request_headers, "authorization") {
        return Err(CacheRejection::RequestAuthorization);
    }
    if header_present_ignore_case(request_headers, "cookie") {
        return Err(CacheRejection::RequestCookie);
    }
    if header_present_ignore_case(request_headers, "range") {
        return Err(CacheRejection::RequestRange);
    }
    if header_present_ignore_case(request_headers, "upgrade") {
        return Err(CacheRejection::RequestUpgrade);
    }
    if request_cache_control_blocks_lookup(request_headers) {
        return Err(CacheRejection::RequestCacheControl);
    }
    if request_pragma_blocks_lookup(request_headers) {
        return Err(CacheRejection::RequestPragma);
    }
    if connection_requests_upgrade(request_headers) {
        return Err(CacheRejection::RequestUpgrade);
    }
    if let Some(reason) = response_headers_block_storage(response_headers) {
        return Err(reason);
    }
    if header_present_ignore_case(response_headers, "set-cookie") {
        return Err(CacheRejection::ResponseSetCookie);
    }
    if let Some(value) = header_values_joined_ignore_case(response_headers, "cache-control") {
        let lower = value.to_ascii_lowercase();
        if cache_control_token_present(&lower, "no-store")
            || cache_control_token_present(&lower, "private")
        {
            return Err(CacheRejection::ResponseCacheControl);
        }
    }
    if let Some(value) = header_values_joined_ignore_case(response_headers, "vary") {
        let trimmed = value.trim();
        if trimmed == "*" || !trimmed.is_empty() {
            return Err(CacheRejection::ResponseVary);
        }
    }
    if body_len > max_object_bytes {
        return Err(CacheRejection::BodyTooLarge);
    }
    Ok(())
}

fn header_present_ignore_case(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn header_value_ignore_case<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Join all instances of a header (comma-separated) for conservative policy checks.
/// Split `Cache-Control` / `Vary` must not bypass privacy rules.
fn header_values_joined_ignore_case(headers: &[(String, String)], name: &str) -> Option<String> {
    let mut joined = String::new();
    for (k, v) in headers {
        if !k.eq_ignore_ascii_case(name) {
            continue;
        }
        if !joined.is_empty() {
            joined.push(',');
        }
        joined.push_str(v);
    }
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn request_cache_control_blocks_lookup(headers: &[(String, String)]) -> bool {
    header_values_joined_ignore_case(headers, "cache-control").is_some_and(|value| {
        let lower = value.to_ascii_lowercase();
        cache_control_token_present(&lower, "no-cache")
            || cache_control_token_present(&lower, "no-store")
    })
}

fn request_pragma_blocks_lookup(headers: &[(String, String)]) -> bool {
    header_values_joined_ignore_case(headers, "pragma")
        .is_some_and(|value| value.to_ascii_lowercase().contains("no-cache"))
}

fn connection_requests_upgrade(headers: &[(String, String)]) -> bool {
    header_values_joined_ignore_case(headers, "connection")
        .is_some_and(|value| value.to_ascii_lowercase().contains("upgrade"))
}

fn response_headers_block_storage(headers: &[(String, String)]) -> Option<CacheRejection> {
    if header_present_ignore_case(headers, "content-range") {
        return Some(CacheRejection::ResponseContentRange);
    }
    if headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
    }) {
        return Some(CacheRejection::ResponseStreaming);
    }
    if let Some(ce) = header_values_joined_ignore_case(headers, "content-encoding") {
        let trimmed = ce.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("identity") {
            return Some(CacheRejection::ResponseContentEncoding);
        }
    }
    for (name, _) in headers {
        let lower = name.to_ascii_lowercase();
        if HOP_BY_HOP_RESPONSE.iter().any(|hop| lower == *hop) {
            return Some(CacheRejection::ResponseHopByHop);
        }
    }
    None
}

/// Filter response headers to the cache-safe subset.
pub fn filter_storable_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            let lower = name.to_ascii_lowercase();
            CACHE_STORE_HEADER_NAMES
                .iter()
                .any(|allowed| lower == *allowed)
        })
        .cloned()
        .collect()
}

/// FPC store rejection reasons (WC2C) — distinct from Plan 12 [`CacheRejection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FpcStoreReject {
    Method,
    HeadMustNotStore,
    Status,
    SetCookie,
    Private,
    NoStore,
    NoCache,
    UnsupportedVary,
    Streaming,
    ContentRange,
    ContentEncoding,
    HopByHop,
    BodyTooLarge,
    TtlInvalid,
}

impl FpcStoreReject {
    pub fn as_metric_label(self) -> &'static str {
        match self {
            Self::Method => "method",
            Self::HeadMustNotStore => "head_must_not_store",
            Self::Status => "status",
            Self::SetCookie => "set_cookie",
            Self::Private => "private",
            Self::NoStore => "no_store",
            Self::NoCache => "no_cache",
            Self::UnsupportedVary => "unsupported_vary",
            Self::Streaming => "streaming",
            Self::ContentRange => "content_range",
            Self::ContentEncoding => "content_encoding",
            Self::HopByHop => "hop_by_hop",
            Self::BodyTooLarge => "body_too_large",
            Self::TtlInvalid => "ttl_invalid",
        }
    }
}

/// Assess FPC response store eligibility and compute clamped TTL (WC2C).
///
/// Does **not** re-apply Plan 12 “any Cookie → reject” — request eligibility is
/// already decided by [`fpc_request_evaluate`]. Only GET may populate the store;
/// HEAD never stores.
pub fn fpc_assess_store(
    method: &str,
    status: u16,
    response_headers: &[(String, String)],
    body_len: usize,
    max_object_bytes: usize,
    default_ttl: Duration,
    max_ttl: Duration,
) -> Result<Duration, FpcStoreReject> {
    let method = method.to_ascii_uppercase();
    if method == "HEAD" {
        return Err(FpcStoreReject::HeadMustNotStore);
    }
    if method != "GET" {
        return Err(FpcStoreReject::Method);
    }
    if status != 200 {
        return Err(FpcStoreReject::Status);
    }
    if header_present_ignore_case(response_headers, "set-cookie") {
        return Err(FpcStoreReject::SetCookie);
    }
    if let Some(reason) = fpc_response_headers_block_store(response_headers) {
        return Err(reason);
    }
    if body_len > max_object_bytes {
        return Err(FpcStoreReject::BodyTooLarge);
    }
    let ttl = fpc_effective_ttl(response_headers, default_ttl, max_ttl)?;
    Ok(ttl)
}

fn fpc_response_headers_block_store(headers: &[(String, String)]) -> Option<FpcStoreReject> {
    if header_present_ignore_case(headers, "content-range") {
        return Some(FpcStoreReject::ContentRange);
    }
    if header_value_ignore_case(headers, "content-type").is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
    }) {
        return Some(FpcStoreReject::Streaming);
    }
    // Cap057 LA-CAP057-001: FPC lookup keys force content_encoding=identity.
    // Never store a non-identity representation under that identity key.
    if let Some(ce) = header_values_joined_ignore_case(headers, "content-encoding") {
        let trimmed = ce.trim();
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("identity") {
            return Some(FpcStoreReject::ContentEncoding);
        }
    }
    for (name, _) in headers {
        let lower = name.to_ascii_lowercase();
        if HOP_BY_HOP_RESPONSE.iter().any(|hop| lower == *hop) {
            return Some(FpcStoreReject::HopByHop);
        }
    }
    if let Some(value) = header_values_joined_ignore_case(headers, "cache-control") {
        let lower = value.to_ascii_lowercase();
        if cache_control_token_present(&lower, "private") {
            return Some(FpcStoreReject::Private);
        }
        if cache_control_token_present(&lower, "no-store") {
            return Some(FpcStoreReject::NoStore);
        }
        if cache_control_token_present(&lower, "no-cache") {
            return Some(FpcStoreReject::NoCache);
        }
    }
    if let Some(value) = header_values_joined_ignore_case(headers, "vary") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            // v0: any Vary (including *) → reject.
            return Some(FpcStoreReject::UnsupportedVary);
        }
    }
    None
}

fn cache_control_token_present(lower_cc: &str, token: &str) -> bool {
    lower_cc.split(',').any(|part| {
        let p = part.trim();
        p == token || p.starts_with(&format!("{token}="))
    })
}

fn fpc_effective_ttl(
    response_headers: &[(String, String)],
    default_ttl: Duration,
    max_ttl: Duration,
) -> Result<Duration, FpcStoreReject> {
    if max_ttl.is_zero() || default_ttl.is_zero() {
        return Err(FpcStoreReject::TtlInvalid);
    }
    let mut from_header: Option<u64> = None;
    if let Some(value) = header_values_joined_ignore_case(response_headers, "cache-control") {
        let lower = value.to_ascii_lowercase();
        if let Some(v) = cache_control_seconds(&lower, "s-maxage") {
            from_header = Some(v);
        } else if let Some(v) = cache_control_seconds(&lower, "max-age") {
            from_header = Some(v);
        }
    }
    let secs = match from_header {
        Some(0) => return Err(FpcStoreReject::TtlInvalid),
        Some(v) => v,
        None => default_ttl.as_secs().max(1),
    };
    let capped = secs.min(max_ttl.as_secs().max(1));
    Ok(Duration::from_secs(capped))
}

fn cache_control_seconds(lower_cc: &str, name: &str) -> Option<u64> {
    let prefix = format!("{name}=");
    for part in lower_cc.split(',') {
        let p = part.trim();
        if let Some(rest) = p.strip_prefix(&prefix) {
            let num = rest.split(';').next().unwrap_or(rest).trim();
            return num.parse::<u64>().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_response_cache_control_private_rejects_store() {
        assert_eq!(
            assess_cacheability(
                "GET",
                &[],
                200,
                &[
                    ("cache-control".into(), "public".into()),
                    ("cache-control".into(), "private".into()),
                ],
                1,
                1024,
            )
            .unwrap_err(),
            CacheRejection::ResponseCacheControl
        );
    }

    #[test]
    fn split_request_cache_control_no_cache_not_eligible() {
        assert!(!request_eligible_for_cache(
            "GET",
            &[
                ("Cache-Control".into(), "max-age=0".into()),
                ("Cache-Control".into(), "no-cache".into()),
            ]
        ));
    }

    #[test]
    fn non_identity_content_encoding_rejects_store() {
        assert_eq!(
            assess_cacheability(
                "GET",
                &[],
                200,
                &[("content-encoding".into(), "gzip".into())],
                4,
                1024,
            )
            .unwrap_err(),
            CacheRejection::ResponseContentEncoding
        );
    }

    #[test]
    fn get_200_without_blockers_is_cacheable() {
        assert!(assess_cacheability(
            "GET",
            &[],
            200,
            &[("content-type".into(), "text/plain".into())],
            4,
            1024,
        )
        .is_ok());
    }

    #[test]
    fn authorization_skips_cache_lookup() {
        assert!(!request_eligible_for_cache(
            "GET",
            &[(String::from("Authorization"), String::from("x"))]
        ));
    }

    #[test]
    fn cap020_conditional_headers_skip_cache_lookup() {
        assert!(!request_eligible_for_cache(
            "GET",
            &[("If-None-Match".into(), "W/\"exq-1\"".into())]
        ));
        assert!(!request_eligible_for_cache(
            "GET",
            &[(
                "If-Modified-Since".into(),
                "Sun, 06 Nov 1994 08:49:37 GMT".into()
            )]
        ));
        assert_eq!(
            fpc_request_evaluate(
                "GET",
                "/a",
                "",
                &[("If-None-Match".into(), "W/\"x\"".into())],
                &[],
                &[],
            )
            .unwrap_err(),
            BypassReason::Conditional
        );
    }

    #[test]
    fn post_is_not_cacheable() {
        assert_eq!(
            assess_cacheability("POST", &[], 200, &[], 0, 1024).unwrap_err(),
            CacheRejection::Method
        );
    }

    #[test]
    fn request_cache_control_no_store_not_eligible() {
        assert!(!request_eligible_for_cache(
            "GET",
            &[("Cache-Control".into(), "no-store".into())]
        ));
    }

    #[test]
    fn vary_header_rejects_storage() {
        assert_eq!(
            assess_cacheability(
                "GET",
                &[],
                200,
                &[("vary".into(), "Accept-Language".into())],
                4,
                1024,
            )
            .unwrap_err(),
            CacheRejection::ResponseVary
        );
    }

    #[test]
    fn sse_content_type_rejects_storage() {
        assert_eq!(
            assess_cacheability(
                "GET",
                &[],
                200,
                &[("content-type".into(), "text/event-stream".into())],
                0,
                1024,
            )
            .unwrap_err(),
            CacheRejection::ResponseStreaming
        );
    }

    #[test]
    fn set_cookie_rejects_storage() {
        assert_eq!(
            assess_cacheability(
                "GET",
                &[],
                200,
                &[("set-cookie".into(), "a=b".into())],
                0,
                1024,
            )
            .unwrap_err(),
            CacheRejection::ResponseSetCookie
        );
    }

    #[test]
    fn oversized_body_rejects_storage() {
        assert_eq!(
            assess_cacheability("GET", &[], 200, &[], 2048, 1024).unwrap_err(),
            CacheRejection::BodyTooLarge
        );
    }

    #[test]
    fn no_store_rejects() {
        assert_eq!(
            assess_cacheability(
                "GET",
                &[],
                200,
                &[("cache-control".into(), "no-store".into())],
                0,
                1024,
            )
            .unwrap_err(),
            CacheRejection::ResponseCacheControl
        );
    }

    #[test]
    fn fpc_wp_admin_bypasses() {
        assert_eq!(
            fpc_path_query_bypass("/wp-admin/plugins.php", ""),
            Some(BypassReason::WpAdmin)
        );
        assert_eq!(
            fpc_path_query_bypass("/wp-login.php", ""),
            Some(BypassReason::WpLogin)
        );
        assert_eq!(fpc_path_query_bypass("/blog/", ""), None);
    }

    #[test]
    fn fpc_preview_and_commerce_bypass() {
        assert_eq!(
            fpc_path_query_bypass("/", "preview=true"),
            Some(BypassReason::Preview)
        );
        assert_eq!(
            fpc_path_query_bypass("/checkout", ""),
            Some(BypassReason::CommercePath)
        );
        assert_eq!(
            fpc_path_query_bypass("/", "wp_customize=1"),
            Some(BypassReason::Customize)
        );
    }

    #[test]
    fn fpc_cookie_hard_deny_and_allowlist() {
        assert_eq!(
            fpc_cookie_bypass(
                &[("Cookie".into(), "wordpress_logged_in_abc=1".into())],
                &[]
            ),
            Some(BypassReason::CookieDenied)
        );
        assert_eq!(
            fpc_cookie_bypass(&[("Cookie".into(), "tracking=1".into())], &[]),
            Some(BypassReason::CookiePresent)
        );
        assert_eq!(
            fpc_cookie_bypass(&[("Cookie".into(), "tracking=1".into())], &["tracking"]),
            None
        );
        // Aggregate all Cookie headers (second may carry session).
        assert_eq!(
            fpc_cookie_bypass(
                &[
                    ("Cookie".into(), "a=1".into()),
                    ("Cookie".into(), "wordpress_logged_in_x=1".into()),
                ],
                &["a"]
            ),
            Some(BypassReason::CookieDenied)
        );
        assert!(fpc_request_bypass("GET", "/", "", &[], &[]).is_none());
        assert_eq!(
            fpc_request_bypass("GET", "/wp-json/wp/v2/posts", "", &[], &[]),
            Some(BypassReason::WpJson)
        );
        assert_eq!(
            fpc_request_bypass("GET", "/", "p=1", &[], &[]),
            Some(BypassReason::UnsafeQuery)
        );
        assert!(!request_eligible_for_cache(
            "GET",
            &[("Cookie".into(), "tracking=1".into())]
        ));
    }

    #[test]
    fn fpc_path_encoding_bypass_and_query_policy() {
        assert_eq!(fpc_canonicalize_path("/wp-admin/%2e%2e/x"), None);
        assert_eq!(
            fpc_path_query_bypass("/%2fwp-admin/", ""),
            Some(BypassReason::UnsafePath)
        );
        assert_eq!(
            fpc_query_decision("utm_source=x", &[]),
            FpcQueryDecision::Eligible {
                canonical: String::new()
            }
        );
        assert_eq!(
            fpc_query_decision("utm_source=x&p=1", &[]),
            FpcQueryDecision::Bypass(BypassReason::UnsafeQuery)
        );
        assert_eq!(
            fpc_query_decision("b=2&a=1", &["a", "b"]),
            FpcQueryDecision::Eligible {
                canonical: "a=1&b=2".into()
            }
        );
        let ok = fpc_request_evaluate("GET", "/blog/", "", &[], &[], &[]).unwrap();
        assert_eq!(ok.0, "/blog");
        assert_eq!(ok.1, "");
    }

    #[test]
    fn fpc_assess_stores_public_200() {
        let ttl = fpc_assess_store(
            "GET",
            200,
            &[("content-type".into(), "text/plain".into())],
            4,
            1024,
            Duration::from_secs(30),
            Duration::from_secs(3600),
        )
        .expect("store");
        assert_eq!(ttl, Duration::from_secs(30));
    }

    #[test]
    fn fpc_assess_rejects_set_cookie_private_nostore_nocache_vary() {
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("set-cookie".into(), "a=1".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::SetCookie
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("cache-control".into(), "private, max-age=60".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::Private
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("cache-control".into(), "no-store".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::NoStore
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("cache-control".into(), "no-cache".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::NoCache
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("vary".into(), "*".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::UnsupportedVary
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("vary".into(), "Accept-Encoding".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::UnsupportedVary
        );
    }

    #[test]
    fn fpc_assess_rejects_non_identity_content_encoding() {
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[
                    ("content-type".into(), "text/plain".into()),
                    ("content-encoding".into(), "gzip".into()),
                ],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::ContentEncoding
        );
        assert!(fpc_assess_store(
            "GET",
            200,
            &[
                ("content-type".into(), "text/plain".into()),
                ("content-encoding".into(), "identity".into()),
            ],
            1,
            1024,
            Duration::from_secs(30),
            Duration::from_secs(60),
        )
        .is_ok());
    }

    #[test]
    fn fpc_assess_rejects_sse_content_type_with_leading_space() {
        // Cap057 LA-CAP057-004: trim before mime compare (Plan12 already trims).
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("content-type".into(), " text/event-stream".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::Streaming
        );
    }

    #[test]
    fn fpc_assess_rejects_status_body_head_and_clamps_ttl() {
        assert_eq!(
            fpc_assess_store(
                "GET",
                206,
                &[],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::Status
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                404,
                &[],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::Status
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[],
                2048,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::BodyTooLarge
        );
        assert_eq!(
            fpc_assess_store(
                "HEAD",
                200,
                &[],
                0,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::HeadMustNotStore
        );
        let ttl = fpc_assess_store(
            "GET",
            200,
            &[("cache-control".into(), "max-age=99999".into())],
            1,
            1024,
            Duration::from_secs(30),
            Duration::from_secs(60),
        )
        .expect("ttl");
        assert_eq!(ttl, Duration::from_secs(60));
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[("cache-control".into(), "max-age=0".into())],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::TtlInvalid
        );
    }

    #[test]
    fn fpc_assess_rejects_split_cache_control_and_vary() {
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[
                    ("cache-control".into(), "public".into()),
                    ("cache-control".into(), "private".into()),
                ],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::Private
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[
                    ("cache-control".into(), "max-age=60".into()),
                    ("cache-control".into(), "no-store".into()),
                ],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::NoStore
        );
        assert_eq!(
            fpc_assess_store(
                "GET",
                200,
                &[
                    ("vary".into(), "Accept-Encoding".into()),
                    ("vary".into(), "User-Agent".into()),
                ],
                1,
                1024,
                Duration::from_secs(30),
                Duration::from_secs(60),
            )
            .unwrap_err(),
            FpcStoreReject::UnsupportedVary
        );
    }
}
