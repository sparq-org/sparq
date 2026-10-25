//! Adapt the native filesystem backend to the composite store's blob seam.

use async_trait::async_trait;
use bytes::Bytes;
use sparq_lws_blob_fs::{Error, FsBlobStore};

use super::{BlobEntry, BlobError, BlobStore};

fn map_error(error: Error) -> BlobError {
    match error {
        Error::NotFound => BlobError::NotFound,
        error => BlobError::Backend(error.to_string()),
    }
}

fn map_entry(entry: sparq_lws_blob_fs::BlobEntry) -> BlobEntry {
    BlobEntry {
        key: entry.key,
        last_modified: entry.last_modified,
        generation: entry.generation,
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        FsBlobStore::get(self, key).await.map_err(map_error)
    }

    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        FsBlobStore::put(self, key, body).await.map_err(map_error)
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        FsBlobStore::exists(self, key).await.map_err(map_error)
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        FsBlobStore::delete(self, key).await.map_err(map_error)
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        FsBlobStore::list(self)
            .await
            .map(|entries| entries.into_iter().map(map_entry).collect())
            .map_err(map_error)
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        match FsBlobStore::stat(self, key).await {
            Ok(entry) => Ok(Some(map_entry(entry))),
            Err(Error::NotFound) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    async fn delete_if_unchanged(
        &self,
        key: &str,
        _expected_generation: u64,
    ) -> Result<bool, BlobError> {
        // No numeric generation can match this versionless backend. Still validate
        // the key and surface backend faults; reconciliation uses plain delete.
        BlobStore::stat(self, key).await?;
        Ok(false)
    }
}
