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
//! Native journald submission (Linux) via the journal datagram socket.
//!
//! Cap061: emit only scrubbed lines from FanoutLayer — never a raw tracing
//! field layer that could bypass redaction.

use std::io;
use std::os::unix::net::UnixDatagram;

const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

/// Fail-closed probe used at PREPARE when `logging.journald.enabled`.
pub fn probe() -> anyhow::Result<()> {
    emit_line("exyonq journald probe").map_err(|err| {
        anyhow::anyhow!("native journald unavailable ({err}) (STARTUP_REJECT_CONFIG)")
    })
}

/// Submit one already-scrubbed log line as MESSAGE= (native journal protocol).
pub fn emit_line(line: &str) -> io::Result<()> {
    // Journal protocol: KEY=value\n (no newlines inside value for the simple form).
    let safe = line.replace('\n', " ");
    let payload = format!("MESSAGE={safe}\nPRIORITY=6\nSYSLOG_IDENTIFIER=exyonq\n");
    let sock = UnixDatagram::unbound()?;
    sock.send_to(payload.as_bytes(), JOURNAL_SOCKET)?;
    Ok(())
}
