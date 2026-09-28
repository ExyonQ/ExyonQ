use crate::error::CtrlError;
use crate::types::{VhostCreate, VhostUpstream};
use std::path::{Path, PathBuf};
use toml::value::{Array, Table};
use toml::Value;

const CONTROL_PREFIX: &str = "ctrl-api-";

#[derive(Debug, Clone)]
pub struct MutationSummary {
    pub domain: String,
    pub listen: String,
    pub path: String,
    pub root: Option<String>,
    pub upstream: Option<VhostUpstream>,
}

#[derive(Debug, Clone)]
pub struct StagedMutation {
    pub backup_path: PathBuf,
    pub summary: MutationSummary,
}

pub fn stage_create(config_path: &Path, input: &VhostCreate) -> Result<StagedMutation, CtrlError> {
    let original = std::fs::read_to_string(config_path)?;
    let mut value: Value = toml::from_str(&original)?;
    let summary = create_vhost_in_value(&mut value, input)?;
    write_staged(config_path, &original, &value)?;
    Ok(StagedMutation {
        backup_path: backup_path(config_path),
        summary,
    })
}

pub fn stage_delete(config_path: &Path, domain: &str) -> Result<StagedMutation, CtrlError> {
    let original = std::fs::read_to_string(config_path)?;
    let mut value: Value = toml::from_str(&original)?;
    let summary = delete_vhost_in_value(&mut value, domain)?;
    write_staged(config_path, &original, &value)?;
    Ok(StagedMutation {
        backup_path: backup_path(config_path),
        summary,
    })
}

pub fn restore_backup(config_path: &Path, backup_path: &Path) -> Result<(), CtrlError> {
    std::fs::copy(backup_path, config_path)?;
    Ok(())
}

pub fn create_vhost_in_value(
    value: &mut Value,
    input: &VhostCreate,
) -> Result<MutationSummary, CtrlError> {
    validate_vhost_create(input)?;
    let primary = primary_listen(value)?.to_owned();
    if let Some(listen) = input.listen.as_deref() {
        if listen != primary {
            return Err(CtrlError::BadRequest(
                "vhost listen must equal primary listen".to_owned(),
            ));
        }
    }
    if domain_exists(value, &input.domain) {
        return Err(CtrlError::DuplicateVhost);
    }

    let route_name = route_name(&input.domain);
    let upstream_name = input
        .upstream
        .as_ref()
        .map(|_| upstream_name(&input.domain));

    // Cap013 forbids changing the listen/server-count set on reload (EXY-RELOAD-0005).
    // Cap016 MVP therefore attaches server_name + route to the existing primary server.
    append_route(value, &route_name, input, upstream_name.as_deref())?;
    if let Some(upstream) = &input.upstream {
        append_upstream(
            value,
            upstream_name.as_deref().expect("upstream name"),
            upstream,
        )?;
    }
    attach_to_primary_server(value, &input.domain, &route_name)?;

    Ok(MutationSummary {
        domain: input.domain.clone(),
        listen: primary,
        path: normalized_path(&input.path),
        root: input.root.clone(),
        upstream: input.upstream.clone(),
    })
}

pub fn delete_vhost_in_value(
    value: &mut Value,
    domain: &str,
) -> Result<MutationSummary, CtrlError> {
    let primary = primary_listen(value)?.to_owned();
    let route_name = route_name(domain);
    let upstream_name = upstream_name(domain);
    // Only Cap016-managed routes (ctrl-api-* prefix) may be deleted via this API.
    if !route_exists(value, &route_name) {
        return Err(CtrlError::NotFound);
    }
    detach_from_servers(value, domain, &route_name)?;
    let path = route_path(value, &route_name).unwrap_or_else(|| "/".to_owned());
    remove_named_item(value, "route", |table| {
        table
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name == route_name)
    });
    remove_named_item(value, "upstream", |table| {
        table
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name == upstream_name)
    });
    Ok(MutationSummary {
        domain: domain.to_owned(),
        listen: primary,
        path,
        root: None,
        upstream: None,
    })
}

fn validate_vhost_create(input: &VhostCreate) -> Result<(), CtrlError> {
    if !is_valid_vhost_domain(&input.domain) {
        return Err(CtrlError::BadRequest("invalid vhost domain".to_owned()));
    }
    if input.root.is_some() == input.upstream.is_some() {
        return Err(CtrlError::BadRequest(
            "exactly one of root or upstream is required".to_owned(),
        ));
    }
    if !normalized_path(&input.path).starts_with('/') {
        return Err(CtrlError::BadRequest(
            "vhost path must start with '/'".to_owned(),
        ));
    }
    Ok(())
}

/// OpenAPI `VhostCreate.domain` pattern `^[A-Za-z0-9._*-]+$` (bare `*` rejected).
fn is_valid_vhost_domain(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 253 || domain == "*" {
        return false;
    }
    domain
        .bytes()
        .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'*'))
}

fn primary_listen(value: &Value) -> Result<&str, CtrlError> {
    value
        .get("server")
        .and_then(Value::as_array)
        .and_then(|servers| servers.first())
        .and_then(|server| server.get("listen"))
        .and_then(Value::as_str)
        .ok_or_else(|| CtrlError::BadRequest("missing primary server listen".to_owned()))
}

fn domain_exists(value: &Value, domain: &str) -> bool {
    value
        .get("server")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .any(|server| server_names(server).iter().any(|name| name == domain))
}

fn attach_to_primary_server(
    value: &mut Value,
    domain: &str,
    route_name: &str,
) -> Result<(), CtrlError> {
    let servers = array_mut(value, "server")?;
    let Some(primary) = servers.first_mut().and_then(Value::as_table_mut) else {
        return Err(CtrlError::BadRequest("missing primary server".to_owned()));
    };
    let mut names = server_names_from_table(primary);
    if !names.iter().any(|n| n == domain) {
        names.push(domain.to_owned());
    }
    primary.insert(
        "server_name".to_owned(),
        Value::Array(names.into_iter().map(Value::String).collect()),
    );
    let mut routes = string_array(primary.get("routes")).unwrap_or_default();
    if !routes.iter().any(|r| r == route_name) {
        routes.push(route_name.to_owned());
    }
    primary.insert(
        "routes".to_owned(),
        Value::Array(routes.into_iter().map(Value::String).collect()),
    );
    Ok(())
}

fn detach_from_servers(value: &mut Value, domain: &str, route_name: &str) -> Result<(), CtrlError> {
    let servers = array_mut(value, "server")?;
    let mut found = false;
    for server in servers.iter_mut() {
        let Some(table) = server.as_table_mut() else {
            continue;
        };
        let mut names = server_names_from_table(table);
        let before = names.len();
        names.retain(|n| n != domain);
        if names.len() != before {
            found = true;
            if names.is_empty() {
                table.remove("server_name");
            } else {
                table.insert(
                    "server_name".to_owned(),
                    Value::Array(names.into_iter().map(Value::String).collect()),
                );
            }
        }
        if let Some(routes) = string_array(table.get("routes")) {
            let filtered: Vec<String> = routes.into_iter().filter(|r| r != route_name).collect();
            table.insert(
                "routes".to_owned(),
                Value::Array(filtered.into_iter().map(Value::String).collect()),
            );
        }
    }
    if !found {
        return Err(CtrlError::NotFound);
    }
    Ok(())
}

fn route_exists(value: &Value, route_name: &str) -> bool {
    value
        .get("route")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_table)
        .any(|table| table.get("name").and_then(Value::as_str) == Some(route_name))
}

fn server_names_from_table(table: &Table) -> Vec<String> {
    match table.get("server_name") {
        Some(raw) => string_array(Some(raw))
            .unwrap_or_else(|| raw.as_str().map(|s| vec![s.to_owned()]).unwrap_or_default()),
        None => Vec::new(),
    }
}

fn append_route(
    value: &mut Value,
    route_name: &str,
    input: &VhostCreate,
    upstream_name: Option<&str>,
) -> Result<(), CtrlError> {
    let mut route = Table::new();
    route.insert("name".to_owned(), Value::String(route_name.to_owned()));
    let mut matcher = Table::new();
    matcher.insert(
        "path".to_owned(),
        Value::String(normalized_path(&input.path)),
    );
    // RouteIndex matches Host via match.host — server_name alone is not consulted.
    matcher.insert("host".to_owned(), Value::String(input.domain.clone()));
    route.insert("match".to_owned(), Value::Table(matcher));
    if let Some(root) = &input.root {
        route.insert("root".to_owned(), Value::String(root.clone()));
    }
    if let Some(name) = upstream_name {
        route.insert("upstream".to_owned(), Value::String(name.to_owned()));
    }
    array_mut(value, "route")?.push(Value::Table(route));
    Ok(())
}

fn append_upstream(
    value: &mut Value,
    upstream_name: &str,
    upstream: &VhostUpstream,
) -> Result<(), CtrlError> {
    let mut table = Table::new();
    table.insert("name".to_owned(), Value::String(upstream_name.to_owned()));
    table.insert("target".to_owned(), Value::String(upstream.target.clone()));
    table.insert(
        "timeout_ms".to_owned(),
        Value::Integer(upstream.timeout_ms.try_into().unwrap_or(i64::MAX)),
    );
    array_mut(value, "upstream")?.push(Value::Table(table));
    Ok(())
}

fn array_mut<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Array, CtrlError> {
    let Some(root) = value.as_table_mut() else {
        return Err(CtrlError::BadRequest(
            "config root must be a table".to_owned(),
        ));
    };
    let entry = root
        .entry(key.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    entry
        .as_array_mut()
        .ok_or_else(|| CtrlError::BadRequest(format!("{key} must be an array")))
}

fn remove_named_item<F>(value: &mut Value, key: &str, mut should_remove: F)
where
    F: FnMut(&Table) -> bool,
{
    if let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) {
        items.retain(|item| !item.as_table().is_some_and(&mut should_remove));
    }
}

fn route_path(value: &Value, route_name: &str) -> Option<String> {
    value
        .get("route")?
        .as_array()?
        .iter()
        .filter_map(Value::as_table)
        .find(|table| table.get("name").and_then(Value::as_str) == Some(route_name))?
        .get("match")?
        .get("path")?
        .as_str()
        .map(ToOwned::to_owned)
}

fn server_names(server: &Value) -> Vec<String> {
    let Some(raw) = server.get("server_name") else {
        return Vec::new();
    };
    string_array(Some(raw))
        .unwrap_or_else(|| raw.as_str().map(|s| vec![s.to_owned()]).unwrap_or_default())
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(Value::as_array).map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect()
    })
}

fn normalized_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_owned()
    } else {
        path.to_owned()
    }
}

pub(crate) fn route_name(domain: &str) -> String {
    format!("{CONTROL_PREFIX}route-{}", hex_name(domain))
}

fn upstream_name(domain: &str) -> String {
    format!("{CONTROL_PREFIX}upstream-{}", hex_name(domain))
}

fn hex_name(domain: &str) -> String {
    let mut out = String::with_capacity(domain.len() * 2);
    for byte in domain.as_bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn write_staged(config_path: &Path, original: &str, value: &Value) -> Result<(), CtrlError> {
    let backup = backup_path(config_path);
    let tmp = tmp_path(config_path);
    std::fs::write(&backup, original)?;
    std::fs::write(&tmp, toml::to_string_pretty(value)?)?;
    std::fs::rename(tmp, config_path)?;
    Ok(())
}

fn backup_path(config_path: &Path) -> PathBuf {
    config_path.with_extension("bak")
}

fn tmp_path(config_path: &Path) -> PathBuf {
    config_path.with_extension("tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> Value {
        toml::from_str(
            r#"
config_version = 1

[[server]]
listen = "127.0.0.1:8080"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "/srv/site"
"#,
        )
        .expect("valid base config")
    }

    #[test]
    fn create_sets_match_host_to_domain() {
        let mut config = base_config();
        let input = VhostCreate {
            domain: "example.test".to_owned(),
            listen: None,
            path: "/".to_owned(),
            root: Some("/srv/example".to_owned()),
            upstream: None,
        };
        create_vhost_in_value(&mut config, &input).expect("create");
        let host = config
            .get("route")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_table)
            .find(|table| {
                table.get("name").and_then(Value::as_str) == Some(&route_name("example.test"))
            })
            .and_then(|table| table.get("match"))
            .and_then(Value::as_table)
            .and_then(|m| m.get("host"))
            .and_then(Value::as_str);
        assert_eq!(host, Some("example.test"));
    }

    #[test]
    fn create_rejects_invalid_domain_charset() {
        let mut config = base_config();
        let input = VhostCreate {
            domain: "bad domain".to_owned(),
            listen: None,
            path: "/".to_owned(),
            root: Some("/srv/example".to_owned()),
            upstream: None,
        };
        assert!(matches!(
            create_vhost_in_value(&mut config, &input),
            Err(CtrlError::BadRequest(_))
        ));
    }

    #[test]
    fn create_rejects_duplicate_domain() {
        let mut config = base_config();
        let input = VhostCreate {
            domain: "example.test".to_owned(),
            listen: Some("127.0.0.1:8080".to_owned()),
            path: "/".to_owned(),
            root: Some("/srv/example".to_owned()),
            upstream: None,
        };
        create_vhost_in_value(&mut config, &input).expect("first create");
        assert!(matches!(
            create_vhost_in_value(&mut config, &input),
            Err(CtrlError::DuplicateVhost)
        ));
    }

    #[test]
    fn create_rejects_listen_change() {
        let mut config = base_config();
        let input = VhostCreate {
            domain: "example.test".to_owned(),
            listen: Some("127.0.0.1:9090".to_owned()),
            path: "/".to_owned(),
            root: Some("/srv/example".to_owned()),
            upstream: None,
        };
        assert!(matches!(
            create_vhost_in_value(&mut config, &input),
            Err(CtrlError::BadRequest(_))
        ));
    }

    #[test]
    fn delete_removes_only_control_api_named_objects() {
        let mut config = base_config();
        let input = VhostCreate {
            domain: "example.test".to_owned(),
            listen: None,
            path: "/api".to_owned(),
            root: None,
            upstream: Some(VhostUpstream {
                target: "http://127.0.0.1:9000".to_owned(),
                timeout_ms: 5000,
            }),
        };
        create_vhost_in_value(&mut config, &input).expect("create");
        delete_vhost_in_value(&mut config, "example.test").expect("delete");
        assert!(!domain_exists(&config, "example.test"));
        assert!(config
            .get("route")
            .and_then(Value::as_array)
            .expect("routes")
            .iter()
            .any(|route| route.get("name").and_then(Value::as_str) == Some("site")));
    }
}
