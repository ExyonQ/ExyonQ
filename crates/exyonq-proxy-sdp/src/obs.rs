/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct DataplaneCounters {
    pub requests: AtomicU64,
    pub responses_2xx: AtomicU64,
    pub responses_4xx: AtomicU64,
    pub responses_5xx: AtomicU64,
    pub waf_deny: AtomicU64,
    pub handoff_legacy: AtomicU64,
    pub errors: AtomicU64,
    pub upstream_reuse: AtomicU64,
}

impl DataplaneCounters {
    pub fn snapshot(&self) -> PathIdentity {
        PathIdentity {
            path: "sdp-p1",
            requests: self.requests.load(Ordering::Relaxed),
            responses_2xx: self.responses_2xx.load(Ordering::Relaxed),
            responses_4xx: self.responses_4xx.load(Ordering::Relaxed),
            responses_5xx: self.responses_5xx.load(Ordering::Relaxed),
            waf_deny: self.waf_deny.load(Ordering::Relaxed),
            handoff_legacy: self.handoff_legacy.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            upstream_reuse: self.upstream_reuse.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PathIdentity {
    pub path: &'static str,
    pub requests: u64,
    pub responses_2xx: u64,
    pub responses_4xx: u64,
    pub responses_5xx: u64,
    pub waf_deny: u64,
    pub handoff_legacy: u64,
    pub errors: u64,
    pub upstream_reuse: u64,
}
