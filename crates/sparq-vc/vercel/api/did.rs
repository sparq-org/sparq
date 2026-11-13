const ROUTE: &str = "/did.json";
include!("../adapter.rs");

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    vercel_runtime::run(handler).await
}
