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
//! Phase 6B FastCGI path tests — scripted peer (non-terminal; not PHP-FPM proof).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use exyonq_cfd_gen::{
    BackendKind, BackendTarget, CompiledFcgiPool, CompiledRoute, FcgiTransport, RouteTable,
};
use exyonq_fastcgi_wire::{
    encode_record_frame, exchange_once, parse_record, CommitStage, ExchangeError,
    MinForwardRequest, ParamsError, FCGI_END_REQUEST, FCGI_PARAMS, FCGI_REQUEST_COMPLETE,
    FCGI_STDIN, FCGI_STDOUT, RECORD_HEADER_LEN,
};

fn scripted_fcgi_peer(trailing: bool) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut scratch = [0u8; 4096];
        let mut saw_empty_stdin = false;
        while !saw_empty_stdin {
            let n = match sock.read(&mut scratch) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            buf.extend_from_slice(&scratch[..n]);
            while buf.len() >= RECORD_HEADER_LEN {
                let cl = u16::from_be_bytes([buf[4], buf[5]]) as usize;
                let pad = buf[6] as usize;
                let flen = RECORD_HEADER_LEN + cl + pad;
                if buf.len() < flen {
                    break;
                }
                let frame = buf[..flen].to_vec();
                buf.drain(..flen);
                let parsed = parse_record(&frame).unwrap();
                let _ = FCGI_PARAMS;
                if parsed.header.record_type == FCGI_STDIN && parsed.content.is_empty() {
                    saw_empty_stdin = true;
                    break;
                }
            }
        }
        let stdout = encode_record_frame(
            1,
            FCGI_STDOUT,
            b"Status: 200\r\nContent-Type: text/plain\r\n\r\nhello-fcgi",
        )
        .unwrap();
        let mut end = [0u8; 8];
        end[4] = FCGI_REQUEST_COMPLETE;
        let end_frame = encode_record_frame(1, FCGI_END_REQUEST, &end).unwrap();
        // Co-buffer END_REQUEST (+ optional trailing) into one write so the product
        // trailing check observes leftover bytes in the same read buffer. Separate
        // write_all(END) then write_all(TRAIL) races the consumer and intermittently
        // yields Ok (harness race — not a parser miss of co-buffered trailing).
        let mut out = stdout;
        out.extend_from_slice(&end_frame);
        if trailing {
            out.extend_from_slice(b"TRAIL");
        }
        sock.write_all(&out).unwrap();
    });
    (addr, h)
}

#[test]
fn get_zero_body_against_scripted_peer() {
    let (addr, h) = scripted_fcgi_peer(false);
    let req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
    let params = req.to_fcgi_params().unwrap();
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    let attempt = exchange_once(&mut stream, 1, true, &params, &[]);
    let resp = attempt.result.expect("exchange ok");
    assert!(resp.stdout.windows(10).any(|w| w == b"hello-fcgi"));
    h.join().unwrap();
}

#[test]
fn trailing_data_forces_discard_error() {
    let (addr, h) = scripted_fcgi_peer(true);
    let req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
    let params = req.to_fcgi_params().unwrap();
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    let attempt = exchange_once(&mut stream, 1, true, &params, &[]);
    assert_eq!(attempt.result.unwrap_err(), ExchangeError::TrailingData);
    h.join().unwrap();
}

#[test]
fn content_length_mismatch_reject() {
    let mut req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
    req.stdin = b"abcd".to_vec();
    req.content_type = Some("text/plain".into());
    req.declared_content_length = Some(1);
    assert!(matches!(
        req.to_fcgi_params(),
        Err(ParamsError::ContentLengthMismatch { .. })
    ));
}

#[test]
fn not_started_retry_gate() {
    assert!(CommitStage::NotStarted.allows_safe_retry());
    assert!(!CommitStage::BeginCommitted.allows_safe_retry());
}

#[test]
fn route_table_fcgi_lookup_and_cfdrt003() {
    let addr: std::net::SocketAddr = "127.0.0.1:9000".parse().unwrap();
    let t = RouteTable {
        upstreams: Vec::new(),
        fcgi_pools: vec![CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Tcp(addr),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: 4,
            idle_timeout_ms: 60_000,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 30_000,
            write_timeout_ms: 30_000,
            total_timeout_ms: 60_000,
            directory_index: Vec::new(),
            front_controller: None,
        }],
        static_policies: Vec::new(),
        routes: vec![CompiledRoute {
            route_id: 1,
            host: None,
            path: "/app".into(),
            backend_kind: BackendKind::Fastcgi,
            backend_id: 1,
        }],
    };
    let bytes = t.encode().unwrap();
    assert_eq!(&bytes[0..8], b"CFDRT005");
    let d = RouteTable::decode(&bytes).unwrap();
    match d.lookup("/app/x.php", None).unwrap().1 {
        BackendTarget::Fastcgi(p) => assert_eq!(p.id, 1),
        _ => panic!("expected fcgi"),
    }
}
