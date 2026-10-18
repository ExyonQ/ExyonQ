//! Q-06 / ADR-059 — effective value provenance for `config explain --path --pointer`.

use crate::output::{sanitize_control_chars, CliExit, OutputFormat, ToolResult};
use exyonq_config_ir::canonical_json;
use exyonq_config_merge::load_with_includes;
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct Binding {
    pointer: String,
    value_display: String,
    file: PathBuf,
    line: u32,
    layer: &'static str,
}

#[derive(Debug, Clone)]
struct LayerOut {
    rank: u32,
    kind: &'static str,
    path: Option<String>,
    line: Option<u32>,
    note: String,
    winner: bool,
}

/// Explain where an effective config key comes from (file:line + precedence).
pub fn explain_provenance(config_path: &Path, pointer: &str, format: OutputFormat) -> ToolResult {
    let pointer = normalize_pointer(pointer);
    if pointer.is_empty() {
        return ToolResult {
            exit: CliExit::UsageOrTool,
            stdout: String::new(),
            stderr: "explain provenance: --pointer KEY is required with --path\n".into(),
        };
    }

    let cfg = match load_with_includes(config_path) {
        Ok(c) => c,
        Err(e) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: sanitize_control_chars(&e.to_string()),
            };
        }
    };

    let effective: JsonValue = match serde_json::from_str(&canonical_json(&cfg)) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult {
                exit: CliExit::UsageOrTool,
                stdout: String::new(),
                stderr: format!("explain provenance: canonical IR parse failed: {e}\n"),
            };
        }
    };

    let value = match resolve_effective(&effective, &pointer) {
        Ok(v) => v,
        Err(msg) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: format!("error[EXY-CONFIG-0015]: {msg}\n"),
            };
        }
    };
    let value_display = json_display(&value);

    let bindings = match collect_bindings(config_path) {
        Ok(b) => b,
        Err(e) => {
            return ToolResult {
                exit: CliExit::DiagnosticError,
                stdout: String::new(),
                stderr: sanitize_control_chars(&e),
            };
        }
    };

    let matching: Vec<&Binding> = bindings.iter().filter(|b| b.pointer == pointer).collect();
    let mut layers: Vec<LayerOut> = Vec::new();
    layers.push(LayerOut {
        rank: 1,
        kind: "default",
        path: None,
        line: None,
        note: "IR serde defaults when key absent from TOML".into(),
        winner: false,
    });

    let mut rank = 2u32;
    for b in &matching {
        layers.push(LayerOut {
            rank,
            kind: b.layer,
            path: Some(b.file.display().to_string()),
            line: Some(b.line),
            note: format!("assignment value {}", b.value_display),
            winner: false,
        });
        rank += 1;
    }

    if matching.is_empty() {
        layers[0].winner = true;
    } else if let Some(l) = layers.last_mut() {
        l.winner = true;
    }

    let origin = layers
        .iter()
        .find(|l| l.winner)
        .cloned()
        .unwrap_or_else(|| layers[0].clone());

    match format {
        OutputFormat::Human => {
            let mut out = String::new();
            out.push_str(&format!("pointer = {pointer}\n"));
            out.push_str(&format!("value = {value_display}\n"));
            out.push_str(&format!("origin.kind = {}\n", origin.kind));
            if let Some(p) = &origin.path {
                out.push_str(&format!("origin.path = {p}\n"));
            }
            if let Some(line) = origin.line {
                out.push_str(&format!("origin.line = {line}\n"));
            }
            out.push_str("precedence =\n");
            for l in &layers {
                let mark = if l.winner { " *" } else { "" };
                match (&l.path, l.line) {
                    (Some(p), Some(line)) => {
                        out.push_str(&format!(
                            "  {}. {} {}:{} — {}{mark}\n",
                            l.rank, l.kind, p, line, l.note
                        ));
                    }
                    _ => {
                        out.push_str(&format!("  {}. {} — {}{mark}\n", l.rank, l.kind, l.note));
                    }
                }
            }
            ToolResult {
                exit: CliExit::Ok,
                stdout: out,
                stderr: String::new(),
            }
        }
        OutputFormat::Json => {
            let mut prec = String::from("[");
            for (i, l) in layers.iter().enumerate() {
                if i > 0 {
                    prec.push(',');
                }
                prec.push_str("\n    {");
                prec.push_str(&format!("\n      \"rank\": {},", l.rank));
                prec.push_str(&format!("\n      \"kind\": \"{}\",", l.kind));
                match &l.path {
                    Some(p) => prec.push_str(&format!("\n      \"path\": \"{}\",", json_escape(p))),
                    None => prec.push_str("\n      \"path\": null,"),
                }
                match l.line {
                    Some(n) => prec.push_str(&format!("\n      \"line\": {n},")),
                    None => prec.push_str("\n      \"line\": null,"),
                }
                prec.push_str(&format!("\n      \"note\": \"{}\",", json_escape(&l.note)));
                prec.push_str(&format!("\n      \"winner\": {}", l.winner));
                prec.push_str("\n    }");
            }
            prec.push_str("\n  ]");
            let origin_path = origin
                .path
                .as_ref()
                .map(|p| format!("\"{}\"", json_escape(p)))
                .unwrap_or_else(|| "null".into());
            let origin_line = origin
                .line
                .map(|n| n.to_string())
                .unwrap_or_else(|| "null".into());
            ToolResult {
                exit: CliExit::Ok,
                stdout: format!(
                    "{{\n  \"schema_version\": 1,\n  \"pointer\": \"{}\",\n  \"value\": {},\n  \"origin\": {{\n    \"kind\": \"{}\",\n    \"path\": {origin_path},\n    \"line\": {origin_line}\n  }},\n  \"precedence\": {prec}\n}}",
                    json_escape(&pointer),
                    value,
                    origin.kind,
                ),
                stderr: String::new(),
            }
        }
    }
}

fn normalize_pointer(raw: &str) -> String {
    raw.trim().trim_start_matches('/').replace('/', ".")
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

fn json_display(v: &JsonValue) -> String {
    match v {
        JsonValue::String(s) => s.clone(),
        JsonValue::Null => "null".into(),
        other => other.to_string(),
    }
}

fn resolve_effective(root: &JsonValue, pointer: &str) -> Result<JsonValue, String> {
    let segs = parse_pointer(pointer)?;
    let mut cur = root;
    for seg in &segs {
        match seg {
            Seg::Key(k) => {
                cur = cur
                    .get(k)
                    .ok_or_else(|| format!("unknown key `{pointer}` (missing `{k}`)"))?;
            }
            Seg::Index(i) => {
                cur = cur
                    .as_array()
                    .and_then(|a| a.get(*i))
                    .ok_or_else(|| format!("unknown key `{pointer}` (index {i} out of range)"))?;
            }
            Seg::Name(field, name) => {
                let arr = cur
                    .as_array()
                    .ok_or_else(|| format!("unknown key `{pointer}` (expected array before name=)"))?;
                let found = arr.iter().find(|el| {
                    el.get(field)
                        .and_then(|v| v.as_str())
                        .map(|s| s == name)
                        .unwrap_or(false)
                });
                // After name match we are ON the element; next segs are fields.
                // Represent name selector as selecting the element — field name is always "name" for our grammar route[name=x]
                let _ = field;
                cur = found.ok_or_else(|| {
                    format!("unknown key `{pointer}` (no element with name=`{name}`)")
                })?;
            }
        }
    }
    Ok(cur.clone())
}

#[derive(Debug)]
enum Seg {
    Key(String),
    Index(usize),
    /// Select array element where `name` field equals value — used as `route[name=main]`
    Name(&'static str, String),
}

fn parse_pointer(pointer: &str) -> Result<Vec<Seg>, String> {
    let mut segs = Vec::new();
    for part in pointer.split('.') {
        if part.is_empty() {
            return Err("empty pointer segment".into());
        }
        if let Some((head, rest)) = part.split_once('[') {
            if !head.is_empty() {
                segs.push(Seg::Key(head.to_string()));
            }
            let inner = rest
                .strip_suffix(']')
                .ok_or_else(|| format!("unclosed selector in `{part}`"))?;
            if let Some(name) = inner.strip_prefix("name=") {
                let name = name.trim_matches('"').trim_matches('\'').to_string();
                segs.push(Seg::Name("name", name));
            } else {
                let idx: usize = inner
                    .parse()
                    .map_err(|_| format!("bad index `{inner}` in `{part}`"))?;
                segs.push(Seg::Index(idx));
            }
        } else {
            segs.push(Seg::Key(part.to_string()));
        }
    }
    Ok(segs)
}

fn collect_bindings(config_path: &Path) -> Result<Vec<Binding>, String> {
    let root_src = std::fs::read_to_string(config_path)
        .map_err(|e| format!("read {}: {e}", config_path.display()))?;
    let root_val: toml::Value =
        toml::from_str(&root_src).map_err(|e| format!("parse {}: {e}", config_path.display()))?;

    let mut out = Vec::new();
    index_value(
        &root_src,
        config_path,
        "root",
        &root_val,
        &mut out,
    );

    let includes = root_val
        .get("include")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let base = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    for inc in includes {
        let Some(rel) = inc.as_str() else {
            continue;
        };
        let frag_path = base.join(rel);
        let frag_src = std::fs::read_to_string(&frag_path)
            .map_err(|e| format!("read {}: {e}", frag_path.display()))?;
        let frag_val: toml::Value = toml::from_str(&frag_src)
            .map_err(|e| format!("parse {}: {e}", frag_path.display()))?;
        index_value(&frag_src, &frag_path, "include", &frag_val, &mut out);
    }
    Ok(out)
}

fn index_value(
    src: &str,
    file: &Path,
    layer: &'static str,
    value: &toml::Value,
    out: &mut Vec<Binding>,
) {
    // Scalars / tables at document root via line scanner (more reliable for line #).
    scan_document(src, file, layer, out);
    let _ = value; // parse validates; lines come from scan
}

fn scan_document(src: &str, file: &Path, layer: &'static str, out: &mut Vec<Binding>) {
    let mut table_path: Vec<String> = Vec::new();
    let mut aot_kind: Option<String> = None;
    let mut aot_buf: Vec<(String, String, u32)> = Vec::new();
    let mut aot_name: Option<String> = None;

    let flush_aot = |kind: &str,
                     name: &Option<String>,
                     buf: &[(String, String, u32)],
                     out: &mut Vec<Binding>,
                     file: &Path,
                     layer: &'static str| {
        let Some(name) = name else {
            return;
        };
        for (k, v, line) in buf {
            out.push(Binding {
                pointer: format!("{kind}[name={name}].{k}"),
                value_display: v.clone(),
                file: file.to_path_buf(),
                line: *line,
                layer,
            });
        }
    };

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let stripped = strip_comment(raw_line);
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("[[") && line.ends_with("]]") {
            if let Some(kind) = aot_kind.take() {
                flush_aot(&kind, &aot_name, &aot_buf, out, file, layer);
            }
            aot_buf.clear();
            aot_name = None;
            let kind = line.trim_start_matches('[').trim_end_matches(']').trim();
            let kind = kind.trim_start_matches('[').trim_end_matches(']');
            aot_kind = Some(kind.to_string());
            table_path.clear();
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && !line.starts_with("[[") {
            if let Some(kind) = aot_kind.take() {
                flush_aot(&kind, &aot_name, &aot_buf, out, file, layer);
                aot_buf.clear();
                aot_name = None;
            }
            let path = line.trim_start_matches('[').trim_end_matches(']').trim();
            table_path = path
                .split('.')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            continue;
        }
        if let Some((key, val)) = split_assignment(line) {
            if let Some(kind) = &aot_kind {
                if key == "name" {
                    aot_name = Some(unquote(val));
                }
                aot_buf.push((key.to_string(), unquote(val), line_no));
                let _ = kind;
                continue;
            }
            let mut pointer = table_path.join(".");
            if !pointer.is_empty() {
                pointer.push('.');
            }
            pointer.push_str(key);
            out.push(Binding {
                pointer,
                value_display: unquote(val),
                file: file.to_path_buf(),
                line: line_no,
                layer,
            });
        }
    }
    if let Some(kind) = aot_kind.take() {
        flush_aot(&kind, &aot_name, &aot_buf, out, file, layer);
    }
}

fn strip_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            in_str = !in_str;
            out.push(c);
            continue;
        }
        if c == '#' && !in_str {
            break;
        }
        out.push(c);
    }
    out
}

fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    let val = line[eq + 1..].trim();
    if key.is_empty() || key.contains('[') {
        return None;
    }
    Some((key, val))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        return s[1..s.len() - 1].replace("\\\"", "\"");
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "exyonq-q06-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn provenance_root_scalar_line() {
        let dir = tmp();
        let main = dir.join("main.toml");
        fs::write(
            &main,
            r#"config_version = 2
[[server]]
listen = "127.0.0.1:0"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#,
        )
        .unwrap();
        let r = explain_provenance(&main, "config_version", OutputFormat::Human);
        assert_eq!(r.exit, CliExit::Ok, "{}", r.stderr);
        assert!(r.stdout.contains("value = 2"), "{}", r.stdout);
        assert!(r.stdout.contains("origin.line = 1"), "{}", r.stdout);
        assert!(r.stdout.contains("origin.kind = root"), "{}", r.stdout);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn provenance_include_named_route() {
        let dir = tmp();
        let frag = dir.join("frag.toml");
        fs::write(
            &frag,
            r#"
[[route]]
name = "extra"
match = { path = "/x" }
root = "/from-include"
"#,
        )
        .unwrap();
        let main = dir.join("main.toml");
        fs::write(
            &main,
            r#"config_version = 2
include = ["frag.toml"]
[[server]]
listen = "127.0.0.1:0"
routes = ["r", "extra"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#,
        )
        .unwrap();
        let r = explain_provenance(&main, "route[name=extra].root", OutputFormat::Human);
        assert_eq!(r.exit, CliExit::Ok, "{}", r.stderr);
        assert!(r.stdout.contains("/from-include"), "{}", r.stdout);
        assert!(r.stdout.contains("origin.kind = include"), "{}", r.stdout);
        assert!(r.stdout.contains("frag.toml"), "{}", r.stdout);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn provenance_default_when_absent() {
        let dir = tmp();
        let main = dir.join("main.toml");
        fs::write(
            &main,
            r#"config_version = 2
[[server]]
listen = "127.0.0.1:0"
routes = ["r"]
[[route]]
name = "r"
match = { path = "/" }
root = "/tmp"
"#,
        )
        .unwrap();
        let r = explain_provenance(&main, "full_page_cache.enabled", OutputFormat::Json);
        assert_eq!(r.exit, CliExit::Ok, "{}", r.stderr);
        assert!(r.stdout.contains("\"kind\": \"default\""), "{}", r.stdout);
        assert!(r.stdout.contains("\"winner\": true"), "{}", r.stdout);
        let _ = fs::remove_dir_all(dir);
    }
}
