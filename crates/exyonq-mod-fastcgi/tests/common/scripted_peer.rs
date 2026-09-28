//! In-process scripted FastCGI peer for wire tests — no Docker, no php-fpm.

#![allow(dead_code)]

use std::sync::atomic::{AtomicU64, Ordering};

static PEER_BIND_SEQ: AtomicU64 = AtomicU64::new(0);

use exyonq_mod_fastcgi::{
    encode_record_frame, parse_record, FastcgiRecordTransport, ScriptedFpmConfig,
    ScriptedFpmTransport, END_REQUEST_BODY_LEN, FCGI_END_REQUEST, FCGI_PARAMS,
    FCGI_REQUEST_COMPLETE, FCGI_STDIN,
};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Scripted peer behavior modes for failure injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerMode {
    Normal,
    DropAfterParams,
    SendGarbage,
    OmitEndRequest,
    SlowRead,
}

#[derive(Debug, Clone)]
pub struct PeerConfig {
    pub request_id: u16,
    pub stdout_body: Vec<u8>,
    pub app_status: u32,
    pub protocol_status: u8,
    pub mode: PeerMode,
    pub slow_delay: Duration,
}

impl Default for PeerConfig {
    fn default() -> Self {
        Self {
            request_id: 1,
            stdout_body: b"Content-Type: text/plain\r\n\r\nOK".to_vec(),
            app_status: 0,
            protocol_status: FCGI_REQUEST_COMPLETE,
            mode: PeerMode::Normal,
            slow_delay: Duration::from_millis(50),
        }
    }
}

/// Spawn unix scripted peer; returns socket path and join handle.
pub fn spawn_unix_peer(config: PeerConfig) -> (PathBuf, thread::JoinHandle<()>) {
    let dir = std::env::temp_dir().join(format!("exyonq-fcgi-peer-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (path, listener) = bind_unix_listener(&dir);
    let cfg = config;
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            serve_connection(&mut stream, &cfg);
        }
    });
    (path, handle)
}

fn bind_unix_listener(dir: &Path) -> (PathBuf, UnixListener) {
    for attempt in 0..32 {
        let path = dir.join(format!(
            "peer-{}-{}-{}-{}.sock",
            std::process::id(),
            PEER_BIND_SEQ.fetch_add(1, Ordering::Relaxed),
            rand_suffix(),
            attempt
        ));
        let _ = std::fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => return (path, listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => panic!("bind unix peer: {e}"),
        }
    }
    panic!("bind unix peer: exhausted retries");
}

use std::os::unix::net::UnixListener;

/// Spawn TCP loopback peer; returns bound port and join handle.
pub fn spawn_tcp_peer(config: PeerConfig) -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind tcp peer");
    let port = listener.local_addr().expect("local addr").port();
    let cfg = config;
    let handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            serve_connection(&mut stream, &cfg);
        }
    });
    thread::sleep(Duration::from_millis(20));
    (port, handle)
}

fn rand_suffix() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

fn serve_connection(stream: &mut dyn ReadWrite, config: &PeerConfig) {
    let peer = ScriptedFpmTransport::new(ScriptedFpmConfig {
        request_id: config.request_id,
        stdout_body: config.stdout_body.clone(),
        app_status: config.app_status,
        protocol_status: config.protocol_status,
    });

    let mut buffer = Vec::new();
    let mut scratch = [0u8; 4096];
    let mut request_complete = false;

    loop {
        if request_complete {
            break;
        }

        match stream.read(&mut scratch) {
            Ok(0) => break,
            Ok(n) => buffer.extend_from_slice(&scratch[..n]),
            Err(_) => break,
        }

        while buffer.len() >= 8 {
            let frame_len = match frame_len_at(&buffer) {
                Ok(len) => len,
                Err(_) => break,
            };
            if buffer.len() < frame_len {
                break;
            }
            let frame = buffer[..frame_len].to_vec();
            buffer.drain(..frame_len);

            if config.mode == PeerMode::SendGarbage {
                let _ = stream.write_all(b"\xff\xfe\xfd");
                return;
            }

            if let Ok(parsed) = parse_record(&frame) {
                if config.mode == PeerMode::DropAfterParams
                    && parsed.header.record_type == FCGI_PARAMS
                    && parsed.content.is_empty()
                {
                    return;
                }
                if parsed.header.record_type == FCGI_STDIN && parsed.content.is_empty() {
                    request_complete = true;
                }
            }

            if peer.submit_frame(&frame).is_err() {
                return;
            }

            if request_complete {
                if config.mode == PeerMode::SlowRead {
                    thread::sleep(config.slow_delay);
                }
                if let Ok(frames) = peer.take_response() {
                    if config.mode == PeerMode::OmitEndRequest {
                        for f in &frames {
                            if f.get(1) != Some(&FCGI_END_REQUEST) {
                                let _ = stream.write_all(f);
                            }
                        }
                    } else {
                        for f in &frames {
                            let _ = stream.write_all(f);
                        }
                    }
                }
                return;
            }
        }
    }
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn frame_len_at(buf: &[u8]) -> Result<usize, ()> {
    if buf.len() < 8 {
        return Err(());
    }
    let content = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let pad = buf[6] as usize;
    Ok(8 + content + pad)
}

/// Build standalone END_REQUEST frame (test helper).
#[allow(dead_code)]
pub fn end_request_frame(request_id: u16, app_status: u32, protocol_status: u8) -> Vec<u8> {
    let mut body = [0u8; END_REQUEST_BODY_LEN];
    body[0..4].copy_from_slice(&app_status.to_be_bytes());
    body[4] = protocol_status;
    encode_record_frame(request_id, FCGI_END_REQUEST, &body).expect("end request")
}

/// Wait for peer thread (ignore panics in tests).
pub fn join_peer(handle: thread::JoinHandle<()>) {
    let _ = handle.join();
}

/// Remove unix socket path after test.
pub fn cleanup_unix(path: &Path) {
    let _ = std::fs::remove_file(path);
}
