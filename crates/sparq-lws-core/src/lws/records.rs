//! The lifecycle of a service container's records (access grants and requests, notification
//! subscriptions), shared by every kind so their rules cannot drift: a record is created in one
//! store write and removed in one store step, memory follows the store (a record is in force
//! only once it is confirmed stored, and leaves only once it is confirmed gone), the locks a
//! request takes are waited for a bounded time ([`within_wait`]), the steps after the locks are
//! held run in a task holding the request's admission, and each change is announced once it is
//! made ([`announce_record`]).

use super::*;

/// Delete the stored record `iri` of a service container (a grant, a request, a subscription).
/// The index commit is the deletion point: when the store reports a failure but the record no
/// longer exists (what failed was the cleanup of its bytes, which the reconciler collects), the
/// record is gone.
pub(crate) async fn delete_record<S: Store>(
    store: &S,
    iri: &str,
    container: &str,
) -> Result<(), crate::error::ServerError> {
    match remove_member(store, iri, Some(container)).await {
        Ok(_) | Err(crate::error::ServerError::NotFound) => Ok(()),
        Err(e) => match store.exists(iri).await {
            Ok(false) => Ok(()),
            _ => Err(e),
        },
    }
}

/// Whether the record `iri` is stored as a start would load it: listed in `container` and there
/// to read. An error when that cannot be told. This is only an answer because a [`Store`] reads
/// its own writes (see the trait): a listing that lagged a committed write would say `false` for
/// a record the next start loads.
pub(crate) async fn still_listed<S: Store>(
    store: &S,
    iri: &str,
    container: &str,
) -> Result<bool, crate::error::ServerError> {
    let listed = store
        .list_children(container)
        .await?
        .iter()
        .any(|child| child.as_str() == iri);
    if !listed {
        return Ok(false);
    }
    match store.read(iri).await {
        Ok(_) => Ok(true),
        Err(crate::error::ServerError::NotFound) => Ok(false),
        Err(e) => Err(e),
    }
}

/// How long a request to a service container (a create, a revocation) waits for a lock before
/// answering `503`: a request never queues behind a lock held by a change that is being put
/// back, or by a store call that stalls.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Take a lock (`lock`, from [`resources::IriLocks`]) in the request path, waiting at most
/// [`LOCK_WAIT`]: `503` when it is busy, or set aside. Only once it is held may the request
/// start a task of its own, so no request leaves a detached task waiting on a lock.
pub(crate) async fn within_wait<T>(
    lock: impl std::future::Future<Output = Option<T>>,
) -> Result<T, Response> {
    match tokio::time::timeout(LOCK_WAIT, lock).await {
        Ok(Some(guard)) => Ok(guard),
        Ok(None) => Err(resources::set_aside_meanwhile()),
        Err(_) => Err(resources::retry_later("the resource is busy")),
    }
}

/// Store a new record (an access grant or request, a subscription) at `iri` in `container` in
/// one write and,
/// once it is confirmed stored, put it in force in memory with `register`. Memory follows the
/// store, and the answer says only what is confirmed:
///
/// - The container is taken (shared, unless the caller holds it exclusively) in the request,
///   waiting at most [`LOCK_WAIT`] (then `503`), so a conditional create sees no member
///   arrive between its check and its own registration. The write and the registration then run
///   in a task of their own, holding the request's share of its admission permit (`admission`):
///   a client that goes away cannot stop a stored record from being registered.
/// - The record's own lock is held from before it is created until it is registered, so a
///   revocation of it waits for that. A name already stored is refused.
/// - A write that succeeds registers the record, and announces it (`Create`, its `author` one
///   of who may hear of it, [`announce_record`]) as it does. One that fails is checked against the listing
///   ([`still_listed`]): stored after all, it is registered; not stored, `register` (and the
///   quota place it holds) is dropped and the create fails; not known, the create answers that,
///   and a task holding the record's lock and the quota place settles it from the listing once
///   the store answers, registering it under the container's lock, shared. A start loads whatever the listing holds.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_record<S, F>(
    state: &LwsState<S>,
    container: &str,
    iri: &str,
    body: Bytes,
    admission: Option<crate::overload::AdmissionSlot>,
    held: Option<resources::IriGuard>,
    author: Option<String>,
    register: F,
) -> Result<(), Response>
where
    S: Store + 'static,
    F: FnOnce() + Send + 'static,
{
    // Announced as it is put in force, wherever that happens (in the request's task, or in the
    // task that settles a write whose outcome was not known), so no record in force goes
    // unannounced, and none is announced that is not.
    let register = {
        let (state, container, iri) = (state.clone(), container.to_string(), iri.to_string());
        move || {
            register();
            announce_record(&state, "Create", &iri, &container, author);
        }
    };
    use crate::error::ServerError;
    let failed = |e: ServerError| problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    let shared = match held {
        Some(_) => None,
        None => Some(within_wait(state.locks.read(container)).await?),
    };
    // A new IRI: its lock is free unless the name was just taken, and it is only tried, so
    // taking it under the container's cannot deadlock.
    let Some(own) = state.locks.try_lock(iri) else {
        return Err(failed(ServerError::Conflict(
            "the record's name is taken".into(),
        )));
    };
    let (state, container, iri) = (state.clone(), container.to_string(), iri.to_string());
    let task = async move {
        let _admission = admission;
        let _container = (shared, held);
        // Under the lock, the name must not be stored either (a quarantined record holds its
        // name, and no lock).
        match state.store.exists(&iri).await {
            Ok(false) => {}
            Ok(true) => return Err(ServerError::Conflict("the record's name is taken".into())),
            Err(e) => return Err(e),
        }
        let written = state
            .store
            .create_in_container(&container, &iri, body, LWS_JSON)
            .await;
        let Err(e) = written else {
            register();
            return Ok(());
        };
        match still_listed(&state.store, &iri, &container).await {
            Ok(true) => {
                register();
                Ok(())
            }
            Ok(false) => Err(e),
            Err(_) => {
                // Settled holding the record's own lock (a revocation of it waits); the
                // container's is taken only to register it.
                let state = state.clone();
                tokio::spawn(async move {
                    let _own = own;
                    if until_done(|| still_listed(&state.store, &iri, &container)).await {
                        // Registered under the container's lock, shared (taken after the
                        // record's, as a revocation takes them), so no conditional create sees
                        // the listing change between its check and its own registration. This
                        // task holds no admission, so it may wait for it.
                        let _listing =
                            until_done(|| async { state.locks.read(&container).await.ok_or(()) })
                                .await;
                        register();
                    }
                });
                Err(ServerError::Storage(
                    "whether the record was stored is not known; the request may be retried".into(),
                ))
            }
        }
    };
    tokio::spawn(task)
        .await
        .unwrap_or_else(|e| Err(ServerError::Storage(format!("the create failed: {e}"))))
        .map_err(failed)
}

/// The preconditions of a create in a service container (grants, requests, subscriptions),
/// evaluated against `listing` with the container held exclusively: `Ok(None)` when the create is
/// unconditional, `Ok(Some(guard))` when its preconditions hold (the guard is passed on to
/// [`create_record`], so no member arrives or leaves until the new one is registered), and the
/// response to send otherwise. A listing that cannot be produced (a representation the client does
/// not accept, a page that does not exist) is that response: its absent `ETag` is not the absence
/// of the container.
pub(crate) async fn service_preconditions<S: Store + 'static>(
    state: &LwsState<S>,
    req: &LwsRequest,
    container: &str,
    listing: impl FnOnce(&LwsRequest) -> axum::response::Response,
) -> Result<Option<resources::IriGuard>, axum::response::Response> {
    if !resources::is_conditional(req) {
        return Ok(None);
    }
    let guard = within_wait(state.locks.lock(container)).await?;
    let current = listing(&resources::plain_get(req));
    if !current.status().is_success() {
        return Err(current);
    }
    let (etag, modified) = resources::validators_of(&current);
    match resources::unless_preconditions(req, etag.as_deref(), modified) {
        Some(refused) => Err(refused),
        None => Ok(Some(guard)),
    }
}

/// What [`remove_record`] found.
pub(crate) enum Removed {
    /// The record was removed, and is confirmed gone.
    Gone,
    /// The record was not in force when its locks were held: removed by another request
    /// meanwhile.
    Absent,
}

/// Remove the stored record `iri` from `container` in one store step (`what` names it in an
/// answer: "grant", "subscription"). Memory follows the store:
///
/// - The record's own lock is taken first (a create of it still settling holds it, see
///   [`create_record`]), then the container's, shared, both in the request, each waiting at most
///   [`LOCK_WAIT`] (then `503`): a removal that stalls in the store answers later requests `503`
///   rather than queueing them.
/// - The steps then run in a task of their own, holding the locks and `admission` (the request's
///   share of its admission permit), so a client that goes away cannot cut them short:
///   `set_aside` takes the record out of force while its removal runs (`None` when it is not in
///   force: [`Removed::Absent`]); the record is deleted ([`delete_record`]), and a failure is
///   checked against the listing ([`still_listed`]); `settle` is told whether it is confirmed
///   gone, and removes it from memory or puts it back, giving the author to announce it to.
/// - Only a record confirmed gone is announced (`Delete`, [`announce_record`]), once its locks
///   are released. Otherwise the answer is `500`, saying whether it is still stored or that this
///   is not known (it is left as it was, and the request may be retried).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn remove_record<S, H, A, Z>(
    state: &LwsState<S>,
    container: &str,
    iri: &str,
    what: &'static str,
    admission: Option<crate::overload::AdmissionSlot>,
    set_aside: A,
    settle: Z,
) -> Result<Removed, Response>
where
    S: Store + 'static,
    H: Send + 'static,
    A: FnOnce() -> Option<H> + Send + 'static,
    Z: FnOnce(bool, H) -> Option<String> + Send + 'static,
{
    let own = within_wait(state.locks.lock(iri)).await?;
    let listing = within_wait(state.locks.read(container)).await?;
    let (state, container, iri) = (state.clone(), container.to_string(), iri.to_string());
    let task = async move {
        let _admission = admission;
        let Some(held) = set_aside() else {
            return Ok(Removed::Absent);
        };
        let gone = match delete_record(&state.store, &iri, &container).await {
            Ok(()) => Some(true),
            Err(_) => still_listed(&state.store, &iri, &container)
                .await
                .ok()
                .map(|listed| !listed),
        };
        let author = settle(gone == Some(true), held);
        drop((own, listing));
        match gone {
            Some(true) => {
                announce_record(&state, "Delete", &iri, &container, author);
                Ok(Removed::Gone)
            }
            Some(false) => Err(format!(
                "the {what} could not be removed; it is still stored"
            )),
            None => Err(format!(
                "whether the {what} was removed is not known; it is left as it was, and the \
                 request may be retried"
            )),
        }
    };
    match tokio::spawn(task).await {
        Ok(Ok(removed)) => Ok(removed),
        Ok(Err(why)) => Err(problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&why))),
        Err(e) => Err(problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            Some(&e.to_string()),
        )),
    }
}

/// Announce that the service record `iri` in `container` (an access grant or request, a
/// subscription) was created or removed (`kind`) to the subscriptions whose topics cover it and
/// who may read it: the storage owner and its `author`. In the background, once the change is
/// made: nothing waits on it, and no lock is held for it.
pub(crate) fn announce_record<S: Store + 'static>(
    state: &LwsState<S>,
    kind: &'static str,
    iri: &str,
    container: &str,
    author: Option<String>,
) {
    let relation = if kind == "Create" { "target" } else { "origin" };
    let event = super::notify::Event {
        kind,
        uri: iri.to_string(),
        is_container: false,
        relation: Some((relation, container.to_string())),
    };
    let state = state.clone();
    tokio::spawn(async move { state.notify.announce_service(&state, event, author).await });
}
