# Logging and retention

ExyonQ writes three kinds of records: **access** events, **WAF** events and
**audit** events, plus ordinary diagnostics. This page says where each one goes,
how big it can grow and how to cap it. It documents the schema shipped in
`exyonq` 0.9.2 and later; every snippet below validates with
`exyonqctl config test`.

## Where the records go

| Sink | Config | Default | What it carries |
| --- | --- | --- | --- |
| Console / journald capture | `[logging.console]`, `[logging.level]` | on, `info` | everything, including access and WAF lines |
| Native journald | `[logging.journald]` `enabled = true` | off | structured submission instead of stdout capture |
| File | `[logging.file]` | off | one file sink for the filtered stream |
| Access toggle | `[logging.access]` `enabled` | `true` | whether access events are emitted at all |
| Audit toggle | `[logging.audit]` `enabled` | off | audit events |
| Syslog / OTLP | `[logging.syslog]`, `[logging.otel]` | off | the same stream to a remote collector |

The service runs under systemd, so by default everything lands in the journal.
That is convenient and unbounded: a busy site produced close to 1 GiB of journal
before an explicit cap was configured.

## Cap the journal

These are systemd settings, not ExyonQ configuration. Put them in
`/etc/systemd/journald.conf` (or a drop-in) and restart journald:

```ini
[Journal]
Storage=persistent
SystemMaxUse=1G
SystemMaxFileSize=50M
SystemMaxFiles=10
MaxRetentionSec=30day
```

Verify after the reload:

```bash
journalctl --disk-usage
systemctl show systemd-journald -p SystemMaxUse -p MaxRetentionSec
```

The values are examples: size them against the volume you are willing to give
the journal, not against a fixed recommendation. `journalctl --vacuum-size=1G`
applies a cap immediately when the journal already grew.

## Rotate log files with ExyonQ

`[logging.file]` writes one stream to one path. `[logging.file.rotation]` is the
built-in size rotation (strategy A in `config/ir/src/logging.rs`):

```toml
[logging.file]
enabled = true
path = "/var/log/exyonq/exyonq.log"

[logging.file.rotation]
enabled = true
max_bytes = 104857600   # 100 MiB per file
keep = 5                # rotated files kept beside the active one
```

`max_bytes` and `keep` are only honoured when `rotation.enabled = true`; the
sink itself needs `file.enabled = true` and a writable `path`. The runtime user
must own the directory.

Verify the cap is active by writing past `max_bytes` on a non-production
instance and listing the directory: the active file is replaced and the older
files carry the rotation suffix.

## External `logrotate` (strategy B)

If you already rotate with `logrotate`, keep ExyonQ's rotation **disabled** and
let `logrotate` own the files. Use `copytruncate` unless you can restart or
signal the service: the file sink keeps its descriptor open, so renaming the
file under it without `copytruncate` sends new records to the renamed file.

```text
/var/log/exyonq/*.log {
    daily
    rotate 14
    compress
    delaycompress
    missingok
    notifempty
    copytruncate
}
```

Do not enable both strategies for the same path: two rotations on one file
interleave and lose records.

## Keep WAF events apart

WAF records are emitted with `target = "exyonq_waf"`, and the abuse-gate trace
uses `target = "exyonq::waf_abuse_trace"` when `EXYONQ_WAF_ABUSE_TRACE` is set.
Use the filter to route them to their own systemd unit or file instead of
parsing the combined stream:

```ini
# /etc/systemd/system/exyonq-waf-log.service.d/filter.conf
[Service]
ExecStart=
ExecStart=/usr/bin/journalctl -f -t exyonq --output=json
```

A simpler and supported option is to keep one file sink for the whole stream and
select the WAF records at read time:

```bash
journalctl -u exyonq -o cat | grep 'target=exyonq_waf'
jq -r 'select(.SYSLOG_IDENTIFIER=="exyonq") | .MESSAGE' \
  <(journalctl -u exyonq -o json) | grep '"waf violation"'
```

Since 0.9.3 every violation line carries `client_ip`, `host`, `method`, `path`,
`user_agent` and `rule_id`, so a violation is actionable on its own. The
matching access event also carries `host`.

## Retention and privacy

- Access and WAF lines contain client IPs, host names, paths and User-Agent
  strings. Treat them as personal data: set `MaxRetentionSec`/`keep` to the
  shortest window the incident-response runbook needs, and do not ship them to a
  third-party collector without a documented basis.
- `[logging.access] enabled = false` removes access events entirely; WAF lines
  keep being emitted because they are security events.
- Request identifiers are logged as `request_id` (internal, generated) and
  `external_request_id` (the client's `X-Request-ID`, only when it passes
  validation). Do not configure a sink that adds the raw header to a path
  without validation.
- Audit records (`[logging.audit]`) are the ones to keep longest; they are off
  by default and do not carry request bodies.

## Checklist before going to production

1. `journalctl --disk-usage` is bounded and the cap is visible in
   `systemctl show systemd-journald`.
2. Either `[logging.file.rotation]` or an external `logrotate` entry owns every
   file path, never both.
3. WAF records are readable separately (filter or file) and the violation line
   includes host and client IP.
4. The retention window is written down and matches the privacy policy.
