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
//! Shard-local FastCGI connection pool — no global hot lock.
//!
//! ```text
//! UNIX_SOCKET_TRUST_MODEL = SINGLE_TENANT_ADMIN_FS
//! ```
//! Absolute Unix paths only. Generation-scoped checkout; probe discard on
//! EOF / unexpected readable; checkin only after clean END_REQUEST (no trailing).

use std::collections::HashMap;
use std::io::{self, Read};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use exyonq_cfd_gen::{CompiledFcgiPool, FcgiTransport};

/// Stream type for pooled FastCGI peers.
pub enum FcgiStream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Read for FcgiStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(buf),
            Self::Unix(s) => s.read(buf),
        }
    }
}

impl std::io::Write for FcgiStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(s) => std::io::Write::write(s, buf),
            Self::Unix(s) => std::io::Write::write(s, buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(s) => std::io::Write::flush(s),
            Self::Unix(s) => std::io::Write::flush(s),
        }
    }
}

impl FcgiStream {
    pub(crate) fn set_read_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_read_timeout(t),
            Self::Unix(s) => s.set_read_timeout(t),
        }
    }

    pub(crate) fn set_write_timeout(&self, t: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_write_timeout(t),
            Self::Unix(s) => s.set_write_timeout(t),
        }
    }

    fn shutdown_both(self) {
        match self {
            Self::Tcp(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
            Self::Unix(s) => {
                let _ = s.shutdown(Shutdown::Both);
            }
        }
    }
}

struct IdleConn {
    stream: FcgiStream,
    idle_since: Instant,
    generation: u64,
}

/// Per-pool idle list (shard-local).
struct PoolIdle {
    max_connections: usize,
    idle_ttl: Duration,
    idle: Vec<IdleConn>,
}

/// Shard-local map of FastCGI pools keyed by pool id.
#[derive(Default)]
pub struct FcgiPoolMap {
    by_id: HashMap<u32, PoolIdle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Clean,
    Dirty,
}

impl FcgiPoolMap {
    /// Bind `pool_id` to the presented [`CompiledFcgiPool`] policy (ADR-040 / P6D-L1-001).
    ///
    /// Same `pool_id` across generation reload does **not** freeze first-seen limits:
    /// `idle_ttl` and `max_connections` are refreshed as one snapshot, then idle that
    /// violate the new TTL or exceed the new max are discarded immediately.
    /// Active connections are not in this map and are not terminated here.
    pub fn ensure_pool(&mut self, pool: &CompiledFcgiPool) {
        let max_connections = pool.max_connections.max(1) as usize;
        let idle_ttl = Duration::from_millis(u64::from(pool.idle_timeout_ms.max(1)));
        let entry = self.by_id.entry(pool.id).or_insert_with(|| PoolIdle {
            max_connections,
            idle_ttl,
            idle: Vec::new(),
        });
        let policy_changed = entry.max_connections != max_connections || entry.idle_ttl != idle_ttl;
        // Coherent snapshot — never leave new max with stale TTL (or vice versa).
        entry.max_connections = max_connections;
        entry.idle_ttl = idle_ttl;
        if !policy_changed {
            return;
        }
        let now = Instant::now();
        let ttl = entry.idle_ttl;
        let max = entry.max_connections;
        let mut kept = Vec::with_capacity(entry.idle.len().min(max));
        for c in entry.idle.drain(..) {
            if now.duration_since(c.idle_since) > ttl {
                Self::discard(c.stream);
                continue;
            }
            kept.push(c);
        }
        // Shrink: drop oldest idle first (front); take pops MRU from the end.
        while kept.len() > max {
            let c = kept.remove(0);
            Self::discard(c.stream);
        }
        entry.idle = kept;
    }

    #[cfg(test)]
    fn idle_len(&self, pool_id: u32) -> usize {
        self.by_id.get(&pool_id).map(|p| p.idle.len()).unwrap_or(0)
    }

    #[cfg(test)]
    fn pool_count(&self) -> usize {
        self.by_id.len()
    }

    #[cfg(test)]
    fn effective_limits(&self, pool_id: u32) -> Option<(usize, Duration)> {
        self.by_id
            .get(&pool_id)
            .map(|p| (p.max_connections, p.idle_ttl))
    }

    pub fn discard(stream: FcgiStream) {
        stream.shutdown_both();
    }

    /// Non-blocking one-byte probe. Any consumed byte/EOF/error ⇒ Dirty.
    ///
    /// Side effect: may leave `read_timeout` at 1ms. Callers that enter
    /// `exchange_once` MUST run [`Self::apply_timeouts`] after this probe
    /// (ADR-039 `POST_ACTIVATION_TIMEOUT_RESTORE`).
    pub fn probe(stream: &mut FcgiStream) -> Probe {
        let _ = stream.set_read_timeout(Some(Duration::from_millis(1)));
        let mut buf = [0u8; 1];
        let r = match stream {
            FcgiStream::Tcp(s) => {
                let _ = s.set_nonblocking(true);
                let r = s.read(&mut buf);
                let _ = s.set_nonblocking(false);
                r
            }
            FcgiStream::Unix(s) => {
                let _ = s.set_nonblocking(true);
                let r = s.read(&mut buf);
                let _ = s.set_nonblocking(false);
                r
            }
        };
        match r {
            Ok(0) => Probe::Dirty,
            Ok(_) => Probe::Dirty,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Probe::Clean,
            Err(_) => Probe::Dirty,
        }
    }

    /// Final pooled→active validation immediately before `FCGI_BEGIN_REQUEST`
    /// (ADR-039 / FCGI-RACE-01). Same semantics as [`Self::probe`].
    #[inline]
    pub fn activate_before_begin(stream: &mut FcgiStream) -> Probe {
        Self::probe(stream)
    }

    /// Connect using pool transport. Unix paths must be absolute (SG-FCGI-06).
    pub fn connect(pool: &CompiledFcgiPool) -> io::Result<FcgiStream> {
        let connect_to = Duration::from_millis(u64::from(pool.connect_timeout_ms.max(1)));
        match &pool.transport {
            FcgiTransport::Tcp(addr) => {
                let s = TcpStream::connect_timeout(addr, connect_to)?;
                s.set_nodelay(true)?;
                Ok(FcgiStream::Tcp(s))
            }
            FcgiTransport::Unix(path) => {
                validate_absolute_unix(path)?;
                // std UnixStream has no connect_timeout; best-effort connect.
                let s = UnixStream::connect(path)?;
                Ok(FcgiStream::Unix(s))
            }
        }
    }

    pub fn apply_timeouts(stream: &FcgiStream, pool: &CompiledFcgiPool) -> io::Result<()> {
        let r = Duration::from_millis(u64::from(pool.read_timeout_ms.max(1)));
        let w = Duration::from_millis(u64::from(pool.write_timeout_ms.max(1)));
        stream.set_read_timeout(Some(r))?;
        stream.set_write_timeout(Some(w))?;
        Ok(())
    }

    /// Checkout with generation + probe. Returns `(stream, pool_hit)`.
    pub fn take(&mut self, pool: &CompiledFcgiPool, generation: u64) -> Option<(FcgiStream, bool)> {
        self.ensure_pool(pool);
        let idle = self.by_id.get_mut(&pool.id)?;
        let now = Instant::now();
        while let Some(mut c) = idle.idle.pop() {
            if now.duration_since(c.idle_since) > idle.idle_ttl {
                Self::discard(c.stream);
                continue;
            }
            if c.generation != generation {
                Self::discard(c.stream);
                continue;
            }
            if Self::probe(&mut c.stream) != Probe::Clean {
                Self::discard(c.stream);
                continue;
            }
            return Some((c.stream, true));
        }
        None
    }

    /// Checkin only when clean (no unexpected readable). Poison ⇒ discard.
    pub fn put(&mut self, pool: &CompiledFcgiPool, mut stream: FcgiStream, generation: u64) {
        self.ensure_pool(pool);
        if Self::probe(&mut stream) != Probe::Clean {
            Self::discard(stream);
            return;
        }
        let idle = self.by_id.get_mut(&pool.id).expect("ensure_pool");
        if idle.idle.len() >= idle.max_connections {
            Self::discard(stream);
            return;
        }
        idle.idle.push(IdleConn {
            stream,
            idle_since: Instant::now(),
            generation,
        });
    }

    pub fn flush_generation(&mut self, keep_generation: u64) {
        for idle in self.by_id.values_mut() {
            let mut kept = Vec::new();
            for c in idle.idle.drain(..) {
                if c.generation == keep_generation {
                    kept.push(c);
                } else {
                    Self::discard(c.stream);
                }
            }
            idle.idle = kept;
        }
    }
}

fn validate_absolute_unix(path: &Path) -> io::Result<()> {
    // UNIX_SOCKET_TRUST_MODEL = SINGLE_TENANT_ADMIN_FS
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "fcgi unix path must be absolute (SINGLE_TENANT_ADMIN_FS)",
        ));
    }
    if path.to_string_lossy().contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "fcgi unix path contains NUL",
        ));
    }
    let _ = PathBuf::from(path);
    Ok(())
}

/// Helper for tests: TCP connect with timeout.
#[allow(dead_code)]
pub fn connect_tcp(addr: SocketAddr, timeout: Duration) -> io::Result<FcgiStream> {
    let s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_nodelay(true)?;
    Ok(FcgiStream::Tcp(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use exyonq_cfd_gen::{CompiledFcgiPool, FcgiTransport};
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    fn tcp_pool(id: u32, addr: SocketAddr, max: u32, idle_ms: u32) -> CompiledFcgiPool {
        CompiledFcgiPool {
            id,
            transport: FcgiTransport::Tcp(addr),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: max,
            idle_timeout_ms: idle_ms,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 5_000,
            write_timeout_ms: 5_000,
            total_timeout_ms: 10_000,
            directory_index: Vec::new(),
            front_controller: None,
        }
    }

    fn accept_n(listener: TcpListener, n: usize) {
        thread::spawn(move || {
            let mut hold = Vec::with_capacity(n);
            for _ in 0..n {
                if let Ok((s, _)) = listener.accept() {
                    hold.push(s);
                }
            }
            // Keep peers open so pool probe stays Clean for the test duration.
            thread::sleep(Duration::from_secs(30));
            drop(hold);
        });
    }

    #[test]
    fn same_pool_id_same_config_preserves_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 1);
        let cfg = tcp_pool(1, addr, 4, 60_000);
        let mut map = FcgiPoolMap::default();
        let stream = FcgiPoolMap::connect(&cfg).unwrap();
        map.put(&cfg, stream, 1);
        map.ensure_pool(&cfg);
        assert_eq!(map.idle_len(1), 1);
        assert_eq!(
            map.effective_limits(1),
            Some((4, Duration::from_millis(60_000)))
        );
        assert!(map.take(&cfg, 1).is_some());
    }

    #[test]
    fn same_pool_id_ttl_decrease_evicts_stale_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 1);
        let long = tcp_pool(7, addr, 4, 60_000);
        let mut map = FcgiPoolMap::default();
        let stream = FcgiPoolMap::connect(&long).unwrap();
        map.put(&long, stream, 1);
        assert_eq!(map.idle_len(7), 1);
        thread::sleep(Duration::from_millis(40));
        // Proven defect case: republish shorter TTL for SAME pool_id.
        let short = tcp_pool(7, addr, 4, 10);
        map.ensure_pool(&short);
        assert_eq!(
            map.effective_limits(7),
            Some((4, Duration::from_millis(10)))
        );
        assert_eq!(
            map.idle_len(7),
            0,
            "idle older than new TTL must not survive ensure_pool refresh"
        );
        assert!(map.take(&short, 1).is_none());
    }

    #[test]
    fn same_pool_id_ttl_increase_keeps_fresh_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 1);
        let short = tcp_pool(8, addr, 4, 50);
        let mut map = FcgiPoolMap::default();
        let stream = FcgiPoolMap::connect(&short).unwrap();
        map.put(&short, stream, 1);
        thread::sleep(Duration::from_millis(20));
        let long = tcp_pool(8, addr, 4, 60_000);
        map.ensure_pool(&long);
        assert_eq!(map.idle_len(8), 1);
        assert!(map.take(&long, 1).is_some());
    }

    #[test]
    fn same_pool_id_max_increase_allows_more_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 3);
        let small = tcp_pool(9, addr, 1, 60_000);
        let mut map = FcgiPoolMap::default();
        map.put(&small, FcgiPoolMap::connect(&small).unwrap(), 1);
        map.put(&small, FcgiPoolMap::connect(&small).unwrap(), 1); // discarded under max=1
        assert_eq!(map.idle_len(9), 1);
        let big = tcp_pool(9, addr, 4, 60_000);
        map.ensure_pool(&big);
        map.put(&big, FcgiPoolMap::connect(&big).unwrap(), 1);
        assert_eq!(
            map.idle_len(9),
            2,
            "max increase must allow additional idle"
        );
    }

    #[test]
    fn same_pool_id_max_decrease_trims_excess_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 3);
        let wide = tcp_pool(10, addr, 4, 60_000);
        let mut map = FcgiPoolMap::default();
        for _ in 0..3 {
            let s = FcgiPoolMap::connect(&wide).unwrap();
            map.put(&wide, s, 1);
        }
        assert_eq!(map.idle_len(10), 3);
        let narrow = tcp_pool(10, addr, 1, 60_000);
        map.ensure_pool(&narrow);
        assert_eq!(map.idle_len(10), 1);
        assert_eq!(
            map.effective_limits(10),
            Some((1, Duration::from_millis(60_000)))
        );
        assert!(map.take(&narrow, 1).is_some());
        assert!(map.take(&narrow, 1).is_none());
    }

    #[test]
    fn different_pool_id_creates_separate_entries() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 2);
        let a = tcp_pool(1, addr, 2, 60_000);
        let b = tcp_pool(2, addr, 2, 60_000);
        let mut map = FcgiPoolMap::default();
        map.put(&a, FcgiPoolMap::connect(&a).unwrap(), 1);
        map.put(&b, FcgiPoolMap::connect(&b).unwrap(), 1);
        assert_eq!(map.pool_count(), 2);
        assert_eq!(map.idle_len(1), 1);
        assert_eq!(map.idle_len(2), 1);
    }

    #[test]
    fn repeated_same_pool_id_reload_does_not_grow_map() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 1);
        let mut map = FcgiPoolMap::default();
        for i in 0..20 {
            let cfg = tcp_pool(42, addr, 2 + (i % 3), 1_000 + i * 10);
            map.ensure_pool(&cfg);
        }
        assert_eq!(map.pool_count(), 1);
    }

    #[test]
    fn reload_during_put_enforces_new_max() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 2);
        let wide = tcp_pool(11, addr, 4, 60_000);
        let mut map = FcgiPoolMap::default();
        map.put(&wide, FcgiPoolMap::connect(&wide).unwrap(), 1);
        let narrow = tcp_pool(11, addr, 1, 60_000);
        map.ensure_pool(&narrow);
        // Second put must discard under new max=1.
        map.put(&narrow, FcgiPoolMap::connect(&narrow).unwrap(), 1);
        assert_eq!(map.idle_len(11), 1);
    }

    #[test]
    fn reload_during_take_sees_new_ttl() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 1);
        let long = tcp_pool(12, addr, 4, 60_000);
        let mut map = FcgiPoolMap::default();
        map.put(&long, FcgiPoolMap::connect(&long).unwrap(), 1);
        thread::sleep(Duration::from_millis(40));
        let short = tcp_pool(12, addr, 4, 10);
        // take calls ensure_pool → must evict before checkout.
        assert!(map.take(&short, 1).is_none());
        assert_eq!(map.idle_len(12), 0);
    }

    #[test]
    fn shrink_below_active_count_eventual_bound_on_put() {
        // Simulate active out of pool: take without put, shrink max, then put back.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        accept_n(listener, 3);
        let wide = tcp_pool(13, addr, 4, 60_000);
        let mut map = FcgiPoolMap::default();
        map.put(&wide, FcgiPoolMap::connect(&wide).unwrap(), 1);
        map.put(&wide, FcgiPoolMap::connect(&wide).unwrap(), 1);
        let (active, _) = map.take(&wide, 1).unwrap();
        assert_eq!(map.idle_len(13), 1);
        let narrow = tcp_pool(13, addr, 1, 60_000);
        map.ensure_pool(&narrow);
        assert_eq!(map.idle_len(13), 1); // one idle kept; active still out
                                         // Put active back: pool already at max → discard (eventual bound).
        map.put(&narrow, active, 1);
        assert_eq!(map.idle_len(13), 1);
    }

    #[test]
    fn generation_mismatch_discards_idle() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        // Hold accepted peer open — `let _ = accept()` would drop/FIN immediately
        // and turn later probes into DirtyEOF under parallel scheduling.
        let _ = thread::spawn(move || {
            let Ok((peer, _)) = listener.accept() else {
                return;
            };
            thread::sleep(Duration::from_secs(2));
            drop(peer);
        });
        let pool_cfg = CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Tcp(addr),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: 4,
            idle_timeout_ms: 60_000,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 5_000,
            write_timeout_ms: 5_000,
            total_timeout_ms: 10_000,
            directory_index: Vec::new(),
            front_controller: None,
        };
        let mut map = FcgiPoolMap::default();
        let stream = FcgiPoolMap::connect(&pool_cfg).unwrap();
        map.put(&pool_cfg, stream, 1);
        map.flush_generation(2);
        assert!(map.take(&pool_cfg, 2).is_none());
    }

    #[test]
    fn absolute_unix_required() {
        let pool = CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Unix(PathBuf::from("relative.sock")),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: 1,
            idle_timeout_ms: 1,
            connect_timeout_ms: 1,
            read_timeout_ms: 1,
            write_timeout_ms: 1,
            total_timeout_ms: 1,
            directory_index: Vec::new(),
            front_controller: None,
        };
        assert!(FcgiPoolMap::connect(&pool).is_err());
    }

    /// Peer closes after accept; checkout probe Clean is impossible on closed —
    /// inject EOF between a successful Clean probe and activate (RACE-01).
    #[test]
    fn activate_detects_eof_after_checkout_clean() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        thread::spawn(move || {
            let (peer, _) = listener.accept().unwrap();
            // Hold open until test signals close.
            let _ = rx.recv();
            drop(peer); // EOF to client
        });
        let mut stream = connect_tcp(addr, Duration::from_secs(2)).unwrap();
        assert_eq!(FcgiPoolMap::probe(&mut stream), Probe::Clean);
        tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            FcgiPoolMap::activate_before_begin(&mut stream),
            Probe::Dirty,
            "EOF after checkout must fail final activation"
        );
    }

    #[test]
    fn activate_detects_pending_byte_after_checkout_clean() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let _ = rx.recv();
            let _ = peer.write_all(&[0x01]); // unexpected FastCGI-ish byte
            let _ = rx.recv(); // hold until test done
        });
        let mut stream = connect_tcp(addr, Duration::from_secs(2)).unwrap();
        assert_eq!(FcgiPoolMap::probe(&mut stream), Probe::Clean);
        tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            FcgiPoolMap::activate_before_begin(&mut stream),
            Probe::Dirty,
            "pending unexpected byte must fail final activation"
        );
        // Second signal to release peer (ignore send err if already joined).
        let _ = tx.send(());
    }

    #[test]
    fn activate_detects_partial_fcgi_header_after_checkout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let _ = rx.recv();
            // Partial FastCGI record header (version+type only).
            let _ = peer.write_all(&[0x01, 0x06]);
            let _ = rx.recv();
        });
        let mut stream = connect_tcp(addr, Duration::from_secs(2)).unwrap();
        assert_eq!(FcgiPoolMap::probe(&mut stream), Probe::Clean);
        tx.send(()).unwrap();
        thread::sleep(Duration::from_millis(50));
        assert_eq!(
            FcgiPoolMap::activate_before_begin(&mut stream),
            Probe::Dirty,
            "partial FastCGI header must fail final activation"
        );
        let _ = tx.send(());
    }

    #[test]
    fn apply_timeouts_after_activate_restores_pool_read_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        // Keep the accepted peer alive for the whole activate/timeout check.
        // `let _ = accept()` drops the peer immediately → FIN/EOF → Probe::Dirty
        // under parallel workspace load (TOCTOU with activate_before_begin).
        thread::spawn(move || {
            let Ok((peer, _)) = listener.accept() else {
                return;
            };
            thread::sleep(Duration::from_secs(2));
            drop(peer);
        });
        let pool = CompiledFcgiPool {
            id: 1,
            transport: FcgiTransport::Tcp(addr),
            document_root: "/var/www".into(),
            script_suffix: String::new(),
            max_connections: 1,
            idle_timeout_ms: 60_000,
            connect_timeout_ms: 2_000,
            read_timeout_ms: 5_000,
            write_timeout_ms: 5_000,
            total_timeout_ms: 10_000,
            directory_index: Vec::new(),
            front_controller: None,
        };
        let mut stream = FcgiPoolMap::connect(&pool).unwrap();
        assert_eq!(
            FcgiPoolMap::activate_before_begin(&mut stream),
            Probe::Clean
        );
        FcgiPoolMap::apply_timeouts(&stream, &pool).unwrap();
        match &stream {
            FcgiStream::Tcp(s) => {
                let t = s.read_timeout().unwrap();
                assert_eq!(t, Some(Duration::from_millis(5_000)));
            }
            FcgiStream::Unix(_) => panic!("expected tcp"),
        }
    }
}
