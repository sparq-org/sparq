// sq-evb1: EL+⊥ completion (CR1–CR5) over S(C) and R(r).
// sq-pbz04.2.1: + CR6 (safe nominals).
//
// The saturator computes the least fixpoint of the Baader–Brandt–Lutz completion rules with
// bottom (the SAME calculus ELK's core implements, restricted to the recognised fragment):
//
//   init   S(C) := {C, ⊤}                                     for every concept C
//   CR1    C ⊑ D ∈ T,  C ∈ S(X)            ⇒  D ∈ S(X)
//   CR2    C1 ⊓ C2 ⊑ D ∈ T,  C1,C2 ∈ S(X)  ⇒  D ∈ S(X)
//   CR3    C ⊑ ∃r.D ∈ T,  C ∈ S(X)         ⇒  (X,D) ∈ R(r)
//   CR4    ∃r.D ⊑ E ∈ T,  (X,Y) ∈ R(r),  D ∈ S(Y)   ⇒  E ∈ S(X)
//   CR5    (X,Y) ∈ R(r),  ⊥ ∈ S(Y)         ⇒  ⊥ ∈ S(X)
//   CR6    {a} ∈ S(X) ∩ S(Y),  X ⇝_R Y     ⇒  S(X) := S(X) ∪ S(Y)
//   CRs1   ∃r.Self ∈ S(X)                  ⇒  (X,X) ∈ R(r)                       [sq-pbz04.2.6]
//   CRs2   (X,X) ∈ R(r),  ∃r.Self ⊑ D ∈ T  ⇒  D ∈ S(X)   (self-concept atom + CR1; see below)
//   CRs3   (X,X) ∈ R(r),  X a NOMINAL {a}  ⇒  ∃r.Self ∈ S(X)                     [sq-8zqwb]
//   CRs4   ∃r.Self ∈ S(X), r ⊑* s            ⇒  ∃s.Self ∈ S(X)                  [sq-l2o9e]
//
// sq-pbz04.2.6 — the two EL++ self-restriction (`owl:hasSelf` / `ObjectHasSelf`)
// completion rules. `∃r.Self` (the LOCAL reflexivity concept `{x | (x,x) ∈ r}`) is extracted as
// a distinguished basic concept atom (`Names::self_concept`), so `X ⊑ ∃r.Self` and `∃r.Self ⊑ D`
// reduce to ordinary `Normal::Sub` axioms and CR1/CR2/CR4 propagate `∃r.Self` memberships for
// free. CRs1 is the ONE genuinely new rule (implemented in the worklist below): when the atom
// `∃r.Self` lands in S(X) — i.e. `X ⊑ ∃r.Self` — it adds the reflexive link `(X,X) ∈ R(r)` (which
// then feeds CR4/CR5). CRs2 is realised BY CR1: `∃r.Self ⊑ D` is the axiom `Sub(self_r, D)`, so
// `∃r.Self ∈ S(X)` fires `D ∈ S(X)` directly. SOUNDNESS side-condition (load-bearing): CRs2's
// premise `(X,X) ∈ R(r)` is tracked as the self-concept membership `∃r.Self ∈ S(X)`, NOT the raw
// R-link — a general self-link from CR3 (`X ⊑ ∃r.X`, whose invariant is only `X ⊑ ∃r.X`, NOT
// `X ⊑ ∃r.Self`) must NEVER trigger CRs2. CRs1 IS sound to add to the ordinary link set because
// `X ⊑ ∃r.Self ⟹ X ⊑ ∃r.X` (the self-successor is X itself), so the R-invariant holds.
//
// sq-8zqwb (EL wave-2) — CRs3, the NOMINAL-REFLEXIVITY converse of CRs1: a SAME-NOMINAL
// self-link `({a},{a}) ∈ R(r)` (e.g. the internalized assertion `a r a`, asserted OR derived)
// reads off as `∃r.Self ∈ S({a})`. SOUND because a nominal denotes a singleton: the R-invariant
// gives `T ⊨ {a} ⊑ ∃r.{a}`, so a^I's r-successor inside `{a^I}` IS a^I itself — `(a^I,a^I) ∈ r^I`,
// which is exactly `a^I ∈ (∃r.Self)^I`, i.e. `T ⊨ {a} ⊑ ∃r.Self`. The nominal guard is
// load-bearing: for a GENERAL X the CRs2 side-condition above stands unchanged (each member of X
// has SOME r-successor in X, not necessarily itself). Implemented at the single link chokepoint
// (`add_link`), so asserted, derived, and (under `rbox`) role-closed self-links all fire it in
// BOTH engines; on a hasSelf-free ontology the `∃r.Self` lookup is `None` and nothing changes.
// Graduates the WG `New-Feature-SelfRestriction-002` converse shape ("Peter likes Peter ⊨ Peter
// ∈ ∃likes.Self") that sq-pbz04.2.6 pinned as an honest boundary (deferred from #1681).
//
// CR4 is the load-bearing existential-traversal rule that OWL 2 RL lacks (spike §1.2): it is
// the only rule that reasons THROUGH an r-successor, and it is why running `--reason owl` over
// an EL ontology returns a silently-incomplete hierarchy. The implementation is a worklist
// saturation: a per-concept queue of newly-derived subsumers drives rule application, so each
// derived membership is processed once. CR6 (the nominal rule, "Pushing the EL Envelope"
// IJCAI-05) runs as a between-fixpoint pass — see `cr6_pass` for the rule, the ⇝_R
// side-condition, and the soundness argument. Single-threaded by default (Phase E1); the opt-in
// `par` feature (Phase E4, sq-wy3i6) adds the deterministic bulk-synchronous parallel engine at
// the tail of this file — same rules, same least fixpoint, identical closure at every thread count.
//
// ## Substrate adoption evaluation -- REASONED NON-ADOPTION sq-pbz04.2.3
//
// Bead sq-pbz04.2.3 evaluated whether the CR1-CR5 saturation joins can adopt the shared
// `sparq_substrate::join` kernels (the `build_table`/`probe_emit`/`hash_probe_serial` and
// `join::delta::DeltaTable` path, analogously to `sparq-reason/src/substrate_join.rs`
// which drives rdfs2/3/7 on the substrate). Disposition: DOCUMENTED NON-ADOPTION.
// Referenced from `research/reasoner-federation-program.md` sections 3 and 6.
//
// Five structural reasons this module's join shapes are not profitable substrate consumers:
//
// 1. PER-EVENT WORKLIST, NOT BATCH BUILD/PROBE. The substrate `build_table`+`probe_emit`
//    requires two static `&[Row]` slices: a build side and a batch of probe rows. The EL
//    fixpoint processes memberships one at a time from a `Vec<(Concept, Concept)>` worklist;
//    each dequeued `(X, D)` immediately fires rules whose outputs re-enter the queue before
//    the outer loop resumes. Batching would require collecting an entire delta round first,
//    but the five rules have heterogeneous triggers and no meaningful "round" boundary --
//    unlike the OWL-RL semi-naive `delta join full` pattern `DeltaTable` was designed for.
//
// 2. SIMULTANEOUS READ AND WRITE ON THE SAME RELATIONS. CR2 reads `S(X)` (checks whether
//    a partner conjunct is already present) while CR1/CR3/CR4/CR5 write new members into
//    `S(X)` within the same worklist pass. The substrate kernels assume `&[Row]` immutability
//    at probe time; there is no safe seam to hand `S(X)` to a kernel as a probe slice while
//    also holding it mutably for insertion. The `add()` helper at the bottom of this file is
//    called inside rule-specific borrows that cannot be widened to a whole-relation borrow.
//
// 3. AXIOMINDEX IS ALREADY AN OPTIMAL SINGLE-KEY HASH TABLE. The `AxiomIndex` fields are
//    `FxHashMap<Concept, Vec<...>>` keyed by `Concept = u32` -- a direct O(1) lookup using
//    the native integer hash. Reshaping to substrate `Row` (`SmallVec<[u32; 4]>`) build
//    tables would add per-axiom `Row::from_slice` allocation and per-lookup key projection
//    through `JoinKeys`, replacing a 4-byte integer hash with a `SmallVec` hash over the
//    same data. Asymptotic complexity is identical; the per-lookup constant is strictly
//    higher with no algorithmic benefit.
//
// 4. CR4 IS A 3-WAY TRIANGLE JOIN WITH THREE GROWING SIDES. CR4 combines S(Y) memberships,
//    R(r) predecessor links, and axiom index lookups, where both S(Y) and R(r) grow during
//    the same pass. The substrate handles 2-way binary joins (static build probed by a
//    batch probe slice) and the 2-way semi-naive delta/full shape. The EL triangle -- where
//    S-sets, R-links, and AxiomIndex all inform the same rule firing -- has no direct
//    mapping to any of the four substrate kernels (merge-join, hash-join, bind-join,
//    trie-join) or to the `DeltaTable` seam. Forcing it would require rebuilding all three
//    sides at each queue step, which is strictly worse than the current per-event probe.
//
// 5. THE QL PRECEDENT APPLIES IN FULL. The OWL 2 QL certain-answer oracle (sq-qo1a9) is a
//    documented non-consumer of the substrate join kernels: its query-rewriting shape
//    (PerfectRef + tree-witness + UCQ minimisation) is not a build/probe or semi-naive
//    fixpoint. The EL worklist fixpoint is a cleaner non-consumer still: QL at least
//    evaluates CQ answers over a static dataset, which could use the engine join path;
//    EL saturation joins never produce a relational tuple output -- they grow set-based
//    state incrementally, and every "join" is a membership test, not a tuple production.
//
// Comparison with the RDFS precedent (`sparq-reason/src/substrate_join.rs`, sq-yk6or):
// rdfs2/3/7 are profitable substrate consumers because (a) the schema closure maps are
// built ONCE and probed many times without mutation, and (b) the output is a multiset
// of triples, exactly what `probe_emit`'s combined-row output models. Neither condition
// holds here: S(X) and R(r) grow during every worklist step, and the output is
// side-effectful set membership, not a combinatorial tuple join.
//
// Also mirroring the note in `substrate_join.rs` module doc: "The OWL-RL semi-naive
// fixpoint (owl.rs): a delta-driven delta/full join with union-find sameAs
// canonicalisation. Genuinely a different (incremental, mutating) join shape than the
// substrate's static &[Row] build/probe kernel." The EL worklist is even more tightly
// coupled, with no round boundary to hang a delta on.
//
// Conclusion: this module stays on its hand-rolled FxHashMap worklist. There is no
// profitable behaviour-neutral seam to the substrate kernels for CR1-CR5.
// sq-wy3i6 (the E4 concurrency this note anticipated): the `par` engine batches the
// frontier into bulk-synchronous ROUNDS, but each rule firing is still a per-membership
// hash-set probe against mutating S/R state (the apply phase mutates between rounds), so
// reasons 2-4 hold unchanged and the non-adoption verdict stands for the parallel path too.

use crate::normal::{Concept, Names, Normal, Role, BOTTOM, TOP};
#[cfg(feature = "rbox")]
use crate::rbox::RoleBox;
use rustc_hash::{FxHashMap, FxHashSet};

/// Indexes the normal-form axioms by the premise shape each completion rule matches on, so a
/// derived membership `C ∈ S(X)` can fire the relevant rules without scanning all axioms.
#[derive(Default)]
struct AxiomIndex {
    /// CR1: `C ⊑ D`, keyed by the subclass `C`.
    sub: FxHashMap<Concept, Vec<Concept>>,
    /// CR2: `C1 ⊓ C2 ⊑ D`, keyed by EACH conjunct (with the partner + head).
    and_by_conjunct: FxHashMap<Concept, Vec<(Concept, Concept)>>,
    /// CR3: `C ⊑ ∃r.D`, keyed by the subclass `C`.
    exists: FxHashMap<Concept, Vec<(Role, Concept)>>,
    /// CR4: `∃r.D ⊑ E`, keyed by `(r, D)` (the filler that must appear in S(Y)).
    exists_sub: FxHashMap<(Role, Concept), Vec<Concept>>,
}

impl AxiomIndex {
    fn build(axioms: &[Normal]) -> AxiomIndex {
        let mut ix = AxiomIndex::default();
        for &ax in axioms {
            match ax {
                Normal::Sub(c, d) => ix.sub.entry(c).or_default().push(d),
                Normal::AndSub(c1, c2, d) => {
                    ix.and_by_conjunct.entry(c1).or_default().push((c2, d));
                    ix.and_by_conjunct.entry(c2).or_default().push((c1, d));
                }
                Normal::SubExists(c, r, d) => ix.exists.entry(c).or_default().push((r, d)),
                Normal::ExistsSub(r, c, d) => ix.exists_sub.entry((r, c)).or_default().push(d),
            }
        }
        ix
    }
}

/// The saturation state: subsumer sets `S(C)` and existential links `R(r)`.
pub struct Saturation {
    /// `S[c]` = the set of basic concepts subsuming `c` (i.e. `c ⊑ each`).
    pub s: Vec<FxHashSet<Concept>>,
    /// For each role, the predecessor index of `R(r)`: `r_pred[r][Y]` = the set of `X` with
    /// `(X, Y) ∈ R(r)`. Indexed by the SUCCESSOR `Y` because CR4/CR5 fire when something new
    /// lands in `S(Y)` and need every predecessor `X`.
    pub r_pred: FxHashMap<Role, FxHashMap<Concept, FxHashSet<Concept>>>,
    /// The forward links `R(r)`: `r_succ[r][X]` = successors `Y`. Kept so CR4 can also fire
    /// when a *new link* (not a new S-membership) is the trigger.
    pub r_succ: FxHashMap<Role, FxHashMap<Concept, FxHashSet<Concept>>>,
}

/// Runs CR1–CR6 (and, under the `rbox` feature, CR10/CR11 role saturation) to a fixpoint over
/// `axioms` for the concepts of `names`. Returns the saturated [`Saturation`]; `S[c]` then
/// holds every concept subsuming `c`.
#[cfg(feature = "rbox")]
pub fn saturate(axioms: &[Normal], names: &Names, role_box: &RoleBox) -> Saturation {
    saturate_inner(axioms, names, role_box)
}

/// Runs CR1–CR6 to a fixpoint (no role hierarchy — roles are compared for equality only). The
/// E1 entry point; the `rbox` build uses the `RoleBox`-aware overload above.
#[cfg(not(feature = "rbox"))]
pub fn saturate(axioms: &[Normal], names: &Names) -> Saturation {
    saturate_inner(axioms, names)
}

/// The shared CR1–CR5 (+CR10/CR11 under `rbox`) worklist fixpoint, alternated with the CR6
/// nominal pass (sq-pbz04.2.1) until neither derives anything new. The `role_box` argument
/// exists only in the `rbox` build; CR3 link insertion routes through it so every asserted
/// existential link is closed under role inclusion + composition before CR4/CR5 fire.
///
/// All rules are monotone (S-sets and R-links only ever grow, bounded by the concept count),
/// so the alternation terminates and reaches the SAME least fixpoint regardless of rule
/// order — classification stays deterministic. On a nominal-free ontology `cr6_pass` returns
/// `false` on its first O(1) check, so the loop runs the CR1–CR5 worklist exactly once —
/// byte-for-byte the pre-CR6 behaviour and cost.
fn saturate_inner(
    axioms: &[Normal],
    names: &Names,
    #[cfg(feature = "rbox")] role_box: &RoleBox,
) -> Saturation {
    let ix = AxiomIndex::build(axioms);
    let n = names.concept_count();
    let mut sat = Saturation {
        s: vec![FxHashSet::default(); n],
        r_pred: FxHashMap::default(),
        r_succ: FxHashMap::default(),
    };

    // Worklist of (concept X, newly-added subsumer D) pairs: each represents "D just entered
    // S(X)". init seeds S(C) = {C, ⊤} for every concept and queues both memberships.
    let mut queue: Vec<(Concept, Concept)> = Vec::new();
    seed_init_rows(&mut sat, 0, n, &mut queue);

    #[cfg(feature = "rbox")]
    drain(&mut sat, &ix, names, role_box, &mut queue);
    #[cfg(not(feature = "rbox"))]
    drain(&mut sat, &ix, names, &mut queue);
    sat
}

/// Seeds `init` — `S(C) := {C, ⊤}` — for the concept rows `from..to`, queueing both memberships
/// so the worklist processes them. Split out of [`saturate_inner`] (sq-clsv6) so the
/// incremental path can seed only the rows a TBox edit newly minted, with IDENTICAL seeding
/// semantics for old and new rows.
fn seed_init_rows(
    sat: &mut Saturation,
    from: usize,
    to: usize,
    queue: &mut Vec<(Concept, Concept)>,
) {
    for c in from as Concept..to as Concept {
        if sat.s[c as usize].insert(c) {
            queue.push((c, c));
        }
        if sat.s[c as usize].insert(TOP) {
            queue.push((c, TOP));
        }
    }
}

/// The CR1–CR5 (+ CRs1, + CR10/CR11 under `rbox`) worklist drain alternated with the CR6
/// nominal pass, run to a fixpoint over whatever `sat` / `queue` it is handed. Extracted from
/// [`saturate_inner`] VERBATIM (sq-clsv6) so the from-scratch and incremental
/// entries execute the SAME rule code — the reason the two agree by construction rather than by
/// a maintained parallel implementation.
///
/// Contract the caller must uphold (the queue-item invariant every rule argument rests on): a
/// pair `(X, D)` is in `queue` only if `D` has ALREADY been inserted into `S(X)`, and every
/// membership whose rules have not yet been fired against `ix` IS in `queue`. Given that, the
/// drain leaves `sat` closed under all rules — see the incremental soundness/completeness note
/// in `incremental.rs`.
fn drain(
    sat: &mut Saturation,
    ix: &AxiomIndex,
    names: &Names,
    #[cfg(feature = "rbox")] role_box: &RoleBox,
    queue: &mut Vec<(Concept, Concept)>,
) {
    // sq-pbz04.2.6: O(1) fast-path guard — on a hasSelf-free ontology CRs1 is skipped
    // entirely, so classification is byte-identical (behaviour AND cost) to the pre-CR-Self path.
    let has_self = names.has_self_restrictions();
    loop {
        while let Some((x, d)) = queue.pop() {
            // CR1: every `D ⊑ E` axiom adds E to S(X).
            if let Some(es) = ix.sub.get(&d) {
                for &e in es {
                    add(&mut sat.s[x as usize], x, e, queue);
                }
            }
            // CR2: every `D ⊓ C2 ⊑ E` axiom fires if C2 also already ∈ S(X).
            if let Some(parts) = ix.and_by_conjunct.get(&d) {
                for &(other, e) in parts {
                    if sat.s[x as usize].contains(&other) {
                        add(&mut sat.s[x as usize], x, e, queue);
                    }
                }
            }
            // CR3: every `D ⊑ ∃r.F` axiom adds the link (X, F) to R(r). Under `rbox` the link
            // is closed under CR10 (role inclusion) + CR11 (composition) before CR4/CR5 fire.
            if let Some(links) = ix.exists.get(&d) {
                for &(r, f) in links {
                    #[cfg(feature = "rbox")]
                    add_link_rbox(sat, r, x, f, ix, names, role_box, queue);
                    #[cfg(not(feature = "rbox"))]
                    add_link(sat, r, x, f, ix, names, queue);
                }
            }
            // CRs1 (sq-pbz04.2.6): the self-restriction concept `∃r.Self` just entered S(X) —
            // i.e. `X ⊑ ∃r.Self` — so add the reflexive link `(X,X) ∈ R(r)`. `add_link` fires
            // CR4/CR5 for it, and under `rbox` `add_link_rbox` closes it under role
            // inclusion/composition. SOUND: `X ⊑ ∃r.Self ⟹ X ⊑ ∃r.X` (the r-successor is X
            // itself), so `(X,X) ∈ R(r)` respects the `(C,D) ∈ R(r) ⟹ T ⊨ C ⊑ ∃r.D` invariant.
            // CRs2 needs NO code here: `∃r.Self ⊑ D` is the axiom `Sub(self_r, D)`, so the `D ∈
            // S(X)` conclusion already fired via CR1 above when `∃r.Self` entered S(X).
            if has_self {
                if let Some(r) = names.self_role(d) {
                    #[cfg(feature = "rbox")]
                    add_self_link_rbox(
                        sat, r, x, ix, names, role_box, queue,
                    );
                    #[cfg(not(feature = "rbox"))]
                    add_link(sat, r, x, x, ix, names, queue);
                }
            }
            // CR4 / CR5 with the new membership `D ∈ S(X)` as the trigger, where X is the
            // SUCCESSOR `Y` of some link `(P, X) ∈ R(r)`. Collect the (predecessor, concept)
            // work FIRST (releasing the immutable `r_pred` borrow) before mutating `sat.s`.
            let mut derived: Vec<(Concept, Concept)> = Vec::new();
            for (&r, preds_by_succ) in &sat.r_pred {
                let Some(preds) = preds_by_succ.get(&x) else {
                    continue;
                };
                // CR4: `∃r.D ⊑ E` and (P, X) ∈ R(r) and D ∈ S(X)  ⇒  E ∈ S(P).
                if let Some(es) = ix.exists_sub.get(&(r, d)) {
                    for &p in preds {
                        for &e in es {
                            derived.push((p, e));
                        }
                    }
                }
                // CR5: ⊥ ∈ S(X) and (P, X) ∈ R(r)  ⇒  ⊥ ∈ S(P).
                if d == BOTTOM {
                    for &p in preds {
                        derived.push((p, BOTTOM));
                    }
                }
            }
            for (p, e) in derived {
                add(&mut sat.s[p as usize], p, e, queue);
            }
        }
        // CR6 (safe nominals): merges may enqueue new memberships, which can in turn create
        // new links / reachability — re-run the worklist, then re-check, until neither moves.
        if !cr6_pass(sat, names, queue) {
            break;
        }
    }
}

/// sq-clsv6 (Phase E5, `incremental`): RESUMES an already-saturated [`Saturation`]
/// after a MONOTONE TBox extension — `axioms` is the full post-edit axiom set and `added` the
/// newly-extracted delta (so `axioms` is a superset of the pre-edit axiom set). Returns the
/// number of retained memberships re-queued as the delta frontier (the incremental work measure).
///
/// The caller MUST have established that the edit is monotone (nothing retracted, no existing
/// axiom's meaning changed) — `crate::incremental` owns that decision and falls back to a full
/// re-classification otherwise. Under `rbox` the caller must also pass a `role_box` rebuilt from
/// the SAME (unchanged) told role axioms at the post-edit role count.
///
/// # Why the result is the same least fixpoint a from-scratch run computes
///
/// Write `L` for the least fixpoint of the completion rules over `axioms` (what [`saturate`]
/// would compute) and `S0` for the retained pre-edit state.
///
/// * **`S0` is contained in `L` (nothing retained is wrong).** Every retained membership/link was
///   derived by these same monotone rules from a SUBSET of `axioms` — monotonicity of the edit is
///   exactly this premise — so it is derivable from `axioms` too, hence in `L`.
/// * **The resumed run ends closed under every rule.** The drain's invariant is "every membership
///   whose rules have not yet fired against the CURRENT axiom index is queued". The seeding below
///   re-establishes it: (a) rows minted by the edit get the full `init` seed; (b) for every axiom
///   in `added`, every retained membership matching one of its TRIGGER keys is re-queued, so each
///   new axiom is re-tried against the whole retained state; (c) axioms that are NOT new already
///   fired against every retained membership before the edit, and fire again for anything
///   inserted after it. Link-triggered CR4/CR5 (and the `rbox` CR10/CR11 closure) run inside
///   `add_link` for every link the drain inserts, and no pre-existing link can need a new axiom
///   without being reached: `ExistsSub(r, c, e)`'s membership-triggered arm is seeded by (b) via
///   the key `c`, which visits every successor `Y` with `c` in `S(Y)` and from there every
///   predecessor link already in `r_pred`.
/// * **Therefore the result is `L`.** It is a fixpoint containing the `init` seeds, so it contains
///   `L`; and it is built from `S0` (contained in `L`) by sound rule applications over `axioms`,
///   so it is contained in `L`.
///
/// Cost: one scan of the retained closure to locate the trigger contexts, then rule work only for
/// the seeded frontier and its cascade — instead of re-firing every rule for every membership as
/// a full re-saturation does. Internal FRESH normalization names are NOT reused across edits (the
/// delta mints its own), so the concept index grows with edit history; the NAMED-class projection
/// is unaffected (a fresh name never carries a dict id), which is why `tests/incremental.rs` can
/// pin the projected hierarchy equal to a from-scratch run.
#[cfg(feature = "incremental")]
pub fn resaturate(
    sat: &mut Saturation,
    axioms: &[Normal],
    added: &[Normal],
    names: &Names,
    #[cfg(feature = "rbox")] role_box: &RoleBox,
) -> usize {
    let ix = AxiomIndex::build(axioms);
    let retained_rows = sat.s.len();
    let n = names.concept_count();
    debug_assert!(n >= retained_rows, "the name table only ever grows");

    let mut queue: Vec<(Concept, Concept)> = Vec::new();
    // (a) rows the edit minted: the ordinary `init` seed, identical to a from-scratch run.
    sat.s.resize(n, FxHashSet::default());
    seed_init_rows(sat, retained_rows, n, &mut queue);

    // (b) the delta frontier: every RETAINED membership that is a trigger key of a NEW axiom. A
    // key that is itself a newly-minted concept needs no seeding — no retained S-set can contain
    // it, and the membership that puts it there is queued when it is inserted.
    let mut keys: FxHashSet<Concept> = FxHashSet::default();
    for &ax in added {
        match ax {
            Normal::Sub(c, _) => {
                keys.insert(c);
            }
            Normal::AndSub(c1, c2, _) => {
                keys.insert(c1);
                keys.insert(c2);
            }
            Normal::SubExists(c, _, _) => {
                keys.insert(c);
            }
            Normal::ExistsSub(_, c, _) => {
                keys.insert(c);
            }
        }
    }
    let mut seeded = 0usize;
    if !keys.is_empty() {
        for (x, members) in sat.s.iter().enumerate().take(retained_rows) {
            for &m in members {
                if keys.contains(&m) {
                    queue.push((x as Concept, m));
                    seeded += 1;
                }
            }
        }
    }

    #[cfg(feature = "rbox")]
    drain(sat, &ix, names, role_box, &mut queue);
    #[cfg(not(feature = "rbox"))]
    drain(sat, &ix, names, &mut queue);
    seeded
}

/// sq-pbz04.2.1 — CR6, the safe-nominal completion rule (Baader–Brandt–Lutz,
/// "Pushing the EL Envelope", IJCAI-05):
///
/// ```text
///   CR6   {a} ∈ S(X) ∩ S(Y),  X ⇝_R Y   ⇒   S(X) := S(X) ∪ S(Y)
/// ```
///
/// where the side-condition `X ⇝_R Y` holds iff Y is reachable from X — or from some nominal
/// `{b}` — via the (role-erased) link graph `∪_r R(r)`, taking reachability reflexively (so
/// `Y = X`, or Y itself a nominal, trivially qualify).
///
/// SOUNDNESS (the load-bearing invariant; the side-condition is exactly what makes the merge
/// valid). The saturation maintains: `D ∈ S(C)` ⟹ `T ⊨ C ⊑ D`, and `(C,D) ∈ R(r)` ⟹
/// `T ⊨ C ⊑ ∃r.D`. In any model, an R-path start being NON-EMPTY forces every set on the
/// path non-empty (each element needs a successor), and a nominal `{b}` is ALWAYS non-empty.
/// So for a firing pair (X, Y): if Y is nominal-rooted, Y is non-empty outright; if Y is
/// reached from X, either X is empty (then `X ⊑ E` holds vacuously for every E) or Y is
/// non-empty. A non-empty Y with `Y ⊑ {a}` is EXACTLY `{a}`; with `X ⊑ {a}` that gives
/// `X ⊆ {a} = Y ⊆ E` for every `E ∈ S(Y)` — so every merged membership is entailed. WITHOUT
/// the side-condition the merge would be UNSOUND: from `X ⊑ {a}` and `Y ⊑ {a}` alone, Y may
/// be empty while X is not, and `X ⊑ E` need not hold (pinned by a negative test).
///
/// COMPLETENESS (honest boundary): this is the classic reachability-based rule — complete for
/// the profile's typical "safe" nominal usage (`hasValue` values, singleton `oneOf` classes)
/// but NOT claimed complete for every EL++ nominal interplay: the ELK line of work (Kazakov &
/// Krötzsch, KR 2012) showed unrestricted nominal interaction needs a stronger calculus.
/// Missing consequences are an incompleteness, never an unsound derivation. Likewise a
/// nominal clash (`⊥ ∈ S({a})`) propagates ⊥ only to concepts CR6 relates to `{a}` — a
/// global-inconsistency verdict (everything entailed) is deliberately out of scope here.
///
/// Runs AFTER the CR1–CR5 worklist drains: one pass over the current saturation, applying
/// every firing merge and queueing new memberships. Returns `true` iff anything was added
/// (the caller then re-runs the worklist and re-checks). Cost on nominal-free input: one
/// `has_nominals` check. With nominals: one scan of the S-sets plus per-candidate BFS over
/// the link graph — fine at the scale nominals occur; revisit if a nominal-heavy corpus shows
/// up.
fn cr6_pass(sat: &mut Saturation, names: &Names, queue: &mut Vec<(Concept, Concept)>) -> bool {
    if !names.has_nominals() {
        return false;
    }
    // Group the concepts whose S-set holds a given nominal: candidates[{a}] = every X with
    // {a} ∈ S(X). Only these can pair in CR6.
    let mut candidates: FxHashMap<Concept, Vec<Concept>> = FxHashMap::default();
    for (x, s) in sat.s.iter().enumerate() {
        for &m in s {
            if names.is_nominal(m) {
                candidates.entry(m).or_default().push(x as Concept);
            }
        }
    }
    // Role-erased successor adjacency of ∪_r R(r), for the ⇝_R reachability side-condition.
    let mut adj: FxHashMap<Concept, FxHashSet<Concept>> = FxHashMap::default();
    for succ_by_x in sat.r_succ.values() {
        for (&x, succs) in succ_by_x {
            adj.entry(x).or_default().extend(succs.iter().copied());
        }
    }
    let bfs = |roots: &[Concept]| -> FxHashSet<Concept> {
        let mut seen: FxHashSet<Concept> = roots.iter().copied().collect();
        let mut stack: Vec<Concept> = roots.to_vec();
        while let Some(c) = stack.pop() {
            if let Some(nexts) = adj.get(&c) {
                for &nx in nexts {
                    if seen.insert(nx) {
                        stack.push(nx);
                    }
                }
            }
        }
        seen
    };
    // Concepts provably non-empty regardless of context: reachable from a nominal root
    // (reflexively — every nominal is its own root).
    let nominal_rooted = bfs(names.nominal_concepts());

    // Fire every merge the current saturation justifies. Deterministic iteration is not
    // required for the RESULT (the least fixpoint is order-independent), only termination:
    // each firing merge strictly grows an S-set.
    let mut changed = false;
    let mut reach_from: FxHashMap<Concept, FxHashSet<Concept>> = FxHashMap::default();
    let mut mates: Vec<Concept> = Vec::new();
    for xs in candidates.values() {
        for &x in xs {
            mates.clear();
            mates.extend(xs.iter().copied().filter(|&y| {
                y != x
                    && (nominal_rooted.contains(&y)
                        || reach_from
                            .entry(x)
                            .or_insert_with(|| bfs(&[x]))
                            .contains(&y))
            }));
            for &y in &mates {
                // S(X) := S(X) ∪ S(Y). Collect first: two rows of `sat.s` cannot be borrowed
                // mutably at once, and Y's row is only read.
                let extra: Vec<Concept> = sat.s[y as usize]
                    .difference(&sat.s[x as usize])
                    .copied()
                    .collect();
                for e in extra {
                    add(&mut sat.s[x as usize], x, e, queue);
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Inserts `e` into `S(x)`, queueing `(x, e)` if it is new. The set is borrowed mutably by the
/// caller (one row of `sat.s`) so CR rules that touch a different row stay borrow-safe.
#[inline]
fn add(set: &mut FxHashSet<Concept>, x: Concept, e: Concept, queue: &mut Vec<(Concept, Concept)>) {
    if set.insert(e) {
        queue.push((x, e));
    }
}

/// Adds the existential link `(x, f) ∈ R(r)` and fires the link-triggered half of CR4/CR5:
/// for the NEW link, every `D ∈ S(f)` with an axiom `∃r.D ⊑ E` yields `E ∈ S(x)`, and a
/// pre-existing `⊥ ∈ S(f)` yields `⊥ ∈ S(x)` (CR5). Also fires CRs3 (sq-8zqwb): a
/// SAME-NOMINAL self-link `({a},{a}) ∈ R(r)` adds `∃r.Self` to `S({a})` — this is the single
/// chokepoint every link insertion (CR3, CRs1, the `rbox` closure, and the `par` apply phase)
/// flows through, so asserted, derived, and role-closed self-links all read off. Returns `true`
/// iff the link was new (so the `rbox` caller can decide whether to close it under CR10/CR11).
fn add_link(
    sat: &mut Saturation,
    r: Role,
    x: Concept,
    f: Concept,
    ix: &AxiomIndex,
    names: &Names,
    queue: &mut Vec<(Concept, Concept)>,
) -> bool {
    let succ = sat.r_succ.entry(r).or_default().entry(x).or_default();
    if !succ.insert(f) {
        return false; // link already present.
    }
    sat.r_pred
        .entry(r)
        .or_default()
        .entry(f)
        .or_default()
        .insert(x);

    // CRs3 (sq-8zqwb): a same-nominal self-link is LOCAL reflexivity — `({a},{a}) ∈
    // R(r)` puts `∃r.Self` into S({a}). Sound because `{a}` is a singleton (see the module doc);
    // the nominal guard is load-bearing (a general `(X,X)` link from `X ⊑ ∃r.X` must NOT fire).
    // On a hasSelf-free ontology `self_concept_of` is `None` and the branch is dead.
    if x == f && names.is_nominal(x) {
        if let Some(sc) = names.self_concept_of(r) {
            add(&mut sat.s[x as usize], x, sc, queue);
        }
    }

    // CR4 for the new link: scan S(f) for fillers D with an `∃r.D ⊑ E` axiom.
    let s_f: Vec<Concept> = sat.s[f as usize].iter().copied().collect();
    for d in s_f {
        if let Some(es) = ix.exists_sub.get(&(r, d)) {
            for &e in es {
                add(&mut sat.s[x as usize], x, e, queue);
            }
        }
        // CR5 for the new link.
        if d == BOTTOM {
            add(&mut sat.s[x as usize], x, BOTTOM, queue);
        }
    }
    true
}

/// Adds the asserted existential link `(x, f) ∈ R(r)` (CR3) and closes it under the RBox role
/// rules to a fixpoint via a link worklist, calling [`add_link`] (which fires CR4/CR5) for every
/// derived link:
///
///   CR10  r ⊑* s          ⇒  (x, f) ∈ R(s)                      for each super-role s of r
///   CR11  r ∘ r2 ⊑ s, (f, z) ∈ R(r2)  ⇒  (x, z) ∈ R(s)         (new link is the FIRST component)
///         r1 ∘ r ⊑ s, (w, x) ∈ R(r1)  ⇒  (w, f) ∈ R(s)         (new link is the SECOND component)
///
/// Newly-derived links are pushed back onto the worklist, so composed links compose again and
/// super-roles of derived roles also propagate — the standard RBox link-saturation. The worklist
/// is bounded by the number of distinct `(role, x, f)` triples (each is stored at most once in
/// `r_succ`), so the fixpoint terminates.
// Each argument is a genuinely distinct saturation input (the mutable state, the
// seed link `(r, x, f)`, three read-only indices, and the shared queue); bundling them into a
// context struct here would obscure the CR3/CR10/CR11 seam without simplifying anything, so we
// take the clippy-documented override for this private RBox helper rather than churn the shape.
#[cfg(feature = "rbox")]
#[allow(clippy::too_many_arguments)]
fn add_link_rbox(
    sat: &mut Saturation,
    r: Role,
    x: Concept,
    f: Concept,
    ix: &AxiomIndex,
    names: &Names,
    role_box: &RoleBox,
    queue: &mut Vec<(Concept, Concept)>,
) {
    // Each work item is a link to STORE (already at its exact role). Seed with the CR3 link.
    let mut links: Vec<(Role, Concept, Concept)> = vec![(r, x, f)];
    while let Some((lr, lx, lf)) = links.pop() {
        if !add_link(sat, lr, lx, lf, ix, names, queue) {
            continue; // link already present — its consequences were already enqueued.
        }
        // CR10: propagate to every strict super-role of `lr` (super_roles includes lr itself).
        for &sup in role_box.super_roles(lr) {
            if sup != lr {
                links.push((sup, lx, lf));
            }
        }
        // CR11 (only when the TBox has any composition axiom — a pure role hierarchy skips
        // every probe). New link as the FIRST component: lr ∘ r2 ⊑ s with (lf, z) ∈ R(r2)
        // ⇒ (lx, z) ∈ R(s); and as the SECOND: r1 ∘ lr ⊑ s with (w, lx) ∈ R(r1) ⇒ (w, lf) ∈ R(s).
        if role_box.has_compositions() {
            for &(r2, s) in role_box.compositions_first(lr) {
                if let Some(z_set) = sat.r_succ.get(&r2).and_then(|m| m.get(&lf)) {
                    let zs: Vec<Concept> = z_set.iter().copied().collect();
                    for z in zs {
                        links.push((s, lx, z));
                    }
                }
            }
            for &(r1, s) in role_box.compositions_second(lr) {
                if let Some(w_set) = sat.r_pred.get(&r1).and_then(|m| m.get(&lx)) {
                    let ws: Vec<Concept> = w_set.iter().copied().collect();
                    for w in ws {
                        links.push((s, w, lf));
                    }
                }
            }
        }
    }
}

/// sq-l2o9e: adds the reflexive link justified by `X ⊑ ∃r.Self` and preserves
/// that stronger local-reflexivity provenance while CR10 lifts it through `r ⊑* s`.
///
/// A generic `(X, X) ∈ R(s)` cannot imply `X ⊑ ∃s.Self`: it may only witness that every member
/// of `X` has some `s`-successor in `X`. Here the seed is specifically justified by the
/// self-concept, so every super-role is locally reflexive on `X`; enqueueing the corresponding
/// minted self-concepts lets ordinary CR1 fire axioms such as `∃s.Self ⊑ D`.
#[cfg(feature = "rbox")]
#[allow(clippy::too_many_arguments)]
fn add_self_link_rbox(
    sat: &mut Saturation,
    r: Role,
    x: Concept,
    ix: &AxiomIndex,
    names: &Names,
    role_box: &RoleBox,
    queue: &mut Vec<(Concept, Concept)>,
) {
    for &sup in role_box.super_roles(r) {
        if let Some(self_concept) = names.self_concept_of(sup) {
            add(&mut sat.s[x as usize], x, self_concept, queue);
        }
    }
    add_link_rbox(sat, r, x, x, ix, names, role_box, queue);
}

/// The classification result: for each NAMED class, the set of NAMED super-classes it is
/// subsumed by (excluding itself, ⊤, and fresh normalization names), plus named-class
/// unsatisfiability and the separate global `⊤ ⊑ ⊥` verdict.
pub struct Classification {
    /// `subsumers[c]` = named concepts strictly subsuming the named concept `c`.
    pub subsumers: FxHashMap<Concept, Vec<Concept>>,
    /// Named concepts `c` with `⊥ ∈ S(c)` — i.e. `c ⊑ owl:Nothing` (unsatisfiable).
    pub unsatisfiable: Vec<Concept>,
    /// sq-26zuf: whether `⊥ ∈ S(⊤)`, i.e. the ontology derives `⊤ ⊑ ⊥`.
    pub thing_unsatisfiable: bool,
}

/// Projects the saturation onto NAMED classes only: drops ⊤/⊥/fresh-name subsumers and
/// reflexive self-subsumption, surfacing the queryable subsumption lattice + the
/// unsatisfiable classes. A class is unsatisfiable iff `⊥ ∈ S(c)`.
pub fn classify(sat: &Saturation, names: &Names) -> Classification {
    let mut subsumers: FxHashMap<Concept, Vec<Concept>> = FxHashMap::default();
    let mut unsatisfiable = Vec::new();
    let thing_unsatisfiable = sat.s[TOP as usize].contains(&BOTTOM);
    for c in 0..sat.s.len() as Concept {
        if names.dict_of(c).is_none() {
            continue; // not a named class (⊤/⊥/fresh).
        }
        let s = &sat.s[c as usize];
        if s.contains(&BOTTOM) {
            unsatisfiable.push(c);
        }
        let mut sup: Vec<Concept> = s
            .iter()
            .copied()
            .filter(|&d| d != c && d != TOP && d != BOTTOM && names.dict_of(d).is_some())
            .collect();
        sup.sort_unstable();
        if !sup.is_empty() {
            subsumers.insert(c, sup);
        }
    }
    Classification {
        subsumers,
        unsatisfiable,
        thing_unsatisfiable,
    }
}

// ---------------------------------------------------------------------------------------------
// sq-wy3i6 (Phase E4): PARALLEL saturation — a deterministic bulk-synchronous
// parallel (BSP) fixpoint over the SAME completion rules as `saturate_inner`.
//
// ## Design (why BSP rounds, not a lock-per-S-set pool)
//
// The single-threaded engine interleaves rule DERIVATION with state MUTATION per queue item.
// The parallel engine splits every worklist drain into rounds:
//
//   1. COMPUTE (parallel, read-only): the current membership frontier — every `(X, D)` pair
//      whose `D ∈ S(X)` was inserted since the previous round — is partitioned across a
//      bounded `std::thread::scope` worker pool. Each worker derives the CR1/CR2/CR3/CRs1
//      and membership-triggered CR4/CR5 firings for its chunk against the ROUND-START
//      snapshot of `Saturation` (shared `&Saturation`; nothing mutates during compute).
//   2. APPLY (sequential): the derived memberships/links are applied IN CHUNK ORDER through
//      the exact single-threaded `add` / `add_link` (/ `add_link_rbox`) machinery, so the
//      link-triggered half of CR4/CR5 — and the whole CR10/CR11 role closure under `rbox` —
//      runs unchanged; newly-inserted memberships form the next round's frontier.
//
// The CR6 nominal pass alternates with the drained worklist exactly as in `saturate_inner`
// (it is a between-fixpoint pass in both engines and stays sequential).
//
// ## Completeness (no rule firing is lost to an interleaving)
//
// The queue-item invariant of the sequential engine is preserved verbatim: a pair `(X, D)`
// is enqueued ONLY after `D` is inserted into `S(X)`, so every frontier membership is
// visible in the snapshot its own round computes against. For any rule instance whose
// premises all eventually hold, order the premise-insertion events; the LAST one is either
// (a) a membership event — its compute round starts after every other premise was inserted,
// so the snapshot contains them all and the instance fires there (for CR2 this is exactly
// the "partner conjunct already present" probe; for membership-triggered CR4/CR5 the link
// premise is already in `r_pred`); or (b) a link event — `add_link` runs in the SEQUENTIAL
// apply phase and scans the LIVE `S(f)` at insertion time, which contains every earlier
// membership (and under `rbox`, `add_link_rbox` closes CR10/CR11 against the live link
// relation exactly as in the sequential engine). Either way the instance fires; no firing
// depends on two premises that each miss the other.
//
// ## Soundness + determinism of the RESULT
//
// Workers only READ the snapshot; since the state grows monotonically, every premise a
// worker observes still holds at apply time, so each applied conclusion is a genuine rule
// firing (duplicates are absorbed by the same set-insert dedup as the sequential engine).
// All rules are monotone and bounded, so the closure is the unique LEAST FIXPOINT — the
// same one `saturate_inner` computes — independent of thread count, chunk boundaries, or
// interleaving. On top of that set-level guarantee, chunk results are applied in
// deterministic (frontier) order, so a given input + thread count replays bit-identically.
//
// `Saturation`, `AxiomIndex` and `Names` are plain owned maps/vecs (no interior
// mutability), hence `Sync`; the scoped borrows need no `unsafe` (the crate forbids it).

/// sq-q0o82 (E4 follow-up): per-phase attribution for one parallel-saturation
/// run — the MEASUREMENT the "should the apply phase be parallelised too?" question is
/// decided on. (`saturate_par`, `add`, `add_link` and `add_link_rbox` below are private-module
/// items, so they are code spans, never intra-doc links: this type is public through the
/// crate-root re-export, so its docs must stay resolvable under the all-features rustdoc gate.)
///
/// The E4 engine parallelises only the COMPUTE phase (membership-triggered rule derivation
/// against the round-start snapshot); the APPLY phase (`add` / `add_link`, and under `rbox`
/// the CR10/CR11 closure inside `add_link_rbox`) stays sequential. Refining apply is only
/// worth its determinism risk if apply actually dominates on a real ontology, so this struct
/// exposes the split instead of guessing.
///
/// # Which fields are deterministic
///
/// `rounds`, `frontier_items`, `derived_members` and `derived_links` are a pure function of
/// the input ontology and are INDEPENDENT of `threads`: chunking changes which worker
/// derives a conclusion, never which conclusions are derived (each frontier item probes the
/// same round-start snapshot in every partition). `tests/par_differential.rs` pins that
/// invariance, so these counters are safe to assert on.
///
/// `compute_nanos` / `apply_nanos` are wall-clock and therefore NOT deterministic and NOT
/// canonical — a contended box inflates both. Use them as a RATIO ([`ParPhaseStats::apply_fraction`]),
/// never as an absolute figure, and never bake one into documentation.
#[cfg(feature = "par")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParPhaseStats {
    /// Bulk-synchronous compute/apply rounds executed (one per drained frontier).
    pub rounds: u64,
    /// Total membership pairs `(X, D)` fed to the compute phase across all rounds.
    pub frontier_items: u64,
    /// Membership conclusions `E ∈ S(X)` emitted by the compute phase (pre-dedup: the apply
    /// phase absorbs duplicates through the same set-insert as the sequential engine).
    pub derived_members: u64,
    /// Link conclusions `(X, F) ∈ R(r)` emitted by the compute phase (pre-dedup).
    pub derived_links: u64,
    /// Wall-clock nanoseconds spent in the PARALLEL compute phase. Non-canonical (see above).
    pub compute_nanos: u64,
    /// Wall-clock nanoseconds spent in the SEQUENTIAL apply phase. Non-canonical (see above).
    pub apply_nanos: u64,
}

#[cfg(feature = "par")]
impl ParPhaseStats {
    /// Share of measured saturation time spent in the SEQUENTIAL apply phase, in `0.0..=1.0`.
    /// This is the decision metric: a fraction near 1 means parallelising compute alone is
    /// Amdahl-bound and an apply-phase refinement is worth its determinism risk; a small
    /// fraction means it is not. Returns `0.0` when neither phase was measured (an empty
    /// ontology derives nothing, so both timers stay at zero).
    ///
    /// The fields are public, so both timers may hold `u64::MAX`; the denominator is summed
    /// in `u128` so the result stays in range instead of panicking (debug) or wrapping to a
    /// division by zero (release).
    pub fn apply_fraction(&self) -> f64 {
        let total = u128::from(self.compute_nanos) + u128::from(self.apply_nanos);
        if total == 0 {
            return 0.0;
        }
        self.apply_nanos as f64 / total as f64
    }
}

/// sq-wy3i6 (E4, `par` + `rbox`): parallel CR1–CR6 + CR10/CR11 saturation. Same
/// least fixpoint as [`saturate`] for every `threads` value — see the module-tail design
/// note. `threads` is the worker-pool BOUND (small frontiers use fewer workers, never more).
/// Also returns the [`ParPhaseStats`] compute/apply attribution (sq-q0o82); the saturation
/// itself is byte-identical with or without reading the stats.
#[cfg(all(feature = "par", feature = "rbox"))]
pub fn saturate_par(
    axioms: &[Normal],
    names: &Names,
    role_box: &RoleBox,
    threads: std::num::NonZeroUsize,
) -> (Saturation, ParPhaseStats) {
    saturate_par_inner(axioms, names, role_box, threads)
}

/// sq-wy3i6 (E4, `par` without `rbox`): parallel CR1–CR6 saturation (roles compared
/// for equality only, exactly like the sequential E1 entry point). Same least fixpoint as
/// [`saturate`] for every `threads` value — see the module-tail design note. Also returns the
/// [`ParPhaseStats`] compute/apply attribution (sq-q0o82).
#[cfg(all(feature = "par", not(feature = "rbox")))]
pub fn saturate_par(
    axioms: &[Normal],
    names: &Names,
    threads: std::num::NonZeroUsize,
) -> (Saturation, ParPhaseStats) {
    saturate_par_inner(axioms, names, threads)
}

/// The BSP round loop shared by both `saturate_par` overloads: drain the frontier in
/// compute/apply rounds, then alternate with `cr6_pass` until neither derives anything —
/// the same alternation structure (and therefore the same fixpoint) as `saturate_inner`.
#[cfg(feature = "par")]
fn saturate_par_inner(
    axioms: &[Normal],
    names: &Names,
    #[cfg(feature = "rbox")] role_box: &RoleBox,
    threads: std::num::NonZeroUsize,
) -> (Saturation, ParPhaseStats) {
    let ix = AxiomIndex::build(axioms);
    let n = names.concept_count();
    let has_self = names.has_self_restrictions();
    let mut sat = Saturation {
        s: vec![FxHashSet::default(); n],
        r_pred: FxHashMap::default(),
        r_succ: FxHashMap::default(),
    };
    // Same seeding (and the same inserted-before-queued invariant) as `saturate_inner`.
    let mut queue: Vec<(Concept, Concept)> = Vec::new();
    for c in 0..n as Concept {
        if sat.s[c as usize].insert(c) {
            queue.push((c, c));
        }
        if sat.s[c as usize].insert(TOP) {
            queue.push((c, TOP));
        }
    }

    // sq-q0o82: per-phase attribution. The counters below are pure bookkeeping
    // (two `Instant` reads per round, negligible beside a round's rule work) and change
    // neither the frontier order nor the applied conclusions — the closure stays the one
    // `saturate_inner` computes, at every thread count.
    let mut stats = ParPhaseStats::default();

    loop {
        while !queue.is_empty() {
            let frontier = std::mem::take(&mut queue);
            stats.rounds += 1;
            stats.frontier_items += frontier.len() as u64;
            // COMPUTE (parallel, read-only against the round-start snapshot).
            let t_compute = std::time::Instant::now();
            let derived = derive_frontier(&sat, &ix, names, has_self, &frontier, threads);
            stats.compute_nanos += elapsed_nanos(t_compute);
            // APPLY (sequential, deterministic chunk order) — reuses the single-threaded
            // machinery so link-triggered CR4/CR5 (+ CR10/CR11 under `rbox`) fire exactly
            // as in `saturate_inner`; new insertions refill `queue` for the next round.
            let t_apply = std::time::Instant::now();
            for chunk in derived {
                stats.derived_members += chunk.members.len() as u64;
                stats.derived_links += chunk.links.len() as u64;
                #[cfg(feature = "rbox")]
                {
                    stats.derived_links += chunk.self_links.len() as u64;
                }
                for (x, e) in chunk.members {
                    add(&mut sat.s[x as usize], x, e, &mut queue);
                }
                for (r, x, f) in chunk.links {
                    #[cfg(feature = "rbox")]
                    add_link_rbox(&mut sat, r, x, f, &ix, names, role_box, &mut queue);
                    #[cfg(not(feature = "rbox"))]
                    add_link(&mut sat, r, x, f, &ix, names, &mut queue);
                }
                #[cfg(feature = "rbox")]
                for (r, x) in chunk.self_links {
                    add_self_link_rbox(
                        &mut sat, r, x, &ix, names, role_box, &mut queue,
                    );
                }
            }
            stats.apply_nanos += elapsed_nanos(t_apply);
        }
        if !cr6_pass(&mut sat, names, &mut queue) {
            break;
        }
    }
    (sat, stats)
}

/// Nanoseconds since `since`, saturated into `u64` (~584 years of headroom — a saturating
/// conversion keeps the counter monotone instead of wrapping on an absurd clock reading).
#[cfg(feature = "par")]
fn elapsed_nanos(since: std::time::Instant) -> u64 {
    u64::try_from(since.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// One worker chunk's derivations, kept separate per chunk so the apply phase preserves
/// deterministic frontier order (a set-level no-op — dedup happens at insert — but it makes
/// a given input + thread count replay bit-identically).
#[cfg(feature = "par")]
#[derive(Default)]
struct Derived {
    /// Membership conclusions `e ∈ S(x)` (from CR1/CR2 and membership-triggered CR4/CR5).
    members: Vec<(Concept, Concept)>,
    /// Link conclusions `(x, f) ∈ R(r)` (from CR3 and CRs1).
    links: Vec<(Role, Concept, Concept)>,
    /// Reflexive links whose provenance is specifically `X ⊑ ∃r.Self`.
    #[cfg(feature = "rbox")]
    self_links: Vec<(Role, Concept)>,
}

/// Partitions `frontier` across at most `threads` scoped workers and returns each chunk's
/// [`Derived`] in chunk order. Small frontiers (< `PAR_MIN_CHUNK` items per would-be
/// worker) run inline — spawning threads for a handful of memberships costs more than the
/// rules themselves, and the result is identical either way (least-fixpoint determinism).
#[cfg(feature = "par")]
fn derive_frontier(
    sat: &Saturation,
    ix: &AxiomIndex,
    names: &Names,
    has_self: bool,
    frontier: &[(Concept, Concept)],
    threads: std::num::NonZeroUsize,
) -> Vec<Derived> {
    /// Minimum frontier items per worker before spawning is worth it.
    const PAR_MIN_CHUNK: usize = 64;
    let workers = threads
        .get()
        .min(frontier.len().div_ceil(PAR_MIN_CHUNK))
        .max(1);
    if workers == 1 {
        return vec![derive_chunk(sat, ix, names, has_self, frontier)];
    }
    let chunk_len = frontier.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let handles: Vec<_> = frontier
            .chunks(chunk_len)
            .map(|items| scope.spawn(move || derive_chunk(sat, ix, names, has_self, items)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("EL parallel-saturation worker panicked"))
            .collect()
    })
}

/// The read-only rule-derivation kernel: mirrors the membership-triggered arm of
/// `saturate_inner`'s worklist body (CR1, CR2, CR3, CRs1, CR4/CR5) but EMITS conclusions
/// instead of applying them. Any drift between this and `saturate_inner` is a
/// completeness/soundness bug — tests/par_differential.rs pins the two engines equal over
/// the whole fixture corpus, and the `el-suite-par` conformance differential pins them
/// equal over the W3C EL cases.
#[cfg(feature = "par")]
fn derive_chunk(
    sat: &Saturation,
    ix: &AxiomIndex,
    names: &Names,
    has_self: bool,
    items: &[(Concept, Concept)],
) -> Derived {
    let mut out = Derived::default();
    for &(x, d) in items {
        // CR1: every `D ⊑ E` axiom concludes E ∈ S(X).
        if let Some(es) = ix.sub.get(&d) {
            for &e in es {
                out.members.push((x, e));
            }
        }
        // CR2: every `D ⊓ C2 ⊑ E` axiom fires if C2 ∈ S(X) in the snapshot. If C2 lands
        // later, ITS frontier item probes with the partner (D) already inserted — the same
        // symmetric argument the sequential worklist relies on.
        if let Some(parts) = ix.and_by_conjunct.get(&d) {
            for &(other, e) in parts {
                if sat.s[x as usize].contains(&other) {
                    out.members.push((x, e));
                }
            }
        }
        // CR3: every `D ⊑ ∃r.F` axiom concludes the link (X, F) ∈ R(r).
        if let Some(links) = ix.exists.get(&d) {
            for &(r, f) in links {
                out.links.push((r, x, f));
            }
        }
        // CRs1: `∃r.Self` entered S(X) ⇒ the reflexive link (X, X) ∈ R(r). (CRs2 is CR1 on
        // the `Sub(self_r, D)` axiom, and CRs3 — the sq-8zqwb nominal-reflexivity converse —
        // is link-triggered, firing inside `add_link` during the SEQUENTIAL apply phase,
        // exactly as in the sequential engine.)
        if has_self {
            if let Some(r) = names.self_role(d) {
                #[cfg(feature = "rbox")]
                out.self_links.push((r, x));
                #[cfg(not(feature = "rbox"))]
                out.links.push((r, x, x));
            }
        }
        // CR4 / CR5, membership-triggered: X is the SUCCESSOR of links in the snapshot.
        // Links inserted later fire their own scan of the live S(X) inside `add_link`.
        for (&r, preds_by_succ) in &sat.r_pred {
            let Some(preds) = preds_by_succ.get(&x) else {
                continue;
            };
            if let Some(es) = ix.exists_sub.get(&(r, d)) {
                for &p in preds {
                    for &e in es {
                        out.members.push((p, e));
                    }
                }
            }
            if d == BOTTOM {
                for &p in preds {
                    out.members.push((p, BOTTOM));
                }
            }
        }
    }
    out
}
