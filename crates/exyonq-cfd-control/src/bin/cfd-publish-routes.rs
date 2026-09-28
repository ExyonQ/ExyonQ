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

//! Publish CFD route table (proxy + FastCGI) into a generation directory.

use std::env;
use std::path::PathBuf;
use std::process;

use exyonq_cfd_control::{parse_routes_file, GenDir, Generation, SCHEMA_VERSION};

fn main() {
    let mut gen_dir = None;
    let mut routes = None;
    let mut gid = 1u64;
    let mut args = env::args().skip(1);
    while let Some(k) = args.next() {
        match k.as_str() {
            "--gen-dir" => gen_dir = args.next().map(PathBuf::from),
            "--routes" => routes = args.next().map(PathBuf::from),
            "--generation-id" => {
                gid = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .expect("--generation-id N");
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: cfd-publish-routes --gen-dir DIR --routes FILE [--generation-id N]"
                );
                process::exit(0);
            }
            other => {
                eprintln!("unknown arg {other}");
                process::exit(2);
            }
        }
    }
    let Some(gen_dir) = gen_dir else {
        eprintln!("missing --gen-dir");
        process::exit(2);
    };
    let Some(routes) = routes else {
        eprintln!("missing --routes");
        process::exit(2);
    };
    let table = match parse_routes_file(&routes) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("routes: {e}");
            process::exit(1);
        }
    };
    let gd = GenDir::new(&gen_dir);
    if let Err(e) = gd.ensure() {
        eprintln!("gen dir: {e}");
        process::exit(1);
    }
    let g = match Generation::from_route_table(gid, &table) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("generation: {e}");
            process::exit(1);
        }
    };
    if let Err(e) = gd.publish(&g) {
        eprintln!("publish: {e}");
        process::exit(1);
    }
    println!(
        "published gen={gid} schema={SCHEMA_VERSION} routes={} fcgi_pools={}",
        table.routes.len(),
        table.fcgi_pools.len()
    );
}
