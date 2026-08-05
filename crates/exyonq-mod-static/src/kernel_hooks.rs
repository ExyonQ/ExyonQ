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
//! Install module-api kernel hooks (KD2.5 composition root).

use crate::runtime::StaticRuntime;
#[cfg(target_os = "linux")]
use exyonq_module_api::static_epoll::{
    self, StaticEpollFsmHooks, StaticEpollInterestHooks, StaticEpollMetricsHooks,
    StaticEpollPumpResult,
};
#[cfg(target_os = "linux")]
use exyonq_module_api::static_wire::BlockingAdmission;
use exyonq_module_api::static_wire::{self, StaticWireBenchHooks, StaticWireEligibilityHooks};
use std::sync::Arc;

/// Register all stable kernel hooks after `StaticRuntime` is constructed.
pub fn install_kernel_hooks(_runtime: Arc<StaticRuntime>) {
    #[cfg(target_os = "linux")]
    crate::epoll_session::pin_runtime(Arc::clone(&_runtime));

    let _ = static_wire::install_wire_eligibility_hooks(StaticWireEligibilityHooks {
        epoll_keep_alive_eligible: crate::wire_eligibility::epoll_keep_alive_eligible,
        is_sendfile_bench_head: crate::wire_eligibility::is_sendfile_bench_head,
        epoll_sendfile_eligible: crate::wire_eligibility::epoll_sendfile_eligible,
        static_wire_use_blocking_pool: crate::wire_eligibility::static_wire_use_blocking_pool,
        static_sendfile_use_blocking_pool:
            crate::wire_eligibility::static_sendfile_use_blocking_pool,
        static_one_m_sendfile_use_blocking:
            crate::wire_eligibility::static_one_m_sendfile_use_blocking,
        might_use_static_wire: crate::wire_eligibility::might_use_static_wire,
    });

    let _ = static_wire::install_wire_bench_hooks(StaticWireBenchHooks {
        p1_bench_wire_rodata: crate::wire::p1_bench_wire_rodata,
    });

    #[cfg(target_os = "linux")]
    {
        let _ = static_wire::install_wire_serve_hooks(static_wire::StaticWireServeHooks {
            serve_blocking_sync: serve_blocking_sync_hook,
            try_spawn_blocking_static: try_spawn_blocking_static_hook,
            shed_blocking_admission: shed_blocking_admission_hook,
        });

        let _ = static_epoll::install_epoll_interest_hooks(StaticEpollInterestHooks {
            interest_reading: crate::sendfile_fsm::interest_reading,
            interest_sending_parked: crate::sendfile_fsm::interest_sending_parked,
        });

        let _ = static_epoll::install_epoll_fsm_hooks(StaticEpollFsmHooks {
            is_head_wire_request: |head| crate::StaticRoot::is_head_wire_request(head),
            match_bench_sendfile: crate::epoll_session::match_bench_sendfile,
            begin_sendfile_session: crate::epoll_session::begin_sendfile_session,
            pump_sendfile_session: |fd, session| {
                map_pump(crate::epoll_session::pump_sendfile_session(fd, session))
            },
            clear_sendfile_session: crate::epoll_session::clear_sendfile_session,
        });

        let _ = static_epoll::install_epoll_bench_hooks(static_epoll::StaticEpollBenchHooks {
            try_write_bench_response: |site_slot, fd, head, p1_wire, write_fn| {
                match crate::epoll_bench::try_write_bench_response(
                    site_slot, fd, head, p1_wire, write_fn,
                ) {
                    crate::epoll_bench::EpollBenchWriteResult::Written => {
                        static_epoll::StaticEpollBenchWriteResult::Written
                    }
                    crate::epoll_bench::EpollBenchWriteResult::Handoff => {
                        static_epoll::StaticEpollBenchWriteResult::Handoff
                    }
                    crate::epoll_bench::EpollBenchWriteResult::NoMatch => {
                        static_epoll::StaticEpollBenchWriteResult::NoMatch
                    }
                }
            },
        });

        let _ = static_epoll::install_epoll_metrics_hooks(StaticEpollMetricsHooks {
            note_complete: crate::sendfile_metrics::note_complete,
            note_parked: crate::sendfile_metrics::note_parked,
            note_error: crate::sendfile_metrics::note_error,
            note_header_reject: crate::sendfile_metrics::note_header_reject,
            note_terminal_503: crate::sendfile_metrics::note_terminal_503,
            note_unknown_fd_event: crate::sendfile_metrics::note_unknown_fd_event,
            append_prometheus: crate::sendfile_metrics::append_prometheus,
            metrics_tuple: crate::sendfile_metrics::epoll_sendfile_metrics,
            note_sendfile_fallback: crate::sendfile_metrics::note_sendfile_fallback,
            note_sendfile_fallback_rejected:
                crate::sendfile_metrics::note_sendfile_fallback_rejected,
            #[cfg(feature = "test-utils")]
            reset_for_test: crate::sendfile_metrics::reset_for_test,
        });
    }

    #[cfg(target_os = "linux")]
    {
        let _ = static_wire::install_wire_async_hooks(static_wire::StaticWireAsyncHooks {
            serve_wire_site_tcp: serve_wire_site_tcp_hook,
        });
    }
}

#[cfg(target_os = "linux")]
fn serve_wire_site_tcp_hook(
    site_slot: u32,
    stream: tokio::net::TcpStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send>> {
    Box::pin(async move {
        let runtime = crate::epoll_session::runtime_for_hooks();
        let root = runtime
            .root_for_slot_public(site_slot)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "site slot"))?;
        crate::wire_conn::serve(root, stream, head, rest).await
    })
}

#[cfg(target_os = "linux")]
fn serve_blocking_sync_hook(
    site_slot: u32,
    stream: &mut std::net::TcpStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> std::io::Result<()> {
    let runtime = crate::epoll_session::runtime_for_hooks();
    let root = runtime
        .root_for_slot_public(site_slot)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::NotFound, "site slot"))?;
    crate::wire_conn::serve_blocking_sync(&root, stream, head, rest)
}

#[cfg(target_os = "linux")]
fn std_to_tokio(stream: std::net::TcpStream) -> tokio::net::TcpStream {
    tokio::net::TcpStream::from_std(stream).expect("valid connected std TcpStream")
}

#[cfg(target_os = "linux")]
fn tokio_to_std(stream: tokio::net::TcpStream) -> std::net::TcpStream {
    stream.into_std().expect("valid connected tokio TcpStream")
}

#[cfg(target_os = "linux")]
fn try_spawn_blocking_static_hook(
    site_slot: u32,
    stream: std::net::TcpStream,
    head: bytes::Bytes,
    rest: bytes::Bytes,
) -> BlockingAdmission {
    let runtime = crate::epoll_session::runtime_for_hooks();
    let Ok(root) = runtime.root_for_slot_public(site_slot) else {
        return BlockingAdmission::Rejected(stream);
    };
    let tokio_stream = std_to_tokio(stream);
    match crate::wire_conn::try_spawn_blocking_static(root, tokio_stream, head, rest) {
        crate::wire_conn::BlockingAdmission::Accepted => BlockingAdmission::Accepted,
        crate::wire_conn::BlockingAdmission::Rejected(s) => {
            BlockingAdmission::Rejected(tokio_to_std(s))
        }
    }
}

#[cfg(target_os = "linux")]
fn shed_blocking_admission_hook(
    stream: std::net::TcpStream,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    let tokio_stream = std_to_tokio(stream);
    Box::pin(crate::wire_conn::shed_blocking_admission(tokio_stream))
}

#[cfg(target_os = "linux")]
fn map_pump(r: StaticEpollPumpResult) -> StaticEpollPumpResult {
    r
}
