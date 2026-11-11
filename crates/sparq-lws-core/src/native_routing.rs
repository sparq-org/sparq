//! Native-only LWS mount; the library router keeps accepting storage-relative paths.

use axum::extract::{Request, State};
use axum::http::{StatusCode, Uri};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;

pub(super) fn mount_lws_router(
    app: Router,
    base_url: &str,
    enabled: bool,
) -> Result<Router, String> {
    if !enabled {
        return Ok(app);
    }
    // Use the HTTP spelling of the path: URL parsing would normalize stored resource names.
    let base: Uri = base_url
        .parse()
        .map_err(|e| format!("invalid LWS mount base URL: {e}"))?;
    let prefix = base.path().trim_end_matches('/');
    if prefix.is_empty() {
        return Ok(app);
    }
    Ok(app.layer(from_fn_with_state(prefix.to_owned(), strip_base_path)))
}

async fn strip_base_path(State(prefix): State<String>, mut req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // RFC 9728 metadata lives at the origin, before the storage path. Forward it verbatim.
    const METADATA: &str = "/.well-known/oauth-protected-resource";
    if path == METADATA || path.starts_with(&format!("{METADATA}/")) {
        return next.run(req).await;
    }
    let Some(suffix) = path.strip_prefix(&prefix) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let suffix = if suffix.is_empty() { "/" } else { suffix };
    if !suffix.starts_with('/') {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Strip exactly once, preserving encoded octets, slash distinctions and the query.
    let path_and_query = match req.uri().query() {
        Some(query) => format!("{suffix}?{query}"),
        None => suffix.to_owned(),
    };
    let mut parts = req.uri().clone().into_parts();
    let Ok(path_and_query) = path_and_query.parse() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    parts.path_and_query = Some(path_and_query);
    let Ok(uri) = Uri::from_parts(parts) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    *req.uri_mut() = uri;
    next.run(req).await
}
