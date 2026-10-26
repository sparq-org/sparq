//! The lifecycle of a service container's records (access grants and requests, notification
//! subscriptions), shared by every kind so their rules cannot drift: a record is created in one
//! store write and removed in one store step, memory follows the store (a record enters force
//! only once confirmed stored, and stays out of force while an unread removal is settled), the locks a
//! request takes are waited for a bounded time ([`within_wait`]), the steps after the locks are
//! held run in a task holding the request's admission, and each change is announced once it is
//! made ([`announce_record`]).

use super::*;

/// Delete the stored record `iri` of a service container (a grant, a request, a subscription).
/// The index commit is the deletion point: when the store reports a failure but the record no
/// longer exists (what failed was the cleanup of its bytes, which the reconciler collects), the
/// record is gone. One that still exists after a failure is not shown to stay unless the failure
/// is settled (see [`still_listed`]): the error is returned as it is, so the caller can tell.
///
/// A removal that failed [unsettled](crate::error::ServerError::Unsettled) can still commit, so
/// what the store says of the record is not yet final: its fence ([`Store::fence`]) is made to
/// succeed first, and only then is the record read. A fence that does not succeed leaves the
/// removal where it was, and the unsettled error (with its fence) is returned rather than the
/// record being reported gone: the caller must not try the removal again before that fence has
/// succeeded (see [`super::settle`]), since the first one may still land after it. Once the fence
/// has succeeded the removal is settled, so a record still there, or one that cannot be read, is
/// answered with a settled error ([`crate::error::ServerError::Storage`]), and may be tried again.
///
/// `within` bounds the wait for the fence, counted from when the store answered the removal. A
/// step that runs while a request waits on it (putting a change back in the request, clearing its
/// intent: [`super::settle`]) passes [`LOCK_WAIT`], so a fence that stalls leaves the step, with
/// its fence first, to be set aside and settled in the background, which holds the locks until
/// then. `None` waits for as long as the store's own call takes (bounded by the store, as every
/// call is) at start-up. Background put-back uses the capped store call timeout,
/// then retries a stalled fence with growing per-attempt budgets.
pub(crate) async fn delete_record<S: Store>(
    store: &S,
    iri: &str,
    container: &str,
    within: Option<std::time::Duration>,
) -> Result<(), crate::error::ServerError> {
    match delete_record_by(store, iri, container, within).await {
        Ok(()) => Ok(()),
        Err(Undeleted::Unfenced { fence, detail, .. }) => {
            Err(crate::error::ServerError::Unsettled { fence, detail })
        }
        Err(Undeleted::Failed(e) | Undeleted::Unread(e)) => Err(e),
    }
}

/// Why [`delete_record_by`] could not report a record gone.
enum Undeleted {
    /// The removal failed and is settled (it can no longer commit): the record is still there,
    /// or what failed was settled from the start.
    Failed(crate::error::ServerError),
    /// The removal failed unsettled and its fence has not succeeded within the wait: it may still
    /// commit.
    Unfenced {
        fence: crate::store::Fence,
        detail: String,
    },
    /// The removal is settled, but whether it committed could not be read:
    /// the outcome is final, but not yet known.
    Unread(crate::error::ServerError),
}

/// [`delete_record`], telling a fenced removal whose outcome could not be read
/// ([`Undeleted::Unread`]) from a settled failure ([`Undeleted::Failed`]).
async fn delete_record_by<S: Store>(
    store: &S,
    iri: &str,
    container: &str,
    within: Option<std::time::Duration>,
) -> Result<(), Undeleted> {
    match remove_member(store, iri, Some(container)).await {
        Ok(_) | Err(crate::error::ServerError::NotFound) => Ok(()),
        Err(crate::error::ServerError::Unsettled { fence, detail }) => {
            // Counted from the store's answer: an unsettled answer comes once the call has
            // timed out, and the fence is given the whole wait after it.
            let deadline = within.map(deadline_after);
            if !fenced_by(store, &fence, deadline).await {
                return Err(Undeleted::Unfenced { fence, detail });
            }
            // Fenced: the removal can no longer commit, so what is read now is final.
            match store.exists(iri).await {
                Ok(false) => Ok(()),
                Ok(true) => Err(Undeleted::Failed(crate::error::ServerError::Storage(
                    detail,
                ))),
                Err(_) => Err(Undeleted::Unread(crate::error::ServerError::Storage(
                    detail,
                ))),
            }
        }
        Err(e) => match store.exists(iri).await {
            Ok(false) => Ok(()),
            Err(_) if matches!(e, crate::error::ServerError::Storage(_)) => {
                Err(Undeleted::Unread(e))
            }
            _ => Err(Undeleted::Failed(e)),
        },
    }
}

/// The instant `within` from now, or, past what the clock can hold, a year from now: a wait
/// that long is not one any caller makes.
pub(crate) fn deadline_after(within: std::time::Duration) -> tokio::time::Instant {
    let now = tokio::time::Instant::now();
    now.checked_add(within)
        .unwrap_or_else(|| now + std::time::Duration::from_secs(365 * 24 * 3600))
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
/// back, or by a store call that stalls. A removal's fence is waited for in the request for as
/// long at most ([`fenced_by`]).
pub(crate) const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether `fence` succeeded by `deadline` (when one is given): one that fails, or does not
/// answer by then, leaves the write it fences unsettled, to be fenced again (by the background
/// settlement, for a removal a request made), so no request waits on a store that stalls.
pub(crate) async fn fenced_by<S: Store>(
    store: &S,
    fence: &crate::store::Fence,
    deadline: Option<tokio::time::Instant>,
) -> bool {
    match deadline {
        Some(deadline) => matches!(
            tokio::time::timeout_at(deadline, store.fence(fence)).await,
            Ok(Ok(()))
        ),
        None => store.fence(fence).await.is_ok(),
    }
}

/// The longest a background attempt at a fence is given, whatever the store's call timeout says.
pub(super) const MAX_FENCE_ATTEMPT: std::time::Duration = std::time::Duration::from_secs(3600);

/// Fence `fence` in the background until it succeeds: each attempt is given up after a time
/// that starts at [`LOCK_WAIT`] and doubles up to the store's own call timeout
/// ([`Store::call_timeout`]: [`crate::store::STORE_CALL_TIMEOUT`] unless the store is configured
/// otherwise, as an HTTP index built with a longer timeout is), clamped to between
/// [`LOCK_WAIT`] and one hour per attempt, then tried again (`until_done`),
/// so a fence that never answers does not hold the settlement on one call, and one that answers
/// slowly within that capped attempt budget is still waited for long enough to succeed.
pub(super) async fn fence_in_background<S: Store>(store: &S, fence: &crate::store::Fence) {
    let cap = store.call_timeout().clamp(LOCK_WAIT, MAX_FENCE_ATTEMPT);
    let mut within = LOCK_WAIT;
    until_done(|| {
        let deadline = deadline_after(within);
        within = within.saturating_mul(2).min(cap);
        async move {
            fenced_by(store, fence, Some(deadline))
                .await
                .then_some(())
                .ok_or(())
        }
    })
    .await
}

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
/// - The lock of every listing the record is shown in ([`Listed`]) is taken (shared, unless the
///   caller's [`service_preconditions`] hold them) in the request, waiting at most [`LOCK_WAIT`]
///   (then `503`), so a conditional create sees no member arrive between its check and its own
///   registration. The write and the registration then run
///   in a task of their own, holding the request's share of its admission permit (`admission`):
///   a client that goes away cannot stop a stored record from being registered.
/// - The record's own lock is held from before it is created until it is registered, so a
///   revocation of it waits for that. A name already stored is refused.
/// - A write that succeeds registers the record, and announces it (`Create`, its `author` one
///   of who may hear of it, [`announce_record`]) as it does. One that fails is checked against
///   the listing ([`still_listed`]): stored after all, it is registered; not stored, and the
///   write settled, `register` (and the quota place it holds) is dropped and the create fails.
///   Otherwise, the create answers that whether it was stored is not known, and a task holding
///   the record's lock, the quota place and the lock of the record's own listing (when it has
///   one, see [`Listed`]) settles it once the store answers, registering it under the
///   container's lock, shared, if it is stored. A write that failed
///   [unsettled](crate::error::ServerError::Unsettled) (an HTTP update that timed out, whose
///   worker may still be running) is fenced first ([`Store::fence`]), so a listing that does not
///   show it is final: its quota place is released only once it can no longer commit. A start
///   loads whatever the listing holds.
/// - The change holds a place among those being made or settled ([`Settlements`]) from the
///   request until it is settled: `503` when there is none. The task that settles one is the
///   state's, and ends when the state is dropped.
/// - `gate` runs once the listing locks and the record's own lock are held, and before anything
///   is stored: it may refuse the create (its answer is the create's), and what it yields (a
///   subscription's: the notification service's raise lock, see `notify::HighWater`) is held
///   until the create's answer is decided: through the write, the listing read after a failed
///   one, and the registration. A create whose outcome is not known gives it up as it answers
///   so, before it is settled: that settling waits on the store for as long as the store does
///   not answer, and holds no other change to the record's listings back but a conditional
///   create's.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_record<S, F, G, C>(
    state: &LwsState<S>,
    container: &str,
    iri: &str,
    body: Bytes,
    admission: Option<crate::overload::AdmissionSlot>,
    listed: Listed<'_>,
    held: Option<ListingLocks>,
    author: Option<String>,
    register: F,
    gate: G,
) -> Result<(), Response>
where
    S: Store + 'static,
    F: FnOnce(&LwsState<S>) + Send + 'static,
    G: std::future::Future<Output = Result<Option<C>, Response>>,
    C: Send + 'static,
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
    // A store with no room for the record is the same answer the record capacities give when
    // they are full, and is told the same way: `507`, the storage is full, not `500`.
    let failed = |e: ServerError| match e {
        ServerError::InsufficientStorage => problem(StatusCode::INSUFFICIENT_STORAGE, None),
        e => problem(StatusCode::INTERNAL_SERVER_ERROR, Some(&e.to_string())),
    };
    let Some(place) = state.settlements.reserve() else {
        return Err(settlements_full());
    };
    let held = match held {
        Some(held) => held,
        None => ListingLocks::shared(state, container, listed).await?,
    };
    // A new IRI: its lock is free unless the name was just taken, and it is only tried, so
    // taking it under the container's cannot deadlock.
    let Some(own) = state.locks.try_lock(iri) else {
        return Err(failed(ServerError::Conflict(
            "the record's name is taken".into(),
        )));
    };
    let gate = gate.await?;
    let (state, container, iri) = (state.detached(), container.to_string(), iri.to_string());
    let task = async move {
        let _admission = admission;
        let ListingLocks {
            own: own_listing,
            container: container_lock,
        } = held;
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
            drop(gate);
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
                // revocation of it waits), the quota place (`register` holds it), the lock of
                // the narrowest listing that shows it, shared, and this change's place among
                // those being settled. A subscription listed to its subscriber keeps that
                // listing's lock (so the subscriber's conditional creates wait for it, and
                // nobody else's, while its other changes go on), and the container's lock is
                // taken again only to register it. A record listed in the container alone keeps
                // the container's: a conditional create evaluated on the whole listing (the
                // owner's) waits for the outcome, as a subscriber's waits on its own, so two
                // with one entity tag cannot both register. An unsettled write is fenced first,
                // so the listing it then reads is final.
                let (own_listing, container_lock) = match own_listing {
                    Some(own) => {
                        drop(container_lock);
                        (Some(own.shared()), None)
                    }
                    None => (None, container_lock.map(ListingGuard::shared)),
                };
                drop(gate);
                let (weak, store, locks) =
                    (state.downgrade(), state.store.clone(), state.locks.clone());
                state.settlements.spawn(async move {
                    let _held = (own, place, own_listing);
                    if let Some(fence) = unsettled {
                        until_done(|| store.fence(&fence)).await;
                    }
                    if !until_done(|| still_listed(&store, &iri, &container)).await {
                        return;
                    }
                    // Registered under the container's lock, shared (taken after the record's,
                    // as a revocation takes them, unless still held), so no conditional create
                    // sees the listing change between its check and its own registration. This
                    // task holds no admission, so it may wait for it.
                    let _listing = match container_lock {
                        Some(held) => held,
                        None => ListingGuard::Shared(
                            until_done(|| async { locks.read(&container).await.ok_or(()) }).await,
                        ),
                    };
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

/// Where a record of a service container is listed, besides the container's own listing (which
/// shows every record to whoever may list them all): a subscription is also listed to its
/// subscriber, alone. A create or removal of the record holds the lock of each listing it
/// changes ([`ListingLocks`]), shared, from before its store step until it is settled, so a
/// conditional create evaluated on one of them, holding that listing's lock alone
/// ([`service_preconditions`]), sees nothing change in it between its check and its own
/// registration, and waits on nothing that changes only other listings. A create whose outcome
/// the store leaves unknown keeps that listing only shared while it is settled: the
/// subscriber's other conditional creates still wait for the outcome, but its unconditional
/// creates and removals go on, so the registration that settles it may follow them.
#[derive(Clone, Copy)]
pub(crate) enum Listed<'a> {
    /// Nowhere: an expired subscription, which every listing leaves out. Its removal changes no
    /// listing and takes no listing lock.
    Nowhere,
    /// In the container's listing alone.
    InContainer,
    /// In the container's listing and in the listing whose lock is named here: a name outside
    /// the storage, so no request's target is it ([`resources::IriLocks`] holds it, apart from
    /// every lock of a resource).
    AlsoIn(&'a str),
}

/// The listing locks a create or removal holds (see [`Listed`]): its own listing's, and the
/// container's. A listing lock is taken before any IRI lock and never while one is held, so it
/// takes no part in the IRI lock order.
pub(crate) struct ListingLocks {
    own: Option<ListingGuard>,
    container: Option<ListingGuard>,
}

/// A listing lock held alone (by a conditional create) or shared (by a change to the listing).
enum ListingGuard {
    // Held for as long as the guard is; never read.
    Alone(#[allow(dead_code)] resources::IriGuard),
    Shared(#[allow(dead_code)] tokio::sync::OwnedRwLockReadGuard<()>),
}

impl ListingGuard {
    /// The same lock, shared: a conditional create's, once its check is behind it, is only a
    /// change to the listing.
    fn shared(self) -> Self {
        match self {
            ListingGuard::Alone(guard) => ListingGuard::Shared(guard.downgrade()),
            shared => shared,
        }
    }
}

impl ListingLocks {
    /// Take the locks of the listings `listed` names, shared: its own first, then the
    /// container's, each waited for at most [`LOCK_WAIT`] (then `503`).
    async fn shared<S: Store + 'static>(
        state: &LwsState<S>,
        container: &str,
        listed: Listed<'_>,
    ) -> Result<Self, Response> {
        let own = match listed {
            Listed::AlsoIn(name) => Some(ListingGuard::Shared(
                within_wait(state.locks.read(name)).await?,
            )),
            Listed::Nowhere | Listed::InContainer => None,
        };
        let container = match listed {
            Listed::Nowhere => None,
            Listed::InContainer | Listed::AlsoIn(_) => Some(ListingGuard::Shared(
                within_wait(state.locks.read(container)).await?,
            )),
        };
        Ok(Self { own, container })
    }
}

/// The preconditions of a create in a service container (grants, requests, subscriptions),
/// evaluated against `listing` when they compare its state (an entity tag; the listing has no
/// date), with that listing held alone, and against existence alone otherwise, with nothing held
/// alone (`Ok(None)`, as for an unconditional create, whose listings are taken shared). Held
/// alone: `own`, a subscriber's own listing ([`Listed::AlsoIn`]), with the container's shared
/// (only the subscriber's own records' changes wait for it, or it for them), or, when `own` is
/// `None`, the container's whole listing. `Ok(None)` when the create is unconditional or its
/// preconditions compare no state and hold, `Ok(Some(locks))` when its tag preconditions hold
/// (the locks are passed on to [`create_record`], so no member arrives or leaves the listing
/// until the new one is registered), and the response to send
/// otherwise. A listing that cannot be produced (a representation the client does not accept, a
/// page that does not exist) is that response: its absent `ETag` is not the absence of the
/// container.
pub(crate) async fn service_preconditions<S: Store + 'static, L>(
    state: &LwsState<S>,
    req: &LwsRequest,
    container: &str,
    own: Option<&str>,
    listing: impl FnOnce(LwsRequest) -> L,
) -> Result<Option<ListingLocks>, axum::response::Response>
where
    L: std::future::Future<Output = axum::response::Response>,
{
    if !resources::is_conditional(req) {
        return Ok(None);
    }
    // A service listing has no `Last-Modified`, so only a tag compares its state; without one
    // (a bare `*`, or a date), only whether the container exists is asked, and it does. Such a
    // precondition reads nothing a member's arrival could change, so the create takes its
    // listings shared, as an unconditional one does (`Ok(None)`): taking the container alone
    // would wait behind every change holding it shared (a removal settling in the background
    // holds it through its fence), and queue every later create and removal behind that wait.
    // In the access containers it is the only form a caller other than the owner may send.
    if !resources::compares_state(&req.headers, false) {
        return match resources::unless_preconditions(req, Some(resources::EXISTS), None) {
            Some(refused) => Err(refused),
            None => Ok(None),
        };
    }
    let locks = match own {
        Some(name) => {
            let own = ListingGuard::Alone(within_wait(state.locks.lock(name)).await?);
            ListingLocks {
                own: Some(own),
                container: Some(ListingGuard::Shared(
                    within_wait(state.locks.read(container)).await?,
                )),
            }
        }
        None => ListingLocks {
            own: None,
            container: Some(ListingGuard::Alone(
                within_wait(state.locks.lock(container)).await?,
            )),
        },
    };
    // Read only now, with the locks held: what it shows (and leaves out) is decided after every
    // change it waited on.
    let current = listing(resources::plain_get(req)).await;
    if !current.status().is_success() {
        return Err(current);
    }
    let (etag, modified) = resources::validators_of(&current);
    match resources::unless_preconditions(req, etag.as_deref(), modified) {
        Some(refused) => Err(refused),
        None => Ok(Some(locks)),
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
/// - The lock of the record's own listing is taken first, shared (when it has one, see
///   [`Listed`]), then the record's own lock (a create of it still settling holds it, see
///   [`create_record`]), then the container's, shared (unless the record is listed nowhere), all
///   in the request, each waiting at most [`LOCK_WAIT`] (then `503`): a removal that stalls in
///   the store answers later requests `503` rather than queueing them.
/// - The steps then run in a task of their own, holding the locks and `admission` (the request's
///   share of its admission permit), so a client that goes away cannot cut them short:
///   `set_aside` takes the record out of force while its removal runs (`None` when it is not in
///   force: [`Removed::Absent`]), deciding under the locks, where it may wait (a decision that
///   needs nothing stored is [`at_once`]); the record is deleted ([`delete_record`]), and a
///   failure is checked against the listing ([`still_listed`]); `settle` is told whether it is
///   confirmed gone, and removes it from memory or puts it back, giving the author to announce
///   it to.
/// - A delete that failed [unsettled](crate::error::ServerError::Unsettled) (an HTTP update that
///   timed out) can still land, so no listing read is final until its fence ([`Store::fence`])
///   has succeeded, whatever the listing shows. The fence is tried in the steps, and once it
///   succeeds the listing read after it settles the removal, every lock still held. One that
///   does not succeed within [`LOCK_WAIT`] (counted from the store's answer to the delete), or
///   whose outcome cannot be read after it has succeeded, is settled in the background (a fence
///   not yet succeeded is tried there first, each attempt allowed longer, up to the store's own
///   call timeout, [`fence_in_background`]; one that has succeeded is not tried again): the record stays out of force, and its own lock, its
///   own listing's and the container's (shared, unless it is listed nowhere) stay held, never
///   given up and taken again, until the delete is fenced and the listing read again; `settle` is then
///   told the outcome, and a record gone is announced. The answer is `500`, saying this is not
///   known yet, and it does not wait on a fence that stalls.
/// - As for a create, the removal holds a place among those being made or settled
///   ([`Settlements`]) until it is settled (`503` when there is none), and the task that settles
///   it is the state's.
/// - Only a record confirmed gone is announced (`Delete`, [`announce_record`]), once its locks
///   are released. Otherwise the answer is `500`, saying whether it is still stored or that this
///   is not known (it is left as it was, and the request may be retried); a delete the store
///   refused for want of room ([`crate::error::ServerError::InsufficientStorage`]), the record
///   still stored, is `507`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn remove_record<S, H, A, F, Z>(
    state: &LwsState<S>,
    container: &str,
    iri: &str,
    what: &'static str,
    listed: Listed<'_>,
    by: Remover,
    admission: Option<crate::overload::AdmissionSlot>,
    set_aside: A,
    settle: Z,
) -> Result<Removed, Response>
where
    S: Store + 'static,
    H: Send + 'static,
    A: FnOnce() -> F + Send + 'static,
    F: std::future::Future<Output = Option<H>> + Send + 'static,
    Z: FnOnce(&LwsState<S>, bool, H) -> Option<String> + Send + 'static,
{
    let place = match by {
        Remover::Request => state.settlements.reserve(),
        Remover::Sweeper => state.settlements.reserve_maintenance(),
    };
    let Some(place) = place else {
        return Err(settlements_full());
    };
    let own_listing = match listed {
        Listed::AlsoIn(name) => Some(within_wait(state.locks.read(name)).await?),
        Listed::Nowhere | Listed::InContainer => None,
    };
    let own = within_wait(state.locks.lock(iri)).await?;
    let listing = match listed {
        Listed::Nowhere => None,
        Listed::InContainer | Listed::AlsoIn(_) => {
            Some(within_wait(state.locks.read(container)).await?)
        }
    };
    let (state, container, iri) = (state.detached(), container.to_string(), iri.to_string());
    let task = async move {
        let _admission = admission;
        let Some(held) = set_aside().await else {
            return Ok(Removed::Absent);
        };
        // A delete the store refused for want of room is told as such (`507`), as a create's
        // refusal is, once the record is known to be still stored.
        let mut refused = false;
        // The fence budget starts at the store's answer. Once spent, settlement continues
        // in the background; no second attempt is made against an expired deadline.
        let pending = match delete_record_by(&state.store, &iri, &container, Some(LOCK_WAIT)).await
        {
            Ok(()) => Ok(Some(true)),
            Err(Undeleted::Unfenced { fence, .. }) => Err(Some(fence)),
            // Settled, but the outcome is unread: try the listing in this request, then keep
            // the record out of force while the background waits for it to become readable.
            Err(Undeleted::Unread(_)) => match still_listed(&state.store, &iri, &container).await {
                Ok(listed) => Ok(Some(!listed)),
                Err(_) => Err(None),
            },
            Err(Undeleted::Failed(e)) => {
                refused = matches!(e, crate::error::ServerError::InsufficientStorage);
                Ok(still_listed(&state.store, &iri, &container)
                    .await
                    .ok()
                    .map(|listed| !listed))
            }
        };
        let gone = match pending {
            Ok(gone) => gone,
            // The sweeper's removal is settled here, within its deadline, holding what it holds:
            // given up past it, it is dropped with everything it holds (its place among them)
            // and leaves the record in force, to be tried again.
            Err(fence) if by == Remover::Sweeper => {
                if let Some(fence) = fence {
                    until_done(|| state.store.fence(&fence)).await;
                }
                let listed = until_done(|| still_listed(&state.store, &iri, &container)).await;
                Some(!listed)
            }
            // `fence` is the delete's, when it has not succeeded yet.
            Err(fence) => {
                // Settled by a task the state owns, holding the record's own lock, its own
                // listing's, the container's (shared) and this change's place among those being
                // settled, with the record out of force until then: the container's lock is
                // held through the fence and the settling, never given up and taken again, so
                // no conditional create evaluated on the whole listing is decided while the
                // removal may still land.
                let (weak, store) = (state.downgrade(), state.store.clone());
                let (iri, container) = (iri.clone(), container.clone());
                state.settlements.spawn(async move {
                    let _held = (place, own_listing);
                    if let Some(fence) = fence {
                        fence_in_background(&store, &fence).await;
                    }
                    let listed = until_done(|| still_listed(&store, &iri, &container)).await;
                    let Some(state) = weak.upgrade() else {
                        return;
                    };
                    let author = settle(&state, !listed, held);
                    drop((own, listing));
                    if !listed {
                        announce_record(&state, "Delete", &iri, &container, author);
                    }
                });
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "whether the {what} was removed is not known yet; it is out of force \
                         until that is settled, and the request may be retried"
                    ),
                ));
            }
        };
        let author = settle(&state, gone == Some(true), held);
        drop((own, listing, place, own_listing));
        match gone {
            Some(true) => {
                announce_record(&state, "Delete", &iri, &container, author);
                Ok(Removed::Gone)
            }
            Some(false) if refused => Err((
                StatusCode::INSUFFICIENT_STORAGE,
                format!("the store had no room to remove the {what}; it is still stored"),
            )),
            Some(false) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("the {what} could not be removed; it is still stored"),
            )),
            None => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "whether the {what} was removed is not known; it is left as it was, and \
                     the request may be retried"
                ),
            )),
        }
    };
    let done = match by {
        Remover::Request => tokio::spawn(task)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        Remover::Sweeper => Ok(task.await),
    };
    match done {
        Ok(Ok(removed)) => Ok(removed),
        Ok(Err((status, why))) | Err((status, why)) => Err(problem(status, Some(&why))),
    }
}

/// A `set_aside` for [`remove_record`] that decides at once, waiting on nothing.
pub(crate) fn at_once<H>(
    set_aside: impl FnOnce() -> Option<H>,
) -> impl FnOnce() -> std::future::Ready<Option<H>> {
    move || std::future::ready(set_aside())
}

/// Who removes a record ([`remove_record`]), and so how its steps run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Remover {
    /// A request: the steps run in a task of their own, so a client that goes away cannot cut
    /// them short, in a place among those [`Settlements`] has for every change.
    Request,
    /// The expiry sweeper (`notify::spawn_expiry_sweeper`): the steps run in its own task, which
    /// owns them and may give them up past a deadline (dropped, they leave the record as the
    /// store has it, in force or gone, and memory as it was, to be tried again), in a place
    /// reserved for it ([`Settlements::reserve_maintenance`]), so no load of requests refuses it.
    /// A delete whose outcome is unknown is settled in those same steps, never in a task of the
    /// state's, so the deadline bounds it, and the place, too. Only for a record whose
    /// `set_aside` changes no memory (an expired subscription): dropped part way, nothing in
    /// memory is left out of force, and a late delete removes only what was already gone from
    /// every listing.
    Sweeper,
}

/// How many record changes (creates and removals of grants, requests and subscriptions) may be
/// in progress or waiting to be settled at once. Past it, a new one is refused with `503`: a
/// store that keeps failing cannot pile up settlement tasks, and the records and locks they hold.
pub(crate) const MAX_SETTLING: usize = 256;

/// How many of the expiry sweeper's removals may be in progress or being settled at once, in
/// places of their own (see [`Remover::Sweeper`]).
pub(crate) const MAINTENANCE_PLACES: usize = 16;

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
    /// The places reserved for the expiry sweeper's removals ([`Remover::Sweeper`]).
    maintenance: Arc<tokio::sync::Semaphore>,
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
            maintenance: Arc::new(tokio::sync::Semaphore::new(MAINTENANCE_PLACES)),
            tasks: Default::default(),
        }
    }

    /// A place reserved for one of the expiry sweeper's removals: `None` when they are all
    /// taken. No request takes one, so however many changes requests have in progress, the
    /// sweeper keeps [`MAINTENANCE_PLACES`].
    pub(crate) fn reserve_maintenance(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.maintenance.clone().try_acquire_owned().ok()
    }

    /// How many places reserved for the sweeper are free (tests).
    #[cfg(test)]
    pub(crate) fn maintenance_free(&self) -> usize {
        self.maintenance.available_permits()
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
    let state = state.detached();
    tokio::spawn(async move { state.notify.announce_service(&state, event, author).await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    /// A gate that is never opened: the unsettled write it holds is still to come.
    fn shut() -> Arc<tokio::sync::Semaphore> {
        Arc::new(tokio::sync::Semaphore::new(0))
    }

    /// A removal that failed unsettled is fenced before the record is read back, so a record the
    /// store does not hold is reported gone only once the removal can no longer commit; a fence
    /// that fails leaves the error as it was.
    #[tokio::test]
    async fn an_unsettled_removal_is_not_done_before_its_fence() {
        let store = super::super::test_store::FlakyStore::new();
        let container = "http://localhost:3000/.lws/grants/";
        let iri = format!("{container}g1");
        // The removal of a record the store does not hold fails unsettled: what `exists` says
        // of it is not final until the fence has succeeded.
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let r = delete_record(&store, &iri, container, None).await;
        assert!(
            matches!(r, Err(crate::error::ServerError::Unsettled { .. })),
            "{:?}",
            r
        );
        assert_eq!(store.fences_tried.load(Ordering::SeqCst), 1);
        // Once the fence succeeds, the same removal is reported done.
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.fail_fence.store(false, Ordering::SeqCst);
        assert!(delete_record(&store, &iri, container, None).await.is_ok());
        assert_eq!(store.fences_tried.load(Ordering::SeqCst), 2);
    }

    /// A removal whose delete failed unsettled is settled only from a listing read after the
    /// delete's fence has succeeded, with every lock it took held until then. Here the record is
    /// stored but its container's listing does not show it, so a listing read before the fence
    /// would report it gone while the delete can still land. With the fence failing, the answer
    /// is that the outcome is not known, nothing is settled and the record's lock stays held;
    /// once the fence succeeds, it is settled as gone and the lock is released.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unsettled_removal_holds_its_locks_until_fenced() {
        let (state, store) = super::super::test_store::state(100).await;
        let container = format!("{}records/", state.cfg.storage());
        let iri = format!("{container}r1");
        state
            .store
            .write(&container, Bytes::new(), "text/turtle")
            .await
            .unwrap();
        // Stored, but not a member of the container: no listing shows it.
        state
            .store
            .write(&iri, Bytes::from_static(b"{}"), "application/json")
            .await
            .unwrap();
        assert!(!still_listed(&state.store, &iri, &container).await.unwrap());
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let told = settled.clone();
        let r = remove_record(
            &state,
            &container,
            &iri,
            "record",
            Listed::InContainer,
            Remover::Request,
            None,
            at_once(|| Some(())),
            move |_: &LwsState<_>, gone: bool, ()| {
                *told.lock().unwrap() = Some(gone);
                None
            },
        )
        .await;
        let Err(r) = r else {
            panic!("an unsettled removal was reported done before its fence succeeded");
        };
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(*settled.lock().unwrap(), None);
        assert!(
            state.locks.try_lock(&iri).is_none(),
            "the record's lock is held"
        );
        store.fail_fence.store(false, Ordering::SeqCst);
        for _ in 0..250 {
            if settled.lock().unwrap().is_some() && state.locks.try_lock(&iri).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(*settled.lock().unwrap(), Some(true));
        assert!(
            state.locks.try_lock(&iri).is_some(),
            "the record's lock is released"
        );
    }

    /// A record stored in `container`, listed there, for a removal to act on.
    async fn listed_record(
        state: &LwsState<super::super::test_store::FlakyStore>,
        name: &str,
    ) -> (String, String) {
        let container = format!("{}records/", state.cfg.storage());
        let iri = format!("{container}{name}");
        if !state.store.exists(&container).await.unwrap() {
            state
                .store
                .create_in_container(&state.cfg.storage(), &container, Bytes::new(), LWS_JSON)
                .await
                .unwrap();
        }
        state
            .store
            .create_in_container(&container, &iri, Bytes::from_static(b"{}"), LWS_JSON)
            .await
            .unwrap();
        (container, iri)
    }

    /// Remove `iri` as a request does, reporting to `told` what it is settled as.
    async fn remove_as_request(
        state: &LwsState<super::super::test_store::FlakyStore>,
        container: &str,
        iri: &str,
        told: Arc<std::sync::Mutex<Option<bool>>>,
    ) -> Result<Removed, Response> {
        remove_record(
            state,
            container,
            iri,
            "record",
            Listed::InContainer,
            Remover::Request,
            None,
            at_once(|| Some(())),
            move |_: &LwsState<_>, gone: bool, ()| {
                *told.lock().unwrap() = Some(gone);
                None
            },
        )
        .await
    }

    /// Review of #7135: a removal whose outcome was not known gave up the container's lock
    /// before its fence and took it again to settle, so a conditional create evaluated on the
    /// whole listing could be decided in between, while the removal could still land. The
    /// container's lock is held, shared, from the request until the removal is settled.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unsettled_removal_keeps_its_container_until_settled() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        let gate = shut();
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let r = remove_as_request(&state, &container, &iri, settled.clone()).await;
        assert_eq!(r.err().unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
        // Fenced again in the background meanwhile: the listing is held throughout.
        for _ in 0..5 {
            assert!(
                state.locks.try_lock(&container).is_none(),
                "the container's lock was given up before the removal was settled"
            );
            tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        }
        assert_eq!(*settled.lock().unwrap(), None);
        store.fail_fence.store(false, Ordering::SeqCst);
        for _ in 0..250 {
            if settled.lock().unwrap().is_some() && state.locks.try_lock(&container).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(
            *settled.lock().unwrap(),
            Some(false),
            "fenced before it landed"
        );
        assert!(state.locks.try_lock(&container).is_some());
    }

    /// Review of #7135: the request waited on the fence of a removal whose outcome was not
    /// known for as long as the store took, so a store whose fence stalls held the request. It
    /// is waited for at most `LOCK_WAIT`: the request answers that the outcome is not known, and
    /// the removal is settled in the background once the fence answers.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stalled_fence_does_not_hold_the_request() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.hang_fence.store(true, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let began = tokio::time::Instant::now();
        let r = tokio::time::timeout(
            LOCK_WAIT * 2,
            remove_as_request(&state, &container, &iri, settled.clone()),
        )
        .await
        .expect("the request answered while the fence stalled");
        assert!(began.elapsed() < LOCK_WAIT + std::time::Duration::from_millis(500));
        let r = r.err().expect("not reported done");
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("not known yet"));
        assert_eq!(*settled.lock().unwrap(), None);
        assert!(
            state.locks.try_lock(&iri).is_none(),
            "settled in the background"
        );
        // The fence answers again: the background settles it.
        store.hang_fence.store(false, Ordering::SeqCst);
        for _ in 0..400 {
            if settled.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(*settled.lock().unwrap(), Some(false));
    }

    /// A slow delete answer does not spend the fence's own request budget.
    #[tokio::test(start_paused = true)]
    async fn the_fence_budget_starts_at_the_delete_answer() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store
            .slow_delete_answer_ms
            .store((LOCK_WAIT * 2).as_millis() as u64, Ordering::SeqCst);
        store
            .slow_fence_ms
            .store((LOCK_WAIT / 2).as_millis() as u64, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let r = remove_as_request(&state, &container, &iri, settled.clone()).await;
        assert_eq!(r.err().unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            *settled.lock().unwrap(),
            Some(false),
            "settled in the request"
        );
        assert_eq!(state.settlements.pending(), 0);
        assert_eq!(store.fences_tried.load(Ordering::SeqCst), 1);
        assert!(state.locks.try_lock(&iri).is_some());
    }

    /// An expired request fence budget is handed to the background without a second try.
    #[tokio::test(start_paused = true)]
    async fn an_expired_fence_budget_is_not_tried_again_in_the_request() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.slow_fence_ms.store((LOCK_WAIT * 2).as_millis() as u64, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let faster = async {
            while store.fences_tried.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            // A second attempt could succeed even with an expired timeout: timeout_at
            // polls an immediately-ready future before checking its deadline.
            store.slow_fence_ms.store(0, Ordering::SeqCst);
        };
        let (answer, ()) = tokio::join!(
            remove_as_request(&state, &container, &iri, settled.clone()), faster);
        let body = axum::body::to_bytes(answer.err().unwrap().into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("not known yet"),
                "a second fence attempt changed the in-request answer");
    }

    /// The actual set-aside worker fences a Remove before it releases the resource.
    #[tokio::test]
    async fn a_set_aside_remove_does_not_outlive_its_put_back() {
        use super::super::{Undo, Unsettled};
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        let gate = shut();
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let lock = state.locks.lock(&iri).await.unwrap();
        state.set_aside(
            Unsettled::new(vec![Undo::Remove {
                iri: iri.clone(),
                parent: container.clone(),
            }]),
            lock,
        );
        while store.fences_tried.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(!state.visible(&iri));
        store.fail_fence.store(false, Ordering::SeqCst);
        tokio::time::timeout(LOCK_WAIT * 4, async {
            while !state.visible(&iri) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("put back never finished");
        state
            .store
            .create_in_container(&container, &iri, Bytes::from_static(b"{}"), LWS_JSON)
            .await
            .unwrap();
        gate.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            state.store.exists(&iri).await.unwrap(),
            "late delete outlived the put-back"
        );
    }

    /// A stalled background put-back fence is retried within a capped attempt.
    #[tokio::test(start_paused = true)]
    async fn a_put_back_retries_a_fence_that_never_answers() {
        use super::super::Undo;
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let task = tokio::spawn(async move {
            super::super::put_back_until_done(
                &Undo::Remove {
                    iri,
                    parent: container,
                },
                &state.store,
            )
            .await;
        });
        // First apply fails its fence; the background attempt then hangs.
        while store.fences_tried.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        store.hang_fence.store(true, Ordering::SeqCst);
        tokio::time::sleep(LOCK_WAIT * 20).await;
        let tried = store.fences_tried.load(Ordering::SeqCst);
        tokio::time::sleep(LOCK_WAIT * 40).await;
        assert!(
            store.fences_tried.load(Ordering::SeqCst) > tried,
            "one stalled fence held every later attempt"
        );
        task.abort();
    }

    /// Commit reports an unsettled write without assuming it was an intent write.
    #[tokio::test(start_paused = true)]
    async fn commit_names_the_unsettled_write() {
        use super::super::Undo;
        let (state, store) = super::super::test_store::state(100).await;
        store.hang_fence.store(true, Ordering::SeqCst);
        let mut journal = state.journal();
        journal.undo.push(Undo::Fence {
            fence: crate::store::Fence::new("stalled", vec![state.cfg.storage()]),
        });
        let (error, left) = journal.commit(None).await.expect_err("unsettled write");
        assert_eq!(
            error.to_string(),
            "storage error: the unsettled write could not be settled"
        );
        assert!(matches!(left.unwrap().0.as_slice(), [Undo::Fence { .. }]));
    }

    /// A fence already stalled before set-aside is retried, and its remaining undo finishes.
    #[tokio::test(start_paused = true)]
    async fn a_set_aside_caps_its_first_fence_call() {
        use super::super::{Kept, Undo, Unsettled};
        for kind in 0..4 {
            let (state, store) = super::super::test_store::state(100).await;
            let (container, iri) = listed_record(&state, "r1").await;
            let prior = store.read(&iri).await.unwrap();
            *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
            let mut undo = Vec::new();
            if kind < 2 {
                let Err(crate::error::ServerError::Unsettled { fence, .. }) = store
                    .delete_container_if_empty(&iri, Some(&container))
                    .await
                else {
                    panic!("expected an unsettled delete");
                };
                undo.push(Undo::Fence { fence });
                if kind == 1 {
                    store.delete(&iri, Some(&container)).await.unwrap();
                }
            }
            undo.push(match kind {
                1 => Undo::Recreate {
                    iri: iri.clone(),
                    parent: Some(container.clone()),
                    kept: Kept::Copied {
                        body: prior.body.clone(),
                        meta: prior.meta.clone(),
                    },
                },
                3 => Undo::Forget {
                    record: iri.clone(),
                    iris: vec![iri.clone()],
                },
                _ => Undo::Remove {
                    iri: iri.clone(),
                    parent: container.clone(),
                },
            });
            store.hang_fence.store(true, Ordering::SeqCst);
            state.set_aside(Unsettled::new(undo), ());
            tokio::time::sleep(if kind < 2 {
                LOCK_WAIT * 3
            } else {
                LOCK_WAIT * 30
            })
            .await;
            assert!(
                store.fences_tried.load(Ordering::SeqCst) > 1,
                "kind={kind}: first fence held every retry"
            );
            assert!(
                state.locks.try_lock(&iri).is_none(),
                "undo still holds the resource"
            );
            store.hang_fence.store(false, Ordering::SeqCst);
            tokio::time::timeout(LOCK_WAIT * 60, async {
                while state.locks.try_lock(&iri).is_none() {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("put-back finished after the hang cleared");
            assert_eq!(store.exists(&iri).await.unwrap(), kind == 1);
            if kind == 1 {
                let restored = store.read(&iri).await.unwrap();
                assert_eq!(restored.body, prior.body);
                assert_eq!(restored.meta.etag, prior.meta.etag);
            }
        }
    }

    /// A fence slower than `LOCK_WAIT` but within the store's own call timeout: the request
    /// answers that the outcome is not known, and the background settlement, whose attempts
    /// are allowed longer each time, settles the removal and releases its locks.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_fence_slower_than_the_lock_wait_still_settles() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        let slow = LOCK_WAIT + std::time::Duration::from_millis(500);
        store
            .slow_fence_ms
            .store(slow.as_millis() as u64, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let r = remove_as_request(&state, &container, &iri, settled.clone()).await;
        assert_eq!(r.err().unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
        let began = tokio::time::Instant::now();
        while began.elapsed() < LOCK_WAIT * 6 {
            if settled.lock().unwrap().is_some()
                && state.locks.try_lock(&container).is_some()
                && state.locks.try_lock(&iri).is_some()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(settled.lock().unwrap().is_some(), "never settled");
        assert!(state.locks.try_lock(&container).is_some());
        assert!(state.locks.try_lock(&iri).is_some());
    }

    /// Removing a record outside a request (putting a change back, clearing an intent) waits
    /// for an unsettled removal's fence as long as the store takes, not `LOCK_WAIT`.
    #[tokio::test]
    async fn a_removal_outside_a_request_waits_for_a_slow_fence() {
        let store = super::super::test_store::FlakyStore::new();
        let container = "http://localhost:3000/.lws/grants/";
        let iri = format!("{container}g1");
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        let slow = LOCK_WAIT + std::time::Duration::from_millis(500);
        store
            .slow_fence_ms
            .store(slow.as_millis() as u64, Ordering::SeqCst);
        delete_record(&store, &iri, container, None)
            .await
            .expect("settled once the slow fence answered");
    }

    /// Review of #7174: putting a change back in a request (a journal's rollback or commit, a
    /// failed create's rollback) waited for an unsettled removal's fence as long as the store
    /// took, with the request's locks held. Each attempt waits `LOCK_WAIT` at most, and a fence
    /// that stalls leaves the step to be set aside; at start-up, with nothing served yet, it is
    /// still waited for as long as the store takes.
    #[tokio::test(start_paused = true)]
    async fn a_put_back_in_a_request_does_not_wait_on_a_stalled_fence() {
        use super::super::Undo;
        let store = super::super::test_store::FlakyStore::new();
        let container = "http://localhost:3000/.lws/grants/";
        let iri = format!("{container}g1");
        store.hang_fence.store(true, Ordering::SeqCst);
        let remove = || {
            vec![Undo::Remove {
                iri: iri.clone(),
                parent: container.to_string(),
            }]
        };
        // A removal that fails unsettled, its fence stalling, is left (its fence first), not
        // held for and not tried again.
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        let left = tokio::time::timeout(LOCK_WAIT * 2, super::super::settle(&store, remove()))
            .await
            .expect("the request was held by a stalled fence")
            .expect("the step is left");
        assert!(matches!(left.0.as_slice(), [Undo::Fence { .. }, Undo::Remove { .. }]));
        // A fence step whose fence stalls is left to be set aside.
        let fence = || {
            vec![
                Undo::Fence {
                    fence: crate::store::Fence::new("stalled", vec![iri.clone()]),
                },
                Undo::Remove {
                    iri: iri.clone(),
                    parent: container.to_string(),
                },
            ]
        };
        let left = tokio::time::timeout(LOCK_WAIT * 2, super::super::settle(&store, fence()))
            .await
            .expect("the request was held by a stalled fence")
            .expect("the step and remaining undo are left");
        assert!(matches!(
            left.0.as_slice(),
            [Undo::Fence { .. }, Undo::Remove { .. }]
        ));
        // At start-up the fence is waited for as long as the store takes.
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        assert!(tokio::time::timeout(
            LOCK_WAIT * 10,
            super::super::settle_at_start(&store, remove())
        )
        .await
        .is_err());
    }

    /// Review of #7183: putting a change back in a request tried a removal whose delete failed
    /// unsettled (its fence not answering within the request's wait) again at once, so the
    /// put-back finished, its locks were released, and the first delete could land later and
    /// remove a record written since. The step is left with its fence first; the set-aside put
    /// it back only once the fence has succeeded, and a record written after it survives the
    /// late delete.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_put_back_never_outlives_an_unfenced_removal() {
        use super::super::Undo;
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        let gate = shut();
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        // The fence answers, but only after the request's wait.
        store.slow_fence_ms.store(
            (LOCK_WAIT + std::time::Duration::from_secs(1)).as_millis() as u64,
            Ordering::SeqCst,
        );
        let undo = vec![Undo::Remove {
            iri: iri.clone(),
            parent: container.clone(),
        }];
        let left = super::super::settle(&state.store, undo)
            .await
            .expect("the step is left, its delete unfenced");
        assert!(matches!(left.0.as_slice(), [Undo::Fence { .. }, Undo::Remove { .. }]));
        // The set-aside puts it back, fence first.
        for undo in &left.0 {
            super::super::put_back_until_done(undo, &state.store).await;
        }
        assert!(!state.store.exists(&iri).await.unwrap());
        // A record written afterwards survives the first delete, which arrives now.
        state
            .store
            .create_in_container(&container, &iri, Bytes::from_static(b"{}"), LWS_JSON)
            .await
            .unwrap();
        gate.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(state.store.exists(&iri).await.unwrap(), "a late delete removed it");
    }

    /// Review of #7183: the background put-back of a set-aside change tried a removal again
    /// after its delete failed unsettled and its fence failed, so the first delete could land
    /// after it. The step is tried again only once its fence has succeeded.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_set_aside_put_back_fences_before_trying_a_removal_again() {
        use super::super::Undo;
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        let gate = shut();
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), gate.clone()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let undo = Undo::Remove {
            iri: iri.clone(),
            parent: container.clone(),
        };
        let task = {
            let store = state.store.clone();
            tokio::spawn(async move { super::super::put_back_until_done(&undo, &store).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        assert!(!task.is_finished(), "tried again before its fence succeeded");
        store.fail_fence.store(false, Ordering::SeqCst);
        tokio::time::timeout(std::time::Duration::from_secs(10), task)
            .await
            .expect("put back once fenced")
            .unwrap();
        state
            .store
            .create_in_container(&container, &iri, Bytes::from_static(b"{}"), LWS_JSON)
            .await
            .unwrap();
        gate.add_permits(1);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(state.store.exists(&iri).await.unwrap(), "a late delete removed it");
    }

    /// Review of #7183: a removal whose fence succeeded but whose outcome could not then be
    /// read was answered as a settled failure, "left as it was", and put back in force though
    /// the delete may have committed. It is settled in the background from the listing, the
    /// record out of force until then, without fencing again.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_fenced_removal_that_cannot_be_read_is_settled_in_the_background() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.fail_reads_after_fence.store(true, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let r = remove_as_request(&state, &container, &iri, settled.clone()).await;
        let r = r.err().expect("not reported done");
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("not known yet"), "{body}");
        assert_eq!(*settled.lock().unwrap(), None, "settled before it was read");
        assert!(state.locks.try_lock(&iri).is_none(), "held until settled");
        store.fail_reads_after_fence.store(false, Ordering::SeqCst);
        store.fail_exists.store(false, Ordering::SeqCst);
        store.fail_list.store(false, Ordering::SeqCst);
        for _ in 0..250 {
            if settled.lock().unwrap().is_some() && state.locks.try_lock(&iri).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(*settled.lock().unwrap(), Some(false));
        assert_eq!(store.fences_tried.load(Ordering::SeqCst), 1, "fenced again");
    }

    /// Review of #7183: a store reporting a call timeout too long for the clock made the
    /// background fence's attempt deadline overflow. Each attempt is capped.
    #[tokio::test(start_paused = true)]
    async fn a_huge_call_timeout_does_not_overflow_the_fence_deadline() {
        let store = super::super::test_store::FlakyStore::new();
        store.fail_fence.store(true, Ordering::SeqCst);
        store.call_timeout_ms.store(u64::MAX, Ordering::SeqCst);
        let fence = crate::store::Fence::new("unreachable", Vec::new());
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(10 * 3600),
            fence_in_background(&store, &fence),
        )
        .await
        .is_err());
    }

    /// Review of #7174: a removal whose delete failed unsettled, whose fence then succeeded
    /// with the record still stored, was returned unsettled, so the request fenced again
    /// against what was left of its deadline and could answer that the outcome was not known
    /// (holding its locks in the background) when it was. Once fenced, it is settled at once
    /// from the listing: still stored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_fenced_removal_whose_record_stays_is_settled_at_once() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        // The delete never lands, and its fence takes most of the request's wait.
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store
            .slow_fence_ms
            .store((LOCK_WAIT * 3 / 4).as_millis() as u64, Ordering::SeqCst);
        let settled = Arc::new(std::sync::Mutex::new(None));
        let r = remove_as_request(&state, &container, &iri, settled.clone()).await;
        let r = r.err().expect("the record is still stored");
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        assert!(body.contains("it is still stored"), "{body}");
        assert_eq!(*settled.lock().unwrap(), Some(false));
        assert_eq!(store.fences_tried.load(Ordering::SeqCst), 1);
        assert!(state.locks.try_lock(&iri).is_some(), "no lock is held");
        assert!(still_listed(&state.store, &iri, &container).await.unwrap());
    }

    /// Review of #7174: a background fence's attempts were capped at the default store call
    /// timeout, even over a store configured with a longer one, so a fence slower than the
    /// default (within the store's own timeout) never succeeded. The cap is the store's own.
    #[tokio::test(start_paused = true)]
    async fn a_background_fence_is_allowed_the_store_s_own_call_timeout() {
        let store = super::super::test_store::FlakyStore::new();
        let container = "http://localhost:3000/.lws/grants/";
        let iri = format!("{container}g1");
        *store.unsettle_next_delete_of.lock().unwrap() = Some((iri.clone(), shut()));
        store.fail_fence.store(true, Ordering::SeqCst);
        let Err(crate::error::ServerError::Unsettled { fence, .. }) =
            delete_record(&store, &iri, container, Some(LOCK_WAIT)).await
        else {
            panic!("the removal is unsettled");
        };
        store.fail_fence.store(false, Ordering::SeqCst);
        let default = crate::store::STORE_CALL_TIMEOUT;
        store.slow_fence_ms.store(
            (default + std::time::Duration::from_secs(10)).as_millis() as u64,
            Ordering::SeqCst,
        );
        store
            .call_timeout_ms
            .store((default * 2).as_millis() as u64, Ordering::SeqCst);
        tokio::time::timeout(
            std::time::Duration::from_secs(3600),
            fence_in_background(&store, &fence),
        )
        .await
        .expect("fenced within the store's own call timeout");
    }

    /// Review of #7135: a removal the store refused for want of room was answered `500`, the
    /// refusal flattened into a backend fault. The record still stored, it is `507`, as a
    /// create the store has no room for is.
    #[tokio::test]
    async fn a_removal_the_store_has_no_room_for_is_a_507() {
        let (state, store) = super::super::test_store::state(100).await;
        let (container, iri) = listed_record(&state, "r1").await;
        *store.refuse_delete_of.lock().unwrap() = Some(iri.clone());
        let settled = Arc::new(std::sync::Mutex::new(None));
        let r = remove_as_request(&state, &container, &iri, settled.clone()).await;
        assert_eq!(r.err().unwrap().status(), StatusCode::INSUFFICIENT_STORAGE);
        assert_eq!(*settled.lock().unwrap(), Some(false));
        assert!(still_listed(&state.store, &iri, &container).await.unwrap());
        // Any other failure, the record still stored, stays `500`.
        *store.refuse_delete_of.lock().unwrap() = None;
        *store.fail_delete_of.lock().unwrap() = Some(iri.clone());
        let r = remove_as_request(&state, &container, &iri, settled).await;
        assert_eq!(r.err().unwrap().status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
