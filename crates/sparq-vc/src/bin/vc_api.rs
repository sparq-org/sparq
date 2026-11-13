//! Native runner for the opt-in credential interoperability endpoint.

use sparq_vc::api::{router, Service};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = Service::from_env()?;
    let address = std::env::var("SPARQ_VC_BIND").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    eprintln!("test-only VC-API listening on {address}");
    axum::serve(listener, router(service)).await?;
    Ok(())
}
