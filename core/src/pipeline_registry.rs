//! Mechanical pipeline seam — delegates to per-snapshot `CrossCuttingPipeline` (KD4.4).

use exyonq_module_pipeline::CrossCuttingPipeline;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Request, Response};
use std::future::Future;

type BoxBody = http_body_util::combinators::BoxBody<bytes::Bytes, hyper::Error>;

pub use exyonq_module_pipeline::inject_client_ip;

pub async fn pipeline_handle_incoming<F, Fut>(
    pipeline: &CrossCuttingPipeline,
    req: Request<Incoming>,
    dispatch_core: F,
) -> Result<Response<BoxBody>, Response<BoxBody>>
where
    F: FnOnce(Request<Incoming>) -> Fut,
    Fut: Future<Output = Response<BoxBody>>,
{
    pipeline
        .handle_incoming_request(req, dispatch_core)
        .await
        .map_err(module_error_response)
}

pub async fn pipeline_handle_http3<F, Fut>(
    pipeline: &CrossCuttingPipeline,
    module_req: exyonq_module_api::HttpRequest,
    dispatch_core: F,
) -> Response<BoxBody>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Response<BoxBody>>,
{
    match pipeline
        .handle_http3_request(module_req, dispatch_core)
        .await
    {
        Ok(response) => response,
        Err(err) => module_error_response(err),
    }
}

fn module_error_response(err: anyhow::Error) -> Response<BoxBody> {
    use http_body_util::Full;
    use hyper::StatusCode;
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header("content-type", "text/plain; charset=utf-8")
        .body(
            Full::from(bytes::Bytes::from(format!("module error: {err}")))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("valid module error response")
}
