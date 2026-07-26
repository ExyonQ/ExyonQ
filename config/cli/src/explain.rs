//! Public lexicon explain (P1.4-WS3).

use crate::output::{sanitize_control_chars, CliExit, OutputFormat, ToolResult};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ExplainRequest {
    pub directive: Option<String>,
    pub path: Option<PathBuf>,
    pub pointer: Option<String>,
    pub format: OutputFormat,
}

struct Entry {
    public_term: &'static str,
    aliases: &'static [&'static str],
    meaning: &'static str,
    surface: &'static str,
    ir_mapping: &'static str,
    required: bool,
    default: Option<&'static str>,
    allowed: &'static [&'static str],
    codes: &'static [&'static str],
    support_limit: Option<&'static str>,
    docs: &'static [&'static str],
}

const CATALOG: &[Entry] = &[
    Entry {
        public_term: "listener",
        aliases: &["listen", "server.listen"],
        meaning: "Bind address for an HTTP listener (optional TLS / HTTP/3 listen).",
        surface: ".exy listener header or TOML [[server]]",
        ir_mapping: "[[server]].listen (+ tls, http3_listen)",
        required: true,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0003", "EXY-CONFIG-0004"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
    Entry {
        public_term: "site",
        aliases: &["vhost", "server_name"],
        meaning: "Virtual host / site identity scope (public lexicon).",
        surface: "public term; IR retains historical [[server]]",
        ir_mapping: "[[server]] / server_name (dual semantics = no)",
        required: false,
        default: Some("empty server_name"),
        allowed: &[],
        codes: &[],
        support_limit: Some("IR table name remains [[server]] for compatibility"),
        docs: &[
            "docs/architecture/p1.4-configuration-product-contract.md",
            "docs/architecture/p1.4-exy-to-ir-mapping.md",
        ],
    },
    Entry {
        public_term: "route",
        aliases: &[],
        meaning: "Path match with a single action (static, proxy, redirect, rewrite, fastcgi).",
        surface: ".exy route block or [[route]]",
        ir_mapping: "[[route]]",
        required: true,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0003", "EXY-CONFIG-0005"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
    Entry {
        public_term: "upstream",
        aliases: &[],
        meaning: "Named HTTP/1.1 upstream target for reverse proxy.",
        surface: ".exy proxy directive or [[upstream]]",
        ir_mapping: "[[upstream]]",
        required: false,
        default: Some("timeout_ms=30000"),
        allowed: &["http:// URI"],
        codes: &["EXY-CONFIG-0004"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-config-defaults-matrix.md"],
    },
    Entry {
        public_term: "module",
        aliases: &["modules"],
        meaning: "Optional closed module namespace (metrics, compression, ratelimit).",
        surface: "[modules.*]",
        ir_mapping: "[modules]",
        required: false,
        default: Some("all disabled"),
        allowed: &["metrics", "compression", "ratelimit"],
        codes: &["EXY-CONFIG-0002"],
        support_limit: Some("closed set — unknown module fields fail closed"),
        docs: &["docs/config/diagnostic-codes.md"],
    },
    Entry {
        public_term: "backend",
        aliases: &[],
        meaning: "Compiled route action in RuntimePlan (not an IR authoring table).",
        surface: "derived at compile",
        ir_mapping: "RuntimePlan BackendTable / slots",
        required: false,
        default: None,
        allowed: &["static", "proxy", "fastcgi", "redirect", "rewrite"],
        codes: &["EXY-CONFIG-0012"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-configuration-product-contract.md"],
    },
    Entry {
        public_term: "http3_listen",
        aliases: &["h3", "h3_listen"],
        meaning: "Optional UDP listen for client-facing HTTP/3.",
        surface: "TOML-only in Serverfile v0",
        ir_mapping: "[[server]].http3_listen",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0004"],
        support_limit: Some("H3 product closed; reload SUPPORT_LIMIT"),
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
    Entry {
        public_term: "pools_fcgi",
        aliases: &["fcgi_pool", "fastcgi"],
        meaning: "FastCGI pool declaration (structural IR).",
        surface: "TOML [[fcgi_pool]]",
        ir_mapping: "[[fcgi_pool]] / route.fastcgi",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0004"],
        support_limit: Some("FastCGI product closed — config fields only"),
        docs: &["docs/architecture/p1.4-config-defaults-matrix.md"],
    },
    Entry {
        public_term: "htaccess",
        aliases: &[],
        meaning: "Per-route .htaccess overlay mode.",
        surface: "TOML route.htaccess",
        ir_mapping: "[[route]].htaccess = off|overlay",
        required: false,
        default: Some("off"),
        allowed: &["off", "overlay"],
        codes: &[],
        support_limit: Some("runtime overlay partial product"),
        docs: &["docs/architecture/p1.4-configuration-current-state-audit.md"],
    },
    Entry {
        public_term: "config_version",
        aliases: &[],
        meaning: "IR schema version.",
        surface: "TOML top-level",
        ir_mapping: "config_version = 1|2",
        required: true,
        default: None,
        allowed: &["1", "2"],
        codes: &["EXY-CONFIG-0001"],
        support_limit: None,
        docs: &["docs/config/diagnostic-codes.md"],
    },
    Entry {
        public_term: "policy",
        aliases: &["cache_policy"],
        meaning: "Named reusable policy (e.g. cache_policy).",
        surface: "[[cache_policy]]",
        ir_mapping: "[[cache_policy]] referenced by route.cache",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0004"],
        support_limit: Some("CACHE_LINE closed — fields remain"),
        docs: &["docs/architecture/p1.4-config-defaults-matrix.md"],
    },
    Entry {
        public_term: "proxy",
        aliases: &[],
        meaning: "Reverse proxy action inside a route (upstream name + port).",
        surface: ".exy `proxy` inside route",
        ir_mapping: "[[route]] proxy action + [[upstream]]",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0004"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
    Entry {
        public_term: "root",
        aliases: &[],
        meaning: "Static files root directory for a route prefix.",
        surface: ".exy `root` inside route",
        ir_mapping: "[[route]].root",
        required: false,
        default: None,
        allowed: &[],
        codes: &[],
        support_limit: None,
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
    Entry {
        public_term: "index",
        aliases: &[],
        meaning: "Default index file inside a static root.",
        surface: ".exy `index` inside route",
        ir_mapping: "[[route]].index",
        required: false,
        default: None,
        allowed: &[],
        codes: &[],
        support_limit: None,
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
    Entry {
        public_term: "metrics",
        aliases: &[],
        meaning: "Enables Prometheus metrics module.",
        surface: ".exy `metrics` / [modules.metrics]",
        ir_mapping: "[modules.metrics]",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0002"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-config-defaults-matrix.md"],
    },
    Entry {
        public_term: "compression",
        aliases: &[],
        meaning: "Enables response compression module.",
        surface: ".exy `compression` / [modules.compression]",
        ir_mapping: "[modules.compression]",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0002"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-config-defaults-matrix.md"],
    },
    Entry {
        public_term: "ratelimit",
        aliases: &[],
        meaning: "Token-bucket rate limiting module.",
        surface: ".exy `ratelimit` / [modules.ratelimit]",
        ir_mapping: "[modules.ratelimit]",
        required: false,
        default: None,
        allowed: &[],
        codes: &["EXY-CONFIG-0002"],
        support_limit: None,
        docs: &["docs/architecture/p1.4-config-defaults-matrix.md"],
    },
    Entry {
        public_term: "timeout",
        aliases: &[],
        meaning: "Optional proxy timeout inside a route block.",
        surface: ".exy `timeout` inside route",
        ir_mapping: "[[route]] proxy timeout fields",
        required: false,
        default: None,
        allowed: &[],
        codes: &[],
        support_limit: None,
        docs: &["docs/architecture/p1.4-exy-to-ir-mapping.md"],
    },
];

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

fn json_string_array(items: &[&str]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(&json_escape(item));
        out.push('"');
    }
    out.push(']');
    out
}

fn explain_json(entry: &Entry) -> String {
    let mut out = String::from("{\n");
    out.push_str("  \"schema_version\": 1,\n");
    out.push_str(&format!(
        "  \"public_term\": \"{}\",\n",
        json_escape(entry.public_term)
    ));
    out.push_str(&format!(
        "  \"meaning\": \"{}\",\n",
        json_escape(entry.meaning)
    ));
    out.push_str(&format!(
        "  \"surface\": \"{}\",\n",
        json_escape(entry.surface)
    ));
    out.push_str(&format!(
        "  \"ir_mapping\": \"{}\",\n",
        json_escape(entry.ir_mapping)
    ));
    out.push_str(&format!("  \"required\": {},\n", entry.required));
    match entry.default {
        Some(d) => out.push_str(&format!("  \"default\": \"{}\",\n", json_escape(d))),
        None => out.push_str("  \"default\": null,\n"),
    }
    out.push_str(&format!(
        "  \"allowed_values\": {},\n",
        json_string_array(entry.allowed)
    ));
    out.push_str(&format!(
        "  \"related_codes\": {},\n",
        json_string_array(entry.codes)
    ));
    match entry.support_limit {
        Some(s) => out.push_str(&format!(
            "  \"support_limit\": \"{}\",\n",
            json_escape(s)
        )),
        None => out.push_str("  \"support_limit\": null,\n"),
    }
    out.push_str(&format!(
        "  \"documentation\": {}\n",
        json_string_array(entry.docs)
    ));
    out.push('}');
    out
}

pub fn explain(req: ExplainRequest) -> ToolResult {
    let term = req
        .directive
        .as_deref()
        .or(req.pointer.as_deref())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if term.is_empty() {
        return ToolResult {
            exit: CliExit::UsageOrTool,
            stdout: String::new(),
            stderr: "explain: provide a directive or --pointer\n".into(),
        };
    }
    if let Some(path) = &req.path {
        let _ = path; // pointer+path reserved; catalog explain is sufficient for WS3
    }

    let entry = CATALOG.iter().find(|e| {
        e.public_term == term || e.aliases.iter().any(|a| a.eq_ignore_ascii_case(&term))
    });

    let Some(entry) = entry else {
        return ToolResult {
            exit: CliExit::DiagnosticError,
            stdout: String::new(),
            stderr: format!("error[EXY-CONFIG-0014]: unknown directive `{term}`\n"),
        };
    };

    match req.format {
        OutputFormat::Json => ToolResult {
            exit: CliExit::Ok,
            stdout: explain_json(entry),
            stderr: String::new(),
        },
        OutputFormat::Human => {
            let mut out = String::new();
            out.push_str(&format!("public_term = {}\n", entry.public_term));
            out.push_str(&format!(
                "meaning = {}\n",
                sanitize_control_chars(entry.meaning)
            ));
            out.push_str(&format!("surface = {}\n", entry.surface));
            out.push_str(&format!("ir_mapping = {}\n", entry.ir_mapping));
            out.push_str(&format!("required = {}\n", entry.required));
            if let Some(d) = entry.default {
                out.push_str(&format!("default = {d}\n"));
            }
            if !entry.allowed.is_empty() {
                out.push_str(&format!("allowed_values = {:?}\n", entry.allowed));
            }
            if !entry.codes.is_empty() {
                out.push_str(&format!("related_codes = {:?}\n", entry.codes));
            }
            if let Some(s) = entry.support_limit {
                out.push_str(&format!("support_limit = {s}\n"));
            }
            out.push_str(&format!("documentation = {:?}\n", entry.docs));
            ToolResult {
                exit: CliExit::Ok,
                stdout: out,
                stderr: String::new(),
            }
        }
    }
}
