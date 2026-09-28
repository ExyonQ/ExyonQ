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

use std::net::SocketAddr;
use std::time::Duration;

/// Fresh per-request execution plan (ONE_REQUEST = ONE_FRESH_PLAN).
#[derive(Debug, Clone)]
pub struct ProxyExecutionPlan {
    pub generation_id: u64,
    pub method: Method,
    pub path_query: String,
    pub host: Option<String>,
    pub route_idx: usize,
    pub cluster_id: u32,
    pub upstream: SocketAddr,
    pub upstream_host_header: String,
    pub client_keepalive: bool,
    pub waf: WafAdmission,
    pub connect_timeout: Duration,
    pub header_timeout: Duration,
    pub idle_timeout: Duration,
    pub path_identity: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
}

#[derive(Debug, Clone)]
pub enum WafAdmission {
    Allow,
    Reject {
        status: u16,
        body: Vec<u8>,
        content_type: String,
    },
}
