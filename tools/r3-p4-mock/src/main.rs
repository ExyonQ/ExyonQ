//! R3 P4 deterministic HTTP mock upstream (benchmark infrastructure only).
//! Not product code. Contract: GET /api → fixed 1024-byte body; keep-alive.

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming};
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::mpsc;
use tracing::info;

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

struct CachedFile {
    body: Bytes,
    content_type: &'static str,
}

struct AppState {
    api_payload: Bytes,
    named_payloads: HashMap<&'static str, CachedFile>,
    site_files: HashMap<String, CachedFile>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let host = std::env::var("MOCK_HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port: u16 = std::env::var("MOCK_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9000);
    let www = PathBuf::from(std::env::var("BENCH_WWW").unwrap_or_else(|_| "/bench/www".into()));

    let state = Arc::new(load_state(&www)?);
    let listen: SocketAddr = format!("{host}:{port}").parse()?;
    let workers = accept_workers();

    let mut tasks = Vec::with_capacity(workers);
    for i in 0..workers {
        let listener = bind_tuned(listen).await.map_err(|err| {
            anyhow::anyhow!("bind worker {i}/{workers} on {listen} failed: {err}")
        })?;
        let state = Arc::clone(&state);
        tasks.push(tokio::spawn(accept_loop(listener, state)));
    }
    info!(%listen, workers, "r3-p4-mock listening (all accept workers bound)");

    std::future::pending::<()>().await;
    Ok(())
}

fn accept_workers() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .clamp(1, 8)
}

async fn accept_loop(listener: TcpListener, state: Arc<AppState>) {
    loop {
        let accept_result = listener.accept().await;
        let (stream, _) = match accept_result {
            Ok(conn) => conn,
            Err(err) => {
                tracing::warn!(%err, "accept error");
                continue;
            }
        };
        let _ = stream.set_nodelay(true);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let state = Arc::clone(&state);
                async move { Ok::<_, hyper::Error>(handle(state, req)) }
            });
            if let Err(err) = http1::Builder::new()
                .keep_alive(true)
                .serve_connection(io, service)
                .await
            {
                tracing::debug!(%err, "connection closed");
            }
        });
    }
}

async fn bind_tuned(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let socket = if addr.is_ipv6() {
        TcpSocket::new_v6()?
    } else {
        TcpSocket::new_v4()?
    };
    socket.set_reuseaddr(true)?;
    #[cfg(all(unix, not(target_os = "solaris")))]
    {
        let _ = socket.set_reuseport(true);
    }
    socket.set_nodelay(true)?;
    socket.bind(addr)?;
    socket.listen(1024)
}

fn load_state(www: &Path) -> anyhow::Result<AppState> {
    let www = www.canonicalize().unwrap_or_else(|_| www.to_path_buf());
    let api_path = www.join("1k.bin");
    let api_payload = if api_path.is_file() {
        Bytes::from(std::fs::read(&api_path)?)
    } else {
        Bytes::from(vec![b'x'; 1024])
    };

    let mut named_payloads = HashMap::new();
    for (route, name) in [
        ("/1k.bin", "1k.bin"),
        ("/64k.bin", "64k.bin"),
        ("/1m.bin", "1m.bin"),
    ] {
        let path = www.join(name);
        if path.is_file() {
            named_payloads.insert(
                route,
                CachedFile {
                    body: Bytes::from(std::fs::read(&path)?),
                    content_type: "application/octet-stream",
                },
            );
        }
    }

    let site_files = preload_site_files(&www);

    Ok(AppState {
        api_payload,
        named_payloads,
        site_files,
    })
}

fn preload_site_files(www: &Path) -> HashMap<String, CachedFile> {
    let mut files = HashMap::new();
    preload_site_dir(www, www, &mut files);
    info!(count = files.len(), "preloaded /site/ assets");
    files
}

fn preload_site_dir(www: &Path, dir: &Path, out: &mut HashMap<String, CachedFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            preload_site_dir(www, &path, out);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let rel = match path.strip_prefix(www) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let request_path = format!("/site/{}", rel.to_string_lossy().replace('\\', "/"));
        let body = match std::fs::read(&path) {
            Ok(bytes) => Bytes::from(bytes),
            Err(_) => continue,
        };
        out.insert(
            request_path,
            CachedFile {
                content_type: content_type_for(&path),
                body,
            },
        );
    }
}

fn handle(state: Arc<AppState>, req: Request<Incoming>) -> Response<BoxBody> {
    if req.method() != Method::GET {
        return text(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
    }

    let path = req.uri().path();
    match path {
        "/health" => text(StatusCode::OK, "ok"),
        "/echo" | "/api/echo" => echo_json(req.headers()),
        "/" | "/api/" | "/api/health" => bytes(
            StatusCode::OK,
            state.api_payload.clone(),
            "application/octet-stream",
        ),
        "/stream" | "/api/stream" => sse_stream(),
        "/stream-clean" | "/api/stream-clean" => sse_clean_stream(),
        "/stream-long" | "/api/stream-long" => sse_long_progressive_stream(),
        other if other.starts_with("/site/") => serve_site(&state, other),
        other => serve_named_or_404(&state, other),
    }
}

fn echo_json(headers: &hyper::HeaderMap) -> Response<BoxBody> {
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let body = serde_json::json!({
        "host": host,
        "x_forwarded_for": headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .unwrap_or(""),
    });
    bytes(
        StatusCode::OK,
        Bytes::from(body.to_string()),
        "application/json",
    )
}

fn serve_site(state: &AppState, request_path: &str) -> Response<BoxBody> {
    let lookup_path = if request_path == "/site/" {
        "/site/index.html"
    } else {
        request_path
    };

    if let Some(file) = state.site_files.get(lookup_path) {
        return bytes(StatusCode::OK, file.body.clone(), file.content_type);
    }

    if let Some(file) = state.site_files.get(request_path) {
        return bytes(StatusCode::OK, file.body.clone(), file.content_type);
    }

    let rel = match request_path.strip_prefix("/site/") {
        Some(value) => value.trim_start_matches('/'),
        None => return text(StatusCode::NOT_FOUND, "not found"),
    };
    let rel = if rel.is_empty() { "index.html" } else { rel };
    if rel.contains("..") {
        return text(StatusCode::FORBIDDEN, "forbidden");
    }

    let mut safe = PathBuf::new();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            _ => return text(StatusCode::FORBIDDEN, "forbidden"),
        }
    }

    text(StatusCode::NOT_FOUND, "not found")
}

fn serve_named_or_404(state: &AppState, path: &str) -> Response<BoxBody> {
    for (suffix, file) in &state.named_payloads {
        if path.ends_with(suffix) {
            return bytes(StatusCode::OK, file.body.clone(), file.content_type);
        }
    }
    text(StatusCode::NOT_FOUND, "not found")
}

fn sse_stream() -> Response<BoxBody> {
    let mut body = Vec::new();
    for i in 0..20 {
        let chunk = format!("data: chunk-{i}\n\n");
        body.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        body.extend_from_slice(chunk.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("transfer-encoding", "chunked")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(
            Full::from(Bytes::from(body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid sse response")
}

/// P8O2 diagnostic fixture: application SSE bytes only, without embedded
/// HTTP transfer-coding syntax. It has the same 20 events and 425-byte body
/// size as `sse_stream`, but is a distinct non-comparable mock contract.
fn sse_clean_stream() -> Response<BoxBody> {
    const TARGET_BYTES: usize = 425;
    let mut body = Vec::with_capacity(TARGET_BYTES);
    for i in 0..20 {
        body.extend_from_slice(format!("data: chunk-{i}\n\n").as_bytes());
    }
    // One SSE comment keeps the body byte count fixed without adding events.
    body.push(b':');
    body.resize(TARGET_BYTES - 1, b' ');
    body.push(b'\n');
    debug_assert_eq!(body.len(), TARGET_BYTES);

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(
            Full::from(Bytes::from(body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid clean SSE response")
}

/// P8O-B / P8T-FIX: true progressive SSE — events yield with delay (Pending between frames).
/// Not comparable to FINITE_SSE_BATCH_20_EVENTS (P8O-A).
fn sse_long_progressive_stream() -> Response<BoxBody> {
    const EVENTS: usize = 50;
    const DELAY_MS: u64 = 5;
    let (tx, rx) = mpsc::channel::<Option<Bytes>>(2);
    tokio::spawn(async move {
        for i in 0..EVENTS {
            let frame = Bytes::from(format!("data: chunk-{i}\n\n"));
            if tx.send(Some(frame)).await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
        }
        let _ = tx.send(None).await;
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(
            ProgressiveChanBody { rx }
                .map_err(|never: Infallible| match never {})
                .boxed(),
        )
        .expect("valid progressive SSE response")
}

struct ProgressiveChanBody {
    rx: mpsc::Receiver<Option<Bytes>>,
}

impl Body for ProgressiveChanBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.rx.poll_recv(cx) {
            Poll::Ready(Some(Some(data))) => Poll::Ready(Some(Ok(Frame::data(data)))),
            Poll::Ready(Some(None)) | Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clean_sse_fixture_is_425_bytes_without_embedded_chunk_syntax() {
        let response = sse_clean_stream();
        assert_eq!(
            response.headers().get(CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        assert_eq!(body.len(), 425);
        assert!(body.starts_with(b"data: chunk-0\n\n"));
        assert!(body.ends_with(b"\n"));
        assert!(!body.windows(3).any(|window| window == b"\r\n"));
    }

    #[tokio::test]
    async fn long_sse_fixture_is_progressive_fifty_events() {
        let response = sse_long_progressive_stream();
        let mut body = response.into_body();
        let mut seen = String::new();
        let mut saw_early_before_late = false;
        while let Some(frame) = body.frame().await {
            let frame = frame.expect("frame");
            let Ok(data) = frame.into_data() else {
                continue;
            };
            seen.push_str(&String::from_utf8_lossy(&data));
            if seen.contains("data: chunk-0") && !seen.contains("data: chunk-49") {
                saw_early_before_late = true;
            }
        }
        assert!(saw_early_before_late, "expected progressive frames; got {seen:?}");
        assert!(seen.contains("data: chunk-0"));
        assert!(seen.contains("data: chunk-49"));
    }
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn text(status: StatusCode, body: &str) -> Response<BoxBody> {
    bytes(
        status,
        Bytes::from(body.to_owned()),
        "text/plain; charset=utf-8",
    )
}

fn bytes(status: StatusCode, body: Bytes, content_type: &str) -> Response<BoxBody> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, body.len())
        .header("connection", "keep-alive")
        .body(Full::from(body).map_err(|never| match never {}).boxed())
        .expect("valid response")
}
