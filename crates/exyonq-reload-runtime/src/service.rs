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
//! Composition-root registration for reload watcher and signal runtime.

use crate::config_watcher::spawn_config_watcher;
use crate::signal_runtime::spawn_shutdown_listener;
use exyonq_module_api::kernel_control::KernelControlPort;
use exyonq_module_api::reload_runtime::{
    register_reload_runtime_service, ReloadRuntimeRegisterError, ReloadRuntimeService,
    ReloadRuntimeSupervisor,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

pub struct FileReloadRuntime;

pub fn register_reload_runtime() -> Result<(), ReloadRuntimeRegisterError> {
    register_reload_runtime_service(Arc::new(FileReloadRuntime))
}

impl ReloadRuntimeService for FileReloadRuntime {
    fn start(
        &self,
        config_path: Option<PathBuf>,
        port: Arc<dyn KernelControlPort>,
    ) -> ReloadRuntimeSupervisor {
        let cancel = CancellationToken::new();
        let mut tasks = Vec::new();
        if let Some(path) = config_path {
            tasks.push(spawn_config_watcher(
                path,
                Arc::clone(&port),
                cancel.clone(),
            ));
        }
        tasks.push(spawn_shutdown_listener(port, cancel.clone()));
        let handles = Arc::new(Mutex::new(tasks));
        let cancel_for_shutdown = cancel.clone();
        let handles_for_shutdown = Arc::clone(&handles);
        ReloadRuntimeSupervisor::new(Some(Box::new(move || {
            cancel_for_shutdown.cancel();
            if let Ok(mut guard) = handles_for_shutdown.lock() {
                for task in guard.drain(..) {
                    task.abort();
                }
            }
        })))
    }
}
