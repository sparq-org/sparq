//! Notifications (LWS 1.0 core section 10) and the webhook subscription type
//! (lws10-notifications-webhook). Placeholder until implemented.

use axum::http::StatusCode;
use axum::response::Response;
use serde_json::Value;

use super::{problem, Agent, LwsConfig, LwsRequest, LwsState};
use crate::store::Store;

/// A change to a storage resource, announced to the subscriptions whose topics cover it.
#[derive(Debug, Clone)]
pub struct Event {
    /// `Create`, `Update` or `Delete`.
    pub kind: &'static str,
    /// The resource that changed.
    pub uri: String,
    pub is_container: bool,
    /// `("target", container)` for a Create, `("origin", container)` for a Delete.
    pub relation: Option<(&'static str, String)>,
}

/// The subscriptions of the storage and the notification sender.
pub struct Notifier {}

impl Notifier {
    pub async fn load<S: Store>(_store: &S, _cfg: &LwsConfig) -> Result<Self, String> {
        Ok(Self {})
    }

    /// Tell every subscriber whose topic covers `event.uri`, and who may read it now, that it
    /// changed. Awaited before a delete, while who may read the resource can still be decided;
    /// delivery itself happens in the background.
    pub async fn announce<S: Store + 'static>(&self, _state: &LwsState<S>, _event: Event) {}

    /// Deliver one notification wrapping `activity` to `inbox` in the background.
    pub fn deliver<S: Store + 'static>(&self, _state: &LwsState<S>, _inbox: &str, _activity: Value, _subscription: Option<&str>) {}
}

pub async fn handle<S: Store + 'static>(_state: &LwsState<S>, _req: &LwsRequest, _agent: &Agent) -> Response {
    problem(StatusCode::NOT_FOUND, None)
}
