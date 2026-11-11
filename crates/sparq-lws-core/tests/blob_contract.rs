#![cfg(feature = "test-support")]

use sparq_lws_core::store::{blob_contract, InMemoryBlobStore};

#[tokio::test]
async fn in_memory_backend_satisfies_the_shared_contract() {
    blob_contract::run(&InMemoryBlobStore::with_immutable_keys()).await;
}
