const ROUTE: &str = "/presentations/verify";
include!("../adapter.rs");

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    vercel_runtime::run(vercel_runtime::service_fn(handler)).await
}
