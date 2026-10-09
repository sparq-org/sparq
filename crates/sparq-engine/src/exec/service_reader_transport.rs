use sparq_engine_service::service::{ReaderTransport, TransportAsReader};

/// Run `f` with the active reader transport — wrapping any installed test
/// `Transport` as a `ReaderTransport`, or using `HttpTransport` directly.
pub(crate) fn with<R>(f: impl FnOnce(&dyn ReaderTransport) -> R) -> R {
    super::service_transport::with_opt(|opt| match opt {
        Some(t) => f(&TransportAsReader(t)),
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                f(&sparq_engine_service::service::HttpTransport::with_budget(
                    super::budget::remaining_timeout(),
                ))
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = f;
                unreachable!("SERVICE has no HTTP transport on wasm")
            }
        }
    })
}
