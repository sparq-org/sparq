const ROUTE: &str = "/credentials/issue";
include!("../adapter.rs");

#[tokio::main]
async fn main() -> Result<(), vercel_runtime::Error> {
    vercel_runtime::run(handler).await
}
