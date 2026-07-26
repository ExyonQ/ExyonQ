use super::schema::performance_contract_registry_path;
use anyhow::Context;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct RegistryRoot {
    references: std::collections::BTreeMap<String, RegistryEntry>,
}

#[derive(Debug, Deserialize)]
struct RegistryEntry {
    path: String,
    #[serde(default)]
    approval_status: Option<String>,
    #[serde(default)]
    arch: Option<String>,
}

pub fn epoll_static_enabled() -> bool {
    std::env::var("EXYONQ_EPOLL_STATIC")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub fn worker_mode_from_env() -> &'static str {
    if epoll_static_enabled() {
        "epoll_static"
    } else {
        "default_tokio"
    }
}

pub fn host_class_for_arch(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("linux-x86_64"),
        "aarch64" => Some("linux-aarch64"),
        _ => None,
    }
}

pub fn registry_key(host_class: &str, worker_mode: &str) -> String {
    format!("{host_class}/{worker_mode}")
}

pub fn resolve_baseline_path(repo_root: &Path) -> anyhow::Result<PathBuf> {
    let arch = std::env::consts::ARCH;
    let host_class = host_class_for_arch(arch)
        .with_context(|| format!("unsupported contract architecture: {arch}"))?;
    let worker_mode = worker_mode_from_env();
    resolve_baseline_path_for(repo_root, host_class, worker_mode)
}

pub fn resolve_baseline_path_for(
    repo_root: &Path,
    host_class: &str,
    worker_mode: &str,
) -> anyhow::Result<PathBuf> {
    if host_class == "linux-aarch64" && worker_mode == "epoll_static" {
        anyhow::bail!(
            "unsupported contract reference: {host_class}/{worker_mode} (no aarch64 epoll_static baseline)"
        );
    }
    let reg_path = performance_contract_registry_path(repo_root);
    let raw = std::fs::read_to_string(&reg_path)
        .with_context(|| format!("read registry {}", reg_path.display()))?;
    let reg: RegistryRoot = serde_json::from_str(&raw)?;
    let key = registry_key(host_class, worker_mode);
    let entry = reg
        .references
        .get(&key)
        .with_context(|| format!("no contract reference for {key}"))?;
    if let Some(status) = entry.approval_status.as_deref() {
        if status != "APPROVED" && status != "accepted" {
            anyhow::bail!(
                "contract reference {key} approval_status={status} (not APPROVED)"
            );
        }
    }
    Ok(repo_root.join("benchmarks/baselines").join(&entry.path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_registry(repo: &Path, json: &str) {
        let dir = repo.join("benchmarks/baselines");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join("performance-contract-registry.json"), json).expect("registry");
    }

    fn touch_baseline(repo: &Path, rel: &str) {
        let path = repo.join("benchmarks/baselines").join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(&path, "{}\n").expect("baseline");
    }

    #[test]
    fn registry_selects_x86_64_default_tokio() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_registry(
            repo,
            r#"{
  "references": {
    "linux-x86_64/default_tokio": {
      "path": "proposed/linux-x86_64-default_tokio-performance-contract-v1.json",
      "approval_status": "APPROVED"
    }
  }
}"#,
        );
        touch_baseline(
            repo,
            "proposed/linux-x86_64-default_tokio-performance-contract-v1.json",
        );
        std::env::remove_var("EXYONQ_EPOLL_STATIC");
        let path =
            resolve_baseline_path_for(repo, "linux-x86_64", "default_tokio").expect("resolve");
        assert!(path.ends_with("linux-x86_64-default_tokio-performance-contract-v1.json"));
    }

    #[test]
    fn registry_selects_x86_64_epoll_static() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_registry(
            repo,
            r#"{
  "references": {
    "linux-x86_64/epoll_static": {
      "path": "proposed/linux-x86_64-epoll_static-performance-contract-v1.json",
      "approval_status": "APPROVED"
    }
  }
}"#,
        );
        touch_baseline(
            repo,
            "proposed/linux-x86_64-epoll_static-performance-contract-v1.json",
        );
        let path = resolve_baseline_path_for(repo, "linux-x86_64", "epoll_static").expect("resolve");
        assert!(path.ends_with("linux-x86_64-epoll_static-performance-contract-v1.json"));
    }

    #[test]
    fn arm_reference_remains_on_aarch64() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_registry(
            repo,
            r#"{
  "references": {
    "linux-aarch64/default_tokio": {
      "path": "performance-contract-v1.json",
      "approval_status": "APPROVED"
    }
  }
}"#,
        );
        touch_baseline(repo, "performance-contract-v1.json");
        let path = resolve_baseline_path_for(repo, "linux-aarch64", "default_tokio").expect("resolve");
        assert!(path.ends_with("performance-contract-v1.json"));
    }

    #[test]
    fn no_x86_64_fallback_to_arm() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_registry(
            repo,
            r#"{
  "references": {
    "linux-aarch64/default_tokio": {
      "path": "performance-contract-v1.json",
      "approval_status": "APPROVED"
    }
  }
}"#,
        );
        touch_baseline(repo, "performance-contract-v1.json");
        let err = resolve_baseline_path_for(repo, "linux-x86_64", "default_tokio").unwrap_err();
        assert!(err.to_string().contains("no contract reference"));
    }

    #[test]
    fn aarch64_epoll_static_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = dir.path();
        write_registry(repo, r#"{"references":{}}"#);
        let err = resolve_baseline_path_for(repo, "linux-aarch64", "epoll_static").unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn epoll_static_env_maps_worker_mode() {
        std::env::set_var("EXYONQ_EPOLL_STATIC", "1");
        assert_eq!(worker_mode_from_env(), "epoll_static");
        std::env::remove_var("EXYONQ_EPOLL_STATIC");
        assert_eq!(worker_mode_from_env(), "default_tokio");
    }
}
