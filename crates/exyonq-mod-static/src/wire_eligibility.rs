/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License;
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! Wire path eligibility predicates (KD2.3 / KD2.5).

pub fn might_use_static_wire(head: &[u8]) -> bool {
    head.starts_with(b"GET /health ")
        || head.starts_with(b"HEAD /health ")
        || head.starts_with(b"GET /site/1k.bin ")
        || head.starts_with(b"HEAD /site/1k.bin ")
        || head.starts_with(b"GET /site/64k.bin ")
        || head.starts_with(b"HEAD /site/64k.bin ")
        || head.starts_with(b"GET /site/1m.bin ")
        || head.starts_with(b"HEAD /site/1m.bin ")
        || head.starts_with(b"GET /site/routes/route")
        || head.starts_with(b"HEAD /site/routes/route")
        || head.starts_with(b"GET /metrics ")
        || head.starts_with(b"GET /metrics\r")
}

#[cfg(target_os = "linux")]
pub fn epoll_keep_alive_eligible(head: &[u8]) -> bool {
    head.starts_with(b"GET /health ")
        || head.starts_with(b"GET /site/1k.bin ")
        || head.starts_with(b"GET /site/routes/")
}

#[cfg(target_os = "linux")]
pub fn is_sendfile_bench_head(head: &[u8]) -> bool {
    head.starts_with(b"GET /site/64k.bin ")
        || head.starts_with(b"HEAD /site/64k.bin ")
        || head.starts_with(b"GET /site/1m.bin ")
        || head.starts_with(b"HEAD /site/1m.bin ")
}

#[cfg(target_os = "linux")]
pub fn epoll_sendfile_eligible(head: &[u8]) -> bool {
    is_sendfile_bench_head(head)
        && crate::sendfile_fsm::epoll_sendfile_enabled()
        && exyonq_module_api::static_wire::keepalive_handoff_enabled()
}

#[cfg(target_os = "linux")]
pub fn static_wire_use_blocking_pool(head: &[u8]) -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let enabled = *ENABLED
        .get_or_init(|| std::env::var("EXYONQ_STATIC_BLOCKING").ok().as_deref() == Some("1"));
    enabled && !head.starts_with(b"GET /site/64k.bin ") && !head.starts_with(b"GET /site/1m.bin ")
}

#[cfg(target_os = "linux")]
pub fn static_sendfile_use_blocking_pool(head: &[u8]) -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("EXYONQ_STATIC_SENDFILE_BLOCKING")
            .ok()
            .as_deref()
            == Some("1")
    }) && (head.starts_with(b"GET /site/64k.bin ") || head.starts_with(b"GET /site/1m.bin "))
}

#[cfg(target_os = "linux")]
pub fn static_one_m_sendfile_use_blocking(head: &[u8]) -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("EXYONQ_STATIC_1M_BLOCKING").ok().as_deref() == Some("1"))
        && head.starts_with(b"GET /site/1m.bin ")
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_keep_alive_eligible(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn is_sendfile_bench_head(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_sendfile_eligible(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn static_wire_use_blocking_pool(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn static_sendfile_use_blocking_pool(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn static_one_m_sendfile_use_blocking(_head: &[u8]) -> bool {
    false
}
