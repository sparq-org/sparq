// [GPT-5.6] sq-6xasp.7: opt-in persistence for the wasm pod — a replayable mutation journal
// sitting behind the existing `Store` trait, plus the byte format a JS host hands to `node:fs`
// or IndexedDB.
//! Opt-in persistence for the wasm pod: a replayable mutation journal.
//!
//! # The problem
//!
//! The pod's metadata and bytes live in wasm linear memory
//! (`CompositeStore<InMemorySparqClient, InMemoryBlobStore>`), so everything a pod holds is lost
//! when the host drops the instance — a listener restart, or the trap-recovery recycle. This
//! module gives a host an opt-in way to carry that state across such a restart while leaving the
//! in-memory pod exactly as it was: `SolidServer::new` is unchanged and journals nothing.
//!
//! # Why a journal, and not a `Store` that calls JavaScript
//!
//! The obvious shape — a `Store` impl whose methods `await` a host-supplied `fetch`/IndexedDB
//! promise — does not typecheck. `Store` is `Send + Sync` and its
//! `#[async_trait]` methods return `Box<dyn Future + Send>`, while `wasm_bindgen_futures::JsFuture`
//! and every `JsValue` it borrows are `!Send`. There is no safe way to hold a JS promise across
//! such an `await`, so per-operation JS round-trips are not available behind this trait.
//!
//! What *is* available is the same trait used as a decorator. `PersistentStore` wraps any
//! `Store`, forwards every method verbatim, and records each
//! mutation that **succeeded** into a [`Journal`]. The host reads the journal as bytes
//! ([`Journal::encode`]) whenever it likes, writes those bytes wherever it keeps state, and on the
//! next boot hands them back; `replay` applies them to a fresh in-memory pod. Persistence policy
//! — which file, which IndexedDB object store, how often to flush — stays entirely in JavaScript,
//! which is where the fs and IndexedDB APIs actually are.
//!
//! # What the journal is
//!
//! It is **not** an append-only log of everything that ever happened. Each mutation is folded into
//! the record set as it arrives, so the journal converges on a description of the pod's *current*
//! contents:
//!
//! - a `PUT` over an existing resource **supersedes** the earlier write of that IRI, so
//!   overwriting one resource a thousand times keeps one body, not a thousand;
//! - a `DELETE` **removes** the records that created the resource, so a created-then-deleted
//!   resource leaves nothing behind.
//!
//! Both folds are refused (and the record kept verbatim) when some other record still names the
//! IRI as its container or parent, because replay applies records in order and
//! `create_in_container` fails with 404 against a container that does not exist yet. That is the
//! only ordering dependency between records, so guarding it is enough to keep replay faithful.
//!
//! # What restart does not preserve
//!
//! Replay re-runs the writes, so each resource's `Last-Modified` becomes the replay instant rather
//! than that of the original write, and it is backed by a freshly minted blob key. The `ETag` is
//! derived from the body alone, so it is unchanged. A client holding a pre-restart
//! `If-None-Match` still gets its 304; one holding `If-Modified-Since` re-fetches a body it
//! already had.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

/// Leading bytes of an encoded journal — `LWS Journal`.
const MAGIC: &[u8; 4] = b"LWSJ";

/// Format version written by [`Journal::encode`] and required by [`decode`].
const VERSION: u8 = 1;

const TAG_WRITE: u8 = 1;
const TAG_CREATE_IN_CONTAINER: u8 = 2;
const TAG_DELETE: u8 = 3;

/// One store mutation, in the form needed to re-apply it to an empty pod.
///
/// Bodies are held as `Vec<u8>` rather than `bytes::Bytes` so this module — the whole format and
/// its folding rules — compiles and is tested on the host, where the wasm-only store dependencies
/// are absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    /// `Store::write`: create-or-replace a resource.
    Write {
        /// The resource IRI.
        iri: String,
        /// The `Content-Type` the body was stored under.
        content_type: String,
        /// The stored bytes.
        body: Vec<u8>,
    },
    /// `Store::create_in_container`: create a resource and record it as a child.
    CreateInContainer {
        /// The container the child was minted under.
        container: String,
        /// The minted child IRI.
        child: String,
        /// The `Content-Type` the body was stored under.
        content_type: String,
        /// The stored bytes.
        body: Vec<u8>,
    },
    /// `Store::delete`: remove a resource and detach it from `parent`.
    ///
    /// Retained only when the deleted IRI cannot be folded away (see the module documentation);
    /// the ordinary create-then-delete pair leaves no record at all.
    Delete {
        /// The deleted resource IRI.
        iri: String,
        /// The container it was detached from, if any.
        parent: Option<String>,
    },
}

impl Mutation {
    /// The IRI this record brings into existence, or removes.
    pub fn target(&self) -> &str {
        match self {
            Mutation::Write { iri, .. } => iri,
            Mutation::CreateInContainer { child, .. } => child,
            Mutation::Delete { iri, .. } => iri,
        }
    }

    /// The IRI this record requires to already exist when it is replayed, if any.
    ///
    /// This is the journal's only ordering dependency, and therefore the only thing the folding
    /// rules have to protect (see the module documentation).
    pub fn container(&self) -> Option<&str> {
        match self {
            Mutation::Write { .. } => None,
            Mutation::CreateInContainer { container, .. } => Some(container),
            Mutation::Delete { parent, .. } => parent.as_deref(),
        }
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Mutation::Write {
                iri,
                content_type,
                body,
            } => {
                out.push(TAG_WRITE);
                put_str(out, iri);
                put_str(out, content_type);
                put_bytes(out, body);
            }
            Mutation::CreateInContainer {
                container,
                child,
                content_type,
                body,
            } => {
                out.push(TAG_CREATE_IN_CONTAINER);
                put_str(out, container);
                put_str(out, child);
                put_str(out, content_type);
                put_bytes(out, body);
            }
            Mutation::Delete { iri, parent } => {
                out.push(TAG_DELETE);
                put_str(out, iri);
                match parent {
                    Some(parent) => {
                        out.push(1);
                        put_str(out, parent);
                    }
                    None => out.push(0),
                }
            }
        }
    }
}

/// Why an encoded journal could not be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes do not begin with the journal magic.
    NotAJournal,
    /// The journal was written by a newer format version.
    UnsupportedVersion(u8),
    /// The bytes end in the middle of a field.
    Truncated,
    /// A length prefix exceeds what this target can address.
    TooLarge,
    /// A record-kind or optional-field tag byte is not one this format defines.
    InvalidTag(u8),
    /// A field that must be a string is not valid UTF-8.
    InvalidUtf8,
    /// Bytes remain after the declared record count was read.
    TrailingBytes,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::NotAJournal => f.write_str("not a sparq-lws journal"),
            DecodeError::UnsupportedVersion(v) => {
                write!(f, "unsupported journal version {}", v)
            }
            DecodeError::Truncated => f.write_str("journal ends mid-record"),
            DecodeError::TooLarge => f.write_str("journal declares a length this target cannot address"),
            DecodeError::InvalidTag(tag) => write!(f, "unknown journal tag byte {}", tag),
            DecodeError::InvalidUtf8 => f.write_str("journal contains a non-UTF-8 string"),
            DecodeError::TrailingBytes => f.write_str("journal has trailing bytes after its records"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// The mutations needed to rebuild a pod, folded as they arrive.
///
/// Shared between the `PersistentStore` decorator (which appends) and the host-facing snapshot
/// accessors (which encode), so it is `Sync` and takes its own lock. On `wasm32` there is exactly
/// one thread, so the lock is never contended; it exists because `Store` requires `Sync`.
#[derive(Debug, Default)]
pub struct Journal {
    records: Mutex<Vec<Mutation>>,
    revision: AtomicU64,
}

impl Journal {
    /// An empty journal at revision 0.
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            revision: AtomicU64::new(0),
        }
    }

    /// Fold one successful mutation into the journal and bump the revision.
    ///
    /// Only call this for an operation the underlying store actually completed: a refused write
    /// (quota, 404 container) must leave the journal describing what the pod really holds.
    pub fn append(&self, mutation: Mutation) {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        fold(&mut records, mutation);
        drop(records);
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    /// A copy of the current records, in replay order.
    pub fn records(&self) -> Vec<Mutation> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// How many records the journal currently holds.
    pub fn len(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether the journal holds no records.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many mutations have been folded in since the journal was created.
    ///
    /// Monotonic, and bumped even by a mutation that folded records away, so a host can persist
    /// only when the value has moved since its last flush.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// Serialize the journal to the bytes a host persists.
    pub fn encode(&self) -> Vec<u8> {
        let records = self.records();
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.extend_from_slice(&(records.len() as u64).to_le_bytes());
        for record in &records {
            record.encode_into(&mut out);
        }
        out
    }
}

/// Read back bytes produced by [`Journal::encode`].
///
/// Every length is checked against the bytes that remain, so a truncated or corrupt snapshot is an
/// error rather than a panic or an over-large allocation.
pub fn decode(bytes: &[u8]) -> Result<Vec<Mutation>, DecodeError> {
    let mut reader = Reader { bytes, pos: 0 };
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(DecodeError::NotAJournal);
    }
    let version = reader.u8()?;
    if version != VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let count = reader.len_prefix()?;
    let mut records = Vec::new();
    for _ in 0..count {
        records.push(reader.record()?);
    }
    if reader.pos != bytes.len() {
        return Err(DecodeError::TrailingBytes);
    }
    Ok(records)
}

/// Fold `mutation` into `records`, preserving replay fidelity.
///
/// See the module documentation for the two folds and the container guard that gates them.
fn fold(records: &mut Vec<Mutation>, mutation: Mutation) {
    match &mutation {
        Mutation::Write { iri, .. } => {
            if is_unreferenced(records, iri) {
                records.retain(
                    |record| !matches!(record, Mutation::Write { iri: earlier, .. } if earlier == iri),
                );
            }
            records.push(mutation);
        }
        Mutation::CreateInContainer { .. } => records.push(mutation),
        Mutation::Delete { iri, .. } => {
            if is_unreferenced(records, iri) {
                // Replay starts from an empty pod, so dropping every record that created this IRI
                // leaves the delete with nothing to do: it is dropped too, rather than kept.
                records.retain(|record| record.target() != iri);
            } else {
                records.push(mutation);
            }
        }
    }
}

/// Whether no retained record needs `iri` to exist before it can be replayed.
fn is_unreferenced(records: &[Mutation], iri: &str) -> bool {
    !records.iter().any(|record| record.container() == Some(iri))
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    put_bytes(out, value.as_bytes());
}

fn put_bytes(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_le_bytes());
    out.extend_from_slice(value);
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(count).ok_or(DecodeError::TooLarge)?;
        let slice = self.bytes.get(self.pos..end).ok_or(DecodeError::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    /// A `u64` length prefix, narrowed to `usize` — on `wasm32` that is 32 bits, so a hostile
    /// length must be rejected rather than wrapped into a small, plausible one.
    fn len_prefix(&mut self) -> Result<usize, DecodeError> {
        let raw = self.take(8)?;
        let value = u64::from_le_bytes(raw.try_into().expect("take(8) yields eight bytes"));
        usize::try_from(value).map_err(|_| DecodeError::TooLarge)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, DecodeError> {
        let len = self.len_prefix()?;
        Ok(self.take(len)?.to_vec())
    }

    fn string(&mut self) -> Result<String, DecodeError> {
        let len = self.len_prefix()?;
        let raw = self.take(len)?;
        String::from_utf8(raw.to_vec()).map_err(|_| DecodeError::InvalidUtf8)
    }

    fn optional_string(&mut self) -> Result<Option<String>, DecodeError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.string()?)),
            other => Err(DecodeError::InvalidTag(other)),
        }
    }

    fn record(&mut self) -> Result<Mutation, DecodeError> {
        match self.u8()? {
            TAG_WRITE => Ok(Mutation::Write {
                iri: self.string()?,
                content_type: self.string()?,
                body: self.bytes()?,
            }),
            TAG_CREATE_IN_CONTAINER => Ok(Mutation::CreateInContainer {
                container: self.string()?,
                child: self.string()?,
                content_type: self.string()?,
                body: self.bytes()?,
            }),
            TAG_DELETE => Ok(Mutation::Delete {
                iri: self.string()?,
                parent: self.optional_string()?,
            }),
            other => Err(DecodeError::InvalidTag(other)),
        }
    }
}

// The `Store` decorator and its replay live behind the same cfg as the rest of the adapter: the
// `sparq-lws-core` dependency is declared only for `wasm32`. Everything above is
// target-independent and covered by ordinary `cargo test`.
#[cfg(target_arch = "wasm32")]
mod wasm_store {
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::body::Bytes;
    use sparq_lws_core::store::{
        DeleteOutcome, ReadPlan, Resource, ResourceMeta, Store, ValidatedChildIri,
    };
    use sparq_lws_core::ServerResult;

    use super::{Journal, Mutation};

    /// A [`Store`] that forwards to `inner` and records every mutation it completes.
    ///
    /// Reads are pass-through, including the `read_plan`/`read_at` overrides, so wrapping a
    /// [`CompositeStore`](sparq_lws_core::store::CompositeStore) keeps its single-round-trip read
    /// path rather than falling back to the trait's per-IRI defaults.
    pub struct PersistentStore<S: Store> {
        inner: S,
        journal: Arc<Journal>,
    }

    impl<S: Store> PersistentStore<S> {
        /// Wrap `inner`, recording into `journal`.
        pub fn new(inner: S, journal: Arc<Journal>) -> Self {
            Self { inner, journal }
        }
    }

    #[async_trait]
    impl<S: Store> Store for PersistentStore<S> {
        async fn read(&self, iri: &str) -> ServerResult<Resource> {
            self.inner.read(iri).await
        }

        async fn meta(&self, iri: &str) -> ServerResult<Option<ResourceMeta>> {
            self.inner.meta(iri).await
        }

        async fn exists(&self, iri: &str) -> ServerResult<bool> {
            self.inner.exists(iri).await
        }

        async fn write(
            &self,
            iri: &str,
            body: Bytes,
            content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            let meta = self.inner.write(iri, body.clone(), content_type).await?;
            self.journal.append(Mutation::Write {
                iri: iri.to_owned(),
                content_type: content_type.to_owned(),
                body: body.to_vec(),
            });
            Ok(meta)
        }

        async fn create_in_container(
            &self,
            container: &str,
            child: &str,
            body: Bytes,
            content_type: &str,
        ) -> ServerResult<ResourceMeta> {
            let meta = self
                .inner
                .create_in_container(container, child, body.clone(), content_type)
                .await?;
            self.journal.append(Mutation::CreateInContainer {
                container: container.to_owned(),
                child: child.to_owned(),
                content_type: content_type.to_owned(),
                body: body.to_vec(),
            });
            Ok(meta)
        }

        async fn delete(&self, iri: &str, parent: Option<&str>) -> ServerResult<()> {
            self.inner.delete(iri, parent).await?;
            self.journal.append(Mutation::Delete {
                iri: iri.to_owned(),
                parent: parent.map(str::to_owned),
            });
            Ok(())
        }

        async fn delete_container_if_empty(
            &self,
            iri: &str,
            parent: Option<&str>,
        ) -> ServerResult<DeleteOutcome> {
            let outcome = self.inner.delete_container_if_empty(iri, parent).await?;
            // `NotEmpty`/`NotFound` changed nothing, so they must not touch the journal.
            if matches!(outcome, DeleteOutcome::Deleted) {
                self.journal.append(Mutation::Delete {
                    iri: iri.to_owned(),
                    parent: parent.map(str::to_owned),
                });
            }
            Ok(outcome)
        }

        async fn list_children(&self, container: &str) -> ServerResult<Vec<ValidatedChildIri>> {
            self.inner.list_children(container).await
        }

        async fn read_plan(&self, target: &str, acl_candidates: &[String]) -> ServerResult<ReadPlan> {
            self.inner.read_plan(target, acl_candidates).await
        }

        async fn read_at(&self, iri: &str, meta: &ResourceMeta) -> ServerResult<Bytes> {
            self.inner.read_at(iri, meta).await
        }
    }

    /// Why a snapshot could not be applied to a fresh pod.
    #[derive(Debug)]
    pub struct ReplayError {
        /// Index of the record that failed, in the decoded order.
        pub record: usize,
        /// The store error it failed with.
        pub cause: String,
    }

    impl std::fmt::Display for ReplayError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "could not replay snapshot record {}: {}",
                self.record, self.cause
            )
        }
    }

    impl std::error::Error for ReplayError {}

    /// Apply `records` to `store`, in order.
    ///
    /// Fails on the first record the store refuses rather than skipping it: a snapshot that only
    /// partly applies would silently lose data, and the host can still choose to start fresh.
    /// Driving this through a [`PersistentStore`] rebuilds the journal as a side effect, so the
    /// restored pod is immediately snapshottable again.
    pub async fn replay<S: Store>(store: &S, records: Vec<Mutation>) -> Result<(), ReplayError> {
        for (index, record) in records.into_iter().enumerate() {
            let outcome = match record {
                Mutation::Write {
                    iri,
                    content_type,
                    body,
                } => store
                    .write(&iri, Bytes::from(body), &content_type)
                    .await
                    .map(|_| ()),
                Mutation::CreateInContainer {
                    container,
                    child,
                    content_type,
                    body,
                } => store
                    .create_in_container(&container, &child, Bytes::from(body), &content_type)
                    .await
                    .map(|_| ()),
                Mutation::Delete { iri, parent } => store.delete(&iri, parent.as_deref()).await,
            };
            outcome.map_err(|error| ReplayError {
                record: index,
                cause: error.to_string(),
            })?;
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm_store::{replay, PersistentStore, ReplayError};

#[cfg(test)]
mod tests {
    use super::*;

    fn write(iri: &str, body: &str) -> Mutation {
        Mutation::Write {
            iri: iri.to_owned(),
            content_type: "text/turtle".to_owned(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn create(container: &str, child: &str, body: &str) -> Mutation {
        Mutation::CreateInContainer {
            container: container.to_owned(),
            child: child.to_owned(),
            content_type: "text/turtle".to_owned(),
            body: body.as_bytes().to_vec(),
        }
    }

    fn delete(iri: &str, parent: Option<&str>) -> Mutation {
        Mutation::Delete {
            iri: iri.to_owned(),
            parent: parent.map(str::to_owned),
        }
    }

    fn journal_of(mutations: impl IntoIterator<Item = Mutation>) -> Journal {
        let journal = Journal::new();
        for mutation in mutations {
            journal.append(mutation);
        }
        journal
    }

    #[test]
    fn every_record_kind_survives_an_encode_decode_round_trip() {
        let records = vec![
            write("https://pod.example/card", "<a> <b> <c> ."),
            create("https://pod.example/box/", "https://pod.example/box/1", "one"),
            delete("https://pod.example/gone", Some("https://pod.example/box/")),
            delete("https://pod.example/loose", None),
        ];
        let journal = Journal::new();
        for record in &records {
            // Bypass folding: push each kind verbatim so the codec is what is under test.
            journal
                .records
                .lock()
                .expect("fresh journal")
                .push(record.clone());
        }
        assert_eq!(decode(&journal.encode()), Ok(records));
    }

    #[test]
    fn an_empty_journal_encodes_to_a_header_that_decodes_to_no_records() {
        assert_eq!(decode(&Journal::new().encode()), Ok(Vec::new()));
    }

    #[test]
    fn a_body_with_arbitrary_bytes_round_trips() {
        let journal = journal_of([Mutation::Write {
            iri: "https://pod.example/blob".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            body: vec![0, 159, 146, 150, 255, 0],
        }]);
        assert_eq!(decode(&journal.encode()), Ok(journal.records()));
    }

    #[test]
    fn decode_rejects_bytes_that_are_not_a_journal() {
        assert_eq!(decode(b"not a journal at all"), Err(DecodeError::NotAJournal));
        assert_eq!(decode(b""), Err(DecodeError::Truncated));
    }

    #[test]
    fn decode_rejects_a_future_format_version() {
        let mut bytes = Journal::new().encode();
        bytes[MAGIC.len()] = VERSION + 1;
        assert_eq!(decode(&bytes), Err(DecodeError::UnsupportedVersion(VERSION + 1)));
    }

    #[test]
    fn decode_rejects_a_truncated_journal_instead_of_panicking() {
        let encoded = journal_of([write("https://pod.example/card", "body")]).encode();
        for cut in MAGIC.len()..encoded.len() {
            assert_eq!(
                decode(&encoded[..cut]),
                Err(DecodeError::Truncated),
                "a journal cut at {} bytes must be an error",
                cut
            );
        }
    }

    #[test]
    fn decode_rejects_an_over_large_length_prefix_instead_of_allocating() {
        let mut encoded = journal_of([write("https://pod.example/card", "body")]).encode();
        // The first length prefix after magic + version + count + record tag is the IRI's.
        let iri_len_at = MAGIC.len() + 1 + 8 + 1;
        encoded[iri_len_at..iri_len_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(matches!(
            decode(&encoded),
            Err(DecodeError::TooLarge) | Err(DecodeError::Truncated)
        ));
    }

    #[test]
    fn decode_rejects_an_unknown_record_tag() {
        let mut encoded = journal_of([write("https://pod.example/card", "body")]).encode();
        encoded[MAGIC.len() + 1 + 8] = 99;
        assert_eq!(decode(&encoded), Err(DecodeError::InvalidTag(99)));
    }

    #[test]
    fn decode_rejects_a_non_utf8_string_field() {
        let mut encoded = journal_of([write("https://pod.example/card", "body")]).encode();
        let iri_at = MAGIC.len() + 1 + 8 + 1 + 8;
        encoded[iri_at] = 0xff;
        assert_eq!(decode(&encoded), Err(DecodeError::InvalidUtf8));
    }

    #[test]
    fn decode_rejects_bytes_appended_after_the_declared_records() {
        let mut encoded = journal_of([write("https://pod.example/card", "body")]).encode();
        encoded.push(0);
        assert_eq!(decode(&encoded), Err(DecodeError::TrailingBytes));
    }

    #[test]
    fn overwriting_a_resource_keeps_only_the_latest_body() {
        let journal = journal_of([
            write("https://pod.example/card", "v1"),
            write("https://pod.example/other", "x"),
            write("https://pod.example/card", "v2"),
            write("https://pod.example/card", "v3"),
        ]);
        assert_eq!(
            journal.records(),
            vec![
                write("https://pod.example/other", "x"),
                write("https://pod.example/card", "v3"),
            ],
            "repeated PUTs of one IRI must not grow the journal"
        );
    }

    #[test]
    fn deleting_a_resource_removes_the_records_that_created_it() {
        let journal = journal_of([
            write("https://pod.example/card", "v1"),
            create("https://pod.example/box/", "https://pod.example/box/1", "one"),
            delete("https://pod.example/box/1", Some("https://pod.example/box/")),
        ]);
        assert_eq!(
            journal.records(),
            vec![write("https://pod.example/card", "v1")],
            "a created-then-deleted child leaves nothing, not even the delete"
        );
        assert!(!journal.is_empty());
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn a_write_is_not_folded_away_while_a_record_still_needs_it_as_a_container() {
        let journal = journal_of([
            write("https://pod.example/box/", "container v1"),
            create("https://pod.example/box/", "https://pod.example/box/1", "one"),
            write("https://pod.example/box/", "container v2"),
        ]);
        assert_eq!(
            journal.records().len(),
            3,
            "dropping the first write would make the child's replay hit a missing container"
        );
        assert_eq!(journal.records()[0], write("https://pod.example/box/", "container v1"));
    }

    #[test]
    fn a_delete_is_kept_verbatim_while_a_record_still_needs_it_as_a_container() {
        let journal = journal_of([
            write("https://pod.example/box/", "container"),
            create("https://pod.example/box/", "https://pod.example/box/1", "one"),
            delete("https://pod.example/box/", None),
        ]);
        assert_eq!(
            journal.records(),
            vec![
                write("https://pod.example/box/", "container"),
                create("https://pod.example/box/", "https://pod.example/box/1", "one"),
                delete("https://pod.example/box/", None),
            ],
            "the delete must replay as itself, in order, when its IRI is still referenced"
        );
    }

    #[test]
    fn deleting_the_last_child_lets_the_container_fold_away_too() {
        let journal = journal_of([
            write("https://pod.example/box/", "container"),
            create("https://pod.example/box/", "https://pod.example/box/1", "one"),
            delete("https://pod.example/box/1", Some("https://pod.example/box/")),
            delete("https://pod.example/box/", None),
        ]);
        assert!(
            journal.is_empty(),
            "an emptied, deleted container leaves an empty journal"
        );
    }

    #[test]
    fn the_revision_advances_on_every_mutation_including_folding_ones() {
        let journal = Journal::new();
        assert_eq!(journal.revision(), 0);
        journal.append(write("https://pod.example/card", "v1"));
        assert_eq!(journal.revision(), 1);
        journal.append(write("https://pod.example/card", "v2"));
        assert_eq!(journal.revision(), 2, "a superseding write still moved the pod");
        journal.append(delete("https://pod.example/card", None));
        assert_eq!(journal.revision(), 3, "so did the delete that emptied it");
        assert!(journal.is_empty());
    }

    #[test]
    fn a_records_container_is_the_iri_its_replay_depends_on() {
        assert_eq!(write("https://pod.example/card", "v").container(), None);
        assert_eq!(
            create("https://pod.example/box/", "https://pod.example/box/1", "one").container(),
            Some("https://pod.example/box/")
        );
        assert_eq!(
            delete("https://pod.example/box/1", Some("https://pod.example/box/")).container(),
            Some("https://pod.example/box/")
        );
        assert_eq!(
            create("https://pod.example/box/", "https://pod.example/box/1", "one").target(),
            "https://pod.example/box/1"
        );
    }
}
