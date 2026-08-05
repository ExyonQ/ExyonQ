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

mod common;

use common::{stub_port, TestReloadRuntime};
use exyonq_module_api::kernel_control::KernelControlPort;
use exyonq_module_api::reload_runtime::ReloadRuntimeService;

#[tokio::test]
async fn shutdown_listener_cancels_cleanly() {
    let port = stub_port();
    let cancel = tokio_util::sync::CancellationToken::new();
    let handle = exyonq_reload_runtime::signal_runtime::spawn_shutdown_listener(
        port.clone(),
        cancel.clone(),
    );

    cancel.cancel();
    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn repeated_shutdown_on_port_is_safe() {
    let port = stub_port();
    let _ = port.request_shutdown();
    let _ = port.request_shutdown();
    assert_eq!(*port.shutdowns.lock().unwrap(), 2);
}

#[tokio::test]
async fn signal_supervisor_cleans_up() {
    let port = stub_port();
    let supervisor = TestReloadRuntime.start(None, port);
    drop(supervisor);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
}
