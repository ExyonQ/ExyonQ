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
//! Precomputes P1 bench wire (header + 1024-byte body) for `.rodata` embed.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let header = b"HTTP/1.1 200 OK\r\nContent-Length: 1024\r\n\r\n";
    let body = vec![b'x'; 1024];
    let mut wire = Vec::with_capacity(header.len() + body.len());
    wire.extend_from_slice(header);
    wire.extend_from_slice(&body);
    fs::write(out_dir.join("p1_bench_wire.bin"), wire).expect("write p1_bench_wire.bin");
    println!("cargo:rerun-if-changed=build.rs");
}
