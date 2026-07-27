//! Product profiles (P1.4-WS4) — offline IR expansion.
//!
//! Distinct from internal deploy presets [`crate::Profile`] (Dev|Edge|Lb = INTERNAL_ONLY).

use exyonq_config_ir::{AppConfig, ConfigError};
use std::path::PathBuf;

/// Product-facing profiles (operator presets). Not Dev/Edge/Lb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductProfile {
    Static,
    Proxy,
    Php,
    Wordpress,
}

impl ProductProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Proxy => "proxy",
            Self::Php => "php",
            Self::Wordpress => "wordpress",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "static" => Some(Self::Static),
            "proxy" => Some(Self::Proxy),
            "php" => Some(Self::Php),
            "wordpress" | "wp" => Some(Self::Wordpress),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Static, Self::Proxy, Self::Php, Self::Wordpress]
    }
}

/// Explicit operator inputs. Unknown free-form keys are not accepted (CLI maps flags only).
#[derive(Debug, Clone)]
pub struct ProductProfileInputs {
    pub listen: String,
    pub document_root: PathBuf,
    pub upstream_target: String,
    pub fpm_address: String,
    pub fpm_transport: String,
    pub index: String,
    pub upstream_timeout_ms: u64,
    pub max_concurrency: u32,
    pub max_connections: u32,
}

impl Default for ProductProfileInputs {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            document_root: PathBuf::from("/var/www/html"),
            upstream_target: "http://127.0.0.1:9000".into(),
            fpm_address: "/run/php/php-fpm.sock".into(),
            fpm_transport: "unix".into(),
            index: "index.html".into(),
            upstream_timeout_ms: 30_000,
            max_concurrency: 16,
            max_connections: 16,
        }
    }
}

/// Disposition of legacy [`crate::Profile`] (Dev|Edge|Lb).
pub const INTERNAL_PROFILE_DISPOSITION: &str = "INTERNAL_ONLY";

/// Expand a product profile to validated `AppConfig` IR (offline, deterministic).
pub fn expand_product_profile(
    profile: ProductProfile,
    inputs: &ProductProfileInputs,
) -> Result<AppConfig, ConfigError> {
    let toml = render_product_profile_toml(profile, inputs);
    let cfg = AppConfig::parse_str(&toml)?;
    assert_product_invariants(profile, &cfg)?;
    Ok(cfg)
}

/// Idempotent TOML emission (golden source for CLI `profile render`).
pub fn render_product_profile_toml(
    profile: ProductProfile,
    inputs: &ProductProfileInputs,
) -> String {
    let listen = toml_string(&inputs.listen);
    let root = toml_string(&inputs.document_root.display().to_string());
    let index = toml_string(&inputs.index);
    let upstream = toml_string(&inputs.upstream_target);
    let fpm = toml_string(&inputs.fpm_address);
    let transport = toml_string(&inputs.fpm_transport);
    let includes = inputs.document_root.join("wp-includes");
    let content = inputs.document_root.join("wp-content");
    let includes_s = toml_string(&includes.display().to_string());
    let content_s = toml_string(&content.display().to_string());

    match profile {
        ProductProfile::Static => format!(
            r#"# ProductProfile = static
# INTERNAL deploy presets Dev|Edge|Lb are NOT applied here.
config_version = 2

[[server]]
listen = {listen}
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = {root}
index = {index}
"#
        ),
        ProductProfile::Proxy => format!(
            r#"# ProductProfile = proxy
# PRODUCT_INHERITED: X-Forwarded-For = do not trust inbound (P1.3a)
# PRODUCT_INHERITED: ONE_SAFE_RETRY = GET/HEAD only (P1.3a)
# No H2/H3 upstream claim.
config_version = 2

[[server]]
listen = {listen}
routes = ["app"]

[[route]]
name = "app"
match = {{ path = "/" }}
upstream = "backend"

[[upstream]]
name = "backend"
target = {upstream}
timeout_ms = {timeout}
"#,
            timeout = inputs.upstream_timeout_ms
        ),
        ProductProfile::Php => format!(
            r#"# ProductProfile = php
# No implicit cache. Pool reuse across generations = NO (reload rebuilds plan).
config_version = 2

[[server]]
listen = {listen}
routes = ["php"]

[[route]]
name = "php"
match = {{ path = "/" }}
fastcgi = "php"

[[fcgi_pool]]
name = "php"
address = {fpm}
transport = {transport}
document_root = {root}
max_concurrency = {mc}
max_connections = {mx}
idle_timeout_ms = 30000
total_timeout_ms = 30000
checkout_timeout_ms = 5000

[full_page_cache]
enabled = false
"#,
            mc = inputs.max_concurrency,
            mx = inputs.max_connections
        ),
        ProductProfile::Wordpress => format!(
            r#"# ProductProfile = wordpress
# = php base + static assets + front controller via htaccess overlay
# FULL_HTACCESS_COMPATIBILITY = NO
# WORDPRESS_CACHE = OFF
config_version = 2

[[server]]
listen = {listen}
routes = ["wp-includes", "wp-content", "wordpress"]

[[route]]
name = "wp-includes"
match = {{ path = "/wp-includes" }}
root = {includes_s}

[[route]]
name = "wp-content"
match = {{ path = "/wp-content" }}
root = {content_s}

[[route]]
name = "wordpress"
match = {{ path = "/" }}
fastcgi = "php"
htaccess = "overlay"

[[fcgi_pool]]
name = "php"
address = {fpm}
transport = {transport}
document_root = {root}
max_concurrency = {mc}
max_connections = {mx}
idle_timeout_ms = 30000
total_timeout_ms = 30000
checkout_timeout_ms = 5000

[full_page_cache]
enabled = false
"#,
            mc = inputs.max_concurrency,
            mx = inputs.max_connections
        ),
    }
}

fn toml_string(s: &str) -> String {
    format!("{s:?}")
}

fn assert_product_invariants(profile: ProductProfile, cfg: &AppConfig) -> Result<(), ConfigError> {
    if matches!(profile, ProductProfile::Php | ProductProfile::Wordpress)
        && cfg.full_page_cache.enabled
    {
        return Err(ConfigError::Parse(
            "product profile forbids full_page_cache.enabled = true".into(),
        ));
    }
    for route in &cfg.routes {
        if route.cache.is_some() {
            return Err(ConfigError::Parse(
                "product profile must not emit route.cache".into(),
            ));
        }
    }
    if !cfg.cache_policies.is_empty() {
        return Err(ConfigError::Parse(
            "product profile must not emit cache_policy".into(),
        ));
    }
    if matches!(profile, ProductProfile::Static) {
        if !cfg.upstreams.is_empty() || !cfg.pools_fcgi.is_empty() {
            return Err(ConfigError::Parse(
                "static profile must not emit upstream or fcgi_pool".into(),
            ));
        }
    }
    if matches!(profile, ProductProfile::Proxy) && !cfg.pools_fcgi.is_empty() {
        return Err(ConfigError::Parse(
            "proxy profile must not emit fcgi_pool".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_profiles_expand_and_validate() {
        let inputs = ProductProfileInputs::default();
        for p in ProductProfile::all() {
            let a = expand_product_profile(*p, &inputs).expect(p.as_str());
            let b = expand_product_profile(*p, &inputs).expect(p.as_str());
            assert_eq!(a, b, "idempotent {}", p.as_str());
            let again = AppConfig::parse_str(&render_product_profile_toml(*p, &inputs)).unwrap();
            assert_eq!(a, again);
        }
    }

    #[test]
    fn wordpress_cache_off_htaccess_overlay() {
        let cfg =
            expand_product_profile(ProductProfile::Wordpress, &ProductProfileInputs::default())
                .unwrap();
        assert!(!cfg.full_page_cache.enabled);
        let wp = cfg.routes.iter().find(|r| r.name == "wordpress").unwrap();
        assert_eq!(wp.htaccess, exyonq_config_ir::HtaccessMode::Overlay);
        assert!(wp.cache.is_none());
    }

    #[test]
    fn static_has_no_proxy() {
        let cfg = expand_product_profile(ProductProfile::Static, &ProductProfileInputs::default())
            .unwrap();
        assert!(cfg.upstreams.is_empty());
        assert!(cfg.pools_fcgi.is_empty());
        assert_eq!(
            cfg.routes[0].root.as_ref().unwrap(),
            PathBuf::from("/var/www/html").as_path()
        );
    }

    #[test]
    fn parse_names() {
        assert_eq!(ProductProfile::parse("WP"), Some(ProductProfile::Wordpress));
        assert!(ProductProfile::parse("edge").is_none());
        assert!(ProductProfile::parse("dev").is_none());
    }

    #[test]
    fn golden_fixtures_match_render() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/profiles");
        let inputs = ProductProfileInputs::default();
        for p in ProductProfile::all() {
            let path = root.join(format!("{}.toml", p.as_str()));
            let expected =
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            let got = render_product_profile_toml(*p, &inputs);
            assert_eq!(got.trim(), expected.trim(), "golden {}", p.as_str());
        }
    }
}
