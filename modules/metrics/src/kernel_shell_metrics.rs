/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
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
//! Shell-level HTTP 501 counters for contract backend routes (KD4.12).

use exyonq_module_api::kernel_observation::KernelObservationService;
use std::sync::atomic::{AtomicU64, Ordering};

static FCGI_HTTP_501: AtomicU64 = AtomicU64::new(0);
static STATIC_HTTP_501: AtomicU64 = AtomicU64::new(0);
static PROXY_HTTP_501: AtomicU64 = AtomicU64::new(0);

pub struct KernelShellMetrics;

impl KernelObservationService for KernelShellMetrics {
    fn note_fastcgi_http_501(&self) {
        FCGI_HTTP_501.fetch_add(1, Ordering::Relaxed);
    }

    fn note_static_http_501(&self) {
        STATIC_HTTP_501.fetch_add(1, Ordering::Relaxed);
    }

    fn note_proxy_http_501(&self) {
        PROXY_HTTP_501.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn fastcgi_http_501_total() -> u64 {
    FCGI_HTTP_501.load(Ordering::Relaxed)
}

pub fn static_http_501_total() -> u64 {
    STATIC_HTTP_501.load(Ordering::Relaxed)
}

pub fn proxy_http_501_total() -> u64 {
    PROXY_HTTP_501.load(Ordering::Relaxed)
}

pub fn append_prometheus(out: &mut String) {
    let fcgi = fastcgi_http_501_total();
    let stat = static_http_501_total();
    let proxy = proxy_http_501_total();
    if fcgi == 0 && stat == 0 && proxy == 0 {
        return;
    }
    out.push_str("# TYPE exyonq_kernel_fastcgi_http_501_total counter\n");
    out.push_str(&format!("exyonq_kernel_fastcgi_http_501_total {fcgi}\n"));
    out.push_str("# TYPE exyonq_kernel_static_http_501_total counter\n");
    out.push_str(&format!("exyonq_kernel_static_http_501_total {stat}\n"));
    out.push_str("# TYPE exyonq_kernel_proxy_http_501_total counter\n");
    out.push_str(&format!("exyonq_kernel_proxy_http_501_total {proxy}\n"));
}
