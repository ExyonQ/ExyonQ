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
//! WC7B2/WC7C — Redis Streams coordination provider (invalidation + generation + HMAC).
//!
//! Redis client types are private to this crate. Public surface = module-api traits only.

mod auth;
mod config;
mod health;
mod metrics;
mod provider;
mod wire;

pub use auth::{
    canonical_signing_bytes, check_issued_at, sign_mac, verify_mac, EventSigningKeys, ReplayPolicy,
};
pub use config::{RedisCoordConfig, RedisCoordSecrets};
pub use health::{CoordinationHealthSnapshot, CoordinationProviderHealth};
pub use metrics::{
    l2_redis_ack_failure_total, l2_redis_command_timeout_total, l2_redis_connect_failure_total,
    l2_redis_connect_total, l2_redis_hmac_reject_total, l2_redis_provider_degraded,
    l2_redis_publish_failure_total, l2_redis_publish_total, l2_redis_receive_total,
    l2_redis_reconcile_failure_total, l2_redis_reconcile_total, l2_redis_reconnect_total,
    l2_redis_replay_reject_total, lock_redis_metrics_for_tests, reset_redis_metrics_for_tests,
};
pub use provider::RedisCoordinationProvider;
pub use wire::encode_signed_event;

/// Transport selected for WC7B2/WC7C.
pub const REDIS_TRANSPORT: &str = "STREAMS";

/// Client identity for docs / containment.
pub const REDIS_CLIENT: &str = "redis";
pub const REDIS_CLIENT_VERSION: &str = "0.27.6";
