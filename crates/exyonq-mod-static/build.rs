//! The static wire `Server` header uses the same artifact version as `exyonq --version`.

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
