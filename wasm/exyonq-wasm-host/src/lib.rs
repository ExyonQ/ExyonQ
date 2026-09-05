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
//! Wasmtime host for ExyonQ L4 WASM plugins (v0.4 prep).
//!
//! Not wired into `exyonq-core` until the v0.4 gate — see ADR-009.

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use wasmtime::{
    Config, Engine, InstanceAllocationStrategy, Linker, Module, PoolingAllocationConfig, Store,
};

#[derive(Debug, thiserror::Error)]
pub enum WasmHostError {
    #[error("Wasmtime engine error: {0}")]
    Engine(#[from] wasmtime::Error),
    #[error("Plugin not found: {0}")]
    PluginNotFound(String),
    #[error("Plugin execution exceeded fuel budget")]
    FuelExhausted,
    #[error("Plugin execution timeout: {0}ms")]
    Timeout(u64),
    #[error("Memory limit exceeded: {0} bytes")]
    MemoryLimit(u64),
    #[error("Invalid plugin ABI: {0}")]
    InvalidAbi(String),
}

pub type Result<T> = std::result::Result<T, WasmHostError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterAction {
    Continue,
    Respond {
        status: u16,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
    },
    CloseConnection,
}

pub struct RequestView<'a> {
    pub id: u64,
    pub conn_id: u64,
    pub method: &'a str,
    pub path: &'a str,
    pub host: &'a str,
    pub remote_addr: &'a str,
    pub remote_port: u16,
    pub headers: &'a [(String, String)],
    pub body: Option<&'a [u8]>,
}

struct PluginHostState {
    pending_metrics: Vec<MetricEvent>,
    response_headers: Vec<(String, String)>,
    kv_store: Arc<SharedKvStore>,
    #[allow(dead_code)]
    current_req_id: u64,
    invocation_start: Instant,
    bytes_written_to_guest: u64,
}

#[derive(Clone)]
#[allow(dead_code)]
struct MetricEvent {
    name: String,
    value: f64,
    labels: Vec<(String, String)>,
    kind: String,
}

pub struct SharedKvStore {
    inner: DashMap<String, KvEntry>,
}

struct KvEntry {
    value: Vec<u8>,
    expires_at: Instant,
}

impl SharedKvStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: DashMap::new(),
        })
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let entry = self.inner.get(key)?;
        if entry.expires_at > Instant::now() {
            Some(entry.value.clone())
        } else {
            drop(entry);
            self.inner.remove(key);
            None
        }
    }

    pub fn cas(&self, key: &str, expected: Option<&[u8]>, new_value: &[u8], ttl: Duration) -> bool {
        use dashmap::mapref::entry::Entry;

        match self.inner.entry(key.to_string()) {
            Entry::Occupied(mut entry) => {
                let current = &entry.get().value;
                let still_valid = entry.get().expires_at > Instant::now();

                if !still_valid {
                    if expected.is_none() {
                        entry.insert(KvEntry {
                            value: new_value.to_vec(),
                            expires_at: Instant::now() + ttl,
                        });
                        return true;
                    }
                    return false;
                }

                match expected {
                    Some(exp) if exp == current.as_slice() => {
                        entry.insert(KvEntry {
                            value: new_value.to_vec(),
                            expires_at: Instant::now() + ttl,
                        });
                        true
                    }
                    None => false,
                    _ => false,
                }
            }
            Entry::Vacant(entry) => {
                if expected.is_none() {
                    entry.insert(KvEntry {
                        value: new_value.to_vec(),
                        expires_at: Instant::now() + ttl,
                    });
                    true
                } else {
                    false
                }
            }
        }
    }
}

pub struct WasmPlugin {
    name: String,
    module: Module,
    #[allow(dead_code)]
    config_json: String,
    engine: Engine,
}

impl WasmPlugin {
    pub fn load(name: &str, wasm_path: &Path, config_json: &str) -> Result<Self> {
        let engine = create_engine()?;
        let module = Module::from_file(&engine, wasm_path)?;
        Ok(Self {
            name: name.to_string(),
            module,
            config_json: config_json.to_string(),
            engine,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn call_on_request_headers(
        &self,
        req: &RequestView<'_>,
        kv_store: Arc<SharedKvStore>,
    ) -> Result<FilterAction> {
        let host_state = PluginHostState {
            pending_metrics: Vec::new(),
            response_headers: Vec::new(),
            kv_store,
            current_req_id: req.id,
            invocation_start: Instant::now(),
            bytes_written_to_guest: 0,
        };

        let mut store = Store::new(&self.engine, host_state);
        store.set_fuel(10_000_000)?;
        store.set_epoch_deadline(5);

        let instance = create_instance_with_host_functions(&mut store, &self.module)?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| WasmHostError::InvalidAbi("no exported memory".into()))?;

        let remote_addr_bytes = req.remote_addr.as_bytes();
        let headers_buf = serialize_headers_null_separated(req.headers);
        let base_offset = 65536u32;
        let remote_addr_ptr = base_offset;
        let headers_ptr = base_offset + remote_addr_bytes.len() as u32 + 1;
        let write_start = base_offset as usize;
        let total_needed = remote_addr_bytes.len() + 1 + headers_buf.len();
        if write_start + total_needed > memory.data_size(&store) {
            return Err(WasmHostError::MemoryLimit(total_needed as u64));
        }

        let mem_data = memory.data_mut(&mut store);
        mem_data[write_start..write_start + remote_addr_bytes.len()]
            .copy_from_slice(remote_addr_bytes);
        mem_data[write_start + remote_addr_bytes.len()] = 0;

        let headers_start = write_start + remote_addr_bytes.len() + 1;
        mem_data[headers_start..headers_start + headers_buf.len()].copy_from_slice(&headers_buf);
        store.data_mut().bytes_written_to_guest = total_needed as u64;

        let func = instance
            .get_typed_func::<(u64, u32, u32, u32, u32), i32>(
                &mut store,
                "exyonq_on_request_headers",
            )
            .map_err(|e| WasmHostError::InvalidAbi(e.to_string()))?;

        let ret_value = match func.call(
            &mut store,
            (
                req.id,
                remote_addr_ptr,
                remote_addr_bytes.len() as u32,
                headers_ptr,
                headers_buf.len() as u32,
            ),
        ) {
            Err(e) if is_fuel_error(&e) => return Err(WasmHostError::FuelExhausted),
            Err(e) if is_trap_error(&e) => return Err(WasmHostError::Engine(e)),
            Err(e) => return Err(WasmHostError::Engine(e)),
            Ok(v) => v,
        };

        let host_state = store.data();
        let elapsed_us = host_state.invocation_start.elapsed().as_micros() as u64;
        let response_headers = host_state.response_headers.clone();

        Ok(match ret_value & 0xFFFF {
            0 => FilterAction::Continue,
            1 => {
                let status = (ret_value >> 16) as u16;
                FilterAction::Respond {
                    status,
                    headers: response_headers,
                    body: Some(
                        format!(
                            "{{\"error\":\"rate limit exceeded\",\"elapsed_us\":{elapsed_us}}}"
                        )
                        .into_bytes(),
                    ),
                }
            }
            2 => FilterAction::CloseConnection,
            _ => FilterAction::Continue,
        })
    }
}

/// Engine with InstancePooling, fuel metering, and epoch interruption enabled.
pub fn engine_for_tests() -> Result<Engine> {
    create_engine()
}

/// Compile WAT bytes and invoke an exported `(func (result i32))` with a fuel budget.
pub fn invoke_i32_with_fuel(wat: &str, export: &str, fuel: u64) -> Result<i32> {
    let wasm_bytes = wat::parse_str(wat).map_err(|e| WasmHostError::InvalidAbi(e.to_string()))?;
    let engine = create_engine()?;
    let module = Module::new(&engine, wasm_bytes)?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(fuel)?;
    // `create_engine` enables epoch interruption. Wasmtime's default deadline is 0
    // (already elapsed), so every call traps immediately unless a future deadline is
    // set. Fuel-only helpers must arm a deadline so fuel remains the limiter; epoch
    // traps also match `is_fuel_interrupt_trap` ("interrupt") and would otherwise
    // report false `FuelExhausted` (see invoke_latency / noop with large fuel).
    store.set_epoch_deadline(u64::MAX / 4);
    let instance = Linker::new(&engine).instantiate(&mut store, &module)?;
    let func = instance.get_typed_func::<(), i32>(&mut store, export)?;
    match func.call(&mut store, ()) {
        Err(e) if is_fuel_error(&e) || is_fuel_interrupt_trap(&e) => {
            Err(WasmHostError::FuelExhausted)
        }
        Err(e) => Err(WasmHostError::Engine(e)),
        Ok(v) => Ok(v),
    }
}

/// Compile WAT and invoke with epoch deadline; caller must increment engine epoch to interrupt.
pub fn invoke_with_epoch_deadline(
    engine: &Engine,
    wat: &str,
    export: &str,
    fuel: u64,
    epoch_deadline: u64,
) -> Result<()> {
    let wasm_bytes = wat::parse_str(wat).map_err(|e| WasmHostError::InvalidAbi(e.to_string()))?;
    let module = Module::new(engine, wasm_bytes)?;
    let mut store = Store::new(engine, ());
    store.set_fuel(fuel)?;
    store.set_epoch_deadline(epoch_deadline);
    let instance = Linker::new(engine).instantiate(&mut store, &module)?;
    let func = instance.get_typed_func::<(), ()>(&mut store, export)?;
    match func.call(&mut store, ()) {
        Err(e) if is_epoch_error(&e) || is_fuel_interrupt_trap(&e) => {
            Err(WasmHostError::Timeout(0))
        }
        Err(e) if is_fuel_error(&e) => Err(WasmHostError::FuelExhausted),
        Err(e) => Err(WasmHostError::Engine(e)),
        Ok(()) => Ok(()),
    }
}

fn create_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);
    config.consume_fuel(true);
    config.epoch_interruption(true);

    let mut pool_config = PoolingAllocationConfig::default();
    pool_config.total_memories(256);
    pool_config.total_tables(256);
    pool_config.max_memory_size(4 * 1024 * 1024);
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool_config));

    Ok(Engine::new(&config)?)
}

fn create_instance_with_host_functions(
    store: &mut Store<PluginHostState>,
    module: &Module,
) -> Result<wasmtime::Instance> {
    let mut linker = Linker::<PluginHostState>::new(store.engine());

    linker.func_wrap(
        "env",
        "monotonic_now_ns",
        |_caller: wasmtime::Caller<'_, PluginHostState>| -> u64 {
            Instant::now().elapsed().as_nanos() as u64
        },
    )?;

    linker.func_wrap(
        "env",
        "kv_cas",
        |mut caller: wasmtime::Caller<'_, PluginHostState>,
         key_ptr: u32,
         key_len: u32,
         exp_ptr: u32,
         exp_len: u32,
         new_ptr: u32,
         new_len: u32,
         ttl_ms: u32|
         -> i32 {
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return -1,
            };
            let mem = memory.data(&caller);
            let key = match read_str_from_guest(mem, key_ptr, key_len) {
                Some(s) => s.to_string(),
                None => return -1,
            };
            let expected: Option<&[u8]> = if exp_len > 0 {
                let start = exp_ptr as usize;
                let end = start + exp_len as usize;
                if end > mem.len() {
                    return -1;
                }
                Some(&mem[start..end])
            } else {
                None
            };
            let new_start = new_ptr as usize;
            let new_end = new_start + new_len as usize;
            if new_end > mem.len() {
                return -1;
            }
            let new_value = &mem[new_start..new_end];
            let ttl = Duration::from_millis(ttl_ms as u64);
            let new_value_owned = new_value.to_vec();
            let expected_owned: Option<Vec<u8>> = expected.map(|e| e.to_vec());
            let swapped =
                caller
                    .data()
                    .kv_store
                    .cas(&key, expected_owned.as_deref(), &new_value_owned, ttl);
            if swapped {
                1
            } else {
                0
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "kv_get",
        |mut caller: wasmtime::Caller<'_, PluginHostState>,
         key_ptr: u32,
         key_len: u32,
         out_ptr: u32,
         out_len: u32|
         -> i32 {
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return -1,
            };
            let key = {
                let mem = memory.data(&caller);
                match read_str_from_guest(mem, key_ptr, key_len) {
                    Some(s) => s.to_string(),
                    None => return -1,
                }
            };
            match caller.data().kv_store.get(&key) {
                None => 0,
                Some(value) => {
                    let copy_len = value.len().min(out_len as usize);
                    let mem_data = memory.data_mut(&mut caller);
                    let start = out_ptr as usize;
                    if start + copy_len > mem_data.len() {
                        return -1;
                    }
                    mem_data[start..start + copy_len].copy_from_slice(&value[..copy_len]);
                    copy_len as i32
                }
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "emit_metric",
        |mut caller: wasmtime::Caller<'_, PluginHostState>,
         name_ptr: u32,
         name_len: u32,
         value: f64,
         _labels_ptr: u32,
         _labels_len: u32,
         kind_ptr: u32,
         kind_len: u32| {
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return,
            };
            let mem = memory.data(&caller);
            let name = read_str_from_guest(mem, name_ptr, name_len)
                .unwrap_or("unknown")
                .to_string();
            let kind = read_str_from_guest(mem, kind_ptr, kind_len)
                .unwrap_or("counter")
                .to_string();
            caller.data_mut().pending_metrics.push(MetricEvent {
                name,
                value,
                labels: Vec::new(),
                kind,
            });
        },
    )?;

    linker.func_wrap(
        "env",
        "set_response_header",
        |mut caller: wasmtime::Caller<'_, PluginHostState>,
         _req_id: u64,
         name_ptr: u32,
         name_len: u32,
         val_ptr: u32,
         val_len: u32|
         -> i32 {
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return -1,
            };
            let mem = memory.data(&caller);
            let name = read_str_from_guest(mem, name_ptr, name_len)
                .unwrap_or("x-plugin-header")
                .to_string();
            let value = read_str_from_guest(mem, val_ptr, val_len)
                .unwrap_or("")
                .to_string();
            caller.data_mut().response_headers.push((name, value));
            0
        },
    )?;

    linker.func_wrap(
        "env",
        "log",
        |mut caller: wasmtime::Caller<'_, PluginHostState>, msg_ptr: u32, msg_len: u32| {
            let memory = match caller.get_export("memory") {
                Some(wasmtime::Extern::Memory(m)) => m,
                _ => return,
            };
            let mem = memory.data(&caller);
            if let Some(msg) = read_str_from_guest(mem, msg_ptr, msg_len) {
                eprintln!("[wasm-plugin] {msg}");
            }
        },
    )?;

    Ok(linker.instantiate(store, module)?)
}

fn read_str_from_guest(mem: &[u8], ptr: u32, len: u32) -> Option<&str> {
    let start = ptr as usize;
    let end = start + len as usize;
    if end > mem.len() {
        return None;
    }
    std::str::from_utf8(&mem[start..end]).ok()
}

fn serialize_headers_null_separated(headers: &[(String, String)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(headers.iter().map(|(k, v)| k.len() + v.len() + 2).sum());
    for (name, value) in headers {
        buf.extend_from_slice(name.as_bytes());
        buf.push(0);
        buf.extend_from_slice(value.as_bytes());
        buf.push(0);
    }
    buf
}

fn is_fuel_interrupt_trap(err: &wasmtime::Error) -> bool {
    format!("{err:#}")
        .to_ascii_lowercase()
        .contains("interrupt")
}

fn is_fuel_error(err: &wasmtime::Error) -> bool {
    // Prefer `{err:#}` so Wasmtime cause-chain text ("wasm trap: all fuel consumed…")
    // is visible; `to_string()` often only shows the outer backtrace header.
    let s = format!("{err:#}").to_ascii_lowercase();
    s.contains("fuel") || s.contains("out of fuel") || s.contains("all fuel")
}

fn is_epoch_error(err: &wasmtime::Error) -> bool {
    err.to_string().to_ascii_lowercase().contains("epoch")
}

fn is_trap_error(err: &wasmtime::Error) -> bool {
    err.to_string().to_ascii_lowercase().contains("trap")
}

pub struct WasmPluginManager {
    plugins: HashMap<String, WasmPlugin>,
    kv_store: Arc<SharedKvStore>,
    total_invocations: AtomicU64,
    total_rejected: AtomicU64,
    total_fuel_exhausted: AtomicU64,
}

impl Default for WasmPluginManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmPluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            kv_store: SharedKvStore::new(),
            total_invocations: AtomicU64::new(0),
            total_rejected: AtomicU64::new(0),
            total_fuel_exhausted: AtomicU64::new(0),
        }
    }

    pub fn load_plugin(&mut self, name: &str, wasm_path: &Path, config_json: &str) -> Result<()> {
        let plugin = WasmPlugin::load(name, wasm_path, config_json)?;
        self.plugins.insert(name.to_string(), plugin);
        Ok(())
    }

    pub fn run_http_filter_chain(&self, req: &RequestView<'_>) -> FilterAction {
        self.total_invocations.fetch_add(1, Ordering::Relaxed);

        for plugin in self.plugins.values() {
            match plugin.call_on_request_headers(req, Arc::clone(&self.kv_store)) {
                Ok(FilterAction::Continue) => continue,
                Ok(action @ FilterAction::Respond { .. }) => {
                    self.total_rejected.fetch_add(1, Ordering::Relaxed);
                    return action;
                }
                Ok(action @ FilterAction::CloseConnection) => {
                    self.total_rejected.fetch_add(1, Ordering::Relaxed);
                    return action;
                }
                Err(WasmHostError::FuelExhausted) => {
                    self.total_fuel_exhausted.fetch_add(1, Ordering::Relaxed);
                    eprintln!(
                        "[wasm-host] Plugin '{}' exhausted fuel budget — skipping",
                        plugin.name()
                    );
                    continue;
                }
                Err(e) => {
                    eprintln!(
                        "[wasm-host] Plugin '{}' error: {e} — skipping",
                        plugin.name()
                    );
                    continue;
                }
            }
        }

        FilterAction::Continue
    }

    pub fn stats(&self) -> PluginManagerStats {
        PluginManagerStats {
            loaded_plugins: self.plugins.len(),
            total_invocations: self.total_invocations.load(Ordering::Relaxed),
            total_rejected: self.total_rejected.load(Ordering::Relaxed),
            total_fuel_exhausted: self.total_fuel_exhausted.load(Ordering::Relaxed),
        }
    }
}

pub struct PluginManagerStats {
    pub loaded_plugins: usize,
    pub total_invocations: u64,
    pub total_rejected: u64,
    pub total_fuel_exhausted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFINITE_LOOP_WAT: &str = r#"
        (module
          (func (export "run") (result i32)
            (local $i i32)
            (local.set $i (i32.const 0))
            (loop $l
              (local.set $i (i32.add (local.get $i) (i32.const 1)))
              (br $l))
            (local.get $i))
        )
    "#;

    const NOOP_WAT: &str = r#"
        (module
          (func (export "run") (result i32)
            i32.const 42)
        )
    "#;

    #[test]
    fn engine_enables_fuel_and_pooling() {
        let engine = engine_for_tests().expect("engine");
        let mut store = Store::new(&engine, ());
        store.set_fuel(1).expect("fuel");
        store.set_epoch_deadline(1);
    }

    #[test]
    fn fuel_trap_on_infinite_loop() {
        let err = invoke_i32_with_fuel(INFINITE_LOOP_WAT, "run", 10_000)
            .expect_err("fuel should exhaust");
        assert!(
            matches!(err, WasmHostError::FuelExhausted),
            "expected FuelExhausted, got {err:?}"
        );
    }

    #[test]
    fn noop_invoke_succeeds_with_fuel_budget() {
        let v = invoke_i32_with_fuel(NOOP_WAT, "run", 1_000_000).expect("noop invoke");
        assert_eq!(v, 42);
    }

    #[test]
    fn epoch_deadline_configured() {
        let engine = engine_for_tests().expect("engine");
        let mut store = Store::new(&engine, ());
        store.set_epoch_deadline(5);
    }

    #[test]
    fn test_kv_cas_first_insert() {
        let store = SharedKvStore::new();
        assert!(store.cas("key1", None, b"value1", Duration::from_secs(60)));
        assert!(!store.cas("key1", None, b"value2", Duration::from_secs(60)));
        assert!(store.cas("key1", Some(b"value1"), b"value2", Duration::from_secs(60)));
    }

    #[test]
    fn test_serialize_headers() {
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Request-ID".to_string(), "abc123".to_string()),
        ];
        let buf = serialize_headers_null_separated(&headers);
        let expected = b"Content-Type\0application/json\0X-Request-ID\0abc123\0";
        assert_eq!(buf, expected);
    }

    #[test]
    fn test_read_str_from_guest() {
        let mem = b"hello world\0extra data";
        assert_eq!(read_str_from_guest(mem, 0, 5), Some("hello"));
        assert_eq!(read_str_from_guest(mem, 6, 5), Some("world"));
        assert_eq!(read_str_from_guest(mem, 100, 5), None);
    }

    // -------------------------------------------------------------------------
    // WASM-001: run_http_filter_chain fail-open en FuelExhausted
    // -------------------------------------------------------------------------
    // Bug: `Err(FuelExhausted) => continue` en run_http_filter_chain (L617-624)
    // Un plugin malicioso o con bug que agota fuel simplemente se salta,
    // permitiendo que la request pase sin filtrado → fail-open.
    //
    // Contrato correcto: FuelExhausted debe ser fail-CLOSED (Reject/Block),
    // no Continue silencioso.

    /// WASM-001 ROJO: PluginManager continúa (fail-open) cuando plugin agota fuel.
    ///
    /// Este test requiere montar PluginManager con un plugin WASM real que
    /// agote fuel. La infraestructura actual de tests usa `invoke_i32_with_fuel`
    /// que opera a nivel más bajo (no pasa por run_http_filter_chain).
    ///
    /// Para probar el path completo necesitamos:
    /// 1. Compilar un plugin WASM con ABI `exyonq_on_request_headers` que haga loop infinito
    /// 2. Cargarlo en WasmPluginManager
    /// 3. Llamar run_http_filter_chain
    /// 4. Verificar que NO devuelve Continue
    ///
    /// Hoy devuelve Continue (fail-open) → este test DEBE FALLAR.
    #[test]
    #[ignore = "WASM-001 gap: need plugin WASM con ABI exyonq_on_request_headers que agote fuel; \
                run_http_filter_chain hace `continue` en FuelExhausted (lib.rs L617-624) — fail-open"]
    fn run_http_filter_chain_must_not_continue_on_fuel_exhausted() {
        // Contrato: cuando un plugin agota fuel, run_http_filter_chain
        // DEBE devolver algo distinto de FilterAction::Continue (fail-closed).
        //
        // El código actual (L617-624) hace:
        //   Err(WasmHostError::FuelExhausted) => {
        //       self.total_fuel_exhausted.fetch_add(1, ...);
        //       eprintln!("[wasm-host] Plugin '{}' exhausted fuel — skipping", ...);
        //       continue;  // <-- BUG: fail-open
        //   }
        //
        // Esto permite bypass de filtros de seguridad (rate limiting, WAF, etc).
        //
        // Para implementar este test necesitamos un plugin .wasm con la firma:
        //   (func (export "exyonq_on_request_headers") (param i64 i32 i32 i32 i32) (result i32)
        //     (loop $inf (br $inf)))  ; loop infinito para agotar fuel
        //
        // El mock sería crear el WASM en memoria, cargarlo con load_plugin,
        // y verificar que run_http_filter_chain NO devuelve Continue.
        //
        // Por ahora documentamos el gap; el test real requiere fixture WASM.
        let manager = WasmPluginManager::new();
        let req = RequestView {
            id: 1,
            conn_id: 1,
            method: "GET",
            path: "/test",
            host: "example.com",
            remote_addr: "127.0.0.1",
            remote_port: 12345,
            headers: &[],
            body: None,
        };

        let action = manager.run_http_filter_chain(&req);

        // Sin plugins cargados, esto es Continue (correcto para cadena vacía)
        // Con plugin que agota fuel, DEBERÍA ser Reject/CloseConnection, no Continue
        assert!(
            !matches!(action, FilterAction::Continue),
            "WASM-001: run_http_filter_chain DEBE fallar cerrado (no Continue) \
             cuando plugin agota fuel. Hoy hace `continue` silencioso (fail-open). \
             Ver lib.rs L617-624: Err(FuelExhausted) => continue"
        );
    }

    /// Verificación auxiliar: fuel_trap_on_infinite_loop confirma que
    /// WasmHostError::FuelExhausted SÍ se genera para loops infinitos.
    /// El problema está en cómo run_http_filter_chain MANEJA ese error.
    #[test]
    fn fuel_exhausted_error_is_generated_for_infinite_loop() {
        // Este test es verde - confirma que el error se genera correctamente
        let err = invoke_i32_with_fuel(INFINITE_LOOP_WAT, "run", 10_000)
            .expect_err("debe agotar fuel");
        assert!(
            matches!(err, WasmHostError::FuelExhausted),
            "WASM-001 baseline: FuelExhausted se genera para loop infinito"
        );
    }
}
