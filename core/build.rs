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
//! Cap061 P3-01: no build-time benchmark wire blobs.
//! The Server header and `exyonq --version` share `EXYONQ_ARTIFACT_VERSION`.

fn main() {
    let artifact = std::env::var("EXYONQ_ARTIFACT_VERSION")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.4.9".to_string())
        });
    println!("cargo:rustc-env=EXYONQ_ARTIFACT_VERSION={artifact}");
    println!("cargo:rerun-if-env-changed=EXYONQ_ARTIFACT_VERSION");
}
