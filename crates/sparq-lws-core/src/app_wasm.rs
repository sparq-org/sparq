//! WebAssembly router assembly for the portable LDP + WAC core.
//!
//! The router has no listener, runtime, TLS, notification, PoP, or OIDC-verifier coupling. Its host
//! drives it as a Tower service and may attach a host-verified `VerifiedToken` request extension.

use std::sync::Arc;

use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;

use crate::auth::VerifiedToken;
use crate::ldp::handler::{
    delete_handler, get_handler, head_handler, options_handler, patch_handler, post_handler,
    put_handler, LdpState,
};
use crate::store::Store;

/// Portable application state: the real LDP/WAC handler state over a wasm-safe store.
pub struct AppState<S: Store> {
    /// Shared LDP state used by every resource route.
    pub ldp: Arc<LdpState<S>>,
}

impl<S: Store> AppState<S> {
    /// Construct router state over an LDP state.
    #[must_use]
    pub fn new(ldp: LdpState<S>) -> Self {
        Self { ldp: Arc::new(ldp) }
    }
}

/// Build the listener-free wasm router over the real LDP handlers and WAC evaluator.
///
/// The JavaScript host may attach `VerifiedToken::from_authenticated_webid(...)` to a request before
/// calling the router. Requests without that extension receive public credentials.
pub fn build_router<S>(state: AppState<S>) -> Router
where
    S: Store + 'static,
{
    let ldp_methods = || {
        get(get_handler::<S>)
            .head(head_handler::<S>)
            .put(put_handler::<S>)
            .post(post_handler::<S>)
            .delete(delete_handler::<S>)
            .patch(patch_handler::<S>)
            .options(options_handler::<S>)
    };

    let router = Router::new()
        .route("/", ldp_methods())
        .route("/{*path}", ldp_methods());
    // sq-r1ei8: this static route and its engine-backed handler are
    // both absent from the core wasm tier.
    #[cfg(feature = "sparql-endpoint")]
    let router = router.route(
        "/sparql",
        get(crate::sparql_endpoint::get_handler::<S>)
            .post(crate::sparql_endpoint::post_handler::<S>),
    );
    router
        .layer(crate::body_limit::layer(
            crate::body_limit::DEFAULT_MAX_BODY_BYTES,
        ))
        .layer(middleware::from_fn(ensure_host_identity))
        .layer(middleware::from_fn(crate::ldp::cors::cors_middleware))
        .with_state(state.ldp)
}

async fn ensure_host_identity(mut request: Request, next: Next) -> Response {
    if request.extensions().get::<VerifiedToken>().is_none() {
        request.extensions_mut().insert(VerifiedToken::public());
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    use super::{build_router, AppState};
    use crate::ldp::handler::LdpState;
    use crate::store::{CompositeStore, InMemoryBlobStore, InMemorySparqClient};

    fn ldp_state() -> LdpState<CompositeStore<InMemorySparqClient, InMemoryBlobStore>> {
        let store = CompositeStore::new(InMemorySparqClient::new(), InMemoryBlobStore::new());
        LdpState::new(store, "https://pod.example")
    }

    // Direct native coverage witness for the portable public constructor.
    #[test]
    fn app_state_new_preserves_ldp_state() {
        let state = AppState::new(ldp_state());

        assert_eq!(state.ldp.base_url(), "https://pod.example");
    }

    // Direct native coverage witness for the portable public router builder.
    #[tokio::test]
    async fn build_router_mounts_ldp_methods() {
        let response = build_router(AppState::new(ldp_state()))
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router service is infallible");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(response
            .headers()
            .get(header::ALLOW)
            .expect("OPTIONS response advertises methods")
            .to_str()
            .expect("Allow is ASCII")
            .contains("GET"));
    }
}
