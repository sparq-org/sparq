//! The LWS authorization server (LWS 1.0 core section 5.2). Placeholder until implemented.

use axum::http::StatusCode;
use axum::response::Response;

use super::{problem, LwsRequest, LwsState};
use crate::store::Store;

pub async fn handle<S: Store + 'static>(_state: &LwsState<S>, _req: &LwsRequest) -> Response {
    problem(StatusCode::NOT_FOUND, None)
}
