//! Phase-3 WAF projection bytes (`CFDWAF01`) — compiled-input wire format.
//!
//! Control validates via `exyonq-waf::compile_waf` then encodes this blob.
//! Dataplane decodes → `WafCompileInput` → `FixedWafEngine` (same compile path).
//! Body-target rules are rejected (NOT_SUPPORTED_IN_PHASE3).

use thiserror::Error;

const MAGIC: &[u8; 8] = b"CFDWAF01";
const MAX_RULES: usize = 512;
const MAX_STR: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfdWafProjection {
    pub enabled: bool,
    /// 0=Disabled 1=Monitor 2=Block
    pub mode: u8,
    pub builtins: CfdBuiltinToggles,
    pub ip_filter: CfdIpFilter,
    pub rules: Vec<CfdWafRule>,
    pub exclusions: Vec<CfdWafExclusion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfdBuiltinToggles {
    pub sql_injection: CfdToggle,
    pub xss: CfdToggle,
    pub path_traversal: CfdToggle,
    pub command_injection: CfdToggle,
    pub header_anomaly: CfdToggle,
    pub bot_ua: CfdToggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfdToggle {
    pub enabled: bool,
    /// 0=Allow 1=Log 2=Block (Challenge/RateLimit forbidden on signature)
    pub action: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CfdIpFilter {
    pub enabled: bool,
    pub whitelist: Vec<String>,
    pub blacklist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfdWafRule {
    pub id: String,
    pub enabled: bool,
    /// bit0=RequestHeaders (Phase3 only requires headers phase)
    pub phases_mask: u8,
    /// 0=Path 1=Query 2=Header 3=Body(FORBIDDEN)
    pub target_kind: u8,
    pub header_name: String,
    pub pattern: String,
    /// 0=Allow 1=Log 2=Block
    pub action: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfdWafExclusion {
    pub host: Option<String>,
    pub path_prefix: Option<String>,
    pub method: Option<String>,
    pub rule_ids: Vec<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WafProjError {
    #[error("invalid magic")]
    InvalidMagic,
    #[error("truncated waf projection")]
    Truncated,
    #[error("utf8")]
    Utf8,
    #[error("string too long")]
    StringTooLong,
    #[error("too many rules")]
    TooManyRules,
    #[error("body target not supported in Phase3")]
    BodyDeferred,
    #[error("invalid mode/action")]
    InvalidEnum,
}

impl Default for CfdBuiltinToggles {
    fn default() -> Self {
        Self {
            sql_injection: CfdToggle {
                enabled: true,
                action: 2,
            },
            xss: CfdToggle {
                enabled: true,
                action: 2,
            },
            path_traversal: CfdToggle {
                enabled: true,
                action: 2,
            },
            command_injection: CfdToggle {
                enabled: true,
                action: 2,
            },
            header_anomaly: CfdToggle {
                enabled: true,
                action: 1,
            },
            bot_ua: CfdToggle {
                enabled: true,
                action: 1,
            },
        }
    }
}

impl CfdWafProjection {
    pub fn encode(&self) -> Result<Vec<u8>, WafProjError> {
        if self.rules.len() > MAX_RULES {
            return Err(WafProjError::TooManyRules);
        }
        for r in &self.rules {
            if r.target_kind == 3 {
                return Err(WafProjError::BodyDeferred);
            }
        }
        let mut buf = Vec::new();
        buf.extend_from_slice(MAGIC);
        buf.push(u8::from(self.enabled));
        buf.push(self.mode);
        encode_toggle(&mut buf, self.builtins.sql_injection);
        encode_toggle(&mut buf, self.builtins.xss);
        encode_toggle(&mut buf, self.builtins.path_traversal);
        encode_toggle(&mut buf, self.builtins.command_injection);
        encode_toggle(&mut buf, self.builtins.header_anomaly);
        encode_toggle(&mut buf, self.builtins.bot_ua);
        buf.push(u8::from(self.ip_filter.enabled));
        put_u16(&mut buf, self.ip_filter.whitelist.len() as u16);
        for s in &self.ip_filter.whitelist {
            put_str(&mut buf, s)?;
        }
        put_u16(&mut buf, self.ip_filter.blacklist.len() as u16);
        for s in &self.ip_filter.blacklist {
            put_str(&mut buf, s)?;
        }
        put_u16(&mut buf, self.rules.len() as u16);
        for r in &self.rules {
            put_str(&mut buf, &r.id)?;
            buf.push(u8::from(r.enabled));
            buf.push(r.phases_mask);
            buf.push(r.target_kind);
            put_str(&mut buf, &r.header_name)?;
            put_str(&mut buf, &r.pattern)?;
            buf.push(r.action);
        }
        put_u16(&mut buf, self.exclusions.len() as u16);
        for ex in &self.exclusions {
            put_str(&mut buf, ex.host.as_deref().unwrap_or(""))?;
            put_str(&mut buf, ex.path_prefix.as_deref().unwrap_or(""))?;
            put_str(&mut buf, ex.method.as_deref().unwrap_or(""))?;
            put_u16(&mut buf, ex.rule_ids.len() as u16);
            for id in &ex.rule_ids {
                put_str(&mut buf, id)?;
            }
        }
        Ok(buf)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, WafProjError> {
        if bytes.len() < 8 {
            return Err(WafProjError::Truncated);
        }
        if &bytes[0..8] != MAGIC {
            return Err(WafProjError::InvalidMagic);
        }
        let mut i = 8usize;
        let enabled = get_u8(bytes, &mut i)? != 0;
        let mode = get_u8(bytes, &mut i)?;
        if mode > 2 {
            return Err(WafProjError::InvalidEnum);
        }
        let builtins = CfdBuiltinToggles {
            sql_injection: decode_toggle(bytes, &mut i)?,
            xss: decode_toggle(bytes, &mut i)?,
            path_traversal: decode_toggle(bytes, &mut i)?,
            command_injection: decode_toggle(bytes, &mut i)?,
            header_anomaly: decode_toggle(bytes, &mut i)?,
            bot_ua: decode_toggle(bytes, &mut i)?,
        };
        let ip_enabled = get_u8(bytes, &mut i)? != 0;
        let nw = get_u16(bytes, &mut i)? as usize;
        let mut whitelist = Vec::with_capacity(nw);
        for _ in 0..nw {
            whitelist.push(get_str(bytes, &mut i)?);
        }
        let nb = get_u16(bytes, &mut i)? as usize;
        let mut blacklist = Vec::with_capacity(nb);
        for _ in 0..nb {
            blacklist.push(get_str(bytes, &mut i)?);
        }
        let nr = get_u16(bytes, &mut i)? as usize;
        if nr > MAX_RULES {
            return Err(WafProjError::TooManyRules);
        }
        let mut rules = Vec::with_capacity(nr);
        for _ in 0..nr {
            let id = get_str(bytes, &mut i)?;
            let enabled = get_u8(bytes, &mut i)? != 0;
            let phases_mask = get_u8(bytes, &mut i)?;
            let target_kind = get_u8(bytes, &mut i)?;
            if target_kind == 3 {
                return Err(WafProjError::BodyDeferred);
            }
            let header_name = get_str(bytes, &mut i)?;
            let pattern = get_str(bytes, &mut i)?;
            let action = get_u8(bytes, &mut i)?;
            if action > 2 {
                return Err(WafProjError::InvalidEnum);
            }
            rules.push(CfdWafRule {
                id,
                enabled,
                phases_mask,
                target_kind,
                header_name,
                pattern,
                action,
            });
        }
        let ne = get_u16(bytes, &mut i)? as usize;
        let mut exclusions = Vec::with_capacity(ne);
        for _ in 0..ne {
            let host = nonempty_opt(get_str(bytes, &mut i)?);
            let path_prefix = nonempty_opt(get_str(bytes, &mut i)?);
            let method = nonempty_opt(get_str(bytes, &mut i)?);
            let nids = get_u16(bytes, &mut i)? as usize;
            let mut rule_ids = Vec::with_capacity(nids);
            for _ in 0..nids {
                rule_ids.push(get_str(bytes, &mut i)?);
            }
            exclusions.push(CfdWafExclusion {
                host,
                path_prefix,
                method,
                rule_ids,
            });
        }
        Ok(Self {
            enabled,
            mode,
            builtins,
            ip_filter: CfdIpFilter {
                enabled: ip_enabled,
                whitelist,
                blacklist,
            },
            rules,
            exclusions,
        })
    }
}

fn nonempty_opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn encode_toggle(buf: &mut Vec<u8>, t: CfdToggle) {
    buf.push(u8::from(t.enabled));
    buf.push(t.action);
}

fn decode_toggle(bytes: &[u8], i: &mut usize) -> Result<CfdToggle, WafProjError> {
    let enabled = get_u8(bytes, i)? != 0;
    let action = get_u8(bytes, i)?;
    if action > 2 {
        return Err(WafProjError::InvalidEnum);
    }
    Ok(CfdToggle { enabled, action })
}

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_str(buf: &mut Vec<u8>, s: &str) -> Result<(), WafProjError> {
    if s.len() > MAX_STR {
        return Err(WafProjError::StringTooLong);
    }
    put_u16(buf, s.len() as u16);
    buf.extend_from_slice(s.as_bytes());
    Ok(())
}

fn get_u8(bytes: &[u8], i: &mut usize) -> Result<u8, WafProjError> {
    if *i >= bytes.len() {
        return Err(WafProjError::Truncated);
    }
    let v = bytes[*i];
    *i += 1;
    Ok(v)
}

fn get_u16(bytes: &[u8], i: &mut usize) -> Result<u16, WafProjError> {
    if *i + 2 > bytes.len() {
        return Err(WafProjError::Truncated);
    }
    let v = u16::from_le_bytes(bytes[*i..*i + 2].try_into().unwrap());
    *i += 2;
    Ok(v)
}

fn get_str(bytes: &[u8], i: &mut usize) -> Result<String, WafProjError> {
    let len = get_u16(bytes, i)? as usize;
    if len > MAX_STR {
        return Err(WafProjError::StringTooLong);
    }
    if *i + len > bytes.len() {
        return Err(WafProjError::Truncated);
    }
    let s = std::str::from_utf8(&bytes[*i..*i + len]).map_err(|_| WafProjError::Utf8)?;
    *i += len;
    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_emptyish() {
        let p = CfdWafProjection {
            enabled: true,
            mode: 2,
            builtins: CfdBuiltinToggles::default(),
            ip_filter: CfdIpFilter::default(),
            rules: vec![CfdWafRule {
                id: "deny-path".into(),
                enabled: true,
                phases_mask: 1,
                target_kind: 0,
                header_name: String::new(),
                pattern: "/evil".into(),
                action: 2,
            }],
            exclusions: vec![],
        };
        let b = p.encode().unwrap();
        let d = CfdWafProjection::decode(&b).unwrap();
        assert_eq!(d, p);
    }

    #[test]
    fn body_rule_rejected() {
        let p = CfdWafProjection {
            enabled: true,
            mode: 2,
            builtins: CfdBuiltinToggles::default(),
            ip_filter: CfdIpFilter::default(),
            rules: vec![CfdWafRule {
                id: "body".into(),
                enabled: true,
                phases_mask: 1,
                target_kind: 3,
                header_name: String::new(),
                pattern: "x".into(),
                action: 2,
            }],
            exclusions: vec![],
        };
        assert_eq!(p.encode().unwrap_err(), WafProjError::BodyDeferred);
    }
}
