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
//! Dev spike proxy — forwards all requests to a fixed upstream (KD3.3 module-owned).

use crate::hyper_client::build_incoming_client;
use crate::hyper_forward;
use crate::upstream_target::UpstreamTarget;
use crate::{global_hyper_metrics, ProxyClient};
use http_body_util::combinators::BoxBody as HttpBoxBody;
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::service::service_fn;
use hyper::{Request, Response, Uri};
use hyper_util::rt::TokioIo;
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::{info, warn};

type BoxBody = HttpBoxBody<bytes::Bytes, hyper::Error>;

async fn forward_spike_request(
    client: &ProxyClient,
    upstream: &UpstreamTarget,
    req: Request<Incoming>,
    x_forwarded_for: Option<&HeaderValue>,
) -> Response<BoxBody> {
    hyper_forward::forward_request(
        client,
        upstream,
        req,
        x_forwarded_for,
        global_hyper_metrics().as_ref(),
    )
    .await
}

/// Minimal listen-and-forward proxy for integration/dev spikes (module-owned, KD3.3).
pub async fn run_spike_proxy(listen: SocketAddr, upstream: Uri) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen).await?;
    info!(%listen, %upstream, "spike proxy listening");

    let client = build_incoming_client();
    let upstream = UpstreamTarget::from_config(&upstream.to_string(), 30_000)?;

    loop {
        let (stream, peer) = listener.accept().await?;
        let upstream = upstream.clone();
        let client = client.clone();
        let xff = HeaderValue::from_str(&peer.ip().to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0.0.0.0"));

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req: Request<Incoming>| {
                let client = client.clone();
                let upstream = upstream.clone();
                let xff = xff.clone();
                async move {
                    Ok::<_, Infallible>(
                        forward_spike_request(&client, &upstream, req, Some(&xff)).await,
                    )
                }
            });

            if let Err(err) = ServerBuilder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection(io, service)
                .await
            {
                warn!(%err, "connection error");
            }
        });
    }
}
