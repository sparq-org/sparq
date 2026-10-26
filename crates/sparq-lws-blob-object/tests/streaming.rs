#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{stream::BoxStream, StreamExt};
use object_store::{
    path::Path, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
    ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult,
};
use sparq_lws_blob_object::{Error, ObjectBlobStore};

#[derive(Debug)]
struct Probe {
    meta: ObjectMeta,
    polls: Arc<AtomicUsize>,
}

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("listing probe")
    }
}

#[async_trait]
impl ObjectStore for Probe {
    async fn put_opts(
        &self,
        _: &Path,
        _: PutPayload,
        _: PutOptions,
    ) -> object_store::Result<PutResult> {
        unreachable!()
    }
    async fn put_multipart_opts(
        &self,
        _: &Path,
        _: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        unreachable!()
    }
    async fn get_opts(&self, _: &Path, _: GetOptions) -> object_store::Result<GetResult> {
        unreachable!()
    }
    fn delete_stream(
        &self,
        _: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        unreachable!()
    }
    async fn list_with_delimiter(&self, _: Option<&Path>) -> object_store::Result<ListResult> {
        unreachable!()
    }
    async fn copy_opts(&self, _: &Path, _: &Path, _: CopyOptions) -> object_store::Result<()> {
        unreachable!()
    }

    fn list(&self, _: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let polls = self.polls.clone();
        let meta = self.meta.clone();
        futures_util::stream::poll_fn(move |_| {
            let next = match polls.fetch_add(1, Ordering::SeqCst) {
                0 => Some(Ok(meta.clone())),
                1 => Some(Err(object_store::Error::Generic {
                    store: "secret provider",
                    source: "secret presigned URL".into(),
                })),
                _ => None,
            };
            std::task::Poll::Ready(next)
        })
        .boxed()
    }
}

#[tokio::test]
async fn listing_is_lazy_and_late_page_errors_are_opaque() {
    use object_store::ObjectStoreExt;
    let raw = object_store::memory::InMemory::new();
    let key = "b1-00000000000000000000000000000000";
    let path = Path::from(format!("blobs/{key}"));
    raw.put(&path, bytes::Bytes::new().into()).await.unwrap();
    let polls = Arc::new(AtomicUsize::new(0));
    let store = ObjectBlobStore::new(
        Arc::new(Probe {
            meta: raw.head(&path).await.unwrap(),
            polls: polls.clone(),
        }),
        "blobs",
    )
    .unwrap();
    let mut listing = store.list();
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(listing.next().await.unwrap().unwrap().key, key);
    assert_eq!(
        polls.load(Ordering::SeqCst),
        1,
        "the next backend page must not be polled eagerly"
    );
    let error = listing.next().await.unwrap().unwrap_err();
    assert_eq!(error, Error::Backend);
    assert!(!format!("{error:?} {error}").contains("secret"));
    assert!(listing.next().await.is_none());
}
