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
//! Serverfile formatting and directive help.

use crate::parse::parse_serverfile;

pub fn fmt_serverfile(input: &str) -> Result<String, crate::parse::ParseError> {
    let ast = parse_serverfile(input)?;
    let mut out = String::new();
    out.push_str(&ast.listener);
    out.push_str(" {\n");
    for route in &ast.routes {
        out.push_str("    route ");
        out.push_str(&route.path);
        out.push_str(" {\n");
        if let Some(proxy) = &route.proxy {
            out.push_str("        proxy ");
            out.push_str(&proxy.upstream_name);
            out.push(' ');
            out.push_str(&proxy.port.to_string());
            out.push('\n');
        }
        if let Some(root) = &route.root {
            out.push_str("        root ");
            out.push_str(root);
            out.push('\n');
        }
        if let Some(index) = &route.index {
            out.push_str("        index ");
            out.push_str(index);
            out.push('\n');
        }
        out.push_str("    }\n");
    }
    if ast.metrics {
        out.push_str("    metrics on\n");
    }
    if ast.compression {
        out.push_str("    compression on\n");
    }
    if let Some((rps, burst)) = ast.ratelimit {
        out.push_str(&format!("    ratelimit {rps}r/s burst {burst}\n"));
    }
    out.push_str("}\n");
    Ok(out)
}

pub fn explain_directive(name: &str) -> Option<&'static str> {
    match name {
        "route" => Some("Declares a path block with proxy, root, or index actions."),
        "proxy" => Some("Sets reverse proxy upstream name and port (maps to [[upstream]] in IR)."),
        "root" => Some("Static files root directory for the route prefix."),
        "index" => Some("Default index file inside a static root."),
        "metrics" => Some("Enables Prometheus metrics module ([modules.metrics])."),
        "compression" => Some("Enables response compression module."),
        "ratelimit" => Some("Enables token-bucket rate limiting (IR modules.ratelimit)."),
        "timeout" => Some("Optional proxy timeout inside a route block (e.g. 30s)."),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_roundtrip_parses() {
        let input = ":8080 {\n    route /api { proxy backend 9000 }\n}\n";
        let formatted = fmt_serverfile(input).unwrap();
        parse_serverfile(&formatted).unwrap();
    }
}
