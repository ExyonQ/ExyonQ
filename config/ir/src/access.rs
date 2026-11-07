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
//! Declarative per-route access policy (AP-05).

use crate::ConfigError;
use ipnetwork::IpNetwork;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;

/// Maximum number of allow or deny network entries per route.
pub const MAX_ACCESS_NETWORKS: usize = 64;
/// Maximum number of allowed methods per route.
pub const MAX_ACCESS_METHODS: usize = 16;
/// Maximum number of ordered allow/deny rules per route.
pub const MAX_ACCESS_RULES: usize = 128;

/// One ordered nginx-style access rule. The first matching rule wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteAccessEffect {
    Allow,
    Deny,
}

/// Ordered access rule used when source-order semantics matter (nginx).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteAccessRule {
    pub effect: RouteAccessEffect,
    /// IP/CIDR or `all`.
    pub source: String,
}

/// Per-route access policy. `deny` wins; a non-empty `allow` list is an
/// allowlist. Methods and schedule are additional AND constraints.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteAccessConfig {
    /// IPs/CIDRs allowed to reach the route. Empty = no IP allowlist.
    #[serde(default)]
    pub allow: Vec<String>,
    /// IPs/CIDRs denied from the route. Deny wins over allow.
    #[serde(default)]
    pub deny: Vec<String>,
    /// Ordered first-match rules. Mutually exclusive with `allow`/`deny`.
    #[serde(default)]
    pub rules: Vec<RouteAccessRule>,
    /// HTTP methods allowed on the route. Empty = all methods.
    #[serde(default)]
    pub methods: Vec<String>,
    /// Optional UTC time window. Absent = no time restriction.
    #[serde(default)]
    pub schedule: Option<RouteAccessSchedule>,
}

/// UTC schedule for a route. A window whose end is earlier than its start
/// crosses midnight and belongs to the configured start day.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouteAccessSchedule {
    /// `mon`..`sun` or full day names. Empty = every day.
    #[serde(default)]
    pub days: Vec<String>,
    /// `HH:MM`, UTC.
    pub start: String,
    /// `HH:MM`, UTC. `start == end` means all day.
    pub end: String,
}

impl RouteAccessConfig {
    /// True when this policy rejects every client and method unconditionally.
    /// Such a policy is a complete route terminal: no upstream, root, redirect,
    /// rewrite or FastCGI backend is required. A conditional policy (allowlist,
    /// method filter or schedule) is not a deny-all terminal.
    pub fn denies_all(&self) -> bool {
        if !self.allow.is_empty() || !self.methods.is_empty() || self.schedule.is_some() {
            return false;
        }
        if self.rules.is_empty() {
            return self
                .deny
                .iter()
                .any(|value| value.trim().eq_ignore_ascii_case("all"));
        }
        !self
            .rules
            .iter()
            .any(|rule| matches!(rule.effect, RouteAccessEffect::Allow))
            && self
                .rules
                .iter()
                .any(|rule| rule.source.trim().eq_ignore_ascii_case("all"))
    }

    pub fn validate(&self, route: &str) -> Result<(), ConfigError> {
        if self.allow.is_empty()
            && self.deny.is_empty()
            && self.rules.is_empty()
            && self.methods.is_empty()
            && self.schedule.is_none()
        {
            return Err(ConfigError::Parse(format!(
                "route `{route}`: access requires allow, deny, rules, methods or schedule"
            )));
        }
        if !self.rules.is_empty() && (!self.allow.is_empty() || !self.deny.is_empty()) {
            return Err(ConfigError::Parse(format!(
                "route `{route}`: access.rules cannot be combined with allow/deny"
            )));
        }
        if self.rules.len() > MAX_ACCESS_RULES {
            return Err(ConfigError::Parse(format!(
                "route `{route}`: access.rules exceeds max {MAX_ACCESS_RULES}"
            )));
        }
        let mut rule_keys = HashSet::new();
        for rule in &self.rules {
            let source = rule.source.trim();
            if source.is_empty() {
                return Err(ConfigError::Parse(format!(
                    "route `{route}`: access.rules contains an empty source"
                )));
            }
            let key = if source.eq_ignore_ascii_case("all") {
                "all".to_string()
            } else {
                parse_network(source)
                    .ok_or_else(|| {
                        ConfigError::Parse(format!(
                            "route `{route}`: access.rules source `{source}` is not an IP/CIDR"
                        ))
                    })?
                    .to_string()
            };
            let effect = match rule.effect {
                RouteAccessEffect::Allow => "allow",
                RouteAccessEffect::Deny => "deny",
            };
            if !rule_keys.insert((effect, key)) {
                return Err(ConfigError::Parse(format!(
                    "route `{route}`: duplicate access.rules entry `{effect} {source}`"
                )));
            }
        }
        validate_networks(route, "allow", &self.allow)?;
        validate_networks(route, "deny", &self.deny)?;
        if self.methods.len() > MAX_ACCESS_METHODS {
            return Err(ConfigError::Parse(format!(
                "route `{route}`: access.methods exceeds max {MAX_ACCESS_METHODS}"
            )));
        }
        let mut methods = HashSet::new();
        for method in &self.methods {
            let method = method.trim();
            if method.is_empty()
                || !method
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            {
                return Err(ConfigError::Parse(format!(
                    "route `{route}`: access.methods entry `{method}` is not an HTTP token"
                )));
            }
            if !methods.insert(method.to_ascii_uppercase()) {
                return Err(ConfigError::Parse(format!(
                    "route `{route}`: duplicate access.methods entry `{method}`"
                )));
            }
        }
        if let Some(schedule) = &self.schedule {
            schedule.validate(route)?;
        }
        Ok(())
    }
}

impl RouteAccessSchedule {
    fn validate(&self, route: &str) -> Result<(), ConfigError> {
        let mut days = HashSet::new();
        for day in &self.days {
            let idx = access_day_index(day).ok_or_else(|| {
                ConfigError::Parse(format!(
                    "route `{route}`: access.schedule.days entry `{day}` is not a weekday"
                ))
            })?;
            if !days.insert(idx) {
                return Err(ConfigError::Parse(format!(
                    "route `{route}`: duplicate access.schedule.days entry `{day}`"
                )));
            }
        }
        parse_hhmm(&self.start).ok_or_else(|| {
            ConfigError::Parse(format!(
                "route `{route}`: access.schedule.start `{}` must be HH:MM",
                self.start
            ))
        })?;
        parse_hhmm(&self.end).ok_or_else(|| {
            ConfigError::Parse(format!(
                "route `{route}`: access.schedule.end `{}` must be HH:MM",
                self.end
            ))
        })?;
        Ok(())
    }
}

fn validate_networks(route: &str, field: &str, values: &[String]) -> Result<(), ConfigError> {
    if values.len() > MAX_ACCESS_NETWORKS {
        return Err(ConfigError::Parse(format!(
            "route `{route}`: access.{field} exceeds max {MAX_ACCESS_NETWORKS}"
        )));
    }
    let mut seen = HashSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            return Err(ConfigError::Parse(format!(
                "route `{route}`: access.{field} contains an empty entry"
            )));
        }
        let key = if value.eq_ignore_ascii_case("all") {
            "all".to_string()
        } else {
            parse_network(value)
                .ok_or_else(|| {
                    ConfigError::Parse(format!(
                        "route `{route}`: access.{field} entry `{value}` is not an IP/CIDR"
                    ))
                })?
                .to_string()
        };
        if !seen.insert(key) {
            return Err(ConfigError::Parse(format!(
                "route `{route}`: duplicate access.{field} entry `{value}`"
            )));
        }
    }
    Ok(())
}

pub fn parse_network(value: &str) -> Option<IpNetwork> {
    value
        .parse::<IpNetwork>()
        .ok()
        .or_else(|| value.parse::<IpAddr>().ok().map(IpNetwork::from))
}

pub fn parse_hhmm(value: &str) -> Option<u16> {
    let (h, m) = value.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let h: u16 = h.parse().ok()?;
    let m: u16 = m.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

pub fn access_day_index(value: &str) -> Option<u8> {
    match value.trim().to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(0),
        "tue" | "tues" | "tuesday" => Some(1),
        "wed" | "wednesday" => Some(2),
        "thu" | "thur" | "thurs" | "thursday" => Some(3),
        "fri" | "friday" => Some(4),
        "sat" | "saturday" => Some(5),
        "sun" | "sunday" => Some(6),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> RouteAccessConfig {
        RouteAccessConfig {
            allow: vec!["10.0.0.0/8".into()],
            deny: vec!["2001:db8::/32".into()],
            rules: Vec::new(),
            methods: vec!["get".into()],
            schedule: Some(RouteAccessSchedule {
                days: vec!["mon".into()],
                start: "22:00".into(),
                end: "02:00".into(),
            }),
        }
    }

    #[test]
    fn accepts_valid_route_access() {
        valid().validate("site").expect("valid access");
    }

    #[test]
    fn deny_all_is_a_terminal_only_when_unconditional() {
        let deny_all = RouteAccessConfig {
            deny: vec!["all".into()],
            ..Default::default()
        };
        assert!(deny_all.denies_all());

        let ordered = RouteAccessConfig {
            rules: vec![RouteAccessRule {
                effect: RouteAccessEffect::Deny,
                source: "all".into(),
            }],
            ..Default::default()
        };
        assert!(ordered.denies_all());

        let allow_then_deny = RouteAccessConfig {
            rules: vec![
                RouteAccessRule {
                    effect: RouteAccessEffect::Allow,
                    source: "10.0.0.0/8".into(),
                },
                RouteAccessRule {
                    effect: RouteAccessEffect::Deny,
                    source: "all".into(),
                },
            ],
            ..Default::default()
        };
        assert!(!allow_then_deny.denies_all());

        let conditional = RouteAccessConfig {
            deny: vec!["all".into()],
            methods: vec!["GET".into()],
            ..Default::default()
        };
        assert!(!conditional.denies_all());
    }

    #[test]
    fn rejects_empty_policy() {
        let err = RouteAccessConfig::default()
            .validate("site")
            .expect_err("empty access");
        assert!(err
            .to_string()
            .contains("requires allow, deny, rules, methods or schedule"));
    }

    #[test]
    fn ordered_rules_are_valid_and_mutually_exclusive() {
        let mut cfg = RouteAccessConfig {
            allow: Vec::new(),
            deny: Vec::new(),
            rules: vec![
                RouteAccessRule {
                    effect: RouteAccessEffect::Allow,
                    source: "10.0.0.0/8".into(),
                },
                RouteAccessRule {
                    effect: RouteAccessEffect::Deny,
                    source: "all".into(),
                },
            ],
            methods: Vec::new(),
            schedule: None,
        };
        cfg.validate("site").expect("ordered rules");
        cfg.allow.push("192.0.2.1".into());
        assert!(cfg.validate("site").is_err());
    }

    #[test]
    fn rejects_bad_or_duplicate_networks() {
        let mut cfg = valid();
        cfg.allow.push("10.0.0.0/33".into());
        assert!(cfg.validate("site").is_err());

        let mut cfg = valid();
        cfg.allow.push("10.0.0.0/8".into());
        assert!(cfg.validate("site").is_err());
    }

    #[test]
    fn rejects_bad_or_duplicate_methods() {
        let mut cfg = valid();
        cfg.methods.push("GET".into());
        assert!(cfg.validate("site").is_err());

        let mut cfg = valid();
        cfg.methods.push("BAD METHOD".into());
        assert!(cfg.validate("site").is_err());
    }

    #[test]
    fn rejects_bad_schedule() {
        let mut cfg = valid();
        cfg.schedule.as_mut().unwrap().days.push("monday".into());
        assert!(cfg.validate("site").is_err());

        let mut cfg = valid();
        cfg.schedule.as_mut().unwrap().start = "24:00".into();
        assert!(cfg.validate("site").is_err());
    }
}
