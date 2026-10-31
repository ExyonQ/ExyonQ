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
//! Competitive Frontier dataplane — serve composition hook (default OFF).

use anyhow::Context;
use exyonq_cfd_control::{
    assert_no_listen_collision, dataplane_enabled, CfdChild, CfdLaunchConfig,
};
use exyonq_config_ir::AppConfig;

/// If `EXYONQ_COMPETITIVE_H1_DATAPLANE` is enabled, start the separate-process
/// competitive H1 dataplane (Phase 2: real GET + compiled routing + headers).
/// Returns `None` when the gate is OFF (current product path).
///
/// Fail-closed: missing binary, listen collision with product servers, or READY
/// timeout aborts serve — never falls back to Hyper for competitive H1.
///
/// Optional `EXYONQ_CFD_ROUTES` file projects Cap033-style routes into G1.
/// Routing SSOT remains the control/product plane; dataplane executes the
/// compiled projection only.
pub fn maybe_start_competitive_h1_dataplane(
    app_config: &AppConfig,
) -> anyhow::Result<Option<CfdChild>> {
    if !dataplane_enabled() {
        return Ok(None);
    }
    let cfg = CfdLaunchConfig::from_env().context("competitive H1 dataplane config")?;
    let mut product_listens: Vec<String> = app_config
        .servers
        .iter()
        .map(|s| s.listen.clone())
        .collect();
    for s in &app_config.servers {
        if let Some(h3) = &s.http3_listen {
            product_listens.push(h3.clone());
        }
    }
    assert_no_listen_collision(&cfg.listen, &product_listens)
        .context("competitive H1 listen exclusivity")?;
    let child = CfdChild::start(&cfg).context("competitive H1 dataplane start")?;
    tracing::info!(
        listen = %child.listen,
        shards = child.shards,
        "competitive H1 dataplane READY (Phase 2 GET+routing+headers; gate default OFF)"
    );
    Ok(Some(child))
}
