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
//! Off-hot-path export: console JSON, file, syslog, journald, OpenMetrics scrape.

use super::file_rotate::SizeRotatingWriter;
use super::hub::ObsHub;
use std::env;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixDatagram;

pub fn run(hub: Arc<ObsHub>, stop: Arc<AtomicBool>) {
    let console = env_flag("EXYONQ_CFD_OBS_CONSOLE_JSON");
    let file_path = env::var("EXYONQ_CFD_OBS_FILE").ok().map(PathBuf::from);
    let max_bytes: u64 = env::var("EXYONQ_CFD_OBS_FILE_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 * 1024 * 1024);
    let keep: u32 = env::var("EXYONQ_CFD_OBS_FILE_KEEP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let syslog = env_flag("EXYONQ_CFD_OBS_SYSLOG");
    let journald = env_flag("EXYONQ_CFD_OBS_JOURNALD");
    let metrics_listen = env::var("EXYONQ_CFD_OBS_METRICS_LISTEN").ok();

    let mut file_writer = file_path.as_ref().and_then(|p| {
        let dir = p.parent().unwrap_or_else(|| std::path::Path::new("."));
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("cfd-obs.log");
        match SizeRotatingWriter::open(dir, name, max_bytes, keep) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("exyonq-dataplane obs file open failed: {e}");
                hub.export_errors.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    });

    #[cfg(unix)]
    let syslog_sock = if syslog {
        match UnixDatagram::unbound() {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("exyonq-dataplane obs syslog socket failed: {e}");
                hub.export_errors.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    } else {
        None
    };

    #[cfg(unix)]
    let journal_sock = if journald {
        match UnixDatagram::unbound() {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("exyonq-dataplane obs journald socket failed: {e}");
                hub.export_errors.fetch_add(1, Ordering::Relaxed);
                None
            }
        }
    } else {
        None
    };

    // Cap061: configured ≠ opened. A sink that failed open still blocks "exported".
    let file_configured = file_path.is_some();
    #[cfg(unix)]
    let syslog_configured = syslog;
    #[cfg(unix)]
    let journald_configured = journald;

    if let Some(addr) = metrics_listen {
        let hub_scrape = Arc::clone(&hub);
        let stop_scrape = Arc::clone(&stop);
        if let Err(e) = thread::Builder::new()
            .name("cfd-obs-metrics".into())
            .spawn(move || metrics_serve_loop(addr, hub_scrape, stop_scrape))
        {
            eprintln!("exyonq-dataplane obs metrics thread spawn failed: {e}");
            hub.export_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    while !stop.load(Ordering::Relaxed) {
        let batch = hub.drain_events(256);
        for rec in &batch {
            hub.events_export_attempts.fetch_add(1, Ordering::Relaxed);
            let line = rec.to_json_line();
            let mut any_sink = false;
            let mut all_ok = true;
            if console {
                any_sink = true;
                if std::io::stdout().write_all(line.as_bytes()).is_err() {
                    all_ok = false;
                    hub.export_errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            if file_configured {
                any_sink = true;
                match file_writer.as_mut() {
                    Some(w) => {
                        if w.write_all(line.as_bytes()).is_err() {
                            all_ok = false;
                            hub.export_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        // Configured but failed to open — never count as exported.
                        all_ok = false;
                        hub.export_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            #[cfg(unix)]
            if syslog_configured {
                any_sink = true;
                match syslog_sock.as_ref() {
                    Some(sock) => {
                        let msg = format!("<134>exyonq-cfd: {}", line.trim_end());
                        if sock.send_to(msg.as_bytes(), "/dev/log").is_err() {
                            all_ok = false;
                            hub.export_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        all_ok = false;
                        hub.export_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            #[cfg(unix)]
            if journald_configured {
                any_sink = true;
                match journal_sock.as_ref() {
                    Some(sock) => {
                        let payload = format!(
                            "PRIORITY=6\nSYSLOG_IDENTIFIER=exyonq-cfd\nMESSAGE={}\n\n",
                            line.trim_end()
                        );
                        if sock
                            .send_to(payload.as_bytes(), "/run/systemd/journal/socket")
                            .is_err()
                        {
                            all_ok = false;
                            hub.export_errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        all_ok = false;
                        hub.export_errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            // Cap061: "exported" means every *configured* event sink accepted the record.
            // A sink that failed to open still blocks success. Metrics scrape is not an event sink.
            if any_sink && all_ok {
                hub.events_exported.fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Some(ref mut w) = file_writer {
            let _ = w.flush();
        }
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(ref mut w) = file_writer {
        let _ = w.flush();
    }
}

fn metrics_serve_loop(addr: String, hub: Arc<ObsHub>, stop: Arc<AtomicBool>) {
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("exyonq-dataplane obs metrics bind {addr} failed: {e}");
            hub.export_errors.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    let _ = listener.set_nonblocking(true);
    eprintln!("exyonq-dataplane obs metrics listening on {addr}");
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let hub = Arc::clone(&hub);
                let _ = thread::Builder::new()
                    .name("cfd-obs-scrape".into())
                    .spawn(move || serve_one_metrics(stream, &hub));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                hub.export_errors.fetch_add(1, Ordering::Relaxed);
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn serve_one_metrics(mut stream: TcpStream, hub: &ObsHub) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 1024];
    let _ = stream.read(&mut buf);
    let body = hub.render_openmetrics();
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/openmetrics-text; version=1.0.0; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn env_flag(name: &str) -> bool {
    matches!(
        env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}
