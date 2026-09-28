//! Causal matrix for TrailingData — real TCP sockets, variants A/B/D/E/F/G.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use exyonq_fastcgi_wire::{
    encode_record_frame, exchange_once, parse_record, ExchangeError, MinForwardRequest,
    FCGI_END_REQUEST, FCGI_PARAMS, FCGI_REQUEST_COMPLETE, FCGI_STDIN, FCGI_STDOUT,
    RECORD_HEADER_LEN,
};

fn wait_empty_stdin(sock: &mut std::net::TcpStream) {
    let mut buf = Vec::new();
    let mut scratch = [0u8; 4096];
    let mut saw = false;
    while !saw {
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
                saw = true;
                break;
            }
        }
    }
}

fn stdout_and_end() -> (Vec<u8>, Vec<u8>) {
    let stdout = encode_record_frame(
        1,
        FCGI_STDOUT,
        b"Status: 200\r\nContent-Type: text/plain\r\n\r\nhello-fcgi",
    )
    .unwrap();
    let mut end = [0u8; 8];
    end[4] = FCGI_REQUEST_COMPLETE;
    let end_frame = encode_record_frame(1, FCGI_END_REQUEST, &end).unwrap();
    (stdout, end_frame)
}

#[derive(Clone, Copy)]
enum Variant {
    /// A: END_REQUEST + TRAIL in one write (co-buffered guarantee when TCP coalesces)
    Cobuffered,
    /// B: separate writes, no delay (legacy race)
    SeparateNoDelay,
    /// B2: END then sleep then TRAIL — late trailing after completion window
    SeparateDelayedTrail,
    /// D: cobuffered trailing then SHUT_WR
    TrailThenShutdownWr,
    /// E: cobuffered with KEEP_CONN (exchange keep_conn=true)
    KeepConnCobuffered,
    /// F: clean no trail
    Clean,
}

fn run_variant(v: Variant) -> Result<(), ExchangeError> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        wait_empty_stdin(&mut sock);
        let (stdout, end_frame) = stdout_and_end();
        match v {
            Variant::Cobuffered | Variant::KeepConnCobuffered => {
                let mut one = stdout;
                one.extend_from_slice(&end_frame);
                one.extend_from_slice(b"TRAIL");
                sock.write_all(&one).unwrap();
            }
            Variant::SeparateNoDelay => {
                sock.write_all(&stdout).unwrap();
                sock.write_all(&end_frame).unwrap();
                sock.write_all(b"TRAIL").unwrap();
            }
            Variant::SeparateDelayedTrail => {
                sock.write_all(&stdout).unwrap();
                sock.write_all(&end_frame).unwrap();
                thread::sleep(Duration::from_millis(50));
                sock.write_all(b"TRAIL").unwrap();
            }
            Variant::TrailThenShutdownWr => {
                let mut one = stdout;
                one.extend_from_slice(&end_frame);
                one.extend_from_slice(b"TRAIL");
                sock.write_all(&one).unwrap();
                let _ = sock.shutdown(std::net::Shutdown::Write);
            }
            Variant::Clean => {
                sock.write_all(&stdout).unwrap();
                sock.write_all(&end_frame).unwrap();
            }
        }
    });

    let req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
    let params = req.to_fcgi_params().unwrap();
    let mut stream = std::net::TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let attempt = exchange_once(&mut stream, 1, true, &params, &[]);
    let _ = h.join();
    attempt.result.map(|_| ())
}

fn classify(r: Result<(), ExchangeError>) -> &'static str {
    match r {
        Ok(()) => "Ok",
        Err(ExchangeError::TrailingData) => "TrailingData",
        Err(_) => "OtherErr",
    }
}

#[test]
fn causal_matrix_twenty_reps() {
    let variants = [
        ("A_COBUFFERED", Variant::Cobuffered),
        ("B_SEPARATE", Variant::SeparateNoDelay),
        ("B2_DELAYED_TRAIL", Variant::SeparateDelayedTrail),
        ("D_SHUTWR", Variant::TrailThenShutdownWr),
        ("E_KEEP_COBUF", Variant::KeepConnCobuffered),
        ("F_CLEAN", Variant::Clean),
    ];
    for (name, v) in variants {
        let mut ok = 0;
        let mut td = 0;
        let mut other = 0;
        for _ in 0..20 {
            match classify(run_variant(v)) {
                "Ok" => ok += 1,
                "TrailingData" => td += 1,
                _ => other += 1,
            }
        }
        eprintln!("{name}: Ok={ok} TrailingData={td} Other={other}");
        match name {
            "A_COBUFFERED" | "D_SHUTWR" | "E_KEEP_COBUF" => {
                assert_eq!(td, 20, "{name} must be deterministic TrailingData");
                assert_eq!(ok, 0);
            }
            "F_CLEAN" => {
                assert_eq!(ok, 20, "clean must be Ok");
                assert_eq!(td, 0);
            }
            "B_SEPARATE" => {
                assert_eq!(ok + td + other, 20);
            }
            "B2_DELAYED_TRAIL" => {
                // Late trail after END is outside the completion buffer check.
                assert_eq!(
                    ok, 20,
                    "delayed trail must complete as Ok (not wait for late bytes)"
                );
                assert_eq!(td, 0);
            }
            _ => {}
        }
    }
}

#[test]
fn causal_negative_oracle_cobuffered_not_ok() {
    let r = run_variant(Variant::Cobuffered);
    assert_eq!(r, Err(ExchangeError::TrailingData));
}
