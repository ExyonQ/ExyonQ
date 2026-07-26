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
//! Shared Linux listen bind (SO_REUSEADDR + SO_REUSEPORT) for platform workers.

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::os::fd::AsRawFd;
use socket2::{Domain, Protocol, Socket, Type};

pub(crate) fn bind_tuned_std(addr: SocketAddr) -> io::Result<TcpListener> {
    let domain = Domain::for_address(addr);
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    unsafe {
        let yes: libc::c_int = 1;
        let rc = libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &yes as *const _ as *const libc::c_void,
            std::mem::size_of_val(&yes) as libc::socklen_t,
        );
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    socket.set_nodelay(true)?;
    socket.bind(&addr.into())?;
    socket.listen(4096)?;
    let listener: TcpListener = socket.into();
    listener.set_nonblocking(false)?;
    Ok(listener)
}
