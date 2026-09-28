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

//! `exyonq-dataplane` — Competitive Frontier separate-process H1 dataplane.
//!
//! Phase 4: exclusive listener, shard-local accept, immutable generation load,
//! real H1 routing, WAF headers, and bounded CL body streaming. No Tokio/Hyper.

mod bounds;
mod exclusive;
mod fcgi_exec;
mod fcgi_pool;
mod fcgi_route;
mod hop;
mod http_parse;
mod obs;
mod shard;
mod static_serve;
mod upstream_pool;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use exyonq_cfd_gen::{GenDir, SCHEMA_VERSION};

fn main() {
    if let Err(e) = real_main() {
        eprintln!("exyonq-dataplane error: {e}");
        let _ = write_failed_best_effort(&e.to_string());
        process::exit(1);
    }
}

fn real_main() -> io::Result<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_usage();
        process::exit(2);
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "serve" => cmd_serve(&args),
        "version" => {
            println!("exyonq-dataplane schema={SCHEMA_VERSION}");
            Ok(())
        }
        _ => {
            print_usage();
            process::exit(2);
        }
    }
}

fn print_usage() {
    eprintln!(
        "usage:\n  exyonq-dataplane serve --listen ADDR --gen-dir DIR --shards N --schema-version V\n  exyonq-dataplane version"
    );
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].as_str())
}

fn cmd_serve(args: &[String]) -> io::Result<()> {
    let listen: SocketAddr = flag(args, "--listen")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing --listen"))?
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let gen_dir = PathBuf::from(
        flag(args, "--gen-dir")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing --gen-dir"))?,
    );
    let shards: usize = flag(args, "--shards")
        .unwrap_or("1")
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    if shards == 0 || shards > 128 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "shards must be 1..=128",
        ));
    }
    let schema: u32 = match flag(args, "--schema-version") {
        Some(s) => s
            .parse()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
        None => SCHEMA_VERSION,
    };
    if schema != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("incompatible schema {schema} (dataplane={SCHEMA_VERSION})"),
        ));
    }

    // Architecture invariants (observational markers for qualification).
    eprintln!("exyonq-dataplane TOKIO_THREADS_IN_DATAPLANE_PROCESS=0");
    eprintln!("exyonq-dataplane HYPER_IN_DATAPLANE_PROCESS=0");
    eprintln!("exyonq-dataplane CONTROL_PLANE_RPCS_PER_REQUEST=0");
    eprintln!("exyonq-dataplane OBS_HOT_PATH_SINK_IO=0");
    eprintln!("exyonq-dataplane OBS_PER_REQUEST_CONTROL_RPC=0");
    eprintln!("exyonq-dataplane SDP_P1=not_invoked");
    eprintln!("exyonq-dataplane PXDP_EXECUTION=phase4_bounded_body_streaming");
    eprintln!(
        "exyonq-dataplane BODY_RELAY_BOUNDS request={} response={} per_connection={} chunk_line={} trailer_bytes={} trailer_count={}",
        bounds::REQUEST_RELAY_BUFFER_BYTES,
        bounds::RESPONSE_RELAY_BUFFER_BYTES,
        bounds::MAX_BODY_BUFFER_BYTES_PER_CONNECTION,
        bounds::MAX_CHUNK_LINE_BYTES,
        bounds::MAX_TRAILER_BYTES,
        bounds::MAX_TRAILER_COUNT
    );

    let gd = GenDir::new(&gen_dir);
    gd.ensure()?;
    harden_gen_dir_perms(&gen_dir)?;
    // Process exclusivity BEFORE bind — blocks second REUSEPORT dataplane owner.
    let _lock = exclusive::acquire(&gen_dir).map_err(|e| {
        let _ = write_status(&gen_dir, &format!("FAILED exclusive_lock:{e}"));
        e
    })?;
    write_status(&gen_dir, "STARTING")?;

    let stop = Arc::new(AtomicBool::new(false));
    let live_gen = Arc::new(AtomicU64::new(0));
    let bound = Arc::new(AtomicUsize::new(0));
    let bind_failed = Arc::new(AtomicBool::new(false));
    let obs = obs::ObsHub::new();
    let spawn_ok = obs::spawn_export_thread(Arc::clone(&obs), Arc::clone(&stop));
    let _obs_export = match obs::export_spawn_outcome(obs::obs_env_configured(), spawn_ok.is_some())
    {
        obs::ExportSpawnOutcome::Started => spawn_ok,
        obs::ExportSpawnOutcome::OptionalAbsent => None,
        obs::ExportSpawnOutcome::RequiredFailed => {
            write_status(&gen_dir, "FAILED obs_export_thread_spawn")?;
            return Err(io::Error::other(
                "observability export thread spawn failed while EXYONQ_CFD_OBS_* configured",
            ));
        }
    };

    let initial = match gd.load() {
        Ok(g) => g,
        Err(e) => {
            write_status(&gen_dir, &format!("FAILED gen_load:{e}"))?;
            return Err(io::Error::other(e));
        }
    };
    live_gen.store(initial.generation_id, Ordering::SeqCst);
    obs.generation_current
        .store(initial.generation_id, Ordering::Relaxed);
    obs.generation_activate.fetch_add(1, Ordering::Relaxed);
    obs.try_push_event(obs::ObsRecord {
        ts_unix_ms: obs::ObsRecord::now_ms(),
        event: obs::ObsEvent::GenerationActivate,
        method: "INTERNAL",
        route_id: 0,
        status: 0,
        status_class: obs::StatusClass::Other,
        bytes_in: 0,
        bytes_out: 0,
        latency_us: 0,
        generation_id: initial.generation_id,
        shard_id: 0,
        error_class: "none",
    });
    eprintln!(
        "exyonq-dataplane loaded generation_id={} schema={}",
        initial.generation_id, initial.schema_version
    );

    {
        let gd = GenDir::new(&gen_dir);
        let stop = Arc::clone(&stop);
        let live_gen = Arc::clone(&live_gen);
        let obs_gen = Arc::clone(&obs);
        thread::Builder::new()
            .name("cfd-gen-watch".into())
            .spawn(move || gen_watch_loop(gd, stop, live_gen, obs_gen))
            .map_err(io::Error::other)?;
    }

    {
        let stop = Arc::clone(&stop);
        let path = gen_dir.join("shutdown");
        thread::Builder::new()
            .name("cfd-shutdown".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    if path.exists() {
                        stop.store(true, Ordering::SeqCst);
                        break;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            })
            .map_err(io::Error::other)?;
    }

    let mut joins = Vec::new();
    for i in 0..shards {
        let stop = Arc::clone(&stop);
        let live_gen = Arc::clone(&live_gen);
        let bound = Arc::clone(&bound);
        let bind_failed = Arc::clone(&bind_failed);
        let gen_dir = gen_dir.clone();
        let obs = Arc::clone(&obs);
        let name = format!("cfd-shard-{i}");
        joins.push(
            thread::Builder::new()
                .name(name)
                .spawn(move || {
                    if let Err(e) = shard::run_shard(shard::ShardArgs {
                        shard_id: i,
                        listen,
                        gen_dir,
                        stop,
                        live_gen,
                        bound,
                        bind_failed,
                        obs,
                    }) {
                        eprintln!("shard {i} exit: {e}");
                    }
                })
                .map_err(io::Error::other)?,
        );
    }

    // READY only after all shards bound successfully (no fake READY).
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if bind_failed.load(Ordering::SeqCst) {
            stop.store(true, Ordering::SeqCst);
            write_status(&gen_dir, "FAILED bind")?;
            for j in joins {
                let _ = j.join();
            }
            return Err(io::Error::other("one or more shards failed to bind"));
        }
        if bound.load(Ordering::SeqCst) >= shards {
            break;
        }
        if Instant::now() >= deadline {
            stop.store(true, Ordering::SeqCst);
            write_status(&gen_dir, "FAILED bind_timeout")?;
            for j in joins {
                let _ = j.join();
            }
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "shards did not bind before READY deadline",
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }

    let gid = live_gen.load(Ordering::SeqCst);
    write_status(&gen_dir, &format!("READY gen={gid} listen={listen}"))?;
    eprintln!("exyonq-dataplane READY shards={shards} listen={listen}");

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(50));
    }
    write_status(&gen_dir, "STOPPING")?;
    for j in joins {
        let _ = j.join();
    }
    write_status(&gen_dir, "STOPPED")?;
    Ok(())
}

fn gen_watch_loop(
    gd: GenDir,
    stop: Arc<AtomicBool>,
    live_gen: Arc<AtomicU64>,
    obs: obs::ObsHubHandle,
) {
    let mut last = None;
    while !stop.load(Ordering::Relaxed) {
        let path = gd.generation_path();
        if let Ok(meta) = fs::metadata(&path) {
            let modified = meta.modified().ok();
            if modified != last {
                last = modified;
                match gd.load() {
                    Ok(g) => {
                        let cur = live_gen.load(Ordering::SeqCst);
                        obs.generation_publishes.fetch_add(1, Ordering::Relaxed);
                        if g.generation_id < cur {
                            obs.generation_reject.fetch_add(1, Ordering::Relaxed);
                            eprintln!(
                                "exyonq-dataplane generation downgrade rejected: {} < {}",
                                g.generation_id, cur
                            );
                        } else {
                            live_gen.store(g.generation_id, Ordering::SeqCst);
                            obs.generation_current
                                .store(g.generation_id, Ordering::Relaxed);
                            obs.generation_activate.fetch_add(1, Ordering::Relaxed);
                            eprintln!("exyonq-dataplane generation updated id={}", g.generation_id);
                        }
                    }
                    Err(e) => {
                        obs.generation_reject.fetch_add(1, Ordering::Relaxed);
                        eprintln!("exyonq-dataplane generation rejected: {e}");
                    }
                }
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn harden_gen_dir_perms(gen_dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(gen_dir)?;
        let mut perms = meta.permissions();
        perms.set_mode(0o700);
        fs::set_permissions(gen_dir, perms)?;
    }
    let _ = gen_dir;
    Ok(())
}

fn write_status(gen_dir: &Path, line: &str) -> io::Result<()> {
    let path = gen_dir.join("status");
    let tmp = gen_dir.join("status.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        writeln!(f, "{line}")?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn write_failed_best_effort(msg: &str) -> io::Result<()> {
    if let Ok(dir) = env::var("EXYONQ_CFD_GEN_DIR") {
        write_status(Path::new(&dir), &format!("FAILED {msg}"))?;
    }
    Ok(())
}
