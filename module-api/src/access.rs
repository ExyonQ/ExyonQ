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
//! Compiled route/overlay access policy (AP-05).

use exyonq_config_ir::{
    access_day_index, parse_hhmm, parse_network, RouteAccessConfig, RouteAccessEffect,
    RouteAccessRule, RouteAccessSchedule,
};
use ipnetwork::IpNetwork;
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
enum AccessNet {
    Any,
    Network(IpNetwork),
}

impl AccessNet {
    fn parse(value: &str) -> Result<Self, String> {
        if value.trim().eq_ignore_ascii_case("all") {
            return Ok(Self::Any);
        }
        parse_network(value.trim())
            .map(Self::Network)
            .ok_or_else(|| format!("invalid IP/CIDR `{value}`"))
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match self {
            Self::Any => true,
            Self::Network(network) => network.contains(ip),
        }
    }
}

/// Precompiled access policy; safe to evaluate without parsing on the request path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAccessPolicy {
    allow: Vec<AccessNet>,
    deny: Vec<AccessNet>,
    rules: Vec<CompiledAccessRule>,
    methods: Vec<String>,
    schedule: Option<CompiledAccessSchedule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledAccessRule {
    allow: bool,
    network: AccessNet,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompiledAccessSchedule {
    days: u8,
    start: u16,
    end: u16,
}

impl CompiledAccessPolicy {
    /// Compile a validated IR policy. Errors are defensive: IR validation runs first.
    pub fn compile(config: &RouteAccessConfig) -> Result<Self, String> {
        let schedule = config
            .schedule
            .as_ref()
            .map(CompiledAccessSchedule::compile)
            .transpose()?;
        Self::compile_parts(
            &config.allow,
            &config.deny,
            &config.rules,
            &config.methods,
            schedule,
        )
    }

    /// Compile `.htaccess` access directives (no schedule support in the overlay).
    pub fn from_parts(
        allow: &[String],
        deny: &[String],
        methods: &[String],
    ) -> Result<Self, String> {
        Self::compile_parts(allow, deny, &[], methods, None)
    }

    fn compile_parts(
        allow: &[String],
        deny: &[String],
        rules: &[RouteAccessRule],
        methods: &[String],
        schedule: Option<CompiledAccessSchedule>,
    ) -> Result<Self, String> {
        if allow.is_empty()
            && deny.is_empty()
            && rules.is_empty()
            && methods.is_empty()
            && schedule.is_none()
        {
            return Err("access policy has no restrictions".into());
        }
        if !rules.is_empty() && (!allow.is_empty() || !deny.is_empty()) {
            return Err("access.rules cannot be combined with allow/deny".into());
        }
        let allow = allow
            .iter()
            .map(|value| AccessNet::parse(value))
            .collect::<Result<Vec<_>, _>>()?;
        let deny = deny
            .iter()
            .map(|value| AccessNet::parse(value))
            .collect::<Result<Vec<_>, _>>()?;
        let rules = rules
            .iter()
            .map(|rule| {
                Ok(CompiledAccessRule {
                    allow: matches!(rule.effect, RouteAccessEffect::Allow),
                    network: AccessNet::parse(&rule.source)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut normalized_methods = Vec::with_capacity(methods.len());
        for method in methods {
            let method = method.trim();
            if method.is_empty()
                || !method
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            {
                return Err(format!("invalid HTTP method `{method}`"));
            }
            normalized_methods.push(method.to_ascii_uppercase());
        }
        Ok(Self {
            allow,
            deny,
            rules,
            methods: normalized_methods,
            schedule,
        })
    }

    /// Evaluate against the current UTC clock.
    pub fn allows(&self, ip: IpAddr, method: &str) -> bool {
        self.allows_at(ip, method, SystemTime::now())
    }

    /// Deterministic evaluation point used by tests and the request path.
    pub fn allows_at(&self, ip: IpAddr, method: &str, now: SystemTime) -> bool {
        if self.rules.is_empty() {
            if self.deny.iter().any(|network| network.contains(ip)) {
                return false;
            }
            if !self.allow.is_empty() && !self.allow.iter().any(|network| network.contains(ip)) {
                return false;
            }
        } else {
            let allowed = self
                .rules
                .iter()
                .find(|rule| rule.network.contains(ip))
                .map(|rule| rule.allow)
                .unwrap_or(true);
            if !allowed {
                return false;
            }
        }
        if !self.methods.is_empty()
            && !self
                .methods
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(method))
        {
            return false;
        }
        if let Some(schedule) = &self.schedule {
            if !schedule.allows(now) {
                return false;
            }
        }
        true
    }
}

impl CompiledAccessSchedule {
    fn compile(config: &RouteAccessSchedule) -> Result<Self, String> {
        let mut days = 0u8;
        if config.days.is_empty() {
            days = 0x7f;
        } else {
            for day in &config.days {
                let idx = access_day_index(day)
                    .ok_or_else(|| format!("invalid access.schedule day `{day}`"))?;
                days |= 1 << idx;
            }
        }
        let start = parse_hhmm(&config.start)
            .ok_or_else(|| format!("invalid access.schedule.start `{}`", config.start))?;
        let end = parse_hhmm(&config.end)
            .ok_or_else(|| format!("invalid access.schedule.end `{}`", config.end))?;
        Ok(Self { days, start, end })
    }

    fn allows(&self, now: SystemTime) -> bool {
        let Some((weekday, minute)) = utc_weekday_minute(now) else {
            return false;
        };
        let today = self.days & (1 << weekday) != 0;
        if self.start == self.end {
            return today;
        }
        if self.start < self.end {
            return today && minute >= self.start && minute < self.end;
        }
        // Overnight window: the configured day owns the pre-midnight part;
        // the following day owns the post-midnight part.
        let yesterday = (weekday + 6) % 7;
        (today && minute >= self.start) || (self.days & (1 << yesterday) != 0 && minute < self.end)
    }
}

fn utc_weekday_minute(now: SystemTime) -> Option<(usize, u16)> {
    let secs = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let days = secs / 86_400;
    let minute = ((secs % 86_400) / 60) as u16;
    // 1970-01-01 was Thursday; Monday = 0.
    let weekday = ((days + 3) % 7) as usize;
    Some((weekday, minute))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn policy(allow: &[&str], deny: &[&str], methods: &[&str]) -> CompiledAccessPolicy {
        CompiledAccessPolicy::from_parts(
            &allow.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &deny.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &methods.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        )
        .expect("policy")
    }

    #[test]
    fn deny_wins_and_allow_is_allowlist() {
        let p = policy(&["10.0.0.0/8"], &["10.1.0.0/16"], &[]);
        assert!(p.allows("10.2.3.4".parse().unwrap(), "GET"));
        assert!(!p.allows("10.1.3.4".parse().unwrap(), "GET"));
        assert!(!p.allows("192.0.2.1".parse().unwrap(), "GET"));
    }

    #[test]
    fn methods_are_case_insensitive() {
        let p = policy(&[], &[], &["get", "HEAD"]);
        assert!(p.allows("192.0.2.1".parse().unwrap(), "GET"));
        assert!(p.allows("192.0.2.1".parse().unwrap(), "head"));
        assert!(!p.allows("192.0.2.1".parse().unwrap(), "POST"));
    }

    #[test]
    fn ordered_rules_preserve_first_match() {
        let cfg = RouteAccessConfig {
            allow: vec![],
            deny: vec![],
            rules: vec![
                RouteAccessRule {
                    effect: RouteAccessEffect::Deny,
                    source: "10.1.0.0/16".into(),
                },
                RouteAccessRule {
                    effect: RouteAccessEffect::Allow,
                    source: "10.0.0.0/8".into(),
                },
                RouteAccessRule {
                    effect: RouteAccessEffect::Allow,
                    source: "all".into(),
                },
            ],
            methods: vec![],
            schedule: None,
        };
        let p = CompiledAccessPolicy::compile(&cfg).expect("ordered policy");
        assert!(p.allows("10.2.3.4".parse().unwrap(), "GET"));
        assert!(!p.allows("10.1.3.4".parse().unwrap(), "GET"));
        assert!(p.allows("192.0.2.1".parse().unwrap(), "GET"));
    }

    #[test]
    fn schedule_utc_and_overnight() {
        // 2026-11-02 00:30 UTC is a Monday.
        let monday_0030 = 1_793_579_400;
        let monday_2330 = 1_793_662_200;
        let tuesday_0130 = monday_0030 + 25 * 3600;
        let cfg = RouteAccessConfig {
            allow: vec![],
            deny: vec![],
            rules: vec![],
            methods: vec![],
            schedule: Some(RouteAccessSchedule {
                days: vec!["mon".into()],
                start: "22:00".into(),
                end: "02:00".into(),
            }),
        };
        let p = CompiledAccessPolicy::compile(&cfg).expect("compile");
        assert!(p.allows_at("192.0.2.1".parse().unwrap(), "GET", at(monday_2330)));
        assert!(p.allows_at("192.0.2.1".parse().unwrap(), "GET", at(tuesday_0130)));
        assert!(!p.allows_at("192.0.2.1".parse().unwrap(), "GET", at(monday_0030)));
        assert!(!p.allows_at(
            "192.0.2.1".parse().unwrap(),
            "GET",
            at(monday_0030 + 3 * 3600)
        ));
    }
}
