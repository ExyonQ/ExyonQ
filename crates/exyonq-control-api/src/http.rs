use crate::auth;
use crate::error::CtrlError;
use crate::revision;
use crate::service::ControlService;
use crate::types::VhostCreate;
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Incoming};
use hyper::service::service_fn;
use hyper::{Method, Request, Response};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ServerBuilder;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tracing::{info, warn};

const MAX_BODY_BYTES: usize = 1024 * 1024;

type RespBody = Full<Bytes>;

#[derive(Clone)]
pub struct ControlHttp {
    service: ControlService,
    token: Arc<[u8]>,
}

#[derive(Debug, Clone)]
pub struct ControlApiBindConfig {
    pub unix_socket: Option<PathBuf>,
    pub tcp_addr: Option<SocketAddr>,
    pub token: Arc<[u8]>,
}

#[derive(Debug)]
pub struct ControlApiHandle {
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl ControlApiHandle {
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }
}

pub async fn spawn_http(
    service: ControlService,
    config: ControlApiBindConfig,
) -> Result<ControlApiHandle, CtrlError> {
    let mut tasks = Vec::new();
    if let Some(addr) = config.tcp_addr {
        validate_loopback(addr)?;
        let listener = TcpListener::bind(addr).await?;
        let http = ControlHttp::new(service.clone(), Arc::clone(&config.token));
        tasks.push(tokio::spawn(async move {
            if let Err(err) = serve_tcp(listener, addr, http).await {
                warn!(%err, %addr, "control api tcp listener stopped");
            }
        }));
    }
    #[cfg(unix)]
    if let Some(path) = config.unix_socket {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        let http = ControlHttp::new(service, Arc::clone(&config.token));
        tasks.push(tokio::spawn(async move {
            if let Err(err) = serve_unix(listener, path.clone(), http).await {
                warn!(%err, path = %path.display(), "control api unix listener stopped");
            }
        }));
    }
    Ok(ControlApiHandle { tasks })
}

impl ControlHttp {
    pub fn new(service: ControlService, token: Arc<[u8]>) -> Self {
        Self { service, token }
    }

    async fn route(&self, req: Request<Incoming>) -> Result<Response<RespBody>, CtrlError> {
        auth::authorize(req.headers(), &self.token)?;
        match (req.method(), req.uri().path()) {
            (&Method::GET, "/api/v1/health") => self.health(),
            (&Method::GET, "/api/v1/stats") => self.json(self.service.stats()?),
            (&Method::GET, "/api/v1/config") => {
                let state = self.service.current_revision()?;
                self.json_with_etag(self.service.runtime_config()?, state.etag())
            }
            (&Method::POST, "/api/v1/config/vhosts") => self.create(req).await,
            (&Method::DELETE, path) if path.starts_with("/api/v1/config/vhosts/") => {
                let domain = path.trim_start_matches("/api/v1/config/vhosts/").to_owned();
                self.delete(req, &domain).await
            }
            _ => Err(CtrlError::BadRequest(
                "unknown control api route".to_owned(),
            )),
        }
    }

    fn health(&self) -> Result<Response<RespBody>, CtrlError> {
        let health = self.service.health()?;
        let status = if health.status == "ok" {
            ::http::StatusCode::OK
        } else {
            ::http::StatusCode::SERVICE_UNAVAILABLE
        };
        json_response(status, &health)
    }

    async fn create(&self, req: Request<Incoming>) -> Result<Response<RespBody>, CtrlError> {
        let if_match = header_str(req.headers(), ::http::header::IF_MATCH);
        let bytes = collect_limited(req.into_body()).await?;
        let input: VhostCreate = serde_json::from_slice(&bytes)?;
        let vhost = self
            .service
            .create_vhost(input, if_match.as_deref())
            .await?;
        let etag = revision::RevisionState {
            revision: vhost.revision,
        }
        .etag();
        let mut response = self.json_with_etag(vhost.clone(), etag)?;
        *response.status_mut() = ::http::StatusCode::CREATED;
        response.headers_mut().insert(
            ::http::header::LOCATION,
            ::http::HeaderValue::from_str(&format!("/api/v1/config/vhosts/{}", vhost.domain))
                .map_err(|_| CtrlError::BadRequest("invalid vhost location".to_owned()))?,
        );
        Ok(response)
    }

    async fn delete(
        &self,
        req: Request<Incoming>,
        domain: &str,
    ) -> Result<Response<RespBody>, CtrlError> {
        if domain.is_empty() {
            return Err(CtrlError::BadRequest("missing vhost domain".to_owned()));
        }
        let if_match = header_str(req.headers(), ::http::header::IF_MATCH);
        let revision = self
            .service
            .delete_vhost(domain, if_match.as_deref())
            .await?;
        let etag = revision::RevisionState {
            revision: revision.revision,
        }
        .etag();
        self.json_with_etag(revision, etag)
    }

    fn json<T: serde::Serialize>(&self, value: T) -> Result<Response<RespBody>, CtrlError> {
        json_response(::http::StatusCode::OK, &value)
    }

    fn json_with_etag<T: serde::Serialize>(
        &self,
        value: T,
        etag: String,
    ) -> Result<Response<RespBody>, CtrlError> {
        let mut response = self.json(value)?;
        response.headers_mut().insert(
            ::http::header::ETAG,
            ::http::HeaderValue::from_str(&etag)
                .map_err(|_| CtrlError::BadRequest("invalid revision etag".to_owned()))?,
        );
        Ok(response)
    }
}

async fn serve_tcp(
    listener: TcpListener,
    addr: SocketAddr,
    http: ControlHttp,
) -> Result<(), CtrlError> {
    info!(%addr, "control api tcp listener started");
    loop {
        let (stream, peer) = listener.accept().await?;
        let http = http.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let http = http.clone();
                async move { Ok::<_, Infallible>(http.response(req).await) }
            });
            let builder = ServerBuilder::new(TokioExecutor::new());
            if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
                warn!(%err, %peer, "control api tcp connection error");
            }
        });
    }
}

#[cfg(unix)]
async fn serve_unix(
    listener: UnixListener,
    path: PathBuf,
    http: ControlHttp,
) -> Result<(), CtrlError> {
    info!(path = %path.display(), "control api unix listener started");
    loop {
        let (stream, _) = listener.accept().await?;
        let http = http.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |req| {
                let http = http.clone();
                async move { Ok::<_, Infallible>(http.response(req).await) }
            });
            let builder = ServerBuilder::new(TokioExecutor::new());
            if let Err(err) = builder.serve_connection_with_upgrades(io, service).await {
                warn!(%err, "control api unix connection error");
            }
        });
    }
}

impl ControlHttp {
    async fn response(&self, req: Request<Incoming>) -> Response<RespBody> {
        match self.route(req).await {
            Ok(response) => response,
            Err(err) => error_response(err),
        }
    }
}

async fn collect_limited(mut body: Incoming) -> Result<Bytes, CtrlError> {
    if body
        .size_hint()
        .upper()
        .is_some_and(|upper| upper > MAX_BODY_BYTES as u64)
    {
        return Err(CtrlError::BodyTooLarge);
    }
    let mut bytes = BytesMut::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|err| CtrlError::BadRequest(err.to_string()))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len() + data.len() > MAX_BODY_BYTES {
                return Err(CtrlError::BodyTooLarge);
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes.freeze())
}

fn header_str(headers: &::http::HeaderMap, name: ::http::header::HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn json_response<T: serde::Serialize>(
    status: ::http::StatusCode,
    value: &T,
) -> Result<Response<RespBody>, CtrlError> {
    let bytes = serde_json::to_vec(value)?;
    Response::builder()
        .status(status)
        .header(::http::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .map_err(|err| CtrlError::BadRequest(err.to_string()))
}

fn error_response(err: CtrlError) -> Response<RespBody> {
    json_response(err.status(), &err.api_error()).unwrap_or_else(|_| {
        Response::builder()
            .status(::http::StatusCode::INTERNAL_SERVER_ERROR)
            .header(::http::header::CONTENT_TYPE, "application/json")
            .body(Full::new(Bytes::from_static(
                br#"{"code":"EXY-CTRL-0005","message":"control api internal error"}"#,
            )))
            .expect("static error response is valid")
    })
}

fn validate_loopback(addr: SocketAddr) -> Result<(), CtrlError> {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => Ok(()),
        IpAddr::V6(ip) if ip.is_loopback() => Ok(()),
        _ => Err(CtrlError::BadRequest(
            "control api tcp bind must be loopback".to_owned(),
        )),
    }
}
