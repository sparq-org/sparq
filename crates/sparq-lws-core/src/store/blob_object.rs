//! Optional native adapter from the independent object blob plane to [`BlobStore`].

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{stream::BoxStream, StreamExt, TryStreamExt};
pub use sparq_lws_blob_object::ObjectBlobStore;

use super::blob::{BlobEntry, BlobError, BlobStore};

/// Server-side wrapper; the underlying crate has no dependency on this server.
#[derive(Debug, Clone)]
pub struct ObjectBlobAdapter(pub ObjectBlobStore);

fn map_error(e: sparq_lws_blob_object::Error) -> BlobError {
    match e {
        sparq_lws_blob_object::Error::NotFound => BlobError::NotFound,
        e => BlobError::Backend(e.to_string()),
    }
}

fn entry(e: sparq_lws_blob_object::Entry) -> BlobEntry {
    BlobEntry {
        key: e.key,
        last_modified: Some(e.last_modified),
        generation: None,
    }
}

#[async_trait]
impl BlobStore for ObjectBlobAdapter {
    async fn put(&self, key: &str, body: Bytes) -> Result<(), BlobError> {
        self.0.put(key, body).await.map_err(map_error)
    }

    async fn get(&self, key: &str) -> Result<Bytes, BlobError> {
        self.0.get(key).await.map_err(map_error)
    }

    async fn exists(&self, key: &str) -> Result<bool, BlobError> {
        self.0.exists(key).await.map_err(map_error)
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        self.0.delete(key).await.map_err(map_error)
    }

    async fn stat(&self, key: &str) -> Result<Option<BlobEntry>, BlobError> {
        self.0
            .stat(key)
            .await
            .map(|e| e.map(entry))
            .map_err(map_error)
    }

    async fn list_stream(&self) -> Result<BoxStream<'_, Result<BlobEntry, BlobError>>, BlobError> {
        Ok(self
            .0
            .list()
            .map(|e| e.map(entry).map_err(map_error))
            .boxed())
    }

    async fn list(&self) -> Result<Vec<BlobEntry>, BlobError> {
        // Compatibility for small callers of the original snapshot API. The reconciler uses
        // list_stream, so no bucket-sized allocation is needed. Refuse oversized snapshots.
        let mut entries = self
            .list_stream()
            .await?
            .take(4097)
            .try_collect::<Vec<_>>()
            .await?;
        if entries.len() > 4096 {
            entries.clear();
            return Err(BlobError::Backend(
                "blob listing exceeds snapshot limit; use list_stream".into(),
            ));
        }
        Ok(entries)
    }

    async fn delete_if_unchanged(
        &self,
        key: &str,
        _expected_generation: u64,
    ) -> Result<bool, BlobError> {
        // This backend never advertises a generation and has no atomic conditional delete.
        // The trait explicitly permits unconditional deletion under unique-per-write keys.
        // The reconciler selects BlobStore::delete for generation=None entries.
        if !self.exists(key).await? {
            return Ok(false);
        }
        self.delete(key).await?;
        Ok(true)
    }
}
