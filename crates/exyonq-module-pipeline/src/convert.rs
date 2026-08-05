//! Convert between hyper handler types and module-api HTTP types.

use exyonq_module_api::{Body, HttpRequest, HttpResponse, CLIENT_IP_HEADER};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::header::HeaderValue;
use hyper::{Request, Response};

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

pub async fn module_request_from_incoming(req: &Request<Incoming>) -> anyhow::Result<HttpRequest> {
    let mut builder = Request::builder().method(req.method()).uri(req.uri());
    for (name, value) in req.headers() {
        builder = builder.header(name, value);
    }
    Ok(builder.body(Body::from(bytes::Bytes::new()))?)
}

pub fn module_request_from_parts(
    req: &http::Request<()>,
    client_ip: &str,
) -> anyhow::Result<HttpRequest> {
    // P14C-C-A: clone HeaderMap once instead of per-header builder.append churn.
    let mut out = Request::builder()
        .method(req.method())
        .uri(req.uri())
        .body(Body::from(bytes::Bytes::new()))?;
    *out.headers_mut() = req.headers().clone();
    out.headers_mut().insert(
        CLIENT_IP_HEADER,
        HeaderValue::from_str(client_ip).expect("valid client ip header"),
    );
    Ok(out)
}

pub async fn module_response_from_boxed(
    response: Response<BoxBody>,
    limit: usize,
) -> anyhow::Result<HttpResponse> {
    let (parts, body) = response.into_parts();
    let bytes = BodyExt::collect(body)
        .await
        .map_err(|err| anyhow::anyhow!("collect response body: {err}"))?
        .to_bytes();
    if bytes.len() > limit {
        anyhow::bail!("response body exceeds module processing limit");
    }
    Ok(Response::from_parts(parts, Body::from(bytes)))
}

pub async fn boxed_from_module_response(
    response: HttpResponse,
) -> anyhow::Result<Response<BoxBody>> {
    let (parts, body) = response.into_parts();
    let bytes = BodyExt::collect(body)
        .await
        .map_err(|err| anyhow::anyhow!("collect module body: {err}"))?
        .to_bytes();
    Ok(Response::from_parts(
        parts,
        http_body_util::Full::from(bytes)
            .map_err(|never| match never {})
            .boxed(),
    ))
}

pub fn inject_client_ip(req: Request<Incoming>, client_ip: &str) -> Request<Incoming> {
    let (mut parts, body) = req.into_parts();
    parts.headers.insert(
        CLIENT_IP_HEADER,
        client_ip.parse().expect("valid client ip header"),
    );
    Request::from_parts(parts, body)
}
