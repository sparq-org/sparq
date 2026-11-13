use axum::body::{to_bytes, Body as AxumBody};
use sparq_vc::api::{router, OfflineContexts, Service};
use tower::ServiceExt as _;
use vercel_runtime::{Body, Error, Request, Response};

include!(concat!(env!("OUT_DIR"), "/contexts.rs"));

pub async fn handler(request: Request) -> Result<Response<Body>, Error> {
    let contexts = OfflineContexts::from_documents(embedded_contexts())?;
    let service = Service::from_env_with_contexts(contexts)?;
    let (parts, body) = request.into_parts();
    let body = match body {
        Body::Empty => Vec::new(),
        Body::Text(text) => text.into_bytes(),
        Body::Binary(bytes) => bytes,
    };
    let mut forwarded = axum::http::Request::builder()
        .method(parts.method.as_str())
        .uri(ROUTE);
    for (name, value) in &parts.headers {
        forwarded = forwarded.header(name.as_str(), value.as_bytes());
    }
    let response = router(service)
        .oneshot(forwarded.body(AxumBody::from(body))?)
        .await?;
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, 1024 * 1024).await?;
    let mut response = Response::builder().status(parts.status.as_u16());
    for (name, value) in &parts.headers {
        response = response.header(name.as_str(), value.as_bytes());
    }
    Ok(response.body(Body::Binary(bytes.to_vec()))?)
}
