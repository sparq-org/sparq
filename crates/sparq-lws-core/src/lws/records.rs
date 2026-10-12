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
/// record is gone. One that still exists after a failure is not shown to stay unless the failure
/// is settled (see [`still_listed`]): the error is returned as it is, so the caller can tell.
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
/// to read. An error when that cannot be told.
///
/// What it reads is final only for a write that is settled. One that failed
/// [unsettled](crate::error::ServerError::Unsettled) (an HTTP update that timed out) can still
/// commit afterwards, until its fence ([`Store::fence`]) has succeeded: before that, a create
/// not listed may still be stored, and a delete still listed may still remove it.
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
///   of who may hear of it, [`announce_record`]) as it does. One that fails is checked against
///   the listing ([`still_listed`]): stored after all, it is registered; not stored, and the
///   write settled, `register` (and the quota place it holds) is dropped and the create fails.
///   Otherwise, the create answers that whether it was stored is not known, and a task holding
///   the record's lock and the quota place settles it once the store answers, registering it
///   under the container's lock, shared, if it is stored. A write that failed
///   [unsettled](crate::error::ServerError::Unsettled) (an HTTP update that timed out, whose
///   worker may still be running) is fenced first ([`Store::fence`]), so a listing that does not
///   show it is final: its quota place is released only once it can no longer commit. A start
///   loads whatever the listing holds.
/// - The change holds a place among those being made or settled ([`Settlements`]) from the
///   request until it is settled: `503` when there is none. The task that settles one is the
///   state's, and ends when the state is dropped.
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
    F: FnOnce(&LwsState<S>) + Send + 'static,
{
    // Announced as it is put in force, wherever that happens (in the request's task, or in the
    // task that settles a write whose outcome was not known), so no record in force goes
    // unannounced, and none is announced that is not.
    let register = {
        let (container, iri) = (container.to_string(), iri.to_string());
        move |state: &LwsState<S>| {
            register(state);
            announce_record(state, "Create", &iri, &container, author);
        }
    };
    use crate::error::ServerError;
    let failed = |e: ServerError| problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string()));
    let Some(place) = state.settlements.reserve() else {
        return Err(settlements_full());
    };
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
            register(&state);
            return Ok(());
        };
        // A record listed is stored for good (its lock keeps a revocation of it out). One not
        // listed is not stored only once the write is settled: a write that failed unsettled
        // (an HTTP update that timed out, whose worker may still be running) can still commit
        // after the listing was read, until its fence succeeds.
        let unsettled = match &e {
            ServerError::Unsettled { fence, .. } => Some(fence.clone()),
            _ => None,
        };
        match still_listed(&state.store, &iri, &container).await {
            Ok(true) => {
                register(&state);
                Ok(())
            }
            Ok(false) if unsettled.is_none() => Err(e),
            Ok(false) | Err(_) => {
                // Settled by a task the state owns, holding the record's own lock (a
                // revocation of it waits), the quota place (`register` holds it) and this
                // change's place among those being settled; the container's lock is taken
                // only to register it. An unsettled write is fenced first, so the listing it
                // then reads is final.
                let (weak, store, locks) =
                    (state.downgrade(), state.store.clone(), state.locks.clone());
                state.settlements.spawn(async move {
                    let _held = (own, place);
                    if let Some(fence) = unsettled {
                        until_done(|| store.fence(&fence)).await;
                    }
                    if !until_done(|| still_listed(&store, &iri, &container)).await {
                        return;
                    }
                    // Registered under the container's lock, shared (taken after the record's,
                    // as a revocation takes them), so no conditional create sees the listing
                    // change between its check and its own registration. This task holds no
                    // admission, so it may wait for it.
                    let _listing =
                        until_done(|| async { locks.read(&container).await.ok_or(()) }).await;
                    if let Some(state) = weak.upgrade() {
                        register(&state);
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
/// - A delete that failed [unsettled](crate::error::ServerError::Unsettled) (an HTTP update that
///   timed out) can still remove the record after the listing showed it. Still listed (or the
///   listing unreadable), it is settled in the background: the record stays out of force, and
///   its own lock held, until the delete is fenced ([`Store::fence`]) and the listing read
///   again; `settle` is then told the outcome under the container's lock, shared, and a record
///   gone is announced. The answer is `500`, saying this is not known yet.
/// - As for a create, the removal holds a place among those being made or settled
///   ([`Settlements`]) until it is settled (`503` when there is none), and the task that settles
///   it is the state's.
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
    Z: FnOnce(&LwsState<S>, bool, H) -> Option<String> + Send + 'static,
{
    let Some(place) = state.settlements.reserve() else {
        return Err(settlements_full());
    };
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
            // A record not listed is gone for good (a delete only removes, and the record's
            // lock keeps a create of it out). One still listed may yet go: the delete failed
            // unsettled and can still commit, until its fence succeeds.
            Err(crate::error::ServerError::Unsettled { fence, .. }) => {
                match still_listed(&state.store, &iri, &container).await {
                    Ok(false) => Some(true),
                    Ok(true) | Err(_) => {
                        // Settled by a task the state owns, holding the record's own lock and
                        // this change's place among those being settled, with the record out
                        // of force until then; the container's lock is taken again only to
                        // settle it.
                        drop(listing);
                        let (weak, store, locks) =
                            (state.downgrade(), state.store.clone(), state.locks.clone());
                        let (iri, container) = (iri.clone(), container.clone());
                        state.settlements.spawn(async move {
                            let _place = place;
                            until_done(|| store.fence(&fence)).await;
                            let listed =
                                until_done(|| still_listed(&store, &iri, &container)).await;
                            let listing =
                                until_done(|| async { locks.read(&container).await.ok_or(()) })
                                    .await;
                            let Some(state) = weak.upgrade() else {
                                return;
                            };
                            let author = settle(&state, !listed, held);
                            drop((own, listing));
                            if !listed {
                                announce_record(&state, "Delete", &iri, &container, author);
                            }
                        });
                        return Err(format!(
                            "whether the {what} was removed is not known yet; it is out of \
                             force until that is settled, and the request may be retried"
                        ));
                    }
                }
            }
            Err(_) => still_listed(&state.store, &iri, &container)
                .await
                .ok()
                .map(|listed| !listed),
        };
        let author = settle(&state, gone == Some(true), held);
        drop((own, listing, place));
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

/// How many record changes (creates and removals of grants, requests and subscriptions) may be
/// in progress or waiting to be settled at once. Past it, a new one is refused with `503`: a
/// store that keeps failing cannot pile up settlement tasks, and the records and locks they hold.
pub(crate) const MAX_SETTLING: usize = 256;

/// The record changes in progress or being settled, owned by the state: each change holds a
/// place ([`Settlements::reserve`]) from its request until it is settled, and the tasks that
/// settle a change whose outcome was not known ([`Settlements::spawn`]) are the state's. Such a
/// task never holds the state across an await: it waits on the store and the locks through their
/// own handles ([`tracked::Tracked`] and [`resources::IriLocks`] are shared), and upgrades its
/// weak handle to the state only for the synchronous step that puts the outcome in memory. So
/// dropping every other handle drops the state even while a store call of a settlement is
/// stalled, and with it these tasks (a [`tokio::task::JoinSet`] aborts its tasks when dropped),
/// releasing the locks and quota places they hold. A change whose settlement is cut short that
/// way is left as the store has it, which the next start loads.
pub(crate) struct Settlements {
    places: Arc<tokio::sync::Semaphore>,
    tasks: std::sync::Mutex<tokio::task::JoinSet<()>>,
}

impl Default for Settlements {
    fn default() -> Self {
        Self::with_capacity(MAX_SETTLING)
    }
}

impl Settlements {
    /// At most `places` changes at once.
    pub(crate) fn with_capacity(places: usize) -> Self {
        Self {
            places: Arc::new(tokio::sync::Semaphore::new(places)),
            tasks: Default::default(),
        }
    }

    /// A place for one record change, held until it is settled: `None` when there is none
    /// (answered with [`settlements_full`]).
    pub(crate) fn reserve(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.places.clone().try_acquire_owned().ok()
    }

    /// Run `settle` as a task the state owns.
    pub(crate) fn spawn(&self, settle: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        while tasks.try_join_next().is_some() {}
        tasks.spawn(settle);
    }

    /// How many settlement tasks have not finished.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        while tasks.try_join_next().is_some() {}
        tasks.len()
    }
}

/// The answer to a record change when [`Settlements`] has no place for it.
fn settlements_full() -> Response {
    resources::retry_later("too many changes to records are still being settled")
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
