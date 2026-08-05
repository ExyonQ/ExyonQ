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

use async_trait::async_trait;
use exyonq_module_api::kernel_control::{
    KernelControlPort, KernelStatusSnapshot, OpsCommand, OpsCommandOutcome,
};
use exyonq_module_api::reload_runtime::ReloadRuntimeService;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct StubPort {
    pub reloads: Mutex<Vec<std::path::PathBuf>>,
    pub shutdowns: Mutex<u32>,
    pub reject_reload: Mutex<bool>,
}

#[async_trait]
impl KernelControlPort for StubPort {
    async fn request_reload(&self, config_path: &Path) -> OpsCommandOutcome {
        if *self.reject_reload.lock().unwrap() {
            return OpsCommandOutcome::from_parts(
                false,
                OpsCommand::Reload,
                snapshot(1),
                Some("reload rejected: shutdown in progress".into()),
                None,
            );
        }
        self.reloads.lock().unwrap().push(config_path.to_path_buf());
        OpsCommandOutcome::from_parts(true, OpsCommand::Reload, snapshot(2), None, None)
    }

    fn request_drain(&self) -> OpsCommandOutcome {
        OpsCommandOutcome::from_parts(true, OpsCommand::Drain, snapshot(1), None, None)
    }

    fn request_shutdown(&self) -> OpsCommandOutcome {
        *self.shutdowns.lock().unwrap() += 1;
        OpsCommandOutcome::from_parts(true, OpsCommand::Shutdown, snapshot(1), None, None)
    }

    fn read_status(&self, _include_version: bool) -> OpsCommandOutcome {
        OpsCommandOutcome::from_parts(true, OpsCommand::Status, snapshot(1), None, None)
    }
}

fn snapshot(generation: u64) -> KernelStatusSnapshot {
    KernelStatusSnapshot {
        generation,
        fingerprint: "test".into(),
        uptime_s: 0,
        active_connections: 0,
        draining: false,
        version: None,
        reload_in_progress: false,
    }
}

pub fn stub_port() -> Arc<StubPort> {
    Arc::new(StubPort::default())
}

pub struct TestReloadRuntime;

impl ReloadRuntimeService for TestReloadRuntime {
    fn start(
        &self,
        config_path: Option<std::path::PathBuf>,
        port: Arc<dyn KernelControlPort>,
    ) -> exyonq_module_api::reload_runtime::ReloadRuntimeSupervisor {
        exyonq_reload_runtime::FileReloadRuntime.start(config_path, port)
    }
}
