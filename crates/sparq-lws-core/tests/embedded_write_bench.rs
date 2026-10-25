#![cfg(feature = "embedded-sparq")]
use sparq_lws_core::store::{embedded::EmbeddedSparqClient, ResourceMeta, SparqClient};

#[test]
#[ignore]
fn embedded_write_bench() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        for durable in [false, true] {
            let dir = std::env::temp_dir().join(format!("lws-write-bench-{}", std::process::id()));
            let seed = EmbeddedSparqClient::in_memory().unwrap();
            let client = if durable {
                seed.save(&dir).await.unwrap();
                EmbeddedSparqClient::open(&dir).unwrap()
            } else { seed };
            let record = |i| ResourceMeta {
                content_type: "text/plain".into(), blob_key: format!("key{i}"),
                etag: format!("tag{i}"), last_modified: None,
            };
            // Seed all graph slots outside timing; overwrites exercise the steady write path.
            for i in 0..4 {
                client.put_meta(&format!("https://pod.example/r{i}"), record(0)).await.unwrap();
            }
            for i in 0..4 {
                client.put_meta(&format!("https://pod.example/r{i}"), record(1)).await.unwrap();
            }
            let start = std::time::Instant::now();
            for i in 0..128 {
                client.put_meta(&format!("https://pod.example/r{}", i % 4), record(i + 2)).await.unwrap();
            }
            let elapsed = start.elapsed().as_nanos();
            println!("WRITE_BENCH {} {}", if durable { "directory" } else { "memory" }, elapsed);
            drop(client);
            let _ = std::fs::remove_dir_all(dir);
        }
    });
}
