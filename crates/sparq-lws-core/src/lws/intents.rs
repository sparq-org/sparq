//! Durable intents: what a multi-step change would have to put back, stored before its first
//! store step and cleared once its outcome is settled. A change that a process stop cuts short
//! (mid-change, or mid-rollback) is put back when the server next starts, before it serves
//! anything.
//!
//! A [`Journal`](super::Journal) stages what each resource it will change holds now, and before
//! its first step stores that as an intent: for each resource, the state to put it back to.
//! Putting a resource back to the state it was staged in is idempotent, so the intent may name
//! steps that never ran. The intent is cleared once the change is kept, or once it is put back;
//! until it is cleared, the change's locks are held (and, after a failure, its resources are set
//! aside), so no later write to those resources can happen while it stands. That is what makes
//! replaying it at start safe: nothing newer than it can be overwritten.
//!
//! Intents are stored as members of a container whose IRI is outside the storage (a `urn:`), so
//! no request can name them, and no listing shows them.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::time::{Duration, UNIX_EPOCH};

use super::{LwsState, Undo};
use crate::error::ServerError;
use crate::store::Store;

const JSON: &str = "application/json";

/// The container the intents of the storage `storage` are kept in.
pub(crate) fn container(storage: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(storage.as_bytes());
    let id: String = digest[..12].iter().map(|b| format!("{b:02x}")).collect();
    format!("urn:x-sparq-lws:intents:{id}/")
}

/// A record as it is to be put back: its bytes and the validators [`Store::restore`] keeps.
#[derive(Serialize, Deserialize)]
struct Prior {
    body: String,
    content_type: String,
    etag: String,
    modified: Option<(u64, u32)>,
}

impl Prior {
    fn of(body: &Bytes, meta: &crate::store::ResourceMeta) -> Self {
        Self {
            body: super::jose::b64url(body),
            content_type: meta.content_type.clone(),
            etag: meta.etag.clone(),
            modified: meta.last_modified.and_then(|t| {
                let d = t.duration_since(UNIX_EPOCH).ok()?;
                Some((d.as_secs(), d.subsec_nanos()))
            }),
        }
    }

    fn into_parts(self) -> Option<(Bytes, crate::store::ResourceMeta)> {
        let body = super::jose::b64url_decode(&self.body)?;
        let meta = crate::store::ResourceMeta {
            content_type: self.content_type,
            blob_key: String::new(),
            etag: self.etag,
            last_modified: self.modified.map(|(s, n)| UNIX_EPOCH + Duration::new(s, n)),
        };
        Some((Bytes::from(body), meta))
    }
}

/// One resource of an intent: the [`Undo`] that puts it back, as stored.
#[derive(Serialize, Deserialize)]
#[serde(tag = "put")]
enum Step {
    /// A key (a resource, or a resource's metadata) put back in place, or removed.
    Restore { key: String, prior: Option<Prior> },
    /// A member removed from its container, recreated in it unless it is there.
    Recreate {
        iri: String,
        parent: Option<String>,
        prior: Prior,
    },
    /// A member that may have been created, removed from its container.
    Remove { iri: String, parent: String },
}

impl Step {
    fn of(undo: &Undo) -> Option<Self> {
        Some(match undo {
            Undo::Restore { key, prior } => Step::Restore {
                key: key.clone(),
                prior: prior.as_ref().map(|(b, m)| Prior::of(b, m)),
            },
            Undo::Recreate {
                iri,
                parent,
                body,
                meta,
            } => Step::Recreate {
                iri: iri.clone(),
                parent: parent.clone(),
                prior: Prior::of(body, meta),
            },
            Undo::Remove { iri, parent } => Step::Remove {
                iri: iri.clone(),
                parent: parent.clone(),
            },
            Undo::Locked { .. } | Undo::Forget { .. } | Undo::Resolve { .. } => return None,
        })
    }

    fn into_undo(self) -> Option<Undo> {
        Some(match self {
            Step::Restore { key, prior } => Undo::Restore {
                key,
                prior: match prior {
                    Some(p) => Some(p.into_parts()?),
                    None => None,
                },
            },
            Step::Recreate { iri, parent, prior } => {
                let (body, meta) = prior.into_parts()?;
                Undo::Recreate {
                    iri,
                    parent,
                    body,
                    meta,
                }
            }
            Step::Remove { iri, parent } => Undo::Remove { iri, parent },
        })
    }
}

/// An intent as stored: its steps in the order they are applied.
#[derive(Serialize, Deserialize)]
struct Stored {
    steps: Vec<Step>,
    /// Containers whose modification time is still to be moved on for a change that was kept
    /// (see [`super::Journal::commit`]): the intent is cleared once it is.
    #[serde(default)]
    touch: Vec<String>,
}

/// Store the intent to apply `undo` (in this order) as `record`, a new member of the intents
/// container, or replace it when `existing`.
pub(crate) async fn store<S: Store>(
    state: &LwsState<S>,
    record: &str,
    existing: bool,
    undo: &[&Undo],
    touch: &[String],
) -> Result<(), ServerError> {
    let stored = Stored {
        steps: undo.iter().filter_map(|u| Step::of(u)).collect(),
        touch: touch.to_vec(),
    };
    let body = Bytes::from(
        serde_json::to_vec(&stored).map_err(|e| ServerError::Storage(format!("intent: {e}")))?,
    );
    if existing {
        state.store.write(record, body, JSON).await.map(drop)
    } else {
        let container = container(&state.cfg.storage());
        state
            .store
            .create_in_container(&container, record, body, JSON)
            .await
            .map(drop)
    }
}

/// A new intent's IRI, in the container of `storage`.
pub(crate) fn mint(storage: &str) -> String {
    format!("{}{}", container(storage), super::jose::random_id())
}

/// Whether the intent `record` says its change was kept: it is gone (cleared), or names only
/// containers to touch. An intent that cannot be read is an error, to try again.
pub(crate) async fn kept<S: Store>(store: &S, record: &str) -> Result<bool, ServerError> {
    match store.read(record).await {
        Ok(r) => {
            let stored: Stored = serde_json::from_slice(&r.body)
                .map_err(|e| ServerError::Storage(format!("intent {record}: {e}")))?;
            Ok(stored.steps.is_empty())
        }
        Err(ServerError::NotFound) => Ok(true),
        Err(e) => Err(e),
    }
}

/// Remove the intent `record`; absent is fine. Its membership goes in the same step.
pub(crate) async fn clear<S: Store>(store: &S, record: &str) -> Result<(), ServerError> {
    let container = record.rfind('/').map_or(record, |i| &record[..=i]);
    super::delete_record(store, record, container).await
}

/// Settle every intent a process stop left, when the server starts and before it serves
/// anything: a change cut short is put back, and a kept change's container is touched. Every
/// intent is read first: one that cannot be read is an error, and then nothing has been changed
/// or started (the server does not start over a change it cannot put back). Only then is each
/// settled; what cannot be put back after a few tries is set aside (its resources answer `503`)
/// and put back in the background, once every intent has been read and settling cannot fail.
pub(crate) async fn recover<S: Store + 'static>(state: &LwsState<S>) -> Result<(), String> {
    let container = container(&state.cfg.storage());
    let store = &state.store;
    if !store
        .exists(&container)
        .await
        .map_err(|e| format!("intents: {e}"))?
    {
        store
            .write(&container, Bytes::new(), JSON)
            .await
            .map_err(|e| format!("intents: {e}"))?;
    }
    let records = store
        .list_children(&container)
        .await
        .map_err(|e| format!("intents: {e}"))?;
    let mut read = Vec::new();
    for record in records {
        let record = record.as_str().to_string();
        let unreadable =
            |why: String| format!("the change recorded at {record} cannot be put back: {why}");
        let body = match store.read(&record).await {
            Ok(r) => r.body,
            // Listed, but gone since: cleared already.
            Err(ServerError::NotFound) => continue,
            Err(e) => return Err(unreadable(e.to_string())),
        };
        let stored: Stored =
            serde_json::from_slice(&body).map_err(|e| unreadable(e.to_string()))?;
        let undo = stored
            .steps
            .into_iter()
            .map(Step::into_undo)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| unreadable("a body is not base64".into()))?;
        read.push((record, undo, stored.touch));
    }
    // Changes cut short are put back first: one may restore a container's metadata from
    // before a kept change's touch, which must then come after it.
    let mut set_aside = Vec::new();
    let mut touches = Vec::new();
    for (record, undo, touch) in read {
        if undo.is_empty() {
            touches.push((record, touch));
            continue;
        }
        let iris = super::iris_of(&undo);
        let forget = Undo::Forget {
            record: record.clone(),
            iris,
        };
        let left = match super::settle(store, undo).await {
            None => super::settle(store, vec![forget]).await,
            Some(mut left) => {
                left.0.push(forget);
                Some(left)
            }
        };
        set_aside.extend(left);
    }
    // Kept changes: their containers are touched and the intents go once they are. While a
    // change is still being put back in the background it could restore a container's metadata
    // after a touch now, so then the touches wait, owed: the containers' listings have no date
    // until the set-aside changes are settled, then they are touched ([`LwsState::set_aside`]).
    for (record, containers) in touches {
        for c in containers {
            state.owe_touch(&c, record.clone());
            if set_aside.is_empty() {
                super::resources::touch_container(state, &c).await;
            } else {
                state.touching(&c);
                state.touched(&c, false);
            }
        }
    }
    state.set_aside_all(set_aside);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::test_store::{request, snapshot, state, FlakyStore};
    use super::super::{meta_key, resources, Agent, LwsConfig, LwsState};
    use super::*;
    use axum::http::{Method, StatusCode};
    use std::sync::atomic::Ordering;

    /// The server started again over the same store, as after a process stop.
    async fn restart(store: &FlakyStore) -> LwsState<FlakyStore> {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        cfg.allow_insecure_fetch = true;
        cfg.page_size = 100;
        LwsState::new(store.clone(), cfg).await.expect("starts")
    }

    async fn call(
        st: &LwsState<FlakyStore>,
        method: Method,
        path: &str,
        h: &[(&str, &str)],
        body: &str,
    ) -> StatusCode {
        resources::handle(st, &request(method, path, h, body), &Agent::anonymous())
            .await
            .status()
    }

    async fn intents_left(st: &LwsState<FlakyStore>) -> usize {
        st.store
            .list_children(&container(&st.cfg.storage()))
            .await
            .unwrap()
            .len()
    }

    /// A turtle resource `/d` stating type A, and the snapshot of it.
    async fn typed_doc() -> (LwsState<FlakyStore>, FlakyStore, String, Vec<String>) {
        let (st, store) = state(100).await;
        let h = [("content-type", "text/turtle"), ("slug", "d")];
        assert_eq!(
            call(&st, Method::POST, "/", &h, "<> a <urn:A> .").await,
            StatusCode::CREATED
        );
        let d = st.cfg.absolute("/d");
        let before = snapshot(&st, &store, std::slice::from_ref(&d), &[]).await;
        (st, store, d, before)
    }

    /// A change whose process stops part way (its journal is dropped mid-change, never put
    /// back nor kept) is put back when the server starts again, validators included, and its
    /// intent goes.
    #[tokio::test]
    async fn a_change_cut_short_is_put_back_at_start() {
        let (st, store, d, before) = typed_doc().await;
        let mut journal = st.journal();
        journal.stage(&meta_key(&d)).await.unwrap();
        journal.stage(&d).await.unwrap();
        let mut meta = st.resource_meta(&d).await.unwrap();
        meta.pending = true;
        journal.write_meta(&d, &meta).await.unwrap();
        journal
            .write(&d, Bytes::from_static(b"<> a <urn:B> ."), "text/turtle")
            .await
            .unwrap();
        std::mem::forget(journal);
        assert_eq!(intents_left(&st).await, 1);
        assert_ne!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            before
        );
        let st = restart(&store).await;
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            before
        );
        assert_eq!(intents_left(&st).await, 0);
    }

    /// The same for a recursive delete cut short: the members it removed are back, in their
    /// containers.
    #[tokio::test]
    async fn a_delete_cut_short_is_put_back_at_start() {
        let (st, store) = state(100).await;
        let container = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
        let h = [("slug", "c"), ("link", container)];
        assert_eq!(
            call(&st, Method::POST, "/", &h, "").await,
            StatusCode::CREATED
        );
        let h = [("slug", "m"), ("content-type", "text/plain")];
        assert_eq!(
            call(&st, Method::POST, "/c/", &h, "m").await,
            StatusCode::CREATED
        );
        let (root, c, m) = (
            st.cfg.storage(),
            st.cfg.absolute("/c/"),
            st.cfg.absolute("/c/m"),
        );
        let all = [m.clone(), c.clone()];
        let listings = [root.clone(), c.clone()];
        let before = snapshot(&st, &store, &all, &listings).await;
        let mut journal = st.journal();
        for (node, parent) in [(&m, &c), (&c, &root)] {
            journal.stage_member(node, Some(parent)).await.unwrap();
            journal.stage(&meta_key(node)).await.unwrap();
        }
        journal.remove_member(&m, Some(&c)).await.unwrap();
        journal.delete_meta(&m).await.unwrap();
        std::mem::forget(journal);
        assert_ne!(snapshot(&st, &store, &all, &listings).await, before);
        let st = restart(&store).await;
        assert_eq!(snapshot(&st, &store, &all, &listings).await, before);
        assert_eq!(intents_left(&st).await, 0);
    }

    /// A change whose rollback kept failing (set aside, put back in the background) when the
    /// process stopped is put back at the next start.
    #[tokio::test]
    async fn a_rollback_cut_short_is_finished_at_start() {
        let (st, store, d, before) = typed_doc().await;
        *store.fail_write_of.lock().unwrap() = Some(d.clone());
        *store.fail_restore_of.lock().unwrap() = Some(d.clone());
        let h = [("content-type", "text/turtle")];
        let status = call(&st, Method::PUT, "/d", &h, "<> a <urn:B> .").await;
        assert!(status.is_server_error(), "{status}");
        assert_eq!(
            call(&st, Method::GET, "/d", &[], "").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(intents_left(&st).await, 1);
        // The process stops; the store is well again by the next start.
        *store.fail_write_of.lock().unwrap() = None;
        *store.fail_restore_of.lock().unwrap() = None;
        let st = restart(&store).await;
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            before
        );
        assert_eq!(intents_left(&st).await, 0);
    }

    /// A change whose intent cannot be recorded as kept once it is done is put back instead
    /// (a start could replay the intent over it) and the request fails; when the intent cannot
    /// be cleared either, its resource stays set aside until it is.
    #[tokio::test]
    async fn a_change_whose_intent_stays_is_put_back() {
        let (st, store, d, before) = typed_doc().await;
        let under = Some(container(&st.cfg.storage()));
        let h = [("content-type", "text/turtle")];
        // The intent cannot be rewritten as kept: the change is put back, and it is cleared.
        // Only the rewrite fails: the first write of an intent is a create.
        *store.fail_write_under.lock().unwrap() = under.clone();
        let status = call(&st, Method::PUT, "/d", &h, "<> a <urn:B> .").await;
        assert!(status.is_server_error(), "{status}");
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            before
        );
        assert_eq!(intents_left(&st).await, 0);
        // Nor cleared: set aside until it is.
        store.fail_delete.store(true, Ordering::SeqCst);
        let status = call(&st, Method::PUT, "/d", &h, "<> a <urn:B> .").await;
        assert!(status.is_server_error(), "{status}");
        assert_eq!(
            call(&st, Method::GET, "/d", &[], "").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        *store.fail_write_under.lock().unwrap() = None;
        store.fail_delete.store(false, Ordering::SeqCst);
        let mut waited = 0;
        while call(&st, Method::GET, "/d", &[], "").await == StatusCode::SERVICE_UNAVAILABLE {
            assert!(waited < 100, "still set aside");
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            before
        );
        assert_eq!(intents_left(&st).await, 0);
    }

    /// Review finding: a kept change cleared its intent before its container was touched, so a
    /// stop in between left the container's date from before the change, with nothing to move it
    /// on. The intent now names the container until the touch lands, and a start touches it.
    #[tokio::test]
    async fn a_kept_change_whose_touch_was_cut_short_is_touched_at_start() {
        let (st, store) = state(100).await;
        let container_link = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
        let h = [("slug", "c"), ("link", container_link)];
        assert_eq!(
            call(&st, Method::POST, "/", &h, "").await,
            StatusCode::CREATED
        );
        let h = [("slug", "m"), ("content-type", "text/plain")];
        assert_eq!(
            call(&st, Method::POST, "/c/", &h, "m").await,
            StatusCode::CREATED
        );
        let (c, m) = (st.cfg.absolute("/c/"), st.cfg.absolute("/c/m"));
        let dated = st.resource_meta(&c).await.unwrap().version;
        let mut journal = st.journal();
        journal.stage_member(&m, Some(&c)).await.unwrap();
        journal.stage(&meta_key(&m)).await.unwrap();
        journal.remove_member(&m, Some(&c)).await.unwrap();
        journal.delete_meta(&m).await.unwrap();
        // Kept, and the process stops before the touch.
        assert!(journal.commit(Some(&c)).await.is_ok());
        assert_eq!(intents_left(&st).await, 1);
        let st = restart(&store).await;
        assert_ne!(st.resource_meta(&c).await.unwrap().version, dated);
        assert!(!st.store.exists(&m).await.unwrap(), "the delete is kept");
        assert_eq!(intents_left(&st).await, 0);
    }

    /// A container `/c/` holding `/c/m`.
    async fn a_member() -> (LwsState<FlakyStore>, FlakyStore, String, String) {
        let (st, store) = state(100).await;
        let container_link = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
        let h = [("slug", "c"), ("link", container_link)];
        assert_eq!(
            call(&st, Method::POST, "/", &h, "").await,
            StatusCode::CREATED
        );
        let h = [("slug", "m"), ("content-type", "text/plain")];
        assert_eq!(
            call(&st, Method::POST, "/c/", &h, "m").await,
            StatusCode::CREATED
        );
        let (c, m) = (st.cfg.absolute("/c/"), st.cfg.absolute("/c/m"));
        (st, store, c, m)
    }

    /// A journal that has removed the member `m` of `c` (its intent stored), not yet kept.
    async fn removing<'a>(
        st: &'a LwsState<FlakyStore>,
        c: &str,
        m: &str,
    ) -> super::super::Journal<'a, FlakyStore> {
        let mut journal = st.journal();
        journal.stage_member(m, Some(c)).await.unwrap();
        journal.stage(&meta_key(m)).await.unwrap();
        journal.remove_member(m, Some(c)).await.unwrap();
        journal.delete_meta(m).await.unwrap();
        journal
    }

    /// Review finding: when the store recorded a change as kept but the reply was lost, the
    /// change was put back from an intent that no longer held the plan to finish that, should
    /// the process stop. The intent is read back first: it says kept, so the change is kept.
    #[tokio::test]
    async fn a_kept_change_whose_reply_was_lost_stays_kept() {
        let (st, store, c, m) = a_member().await;
        let journal = removing(&st, &c, &m).await;
        let record = journal.intent.clone().unwrap();
        let dated = st.resource_meta(&c).await.unwrap().version;
        *store.fail_after_write_of.lock().unwrap() = Some(record);
        assert!(journal.commit(Some(&c)).await.is_ok());
        *store.fail_after_write_of.lock().unwrap() = None;
        assert!(!st.store.exists(&m).await.unwrap(), "the delete is kept");
        // A stop before the touch: the next start touches the container.
        let st = restart(&store).await;
        assert_ne!(st.resource_meta(&c).await.unwrap().version, dated);
        assert!(!st.store.exists(&m).await.unwrap());
        assert_eq!(intents_left(&st).await, 0);
    }

    /// And when the intent cannot be read back either, the outcome is not known: the request
    /// fails with the change left to settle from the intent later, kept or put back, by a start
    /// or in the background, but never put back blindly.
    #[tokio::test]
    async fn a_change_whose_outcome_is_unknown_is_settled_from_its_intent() {
        // The store kept it (the reply was lost); the process stops; the start keeps it.
        let (st, store, c, m) = a_member().await;
        let journal = removing(&st, &c, &m).await;
        let record = journal.intent.clone().unwrap();
        *store.fail_after_write_of.lock().unwrap() = Some(record.clone());
        *store.fail_read_of.lock().unwrap() = Some(record.clone());
        let (_, left) = journal.commit(Some(&c)).await.expect_err("unknown");
        assert!(matches!(
            left.as_ref().map(|l| &l.0[..]),
            Some([Undo::Resolve { .. }])
        ));
        *store.fail_after_write_of.lock().unwrap() = None;
        *store.fail_read_of.lock().unwrap() = None;
        drop(left);
        let st = restart(&store).await;
        assert!(!st.store.exists(&m).await.unwrap(), "the delete is kept");
        assert_eq!(intents_left(&st).await, 0);

        // The store did not keep it; settled in the background, it is put back.
        let (st, store, c, m) = a_member().await;
        let journal = removing(&st, &c, &m).await;
        let record = journal.intent.clone().unwrap();
        *store.fail_write_under.lock().unwrap() = Some(container(&st.cfg.storage()));
        *store.fail_read_of.lock().unwrap() = Some(record);
        let (_, left) = journal.commit(Some(&c)).await.expect_err("unknown");
        st.set_aside(left.expect("to settle"), ());
        *store.fail_write_under.lock().unwrap() = None;
        *store.fail_read_of.lock().unwrap() = None;
        let mut waited = 0;
        while call(&st, Method::GET, "/c/m", &[], "").await != StatusCode::OK {
            assert!(waited < 100, "still set aside");
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
        assert_eq!(intents_left(&st).await, 0);
    }

    /// Review finding: a start touched a container for a kept change, and cleared that intent,
    /// before putting back an earlier change cut short that restores the container's metadata
    /// from before the touch, so its date went backwards. Changes are put back first.
    #[tokio::test]
    async fn a_start_touches_after_putting_back() {
        let (st, store) = state(100).await;
        let container_link = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
        let h = [("slug", "c"), ("link", container_link)];
        assert_eq!(
            call(&st, Method::POST, "/", &h, "").await,
            StatusCode::CREATED
        );
        let c = st.cfg.absolute("/c/");
        let dated = st.resource_meta(&c).await.unwrap().version;
        // Listed first: a kept change still owing its touch of `/c/`.
        store_intent(&st, std::slice::from_ref(&c)).await;
        // Then a change cut short that would put `/c/`'s metadata back as it is now.
        let mut journal = st.journal();
        journal.stage(&meta_key(&c)).await.unwrap();
        let mut meta = st.resource_meta(&c).await.unwrap();
        meta.pending = true;
        journal.write_meta(&c, &meta).await.unwrap();
        std::mem::forget(journal);
        let st = restart(&store).await;
        assert_ne!(st.resource_meta(&c).await.unwrap().version, dated);
        assert_eq!(intents_left(&st).await, 0);
    }

    /// Wait until `done` holds of `st`, or fail.
    async fn until(
        st: &LwsState<FlakyStore>,
        what: &str,
        done: impl Fn(&LwsState<FlakyStore>) -> bool,
    ) {
        let mut waited = 0;
        while !done(st) {
            assert!(waited < 100, "{what}");
            tokio::time::sleep(Duration::from_millis(100)).await;
            waited += 1;
        }
    }

    /// Review finding: touches that waited at start on a change set aside were never made once
    /// it was settled, so the container's listing stayed without a date and the intent stayed.
    #[tokio::test]
    async fn a_touch_that_waited_at_start_is_made_once_settled() {
        let (st, store, d, before) = typed_doc().await;
        let root = st.cfg.storage();
        let dated = st.resource_meta(&root).await.unwrap().version;
        store_intent(&st, std::slice::from_ref(&root)).await;
        // A change cut short that cannot be put back yet.
        let mut journal = st.journal();
        journal.stage(&meta_key(&d)).await.unwrap();
        journal.stage(&d).await.unwrap();
        journal
            .write(&d, Bytes::from_static(b"<> a <urn:B> ."), "text/turtle")
            .await
            .unwrap();
        std::mem::forget(journal);
        *store.fail_restore_of.lock().unwrap() = Some(d.clone());
        let st = restart(&store).await;
        assert!(st.is_untouched(&root), "the touch waits");
        assert_eq!(st.resource_meta(&root).await.unwrap().version, dated);
        *store.fail_restore_of.lock().unwrap() = None;
        until(&st, "no touch owed", |st| {
            !st.is_untouched(&root) && !st.owes_touch(&root)
        })
        .await;
        assert_ne!(st.resource_meta(&root).await.unwrap().version, dated);
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            before
        );
        assert_eq!(intents_left(&st).await, 0);
    }

    /// Review finding: a change settled in the background as kept was never followed by its
    /// container's touch. It is now, and the intent goes.
    #[tokio::test]
    async fn a_kept_change_settled_in_the_background_dates_its_container() {
        let (st, store, c, m) = a_member().await;
        let dated = st.resource_meta(&c).await.unwrap().version;
        let journal = removing(&st, &c, &m).await;
        let record = journal.intent.clone().unwrap();
        *store.fail_after_write_of.lock().unwrap() = Some(record.clone());
        *store.fail_read_of.lock().unwrap() = Some(record);
        let (_, left) = journal.commit(Some(&c)).await.expect_err("unknown");
        assert!(st.is_untouched(&c));
        st.set_aside(left.expect("to settle"), ());
        *store.fail_after_write_of.lock().unwrap() = None;
        *store.fail_read_of.lock().unwrap() = None;
        until(&st, "no touch owed", |st| {
            !st.is_untouched(&c) && !st.owes_touch(&c)
        })
        .await;
        assert_ne!(st.resource_meta(&c).await.unwrap().version, dated);
        assert!(!st.store.exists(&m).await.unwrap(), "the delete is kept");
        assert_eq!(intents_left(&st).await, 0);
    }

    /// Review finding: at start, each change set aside was hidden and put back in turn, so one
    /// put back early could touch a container owed a touch before a later one, still to be
    /// hidden, restored that container's metadata from before the touch: its date went back,
    /// with no touch owed. Every change is now hidden before any is put back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn every_change_set_aside_at_start_is_hidden_before_any_is_put_back() {
        // The interleaving it guards against depends on scheduling: try it a few times.
        for _ in 0..5 {
            let (st, _store) = state(100).await;
            let container_link = "<https://www.w3.org/ns/lws#Container>; rel=\"type\"";
            let h = [("slug", "c"), ("link", container_link)];
            assert_eq!(
                call(&st, Method::POST, "/", &h, "").await,
                StatusCode::CREATED
            );
            let c = st.cfg.absolute("/c/");
            let dated = st.resource_meta(&c).await.unwrap().version;
            // A change to `/c/`'s metadata, whose undo puts back the metadata as it is now.
            let mut journal = st.journal();
            journal.stage(&meta_key(&c)).await.unwrap();
            let mut meta = st.resource_meta(&c).await.unwrap();
            meta.pending = true;
            journal.write_meta(&c, &meta).await.unwrap();
            clear(&st.store, journal.intent.as_ref().unwrap())
                .await
                .unwrap();
            let restores = super::super::Unsettled(std::mem::take(&mut journal.undo));
            std::mem::forget(journal);
            // A kept change owing `/c/` a touch, as a start leaves it.
            let record = mint(&st.cfg.storage());
            store(&st, &record, false, &[], std::slice::from_ref(&c))
                .await
                .unwrap();
            st.owe_touch(&c, record);
            st.touching(&c);
            st.touched(&c, false);
            // Many changes quick to put back, each touching what is owed once it is, and last the
            // one restoring `/c/`: a start that set each aside in turn would touch `/c/` first.
            let quick = |i: usize| {
                super::super::Unsettled(vec![Undo::Forget {
                    record: format!("{}q{i}", container(&st.cfg.storage())),
                    iris: vec![st.cfg.absolute(&format!("/q{i}"))],
                }])
            };
            let mut all: Vec<_> = (0..5000).map(quick).collect();
            all.push(restores);
            st.set_aside_all(all);
            until(&st, "settled and touched", |st| {
                st.visible(&c) && !st.is_untouched(&c) && !st.owes_touch(&c)
            })
            .await;
            let now = st.resource_meta(&c).await.unwrap();
            assert!(!now.pending, "put back");
            assert_ne!(now.version, dated, "touched after it was put back");
        }
    }

    /// Store an intent naming only `touch`, as a kept change leaves it.
    async fn store_intent(st: &LwsState<FlakyStore>, touch: &[String]) {
        let record = mint(&st.cfg.storage());
        store(st, &record, false, &[], touch).await.unwrap();
    }

    /// Review finding: recovery set aside what it could not yet put back, in tasks that outlived
    /// a start that then failed on a later intent, and could put an old state back over writes
    /// made after a later start. Every intent is read before anything is settled or started.
    #[tokio::test]
    async fn a_start_that_fails_leaves_nothing_running() {
        let (st, store, d, _) = typed_doc().await;
        // A change cut short (its process stopped), which cannot be put back yet, and an
        // intent that cannot be read.
        let mut journal = st.journal();
        journal.stage(&meta_key(&d)).await.unwrap();
        journal.stage(&d).await.unwrap();
        journal
            .write(&d, Bytes::from_static(b"<> a <urn:B> ."), "text/turtle")
            .await
            .unwrap();
        std::mem::forget(journal);
        *store.fail_restore_of.lock().unwrap() = Some(d.clone());
        let h = [("content-type", "text/turtle")];
        let bad = mint(&st.cfg.storage());
        st.store
            .create_in_container(
                &container(&st.cfg.storage()),
                &bad,
                Bytes::from_static(b"not json"),
                JSON,
            )
            .await
            .unwrap();
        drop(st);
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        assert!(LwsState::new(store.clone(), cfg).await.is_err());
        // The unreadable intent is repaired; the store is well; the next start puts `/d` back,
        // and a write after it stands, however long a task of the failed start might wait.
        clear(&store, &bad).await.unwrap();
        *store.fail_restore_of.lock().unwrap() = None;
        let st = restart(&store).await;
        let status = call(&st, Method::PUT, "/d", &h, "<> a <urn:C> .").await;
        assert!(status.is_success(), "{status}");
        let after = snapshot(&st, &store, std::slice::from_ref(&d), &[]).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            after
        );
    }

    /// A change kept leaves no intent, and a start over a clean store has none to put back.
    #[tokio::test]
    async fn a_kept_change_leaves_no_intent() {
        let (st, store, d, before) = typed_doc().await;
        let h = [("content-type", "text/turtle")];
        assert!(call(&st, Method::PUT, "/d", &h, "<> a <urn:B> .")
            .await
            .is_success());
        assert_eq!(intents_left(&st).await, 0);
        let after = snapshot(&st, &store, std::slice::from_ref(&d), &[]).await;
        assert_ne!(after, before);
        let st = restart(&store).await;
        assert_eq!(
            snapshot(&st, &store, std::slice::from_ref(&d), &[]).await,
            after
        );
    }

    /// An intent that cannot be read stops the start: the server does not serve over a change
    /// it cannot put back.
    #[tokio::test]
    async fn an_unreadable_intent_stops_the_start() {
        let (st, store) = state(100).await;
        let record = mint(&st.cfg.storage());
        st.store
            .create_in_container(
                &container(&st.cfg.storage()),
                &record,
                Bytes::from_static(b"not json"),
                JSON,
            )
            .await
            .unwrap();
        let mut cfg = LwsConfig::new("http://localhost:3000");
        cfg.open = true;
        let err = LwsState::new(store.clone(), cfg)
            .await
            .err()
            .expect("refused");
        assert!(err.contains(&record), "{err}");
    }
}
