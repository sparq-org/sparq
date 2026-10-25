#![cfg(feature = "embedded-sparq")]
use sparq_lws_core::store::{embedded::EmbeddedSparqClient, ResourceMeta, SparqClient};

#[test]
#[ignore]
fn embedded_memory_bench() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let resources: usize = std::env::var("BENCH_RESOURCES").unwrap().parse().unwrap();
        let client = EmbeddedSparqClient::in_memory().unwrap();
        let record = |i| ResourceMeta {
            content_type: "text/plain".into(), blob_key: format!("key{i}"),
            etag: format!("tag{i}"), last_modified: None,
        };
        for i in 0..resources {
            client.put_meta(&format!("https://pod.example/r{i}"), record(0)).await.unwrap();
        }
        for i in 0..4 {
            client.put_meta(&format!("https://pod.example/r{}", i % resources), record(1)).await.unwrap();
        }
        let start = std::time::Instant::now();
        for i in 0..128 {
            client.put_meta(&format!("https://pod.example/r{}", i % resources), record(i + 2)).await.unwrap();
        }
        println!("MEMORY_BENCH {} {}", resources, start.elapsed().as_nanos());
    });
}
