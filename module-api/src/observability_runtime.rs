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
//! KD4.12 — cold-path Prometheus append hook registry (no formatting in core).

use std::sync::Mutex;

pub type PrometheusAppendFn = fn(&mut String);

static PROMETHEUS_APPENDERS: Mutex<Vec<PrometheusAppendFn>> = Mutex::new(Vec::new());
static RUNTIME_APPEND_REGISTERED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Register a cold-path Prometheus text appender (called once at composition root).
pub fn register_prometheus_appender(append: PrometheusAppendFn) {
    if let Ok(mut appends) = PROMETHEUS_APPENDERS.lock() {
        appends.push(append);
    }
}

/// Invoke all registered appenders (metrics module export path only).
pub fn append_registered_prometheus(out: &mut String) {
    if let Ok(appends) = PROMETHEUS_APPENDERS.lock() {
        for append in appends.iter() {
            append(out);
        }
    }
}

/// Install the composite runtime append hook exactly once.
pub fn ensure_runtime_prometheus_hook(install: fn(PrometheusAppendFn)) {
    let _ = RUNTIME_APPEND_REGISTERED.get_or_init(|| {
        install(append_registered_prometheus);
    });
}

#[doc(hidden)]
pub fn clear_prometheus_appenders_for_tests() {
    if let Ok(mut appends) = PROMETHEUS_APPENDERS.lock() {
        appends.clear();
    }
}
