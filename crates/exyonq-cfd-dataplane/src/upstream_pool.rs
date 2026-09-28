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

//! Shard-local upstream keepalive pool (ADR-021 Hybrid F).
//!
//! Idle sockets stay registered READABLE on the shard Poll. Any readiness,
//! trailing byte, EOF, generation mismatch, or probe ambiguity ⇒ discard.
//! No cross-shard sharing. No Tokio/Hyper.

use std::collections::HashMap;
use std::io::{self, Read};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mio::net::TcpStream;
use mio::{Interest, Registry, Token};

const MAX_IDLE_PER_TARGET: usize = 32;
const IDLE_TTL: Duration = Duration::from_secs(60);

struct IdleConn {
    stream: TcpStream,
    idle_since: Instant,
    generation: u64,
    token: Token,
}

#[derive(Default)]
pub struct UpstreamPool {
    by_addr: HashMap<SocketAddr, Vec<IdleConn>>,
    by_token: HashMap<Token, SocketAddr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Clean,
    Dirty,
}

impl UpstreamPool {
    pub fn is_idle_token(&self, token: Token) -> bool {
        self.by_token.contains_key(&token)
    }

    /// Non-blocking one-byte probe. Any consumed byte/EOF/error ⇒ Dirty (no pushback).
    pub fn probe(stream: &mut TcpStream) -> Probe {
        let mut buf = [0u8; 1];
        match stream.read(&mut buf) {
            Ok(0) => Probe::Dirty,
            Ok(_) => Probe::Dirty,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Probe::Clean,
            Err(_) => Probe::Dirty,
        }
    }

    pub fn discard(stream: TcpStream) {
        drop(stream);
    }

    pub fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
        let std = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        std.set_nonblocking(true)?;
        Ok(TcpStream::from_std(std))
    }

    /// Checkin after RESPONSE_COMPLETE. Registers idle READABLE before take-eligible.
    pub fn put(
        &mut self,
        registry: &Registry,
        addr: SocketAddr,
        mut stream: TcpStream,
        generation: u64,
        next_token: &mut Token,
    ) {
        if Self::probe(&mut stream) != Probe::Clean {
            Self::discard(stream);
            return;
        }
        self.evict_expired(registry, addr);
        let len = self.by_addr.get(&addr).map_or(0, Vec::len);
        if len >= MAX_IDLE_PER_TARGET {
            Self::discard(stream);
            return;
        }

        let token = alloc_token(next_token);
        if registry
            .register(&mut stream, token, Interest::READABLE)
            .is_err()
        {
            Self::discard(stream);
            return;
        }
        // Close edge-triggered probe→register gap (ADR-021 §6).
        if Self::probe(&mut stream) != Probe::Clean {
            let _ = registry.deregister(&mut stream);
            Self::discard(stream);
            return;
        }

        self.by_token.insert(token, addr);
        self.by_addr.entry(addr).or_default().push(IdleConn {
            stream,
            idle_since: Instant::now(),
            generation,
            token,
        });
    }

    /// Checkout with generation + mandatory probe. Caller must hold READABLE until first write.
    pub fn take(
        &mut self,
        registry: &Registry,
        addr: SocketAddr,
        generation: u64,
    ) -> Option<TcpStream> {
        loop {
            self.evict_expired(registry, addr);
            let list = self.by_addr.get_mut(&addr)?;
            let Some(mut idle) = list.pop() else {
                self.by_addr.remove(&addr);
                return None;
            };
            self.by_token.remove(&idle.token);
            let _ = registry.deregister(&mut idle.stream);
            if list.is_empty() {
                self.by_addr.remove(&addr);
            }
            if idle.generation != generation {
                Self::discard(idle.stream);
                continue;
            }
            if Self::probe(&mut idle.stream) != Probe::Clean {
                Self::discard(idle.stream);
                continue;
            }
            return Some(idle.stream);
        }
    }

    /// Idle READABLE | ERROR | HUP ⇒ discard (peer write/close while pooled).
    pub fn on_idle_ready(&mut self, registry: &Registry, token: Token) {
        let Some(addr) = self.by_token.remove(&token) else {
            return;
        };
        let Some(list) = self.by_addr.get_mut(&addr) else {
            return;
        };
        if let Some(idx) = list.iter().position(|c| c.token == token) {
            let mut idle = list.swap_remove(idx);
            let _ = registry.deregister(&mut idle.stream);
            Self::discard(idle.stream);
        }
        if list.is_empty() {
            self.by_addr.remove(&addr);
        }
    }

    pub fn flush_all(&mut self, registry: &Registry) {
        let tokens: Vec<Token> = self.by_token.keys().copied().collect();
        for token in tokens {
            self.on_idle_ready(registry, token);
        }
        self.by_addr.clear();
        self.by_token.clear();
    }

    fn evict_expired(&mut self, registry: &Registry, addr: SocketAddr) {
        let now = Instant::now();
        let Some(list) = self.by_addr.get_mut(&addr) else {
            return;
        };
        let mut kept = Vec::with_capacity(list.len());
        for mut c in list.drain(..) {
            if now.duration_since(c.idle_since) > IDLE_TTL {
                self.by_token.remove(&c.token);
                let _ = registry.deregister(&mut c.stream);
                Self::discard(c.stream);
            } else {
                kept.push(c);
            }
        }
        if kept.is_empty() {
            self.by_addr.remove(&addr);
        } else {
            self.by_addr.insert(addr, kept);
        }
    }
}

fn alloc_token(next: &mut Token) -> Token {
    let token = *next;
    next.0 = next.0.wrapping_add(1).max(1);
    token
}

#[cfg(test)]
mod tests {
    use super::*;
    use mio::{Events, Poll};
    use std::io::Write;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn trailing_byte_at_checkin_discards() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let peer = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            s.write_all(b"Z").expect("write");
            thread::sleep(Duration::from_millis(50));
        });
        let mut stream = UpstreamPool::connect(addr).expect("connect");
        // Peer may not have written yet; wait briefly then probe.
        thread::sleep(Duration::from_millis(20));
        // Drain until dirty or give up.
        let mut dirty = false;
        for _ in 0..20 {
            if UpstreamPool::probe(&mut stream) == Probe::Dirty {
                dirty = true;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(dirty, "trailing byte must poison probe");
        let _ = peer.join();
    }

    #[test]
    fn put_take_clean_roundtrip() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let _peer = thread::spawn(move || {
            let (_s, _) = listener.accept().expect("accept");
            thread::sleep(Duration::from_millis(200));
        });
        let poll = Poll::new().expect("poll");
        let mut pool = UpstreamPool::default();
        let mut next = Token(10);
        let stream = UpstreamPool::connect(addr).expect("connect");
        pool.put(poll.registry(), addr, stream, 1, &mut next);
        assert!(pool.is_idle_token(Token(10)));
        let taken = pool.take(poll.registry(), addr, 1);
        assert!(taken.is_some());
        assert!(!pool.is_idle_token(Token(10)));
        assert!(pool.take(poll.registry(), addr, 1).is_none());
    }

    #[test]
    fn generation_mismatch_discards() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let _peer = thread::spawn(move || {
            let (_s, _) = listener.accept().expect("accept");
            thread::sleep(Duration::from_millis(200));
        });
        let poll = Poll::new().expect("poll");
        let mut pool = UpstreamPool::default();
        let mut next = Token(20);
        let stream = UpstreamPool::connect(addr).expect("connect");
        pool.put(poll.registry(), addr, stream, 1, &mut next);
        assert!(pool.take(poll.registry(), addr, 2).is_none());
    }

    #[test]
    fn idle_readable_discards() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let peer = thread::spawn(move || {
            let (mut s, _) = listener.accept().expect("accept");
            thread::sleep(Duration::from_millis(30));
            let _ = s.write_all(b"X");
            thread::sleep(Duration::from_millis(100));
        });
        let mut poll = Poll::new().expect("poll");
        let mut pool = UpstreamPool::default();
        let mut next = Token(30);
        let stream = UpstreamPool::connect(addr).expect("connect");
        pool.put(poll.registry(), addr, stream, 1, &mut next);
        let idle_token = Token(30);
        assert!(pool.is_idle_token(idle_token));
        let mut events = Events::with_capacity(8);
        let mut saw = false;
        for _ in 0..50 {
            poll.poll(&mut events, Some(Duration::from_millis(20)))
                .expect("poll");
            for ev in events.iter() {
                if ev.token() == idle_token {
                    pool.on_idle_ready(poll.registry(), idle_token);
                    saw = true;
                }
            }
            if saw {
                break;
            }
        }
        assert!(saw, "idle READABLE must fire after trailing byte");
        assert!(pool.take(poll.registry(), addr, 1).is_none());
        let _ = peer.join();
    }
}
