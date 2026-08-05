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
