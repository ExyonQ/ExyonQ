use exyonq_config_ir::AppConfig;
use exyonq_config_merge::load_with_includes;
use exyonq_config_surface::{compile_serverfile, CompileOptions};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    Toml,
    Exy,
}

pub fn detect_kind(path: &Path) -> ConfigKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("exy") => ConfigKind::Exy,
        _ => ConfigKind::Toml,
    }
}

pub fn load_app_config(path: &Path) -> Result<AppConfig, String> {
    match detect_kind(path) {
        ConfigKind::Exy => {
            let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let toml =
                compile_serverfile(&raw, CompileOptions::default()).map_err(|e| e.to_string())?;
            AppConfig::parse_str(&toml).map_err(|e| e.to_string())
        }
        ConfigKind::Toml => load_with_includes(path).map_err(|e| e.to_string()),
    }
}
