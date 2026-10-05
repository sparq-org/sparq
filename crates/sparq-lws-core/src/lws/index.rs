//! The type index and type search services (lws10-index). Placeholder until implemented.

use axum::http::StatusCode;
use axum::response::Response;

use super::{problem, Agent, LwsRequest, LwsState};
use crate::store::Store;

pub async fn handle<S: Store + 'static>(_state: &LwsState<S>, _req: &LwsRequest, _agent: &Agent) -> Response {
    problem(StatusCode::NOT_FOUND, None)
}
