//! [OPUS-4.8] (sq-it1x) MVCC / ACID multi-statement transaction isolation — OPT-IN
//! (`txn` feature), built on the engine's existing copy-on-write delta-overlay substrate.
//!
//! # Model
//!
//! This is **snapshot isolation (SI)** with **first-committer-wins** write-write conflict
//! detection (optimistic concurrency control), exactly the design the project's literature
//! review records (`research/concurrent-serving-litreview-A-mvcc-benchmarks.md` §A.1):
//!
//! * A [`TransactionManager`] owns the current **committed generation** — an immutable
//!   [`Graph`] published behind a lock, tagged with a monotonically increasing `u64`
//!   commit version.
//! * A **read transaction** ([`ReadTxn`], from [`TransactionManager::begin_read`]) holds a
//!   cheap point-in-time [`GraphSnapshot`] of the generation that was committed when it
//!   began. It reads a consistent state and is *immune to any later commit* — that is the
//!   isolation guarantee. Snapshots are `O(pending delta)` (the immutable indexes / frozen
//!   dictionary are `Arc`-shared by [`Graph::fork`]), never `O(triples)`.
//! * A **write transaction** ([`WriteTxn`], from [`TransactionManager::begin_write`]) forks
//!   the committed generation into a *private working copy*, captures the base version, and
//!   applies SPARQL `UPDATE` statements to that fork — so it has read-your-own-writes while
//!   staying invisible to everyone else. [`WriteTxn::commit`] then, under the manager lock,
//!   checks the **first-committer-wins** rule: if any *other* writer has committed since this
//!   txn began *and its write set overlaps this txn's write set*, the commit is rejected with
//!   [`CommitError::Conflict`] and the whole body is discarded (atomic rollback). Otherwise the
//!   working copy becomes the new committed generation and the version advances.
//!
//! # ACID
//!
//! * **Atomicity** — a `WriteTxn` body is applied to a private fork and only swapped into the
//!   committed slot on success; on conflict (or `rollback`/drop) nothing is published.
//! * **Consistency** — every transaction reads one committed snapshot; a writer evaluates its
//!   `WHERE` clauses against (and only against) its own fork of that snapshot.
//! * **Isolation** — snapshot isolation: readers never block writers or vice versa, and a
//!   reader's view never changes underneath it.
//! * **Durability** — inherited from the underlying [`Graph`]: a directory-backed graph
//!   committed into the manager carries its WAL, so the published generation's deltas are
//!   fsync'd by [`Graph::apply_delta`] as the working copy is built. (The in-memory manager is
//!   non-durable by construction — like the rest of the in-memory engine.)
//!
//! # When SI is enough
//!
//! For a *single-writer* store (one in-flight `WriteTxn` at a time — the common SPARQL-endpoint
//! shape) snapshot isolation **is** serializability, so the conflict check never fires and the
//! cost is one fork + one swap per commit. The OCC check exists to keep *concurrent* writers
//! safe: it rejects the classic lost-update / write-write race rather than silently dropping one
//! writer's effect. SI's residual anomaly (write skew) requires two concurrent read-*write*
//! transactions whose write sets are disjoint but whose invariants overlap; this module does not
//! add the full SSI apparatus (it is dead weight for the single-writer case the design targets).

use crate::update::UpdateEffect;
use crate::QueryBudget;
use oxrdf::Term;
use rustc_hash::FxHashSet;
use sparq_core::{Graph, GraphSnapshot};
use spargebra::algebra::GraphTarget;
use std::sync::{Arc, Mutex};

/// A coarse, sound write-set key: the graph slot a write touched plus, for triple-level
/// operations, the exact triple. `None` slot = the default graph; `Some(name)` = a named graph.
///
/// `Clear`/`Drop` of a whole graph (and `Drop` of *all* named graphs) cannot be enumerated to
/// individual triples without scanning, so they record a *slot-level* key
/// ([`WriteKey::Slot`] / [`WriteKey::AllNamed`] / [`WriteKey::Default`]) that conflicts with ANY
/// other write touching the same slot. This over-approximates conflicts (it may reject two
/// writers that a triple-exact analysis would have let through) but is never *unsound* — it can
/// only refuse, never silently merge incompatible writes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum WriteKey {
    /// A single (slot, triple) touched by an insert or delete.
    Triple(Option<Term>, [Term; 3]),
    /// The entire default graph was rewritten wholesale (`CLEAR DEFAULT` / `DROP DEFAULT`).
    Default,
    /// One named graph was rewritten wholesale (`CLEAR GRAPH g` / `DROP GRAPH g`).
    Slot(Term),
    /// Every named graph was rewritten wholesale (`CLEAR NAMED` / `DROP NAMED`).
    AllNamed,
    /// The whole dataset was rewritten wholesale (`CLEAR ALL` / `DROP ALL`).
    All,
}

impl WriteKey {
    /// Whether two write keys conflict (touch overlapping data). Slot-level keys conflict with
    /// any triple/slot key on the same slot; [`WriteKey::All`] conflicts with everything.
    fn conflicts(&self, other: &WriteKey) -> bool {
        use WriteKey::*;
        match (self, other) {
            // Anything touching the whole dataset conflicts with any other write.
            (All, _) | (_, All) => true,
            // Triple-vs-triple: an exact match.
            (Triple(s1, t1), Triple(s2, t2)) => s1 == s2 && t1 == t2,
            // A default-graph triple vs a default-graph wholesale rewrite.
            (Triple(None, _), Default) | (Default, Triple(None, _)) | (Default, Default) => true,
            // A named-graph triple vs that named graph's wholesale rewrite.
            (Triple(Some(n1), _), Slot(n2)) | (Slot(n2), Triple(Some(n1), _)) => n1 == n2,
            (Slot(n1), Slot(n2)) => n1 == n2,
            // A named write of any kind vs an all-named wholesale rewrite.
            (Triple(Some(_), _), AllNamed) | (AllNamed, Triple(Some(_), _)) => true,
            (Slot(_), AllNamed) | (AllNamed, Slot(_)) | (AllNamed, AllNamed) => true,
            // Default-graph and named-graph writes never collide; AllNamed never touches default.
            _ => false,
        }
    }
}

/// The set of write keys one transaction touched. Conflict detection is the membership /
/// overlap test between two such sets.
#[derive(Debug, Clone, Default)]
struct WriteSet {
    triples: FxHashSet<(Option<Term>, [Term; 3])>,
    slots: FxHashSet<WriteKey>,
}

impl WriteSet {
    fn is_empty(&self) -> bool {
        self.triples.is_empty() && self.slots.is_empty()
    }

    /// Folds one captured update's [`UpdateEffect`]s into the write set.
    fn record(&mut self, effects: &[UpdateEffect]) {
        for e in effects {
            match e {
                UpdateEffect::Delta { slot, inserts, deletes } => {
                    for t in inserts.iter().chain(deletes.iter()) {
                        self.triples.insert((slot.clone(), t.clone()));
                    }
                }
                UpdateEffect::Clear(target) | UpdateEffect::Drop(target) => {
                    self.slots.insert(target_key(target));
                }
                // CREATE of an empty graph touches no data — it cannot lose-update anyone.
                UpdateEffect::Create(_) => {}
            }
        }
    }

    /// Whether this write set conflicts with `other` (any overlapping write key).
    fn conflicts(&self, other: &WriteSet) -> bool {
        // Fast path: two pure triple-level sets — hash-set intersection.
        if !self.triples.is_empty() && !other.triples.is_empty() {
            let (small, big) = if self.triples.len() <= other.triples.len() {
                (&self.triples, &other.triples)
            } else {
                (&other.triples, &self.triples)
            };
            if small.iter().any(|k| big.contains(k)) {
                return true;
            }
        }
        // Slot-level keys conflict against the OTHER side's triples and slots (and vice versa).
        for s in self.slots.iter().chain(other.slots.iter()) {
            // Slot vs the opposite side's triples.
            let side_triples = if self.slots.contains(s) { &other.triples } else { &self.triples };
            if side_triples
                .iter()
                .any(|(slot, t)| s.conflicts(&WriteKey::Triple(slot.clone(), t.clone())))
            {
                return true;
            }
        }
        // Slot vs slot (both directions).
        self.slots.iter().any(|a| other.slots.iter().any(|b| a.conflicts(b)))
    }
}

/// The slot-level write key a `CLEAR`/`DROP` target denotes.
fn target_key(target: &GraphTarget) -> WriteKey {
    match target {
        GraphTarget::DefaultGraph => WriteKey::Default,
        GraphTarget::NamedNode(n) => WriteKey::Slot(Term::NamedNode(n.clone())),
        GraphTarget::NamedGraphs => WriteKey::AllNamed,
        GraphTarget::AllGraphs => WriteKey::All,
    }
}

/// One past commit, retained so a still-open writer that began before it can test for a
/// write-write conflict at its own commit time.
struct Committed {
    version: u64,
    write_set: WriteSet,
}

/// The manager's mutable state: the current committed generation, its version, and the log of
/// recent commits (for conflict detection by stale writers).
struct State {
    graph: Arc<Graph>,
    version: u64,
    /// Commits in version order. Pruned to the oldest still-open writer's base version, so it
    /// never grows unboundedly while writers are short-lived. `Default` (the empty manager) and
    /// every commit append here; entries below the oldest open base are dropped on commit.
    log: Vec<Committed>,
    /// The lowest base version any currently-open write transaction is reading from — the
    /// watermark below which the commit log can be pruned. `u64::MAX` when no writer is open.
    oldest_open_base: u64,
    /// Count of open write transactions (to maintain `oldest_open_base`).
    open_writers: u64,
}

/// An MVCC transaction manager over a single logical [`Graph`].
///
/// Cheap to share across threads (`Arc<Mutex<…>>` internally); clone the [`Arc`] you wrap it in,
/// or pass `&TransactionManager`. See the [module docs](self) for the isolation model.
pub struct TransactionManager {
    state: Mutex<State>,
}

impl TransactionManager {
    /// Creates a manager whose initial committed generation is `graph` (committed version 0).
    ///
    /// The graph is taken by value and becomes the immutable base of generation 0; readers and
    /// writers fork/snapshot it copy-on-write. For DURABILITY, pass a directory-backed graph
    /// (opened from disk, under sparq-core's `mmap` feature) — its WAL travels with the published
    /// generations.
    pub fn new(graph: Graph) -> TransactionManager {
        TransactionManager {
            state: Mutex::new(State {
                graph: Arc::new(graph),
                version: 0,
                log: Vec::new(),
                oldest_open_base: u64::MAX,
                open_writers: 0,
            }),
        }
    }

    /// The current committed version (advances by 1 on every successful [`WriteTxn::commit`]
    /// that published a change; an empty/no-op commit does not advance it).
    pub fn committed_version(&self) -> u64 {
        self.lock().version
    }

    /// A read-only, point-in-time snapshot of the *current* committed generation — the same
    /// state a [`begin_read`](Self::begin_read) transaction would see, without the version
    /// bookkeeping. Immutable and immune to later commits.
    pub fn snapshot(&self) -> GraphSnapshot {
        self.lock().graph.snapshot()
    }

    /// Begins a snapshot-isolation **read** transaction over the current committed generation.
    /// Its view is fixed at begin time and unaffected by any later commit.
    pub fn begin_read(&self) -> ReadTxn {
        let st = self.lock();
        ReadTxn { snapshot: st.graph.snapshot(), version: st.version }
    }

    /// Begins a **write** transaction: forks the current committed generation into a private
    /// working copy and records the base version. Apply statements with [`WriteTxn::update`],
    /// then [`WriteTxn::commit`] (first-committer-wins) or [`WriteTxn::rollback`] (discard).
    pub fn begin_write(&self) -> WriteTxn<'_> {
        let mut st = self.lock();
        let base_version = st.version;
        let working = st.graph.fork();
        st.open_writers += 1;
        st.oldest_open_base = st.oldest_open_base.min(base_version);
        WriteTxn {
            manager: self,
            working: Some(working),
            base_version,
            write_set: WriteSet::default(),
            effects: Vec::new(),
            finished: false,
        }
    }

    /// Consumes the manager, returning the current committed generation as an owned [`Graph`].
    ///
    /// Any outstanding [`ReadTxn`]/[`WriteTxn`] borrow `&self`, so they must have ended (or never
    /// existed) for this to be callable. If another `Arc` clone of the committed generation is
    /// still alive (e.g. a leaked snapshot), the graph is cloned out via a fork.
    pub fn into_inner(self) -> Graph {
        let state = self.state.into_inner().unwrap_or_else(|e| e.into_inner());
        Arc::try_unwrap(state.graph).unwrap_or_else(|arc| arc.fork())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A writer ended (committed or rolled back): decrement the open-writer count and, when the
    /// last one closes, reset the prune watermark.
    fn writer_closed(st: &mut State) {
        st.open_writers = st.open_writers.saturating_sub(1);
        if st.open_writers == 0 {
            st.oldest_open_base = u64::MAX;
            st.log.clear(); // no open writer can need the conflict history any more
        }
    }
}

/// A snapshot-isolation read transaction. Derefs to [`&Graph`](Graph), so it is queryable with
/// every read entry point ([`crate::query`], [`crate::ask`], [`crate::count`], …). Its view is
/// fixed at [`begin_read`](TransactionManager::begin_read) time.
pub struct ReadTxn {
    snapshot: GraphSnapshot,
    version: u64,
}

impl ReadTxn {
    /// The committed version this read transaction is observing.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The point-in-time graph (also reachable via the [`Deref`](std::ops::Deref) impl).
    pub fn graph(&self) -> &Graph {
        &self.snapshot
    }
}

impl std::ops::Deref for ReadTxn {
    type Target = Graph;
    fn deref(&self) -> &Graph {
        &self.snapshot
    }
}

/// A write transaction over a private working copy of a committed generation.
///
/// Apply SPARQL `UPDATE` statements with [`update`](Self::update) (each is request-atomic against
/// the working copy), read its in-progress state with [`graph`](Self::graph) (read-your-own-
/// writes), then [`commit`](Self::commit) (first-committer-wins) or [`rollback`](Self::rollback).
/// Dropping a `WriteTxn` without committing rolls it back.
pub struct WriteTxn<'m> {
    manager: &'m TransactionManager,
    /// The private fork. `Some` until the txn finishes (commit/rollback takes it).
    working: Option<Graph>,
    base_version: u64,
    write_set: WriteSet,
    /// The ordered, RESOLVED effect log of every statement applied — replayed onto the
    /// *current* committed generation at commit time so a stale-but-non-conflicting writer
    /// re-applies its delta rather than publishing its (now out-of-date) private fork wholesale
    /// (which would lose-update the intervening committer). Deterministic by construction (it
    /// carries resolved triples), so replay reproduces exactly what the working copy committed.
    effects: Vec<UpdateEffect>,
    finished: bool,
}

impl WriteTxn<'_> {
    /// The base committed version this transaction forked from.
    pub fn base_version(&self) -> u64 {
        self.base_version
    }

    /// The private working copy, reflecting every [`update`](Self::update) applied so far. Read
    /// it with any read entry point for read-your-own-writes inside the transaction.
    pub fn graph(&self) -> &Graph {
        self.working.as_ref().expect("write txn already finished")
    }

    /// Applies one SPARQL `UPDATE` request to the working copy (request-atomic: a parse error or
    /// failing operation leaves the working copy unchanged), recording the *resolved* write set
    /// for commit-time conflict detection.
    ///
    /// The whole request body is applied through the engine's request-atomic in-place path, so a
    /// later operation that fails rolls THIS statement back without affecting earlier committed
    /// statements in the same transaction.
    pub fn update(&mut self, sparql: &str) -> Result<(), String> {
        self.update_with_budget(sparql, &QueryBudget::unlimited())
    }

    /// [`update`](Self::update) under a cooperative [`QueryBudget`] (bounds a `DELETE/INSERT …
    /// WHERE` exactly as the budgeted engine entry points do).
    pub fn update_with_budget(&mut self, sparql: &str, budget: &QueryBudget) -> Result<(), String> {
        let working = self.working.as_mut().expect("write txn already finished");
        // Apply to a throwaway fork first so a failure leaves the working copy untouched
        // (request-atomicity across the txn's statements), capturing the resolved write set.
        let mut staged = working.fork();
        let effects = crate::update::update_in_place_capturing(&mut staged, sparql, budget)?;
        // Success: adopt the staged state, fold the write set in, and keep the resolved effect
        // log so a stale-but-non-conflicting commit can replay this delta onto the current
        // committed generation rather than publishing the (then out-of-date) private fork.
        *working = staged;
        self.write_set.record(&effects);
        self.effects.extend(effects);
        Ok(())
    }

    /// Commits the transaction (first-committer-wins). Returns the new committed version on
    /// success, or [`CommitError::Conflict`] if a concurrent writer committed an overlapping
    /// write set since this transaction began (in which case the whole body is discarded).
    ///
    /// An *empty* write set (no data-touching statement) always commits and never conflicts; if
    /// nothing was published the version is unchanged.
    pub fn commit(mut self) -> Result<u64, CommitError> {
        let working = self.working.take().expect("write txn already finished");
        self.finished = true;
        let mut st = self.manager.lock();

        let stale = st.version != self.base_version;

        // First-committer-wins: if other writers committed since our base, conflict iff any of
        // their write sets overlaps ours. An empty write set can never overlap, so it is exempt.
        if stale && !self.write_set.is_empty() {
            let conflict = st
                .log
                .iter()
                .filter(|c| c.version > self.base_version)
                .any(|c| c.write_set.conflicts(&self.write_set));
            if conflict {
                let current = st.version;
                TransactionManager::writer_closed(&mut st);
                return Err(CommitError::Conflict { base: self.base_version, current });
            }
        }

        // No conflict: publish. An empty write set publishes nothing (version unchanged), so a
        // read-only "transaction" committed for symmetry costs nothing.
        if self.write_set.is_empty() {
            TransactionManager::writer_closed(&mut st);
            return Ok(st.version);
        }

        // The published generation. When we are up to date (base == current) the private fork
        // already reflects exactly the right state, so publish it directly. When we are
        // stale-but-non-conflicting, the private fork is built atop an OLD base and would
        // lose-update the intervening committer if published wholesale; instead REPLAY this
        // txn's resolved effect log onto a fork of the CURRENT committed generation. Replay is
        // deterministic (the effects carry resolved triples), and disjointness was just proven,
        // so the result is the merge of both writers' deltas.
        let published = if stale {
            let mut merged = st.graph.fork();
            crate::update::apply_effects(&mut merged, &self.effects)
                .map_err(|_| CommitError::Conflict { base: self.base_version, current: st.version })?;
            merged
        } else {
            working
        };

        let new_version = st.version + 1;
        st.graph = Arc::new(published);
        st.version = new_version;
        st.log.push(Committed { version: new_version, write_set: std::mem::take(&mut self.write_set) });
        TransactionManager::writer_closed(&mut st);
        // Prune commit history below the oldest still-open writer's base — those entries can no
        // longer be needed by any open transaction's conflict check.
        let watermark = st.oldest_open_base;
        st.log.retain(|c| c.version > watermark);
        Ok(new_version)
    }

    /// Discards the transaction's working copy without publishing anything. (Dropping the
    /// `WriteTxn` does the same — this is the explicit, intention-revealing form.)
    pub fn rollback(mut self) {
        self.working.take();
        self.finished = true;
        let mut st = self.manager.lock();
        TransactionManager::writer_closed(&mut st);
    }
}

impl Drop for WriteTxn<'_> {
    fn drop(&mut self) {
        // A txn dropped without commit/rollback (e.g. on a `?` early-return) must still release
        // its open-writer slot so the conflict log can be pruned.
        if !self.finished {
            let mut st = self.manager.lock();
            TransactionManager::writer_closed(&mut st);
        }
    }
}

/// Why a [`WriteTxn::commit`] failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitError {
    /// A concurrent writer committed an overlapping write set since this transaction began
    /// (first-committer-wins). The transaction's body has been fully discarded; the caller may
    /// retry against the new committed state. `base` is the version this txn forked from;
    /// `current` is the committed version it lost the race to.
    Conflict { base: u64, current: u64 },
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommitError::Conflict { base, current } => write!(
                f,
                "transaction commit conflict: forked from version {base} but the committed version is now {current} with an overlapping write set (first-committer-wins; retry)"
            ),
        }
    }
}

impl std::error::Error for CommitError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(src: &str) -> Graph {
        Graph::load_str(src, "turtle").unwrap()
    }

    #[test]
    fn write_key_default_vs_named_disjoint() {
        let nn = |s: &str| Term::NamedNode(oxrdf::NamedNode::new(s).unwrap());
        let t = [nn("http://x/s"), nn("http://x/p"), nn("http://x/o")];
        let def = WriteKey::Triple(None, t.clone());
        let named = WriteKey::Triple(Some(nn("http://x/g")), t.clone());
        assert!(!def.conflicts(&named), "default-graph and named-graph triples must be disjoint");
        assert!(def.conflicts(&WriteKey::Default));
        assert!(named.conflicts(&WriteKey::Slot(nn("http://x/g"))));
        assert!(!named.conflicts(&WriteKey::Slot(nn("http://x/other"))));
        assert!(WriteKey::All.conflicts(&named));
    }

    #[test]
    fn empty_writeset_never_conflicts() {
        let nn = |s: &str| Term::NamedNode(oxrdf::NamedNode::new(s).unwrap());
        let mut a = WriteSet::default();
        a.triples.insert((None, [nn("http://x/s"), nn("http://x/p"), nn("http://x/o")]));
        assert!(!a.conflicts(&WriteSet::default()));
        assert!(!WriteSet::default().conflicts(&a));
    }

    #[test]
    fn manager_round_trips_a_simple_commit() {
        let m = TransactionManager::new(g("@prefix : <http://ex/> . :a :p :b ."));
        assert_eq!(m.committed_version(), 0);
        let mut w = m.begin_write();
        w.update("PREFIX : <http://ex/> INSERT DATA { :x :p :y }").unwrap();
        let v = w.commit().unwrap();
        assert_eq!(v, 1);
        let r = m.begin_read();
        assert_eq!(crate::count(&r, "SELECT * WHERE { ?s ?p ?o }").unwrap(), 2);
    }
}
