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
//! Prints rustc/asm inspection commands for PS0A codegen review.

fn main() {
    println!("PS0A codegen inspection (run manually from repo root):");
    println!();
    println!("# Release + LTO (workspace default)");
    println!("cargo rustc -p exyonq-platform-boundary-spike-harness --release --bin boundary-spike-bench -- --emit=asm -C llvm-args=--x86-asm-syntax=intel");
    println!();
    println!("# Release without LTO (variant N)");
    println!("cargo rustc -p exyonq-platform-boundary-spike-harness --profile release-spike-nolto --bin boundary-spike-bench -- --emit=asm");
    println!();
    println!("Inspect symbols: plan_wire_after_headers, read_wire_plan_cross_static, read_wire_plan_cross_fnptr");
    println!("Expect: direct call or inlined body for X; indirect call via register for F.");
}
