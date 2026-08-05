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
use std::path::PathBuf;

use exyonq_wasm_host::{FilterAction, RequestView, SharedKvStore, WasmPlugin};

fn compile_fixture(name: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let wat = manifest_dir
        .join("tests/fixtures")
        .join(format!("{name}.wat"));
    let wasm = std::env::temp_dir().join(format!("exyonq-wasm-host-{name}.wasm"));
    let bytes = wat::parse_file(&wat).expect("parse fixture wat");
    std::fs::write(&wasm, bytes).expect("write wasm");
    wasm
}

#[test]
fn plugin_load_and_call_continue() {
    let wasm_path = compile_fixture("noop");
    let plugin = WasmPlugin::load("noop", &wasm_path, "{}").expect("load plugin");
    let kv = SharedKvStore::new();
    let headers: Vec<(String, String)> = vec![("Host".into(), "example.com".into())];
    let req = RequestView {
        id: 1,
        conn_id: 1,
        method: "GET",
        path: "/",
        host: "example.com",
        remote_addr: "127.0.0.1",
        remote_port: 1234,
        headers: &headers,
        body: None,
    };
    let action = plugin
        .call_on_request_headers(&req, kv)
        .expect("invoke plugin");
    assert_eq!(action, FilterAction::Continue);
}
