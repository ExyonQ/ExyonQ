//! Product profile CLI (P1.4-WS4) — list / render / explain / test.

use crate::output::{sanitize_control_chars, CliExit, OutputFormat, ToolResult};
use exyonq_config_merge::{
    expand_product_profile, render_product_profile_toml, ProductProfile, ProductProfileInputs,
    INTERNAL_PROFILE_DISPOSITION,
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ProfileCliInputs {
    pub listen: Option<String>,
    pub document_root: Option<PathBuf>,
    pub upstream_target: Option<String>,
    pub fpm_address: Option<String>,
    pub fpm_transport: Option<String>,
    pub index: Option<String>,
}

impl ProfileCliInputs {
    fn to_inputs(&self) -> ProductProfileInputs {
        let mut i = ProductProfileInputs::default();
        if let Some(v) = &self.listen {
            i.listen = v.clone();
        }
        if let Some(v) = &self.document_root {
            i.document_root = v.clone();
        }
        if let Some(v) = &self.upstream_target {
            i.upstream_target = v.clone();
        }
        if let Some(v) = &self.fpm_address {
            i.fpm_address = v.clone();
        }
        if let Some(v) = &self.fpm_transport {
            i.fpm_transport = v.clone();
        }
        if let Some(v) = &self.index {
            i.index = v.clone();
        }
        i
    }
}

pub fn profile_list(format: OutputFormat) -> ToolResult {
    match format {
        OutputFormat::Json => {
            let names: Vec<&str> = ProductProfile::all().iter().map(|p| p.as_str()).collect();
            let body = format!(
                "{{\n  \"schema_version\": 1,\n  \"product_profiles\": {:?},\n  \"internal_profiles\": [\"dev\", \"edge\", \"lb\"],\n  \"internal_disposition\": {:?}\n}}",
                names, INTERNAL_PROFILE_DISPOSITION
            );
            ToolResult {
                exit: CliExit::Ok,
                stdout: body,
                stderr: String::new(),
            }
        }
        OutputFormat::Human => {
            let mut out = String::from("product_profiles:\n");
            for p in ProductProfile::all() {
                out.push_str(&format!("  - {}\n", p.as_str()));
            }
            out.push_str(&format!(
                "internal_profiles ({}): dev, edge, lb\n",
                INTERNAL_PROFILE_DISPOSITION
            ));
            ToolResult {
                exit: CliExit::Ok,
                stdout: out,
                stderr: String::new(),
            }
        }
    }
}

pub fn profile_render(name: &str, inputs: &ProfileCliInputs, format: OutputFormat) -> ToolResult {
    let Some(profile) = ProductProfile::parse(name) else {
        return unknown_profile(name);
    };
    let toml = render_product_profile_toml(profile, &inputs.to_inputs());
    match format {
        OutputFormat::Human => ToolResult {
            exit: CliExit::Ok,
            stdout: toml,
            stderr: String::new(),
        },
        OutputFormat::Json => {
            let escaped = json_escape(&toml);
            ToolResult {
                exit: CliExit::Ok,
                stdout: format!(
                    "{{\n  \"schema_version\": 1,\n  \"profile\": \"{}\",\n  \"toml\": \"{escaped}\"\n}}",
                    profile.as_str()
                ),
                stderr: String::new(),
            }
        }
    }
}

pub fn profile_explain(name: &str, format: OutputFormat) -> ToolResult {
    let Some(profile) = ProductProfile::parse(name) else {
        return unknown_profile(name);
    };
    let (meaning, defaults, limits) = match profile {
        ProductProfile::Static => (
            "Static site: listener + site root + index. No proxy, FastCGI, or implicit cache.",
            "listen=0.0.0.0:8080 root=/var/www/html index=index.html",
            "Body/header limits and access logging follow runtime product defaults (not IR fields). Optional TLS is TOML-only add-on.",
        ),
        ProductProfile::Proxy => (
            "Reverse proxy: listener + route + upstream. XFF untrusted inbound and ONE_SAFE_RETRY inherited from P1.3a (not IR).",
            "listen=0.0.0.0:8080 upstream=http://127.0.0.1:9000 timeout_ms=30000",
            "No second proxy path. No H2/H3 upstream claim. Optional client TLS/H2/H3 via TOML server fields.",
        ),
        ProductProfile::Php => (
            "PHP-FPM via FastCGI: document root + fcgi_pool (UDS/TCP explicit). No implicit cache.",
            "listen=0.0.0.0:8080 fpm=/run/php/php-fpm.sock document_root=/var/www/html max_concurrency=16",
            "Pool reuse across generations = NO. full_page_cache.enabled=false.",
        ),
        ProductProfile::Wordpress => (
            "WordPress: PHP base + wp-includes/wp-content static + front controller via htaccess=overlay.",
            "Same as php + htaccess=overlay on app route; FULL_HTACCESS_COMPATIBILITY=NO; WORDPRESS_CACHE=OFF",
            "No LSCache claim. Partial htaccess overlay only.",
        ),
    };
    match format {
        OutputFormat::Human => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "public_term = {}\nmeaning = {}\ndefaults = {}\nsupport_limit = {}\nexpansion = offline AppConfig TOML\n",
                profile.as_str(),
                sanitize_control_chars(meaning),
                defaults,
                limits
            ),
            stderr: String::new(),
        },
        OutputFormat::Json => ToolResult {
            exit: CliExit::Ok,
            stdout: format!(
                "{{\n  \"schema_version\": 1,\n  \"public_term\": \"{}\",\n  \"meaning\": \"{}\",\n  \"defaults\": \"{}\",\n  \"support_limit\": \"{}\"\n}}",
                profile.as_str(),
                json_escape(meaning),
                json_escape(defaults),
                json_escape(limits)
            ),
            stderr: String::new(),
        },
    }
}

pub fn profile_test(name: &str, inputs: &ProfileCliInputs, format: OutputFormat) -> ToolResult {
    let Some(profile) = ProductProfile::parse(name) else {
        return unknown_profile(name);
    };
    let cfg = match expand_product_profile(profile, &inputs.to_inputs()) {
        Ok(c) => c,
        Err(e) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: format!(
                    "error[EXY-CONFIG-0004]: {}\n",
                    sanitize_control_chars(&e.to_string())
                ),
            };
        }
    };
    match exyonq_runtime_plan::compile_runtime_plan_from_ir(0, cfg) {
        Ok(plan) => {
            drop(plan);
            match format {
                OutputFormat::Human => ToolResult {
                    exit: CliExit::Ok,
                    stdout: String::new(),
                    stderr: format!(
                        "profile {} OK (expanded + RuntimePlan compile; discarded)\n",
                        profile.as_str()
                    ),
                },
                OutputFormat::Json => ToolResult {
                    exit: CliExit::Ok,
                    stdout: format!(
                        "{{\n  \"schema_version\": 1,\n  \"profile\": \"{}\",\n  \"ok\": true\n}}",
                        profile.as_str()
                    ),
                    stderr: String::new(),
                },
            }
        }
        Err(e) => ToolResult {
            exit: CliExit::DiagnosticError,
            stdout: String::new(),
            stderr: format!(
                "error[EXY-CONFIG-0012]: {}\n",
                sanitize_control_chars(&e.to_string())
            ),
        },
    }
}

fn unknown_profile(name: &str) -> ToolResult {
    ToolResult {
        exit: CliExit::DiagnosticError,
        stdout: String::new(),
        stderr: format!(
            "error[EXY-CONFIG-0014]: unknown product profile `{name}` (not Dev/Edge/Lb)\n"
        ),
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}
