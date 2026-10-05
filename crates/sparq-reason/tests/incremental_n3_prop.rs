//! Property tests for incremental N3 maintenance: after EVERY random mutation batch, the
//! incrementally maintained closure must equal a from-scratch batch run of the SAME rules
//! over the current base (`reason_n3_terms` — the correctness oracle). Same discipline as
//! tests/incremental_prop.rs (RDFS) and tests/incremental_owl_prop.rs (OWL 2 RL), with rule
//! sets exercising: exact counting (multi-atom joins, multi-conclusion rules), the
//! recursive-SCC layer (transitive ancestry), input-stratified negation (`?UNSCOPED
//! log:notIncludes` over an input-only predicate — mutations of it must REBUILD), the
//! whitelisted builtins (log:uri both directions, string:scrape, string:encodeForUri,
//! string:concatenation), the rules-level fallback (an unsupported builtin), the sticky
//! data-level fallback (decimal literals reaching string:concatenation), and the base↔layer
//! OWNERSHIP TRANSFER (asserting/retracting a fact the recursive layer also derives — which
//! must stay incremental, never re-materialize; `sq-6tykl.6`).
//!
//! It also asserts the REAL sparq-solid rule sets' qualification: common.n3 + wac.n3 (and
//! each ACP stratum + common.n3) must take the counting fast path.

use rustc_hash::FxHashSet;
use sparq_reason::n3::Term;
use sparq_reason::{reason_n3_terms, MaterializedN3Graph, N3Mode};

/// Deterministic xorshift64* RNG — no dev-dependency needed, reproducible failures.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn iri(s: &str) -> Term {
    Term::Iri(s.into())
}
fn ex(local: &str) -> Term {
    iri(&format!("http://ex/{local}"))
}
fn s_lit(v: &str) -> Term {
    Term::Lit(v.into(), "http://www.w3.org/2001/XMLSchema#string".into(), None)
}
fn b_true() -> Term {
    Term::Lit("true".into(), "http://www.w3.org/2001/XMLSchema#boolean".into(), None)
}

/// Serialize ground facts for the oracle (simple Iri/Lit shapes only — what the generators
/// produce).
fn serialize(facts: &FxHashSet<[Term; 3]>) -> String {
    let mut out = String::new();
    for f in facts {
        for t in f {
            match t {
                Term::Iri(i) => {
                    out.push('<');
                    out.push_str(i);
                    out.push('>');
                }
                Term::Lit(v, dt, None) => {
                    out.push('"');
                    // generators emit no characters needing escapes
                    out.push_str(v);
                    out.push('"');
                    if dt != "http://www.w3.org/2001/XMLSchema#string" {
                        out.push_str("^^<");
                        out.push_str(dt);
                        out.push('>');
                    }
                }
                other => panic!("unexpected term shape in test base: {other:?}"),
            }
            out.push(' ');
        }
        out.push_str(".\n");
    }
    out
}

fn oracle(rules: &str, base: &FxHashSet<[Term; 3]>) -> FxHashSet<[Term; 3]> {
    let src = format!("{rules}\n{}", serialize(base));
    reason_n3_terms(&src, None).expect("oracle parse").facts.into_iter().collect()
}

fn assert_equal(g: &MaterializedN3Graph, rules: &str, base: &FxHashSet<[Term; 3]>, at: &str) {
    let inc: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    let full = oracle(rules, base);
    if inc != full {
        let missing: Vec<_> = full.difference(&inc).take(5).collect();
        let extra: Vec<_> = inc.difference(&full).take(5).collect();
        panic!(
            "closure diverged at {at} (inc {} vs full {}):\n missing: {missing:?}\n extra: {extra:?}",
            inc.len(),
            full.len()
        );
    }
    assert_eq!(g.len(), inc.len(), "len() inconsistent at {at}");
}

/// Counting + recursive layer + guard + every whitelisted builtin, in one rule set.
const RULES: &str = r#"
@prefix :       <http://ex/> .
@prefix log:    <http://www.w3.org/2000/10/swap/log#> .
@prefix string: <http://www.w3.org/2000/10/swap/string#> .

# recursive layer (transitive ancestry; SCC {ancestor})
{ ?x :parent ?p . } => { ?x :ancestor ?p . } .
{ ?x :parent ?p . ?p :ancestor ?a . } => { ?x :ancestor ?a . } .

# counted rule consuming the layer
{ ?x :ancestor ?a . ?a :status :archived . } => { ?x :flagged true . } .

# input-stratified negation (:hidden is never derived)
{ ?x :name ?n . ?UNSCOPED log:notIncludes { ?x :hidden ?h . } . } => { ?x :visible true . } .

# builtin chain: IRI -> text -> percent-encoded -> minted tag IRI
{ ?x :name ?n . ?x log:uri ?xs . ?xs string:encodeForUri ?xe .
  ("urn:tag?v=" ?xe) string:concatenation ?ts . ?t log:uri ?ts . }
=> { ?x :tag ?t . } .

# scrape: first word of the name
{ ?x :name ?n . (?n "^([a-z]+)") string:scrape ?w . } => { ?x :word ?w . } .

# multi-atom same-predicate join + multi-conclusion firing
{ ?x :likes ?y . ?y :likes ?x . } => { ?x :mutual ?y . ?y :mutual ?x . } .

# log:notEqualTo filter
{ ?x :likes ?y . ?y log:notEqualTo :n0 . } => { ?x :likesOther ?y . } .
"#;

struct World {
    nodes: Vec<Term>,
}

impl World {
    fn random_fact(&self, rng: &mut Rng) -> [Term; 3] {
        let a = self.nodes[rng.below(self.nodes.len())].clone();
        let b = self.nodes[rng.below(self.nodes.len())].clone();
        match rng.below(10) {
            0..=2 => [a, ex("parent"), b],
            3..=4 => [a, ex("likes"), b],
            5 => [a, ex("status"), ex("archived")],
            6..=8 => {
                let names = ["alice smith", "bob", "carol jones", "dave&eve", "x1 y2"];
                [a, ex("name"), s_lit(names[rng.below(names.len())])]
            }
            _ => [a, ex("plain"), b], // predicate no rule mentions
        }
    }
}

#[test]
fn counting_with_layer_guard_and_builtins_matches_from_scratch() {
    let mut rng = Rng(0x5EED_2026_0612_1001);
    let world = World { nodes: (0..30).map(|i| ex(&format!("n{i}"))).collect() };

    // Initial base: a parent forest + names + some likes/status.
    let mut base: FxHashSet<[Term; 3]> = FxHashSet::default();
    for _ in 0..120 {
        base.insert(world.random_fact(&mut rng));
    }
    let base_vec: Vec<[Term; 3]> = base.iter().cloned().collect();
    let mut g = MaterializedN3Graph::new(RULES, &base_vec).expect("rules parse");
    assert_eq!(
        g.mode(),
        N3Mode::Counting,
        "rule set must qualify: {:?}",
        g.fallback_reason()
    );
    assert_equal(&g, RULES, &base, "initial");

    let mut guard_batches = 0usize;
    for batch in 0..80 {
        let op = rng.below(100);
        if op < 8 {
            // Guard-predicate mutation (~8%): toggling :hidden must trigger a full rebuild.
            guard_batches += 1;
            let x = world.nodes[rng.below(world.nodes.len())].clone();
            let t = [x, ex("hidden"), b_true()];
            let before = g.full_rebuilds();
            if base.contains(&t) {
                g.delete(std::slice::from_ref(&t));
                base.remove(&t);
            } else {
                g.insert(std::slice::from_ref(&t));
                base.insert(t);
            }
            assert_eq!(
                g.full_rebuilds(),
                before + 1,
                "guard mutation must rebuild exactly once (batch {batch})"
            );
        } else if op < 54 {
            let n = 1 + rng.below(6);
            let delta: Vec<[Term; 3]> = (0..n).map(|_| world.random_fact(&mut rng)).collect();
            g.insert(&delta);
            base.extend(delta);
        } else {
            let current: Vec<[Term; 3]> = base.iter().cloned().collect();
            let n = 1 + rng.below(6);
            let delta: Vec<[Term; 3]> = (0..n)
                .map(|_| {
                    if rng.below(4) == 0 || current.is_empty() {
                        world.random_fact(&mut rng)
                    } else {
                        current[rng.below(current.len())].clone()
                    }
                })
                .collect();
            g.delete(&delta);
            for t in &delta {
                base.remove(t);
            }
        }
        assert_eq!(g.mode(), N3Mode::Counting, "must stay on the fast path (batch {batch})");
        assert_eq!(g.base_len(), base.len(), "base drifted at batch {batch}");
        assert_equal(&g, RULES, &base, &format!("batch {batch}"));
    }
    assert!(guard_batches > 0, "schedule should have exercised the guard fallback");
}

#[test]
fn unsupported_builtin_rules_run_in_fallback_and_stay_correct() {
    const MATH_RULES: &str = r#"
@prefix :     <http://ex/> .
@prefix math: <http://www.w3.org/2000/10/swap/math#> .
{ ?x :n ?a . (?a 1) math:sum ?b . } => { ?x :m ?b . } .
"#;
    let mut rng = Rng(0x5EED_2026_0612_1002);
    let int = "http://www.w3.org/2001/XMLSchema#integer";
    let mut base: FxHashSet<[Term; 3]> = FxHashSet::default();
    for i in 0..10 {
        base.insert([ex(&format!("n{i}")), ex("n"), Term::Lit(i.to_string(), int.into(), None)]);
    }
    let base_vec: Vec<[Term; 3]> = base.iter().cloned().collect();
    let mut g = MaterializedN3Graph::new(MATH_RULES, &base_vec).expect("parse");
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert!(g.fallback_reason().unwrap().contains("math#sum"), "{:?}", g.fallback_reason());
    assert_equal(&g, MATH_RULES, &base, "initial");
    for batch in 0..8 {
        let t = [
            ex(&format!("n{}", rng.below(14))),
            ex("n"),
            Term::Lit(rng.below(20).to_string(), int.into(), None),
        ];
        let before = g.full_rebuilds();
        if base.contains(&t) {
            g.delete(std::slice::from_ref(&t));
            base.remove(&t);
        } else {
            g.insert(std::slice::from_ref(&t));
            base.insert(t);
        }
        assert!(g.full_rebuilds() > before, "fallback mutations re-materialize");
        assert_equal(&g, MATH_RULES, &base, &format!("batch {batch}"));
    }
}

#[test]
fn decimal_data_reaching_concatenation_falls_back_sticky_and_stays_correct() {
    const CONCAT_RULES: &str = r#"
@prefix :       <http://ex/> .
@prefix string: <http://www.w3.org/2000/10/swap/string#> .
{ ?x :val ?v . ("v:" ?v) string:concatenation ?c . } => { ?x :cstr ?c . } .
"#;
    let dec = "http://www.w3.org/2001/XMLSchema#decimal";
    let mut base: FxHashSet<[Term; 3]> = FxHashSet::default();
    base.insert([ex("a"), ex("val"), s_lit("hello")]);
    let base_vec: Vec<[Term; 3]> = base.iter().cloned().collect();
    let mut g = MaterializedN3Graph::new(CONCAT_RULES, &base_vec).expect("parse");
    assert_eq!(g.mode(), N3Mode::Counting, "{:?}", g.fallback_reason());
    assert_equal(&g, CONCAT_RULES, &base, "initial");

    // A decimal literal flowing into string:concatenation is outside the parity whitelist:
    // the graph must drop to (sticky) engine fallback and stay correct.
    let t = [ex("b"), ex("val"), Term::Lit("1.50".into(), dec.into(), None)];
    g.insert(std::slice::from_ref(&t));
    base.insert(t);
    assert_eq!(g.mode(), N3Mode::Fallback);
    assert!(g.fallback_reason().is_some());
    assert_equal(&g, CONCAT_RULES, &base, "after decimal");

    let t2 = [ex("c"), ex("val"), s_lit("world")];
    g.insert(std::slice::from_ref(&t2));
    base.insert(t2);
    assert_equal(&g, CONCAT_RULES, &base, "sticky fallback insert");
}

/// The real sparq-solid rule sets' qualification matrix: the ?UNSCOPED-migrated WAC rules
/// and the acp-b.n3 stratum are exactly the input-stratified-negation shape and take the
/// counting fast path. acp-a.n3 and acp-c.n3 do NOT qualify — each has a rule with a
/// VARIABLE predicate the analysis cannot statically resolve, so predicate-level
/// stratification is unsound and the analysis must conservatively fall back. In acp-c.n3 a
/// simple-grant rule CONCLUDES `{ ?p ?pred ?r }` (predicate bound from
/// `solidx:allowPred`/`denyPred` data), so the derived-predicate set is not statically known.
/// In acp-a.n3 ([OPUS-4.8] sq-3jtd.5) the CreatorAgent/OwnerAgent candidate rule has a
/// variable-predicate PREMISE `?r ?kind ?w` (`?kind` bound to `solidx:creator`/`owner` from
/// `solidx:provMatcher`), likewise outside the statically-stratifiable whitelist. Both
/// fallbacks are SOUND (the engine path is always correct); they only forgo the counting
/// fast path. This asserts the analysis classifies each stratum correctly.
#[test]
fn sparq_solid_wac_and_acp_rules_qualification_matrix() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../sparq-solid/rules");
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).expect(f);
    let common = read("common.n3");
    for (stratum, expect) in [
        ("wac.n3", N3Mode::Counting),
        ("acp-a.n3", N3Mode::Fallback), // variable PREMISE predicate (?r ?kind ?w), sq-3jtd.5
        ("acp-b.n3", N3Mode::Counting),
        ("acp-c.n3", N3Mode::Fallback), // variable conclusion predicate (?p ?pred ?r)
    ] {
        let src = format!("{common}\n{}", read(stratum));
        let g = MaterializedN3Graph::new(&src, &[]).expect("parse");
        assert_eq!(
            g.mode(),
            expect,
            "{stratum} qualification changed: {:?}",
            g.fallback_reason()
        );
    }
}

/// End-to-end WAC sanity on a small pod: the incremental closure equals the engine, ACL
/// edits maintain incrementally, and an ownAcl edit (a guard predicate) rebuilds.
#[test]
fn wac_small_pod_differential() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../sparq-solid/rules");
    let read = |f: &str| std::fs::read_to_string(format!("{dir}/{f}")).expect(f);
    let rules = format!("{}\n{}", read("common.n3"), read("wac.n3"));

    let solidx = |l: &str| iri(&format!("https://sparq.dev/ns/solidx#{l}"));
    let acl = |l: &str| iri(&format!("http://www.w3.org/ns/auth/acl#{l}"));
    let ty = iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
    let pod = "https://pod.ex/";
    let res = |p: &str| iri(&format!("{pod}{p}"));

    let mut base: FxHashSet<[Term; 3]> = FxHashSet::default();
    // structure: root, /docs/, /docs/a, /docs/b — root has the only ACL
    for r in ["", "docs/", "docs/a", "docs/b"] {
        base.insert([res(r), solidx("isResource"), b_true()]);
    }
    base.insert([res(""), solidx("ownAcl"), res(".acl")]);
    let auth = iri("https://pod.ex/.acl#owner");
    base.insert([auth, ty.clone(), acl("Authorization")]);
    base.insert([iri("https://pod.ex/.acl#owner"), solidx("inDoc"), res(".acl")]);
    base.insert([iri("https://pod.ex/.acl#owner"), acl("agent"), iri("https://alice.ex/#me")]);
    base.insert([iri("https://pod.ex/.acl#owner"), acl("default"), res("")]);
    base.insert([iri("https://pod.ex/.acl#owner"), acl("mode"), acl("Read")]);

    let base_vec: Vec<[Term; 3]> = base.iter().cloned().collect();
    let mut g = MaterializedN3Graph::new(&rules, &base_vec).expect("parse");
    assert_eq!(g.mode(), N3Mode::Counting, "{:?}", g.fallback_reason());
    assert_equal(&g, &rules, &base, "initial pod");

    // Alice can read the inherited resources.
    let auth_read = iri("https://sparq.dev/ns/auth#read");
    assert!(g.contains(&[iri("https://alice.ex/#me"), auth_read.clone(), res("docs/a")]));

    // ACL edit: grant Write too (incremental, no rebuild).
    let before = g.full_rebuilds();
    let t = [iri("https://pod.ex/.acl#owner"), acl("mode"), acl("Write")];
    g.insert(std::slice::from_ref(&t));
    base.insert(t);
    assert_eq!(g.full_rebuilds(), before, "plain ACL edit must stay incremental");
    assert_equal(&g, &rules, &base, "after mode insert");

    // Revoke Read (incremental delete).
    let t = [iri("https://pod.ex/.acl#owner"), acl("mode"), acl("Read")];
    g.delete(std::slice::from_ref(&t));
    base.remove(&t);
    assert_equal(&g, &rules, &base, "after mode delete");
    assert!(!g.contains(&[iri("https://alice.ex/#me"), auth_read, res("docs/a")]));

    // ownAcl is a guard predicate: adding a closer ACL rebuilds (documented fallback).
    let before = g.full_rebuilds();
    let t = [res("docs/"), solidx("ownAcl"), res("docs/.acl")];
    g.insert(std::slice::from_ref(&t));
    base.insert(t);
    assert_eq!(g.full_rebuilds(), before + 1, "ownAcl delta must rebuild");
    assert_equal(&g, &rules, &base, "after ownAcl insert");
}

#[test]
fn delete_of_asserted_layer_derivable_fact_is_an_ownership_transfer() {
    // REGRESSION: a fact both ASSERTED and derivable by a recursive layer is excluded from
    // the layer's derived set while asserted (it seeds the local fixpoint). Deleting its
    // base copy therefore makes it APPEAR in the layer diff during a DELETE round — which
    // the sign-homogeneity check used to debug-panic on (and sticky-fallback in release).
    // It is an ownership transfer, not non-monotonicity: the fact stays in the closure,
    // now owned by the layer, and the graph must stay on the counting fast path.
    let rules = "@prefix : <http://ex/> .\n\
                 { ?x :parent ?y } => { ?x :ancestor ?y } .\n\
                 { ?x :ancestor ?y . ?y :ancestor ?z } => { ?x :ancestor ?z } .\n";
    let base = vec![
        [ex("a"), ex("parent"), ex("b")],
        [ex("b"), ex("parent"), ex("c")],
        [ex("a"), ex("ancestor"), ex("c")], // asserted AND layer-derivable
    ];
    let mut g = MaterializedN3Graph::new(rules, &base).unwrap();
    assert_eq!(g.mode(), N3Mode::Counting);
    let before = g.full_rebuilds();
    g.delete(&[[ex("a"), ex("ancestor"), ex("c")]]);
    assert!(g.contains(&[ex("a"), ex("ancestor"), ex("c")]), "fact stays via the layer");
    assert_eq!(g.mode(), N3Mode::Counting, "no sticky fallback for an ownership transfer");
    assert!(g.fallback_reason().is_none(), "not a data disqualification");
    // sq-6tykl.6: the hand-off is settled by the layer's own local re-derivation — it must
    // NOT cost a full re-materialization.
    assert_eq!(g.full_rebuilds(), before, "ownership transfer must not rebuild");
    // Oracle: closure equals a from-scratch run on the current base.
    let mirror: FxHashSet<[Term; 3]> = base[..2].iter().cloned().collect();
    let src = format!("{rules}\n{}", serialize(&mirror));
    let oracle: FxHashSet<[Term; 3]> =
        reason_n3_terms(&src, None).unwrap().facts.into_iter().collect();
    let got: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    assert_eq!(got, oracle, "closure must equal the from-scratch oracle");
    // And the INSERT direction of the transfer: asserting an already-derived fact.
    let before = g.full_rebuilds();
    g.insert(&[[ex("a"), ex("ancestor"), ex("c")]]);
    assert!(g.contains(&[ex("a"), ex("ancestor"), ex("c")]));
    assert_eq!(g.mode(), N3Mode::Counting);
    assert_eq!(g.full_rebuilds(), before, "the insert direction must not rebuild either");

    // BEHAVIOURAL WITNESS for the insert direction (review round 2). Membership + mode +
    // rebuild count all hold vacuously here: asserting a fact the closure already has adds
    // nothing to `pending`, so this call propagates nothing and the layer keeps its copy
    // alongside the new base copy. That double ownership must be INERT — the only way to see
    // it is to make the two owners disagree. Break the layer's derivation and check the fact
    // survives on its BASE copy alone, then retract that copy and check nothing else keeps it
    // alive. A layer entry that were genuinely stale would show up as the fact outliving its
    // own retraction here.
    g.delete(&[[ex("a"), ex("parent"), ex("b")]]);
    assert!(
        g.contains(&[ex("a"), ex("ancestor"), ex("c")]),
        "the asserted base copy alone must keep it in the closure"
    );
    assert!(
        !g.contains(&[ex("a"), ex("ancestor"), ex("b")]),
        "the derivation through the retracted edge is gone"
    );
    g.delete(&[[ex("a"), ex("ancestor"), ex("c")]]);
    assert!(
        !g.contains(&[ex("a"), ex("ancestor"), ex("c")]),
        "no owner is left — a stale layer entry would wrongly keep it"
    );
    assert_eq!(g.mode(), N3Mode::Counting, "still no fallback");
    assert_eq!(g.full_rebuilds(), before, "and still no re-materialization");
    let mirror: FxHashSet<[Term; 3]> = [base[1].clone()].into_iter().collect();
    let src = format!("{rules}\n{}", serialize(&mirror));
    let oracle: FxHashSet<[Term; 3]> =
        reason_n3_terms(&src, None).unwrap().facts.into_iter().collect();
    let got: FxHashSet<[Term; 3]> = g.closure().into_iter().collect();
    assert_eq!(got, oracle, "closure must equal the from-scratch oracle");
}

/// sq-6tykl.6: randomized differential over a schedule that DELIBERATELY asserts facts the
/// recursive layer also derives, so base↔layer ownership transfers fire in both directions and
/// in batches (several transfers per round, transfers mixed with real deletions, transfers that
/// cascade into a counted rule). The closure must track the from-scratch oracle AND the graph
/// must never leave the incremental path — a transfer is a state hand-off the layer's own local
/// re-derivation settles, not a reason to re-materialize.
///
/// The generator in `counting_with_layer_guard_and_builtins_matches_from_scratch` only ever
/// asserts `:parent`, so it cannot reach this case; this test exists to cover it.
#[test]
fn ownership_transfer_deltas_stay_incremental_and_match_from_scratch() {
    const TRANSFER_RULES: &str = r#"
@prefix : <http://ex/> .

# recursive layer (SCC {ancestor}) — :ancestor is BOTH assertable and layer-derivable
{ ?x :parent ?p . } => { ?x :ancestor ?p . } .
{ ?x :ancestor ?p . ?p :ancestor ?a . } => { ?x :ancestor ?a . } .

# counted rule FEEDING the layer: :parent is a layer premise predicate that is itself
# counted-derived, so a round's layer recompute and its count decrements interleave.
{ ?x :sire ?p . } => { ?x :parent ?p . } .

# counted rule consuming the layer, so a spurious retraction would show up downstream
{ ?x :ancestor ?a . ?a :status :archived . } => { ?x :flagged true . } .
"#;
    let mut rng = Rng(0x0BAD_C0DE_6714_6006);
    // A small world keeps `:ancestor` edges densely re-derivable, so a large share of the
    // asserted `:ancestor` facts are genuine ownership transfers rather than plain base facts.
    let world = World { nodes: (0..7).map(|i| ex(&format!("n{i}"))).collect() };
    let pick = |rng: &mut Rng| world.nodes[rng.below(world.nodes.len())].clone();
    let gen_fact = |rng: &mut Rng| -> [Term; 3] {
        let a = pick(rng);
        let b = pick(rng);
        match rng.below(10) {
            0..=2 => [a, ex("parent"), b],
            3..=7 => [a, ex("ancestor"), b], // asserted AND (usually) layer-derivable
            8 => [a, ex("sire"), b],         // counted-derives a layer premise fact
            _ => [a, ex("status"), ex("archived")],
        }
    };

    let mut base: FxHashSet<[Term; 3]> = FxHashSet::default();
    for _ in 0..25 {
        base.insert(gen_fact(&mut rng));
    }
    let base_vec: Vec<[Term; 3]> = base.iter().cloned().collect();
    let mut g = MaterializedN3Graph::new(TRANSFER_RULES, &base_vec).expect("rules parse");
    assert_eq!(g.mode(), N3Mode::Counting, "must qualify: {:?}", g.fallback_reason());
    assert_equal(&g, TRANSFER_RULES, &base, "initial");
    let rebuilds = g.full_rebuilds();

    let mut transfers = 0usize;
    let mut insert_transfers = 0usize;
    for batch in 0..150 {
        if rng.below(2) == 0 {
            let n = 1 + rng.below(4);
            let delta: Vec<[Term; 3]> = (0..n).map(|_| gen_fact(&mut rng)).collect();
            // Count the INSERT direction of the transfer before it happens: an `:ancestor`
            // fact that is not yet asserted but IS already in the closure. No counted rule
            // concludes `:ancestor`, so the layer is what currently owns it, and asserting it
            // moves that ownership to the base.
            insert_transfers += delta
                .iter()
                .filter(|t| t[1] == ex("ancestor") && !base.contains(*t) && g.contains(t))
                .count();
            g.insert(&delta);
            base.extend(delta);
        } else {
            let current: Vec<[Term; 3]> = base.iter().cloned().collect();
            if current.is_empty() {
                continue;
            }
            let n = 1 + rng.below(4);
            let delta: Vec<[Term; 3]> =
                (0..n).map(|_| current[rng.below(current.len())].clone()).collect();
            let still: Vec<bool> = delta.iter().map(|t| g.contains(t)).collect();
            g.delete(&delta);
            for t in &delta {
                base.remove(t);
            }
            // Count the genuine ownership transfers: an `:ancestor` fact left the base but
            // stayed in the closure. No counted rule concludes `:ancestor`, so the only thing
            // that can still support it is the layer.
            for (t, was) in delta.iter().zip(still) {
                if was && t[1] == ex("ancestor") && !base.contains(t) && g.contains(t) {
                    transfers += 1;
                }
            }
        }
        assert_eq!(g.mode(), N3Mode::Counting, "must stay on the fast path (batch {batch})");
        assert_eq!(
            g.full_rebuilds(),
            rebuilds,
            "ownership transfers must not re-materialize (batch {batch})"
        );
        assert_eq!(g.base_len(), base.len(), "base drifted at batch {batch}");
        assert_equal(&g, TRANSFER_RULES, &base, &format!("batch {batch}"));
    }
    assert!(
        transfers > 0,
        "schedule should have exercised the delete direction of the transfer"
    );
    // Review round 2: state the insert direction's coverage explicitly rather than leaving it
    // incidental. The per-batch `full_rebuilds` assertion above is what makes this direction
    // NON-vacuous — replacing the hand-off branch in `propagate`'s inserting arm with a plain
    // `removed.is_empty()` bail (its pre-`sq-6tykl.6` behaviour) reds this test at batch 0.
    assert!(
        insert_transfers > 0,
        "schedule should have exercised the insert direction of the transfer"
    );
}

// [OPUS-4.8] Regression for reviews 1868 / 1884: a rule with NO plain join atom — an empty `{}`
// premise — must NOT enter the counting profile (which seeds emissions only from plain premise
// atoms, so its conclusion would be silently dropped). It must run in Fallback so the maintained
// closure equals the batch oracle, which DOES derive the constant fact.
#[test]
fn empty_premise_rule_falls_back_and_stays_correct() {
    let rules = r#"@prefix : <http://ex/> .
{ } => { :a :p :b } .
"#;
    let base: Vec<[Term; 3]> = vec![[ex("x"), ex("q"), ex("y")]];
    let g = MaterializedN3Graph::new(rules, &base).unwrap();
    assert_eq!(g.mode(), N3Mode::Fallback, "{{}} rule must disqualify the counting path");
    assert!(g.fallback_reason().is_some(), "must report a disqualification reason");
    assert!(
        g.contains(&[ex("a"), ex("p"), ex("b")]),
        "the constant conclusion of the {{}} rule must be in the closure (1868)"
    );
    let baseset: FxHashSet<[Term; 3]> = base.iter().cloned().collect();
    assert_equal(&g, rules, &baseset, "after empty-premise build");
}

// [OPUS-4.8] Regression for reviews 1868 / 1884: a rule whose premise is ONLY a whitelisted
// builtin (no plain join atom) must also fall back to the batch engine — the counting path would
// never seed it. The builtin-derived constant must still appear in the closure.
#[test]
fn builtin_only_premise_rule_falls_back_and_stays_correct() {
    let rules = r#"@prefix : <http://ex/> .
@prefix string: <http://www.w3.org/2000/10/swap/string#> .
{ ("foo" "bar") string:concatenation ?z } => { :out :is ?z } .
"#;
    let base: Vec<[Term; 3]> = vec![[ex("x"), ex("q"), ex("y")]];
    let g = MaterializedN3Graph::new(rules, &base).unwrap();
    assert_eq!(g.mode(), N3Mode::Fallback, "builtin-only premise must disqualify counting");
    assert!(g.fallback_reason().is_some(), "must report a disqualification reason");
    assert!(
        g.contains(&[ex("out"), ex("is"), s_lit("foobar")]),
        "the builtin-derived conclusion must be in the closure (1868)"
    );
    let baseset: FxHashSet<[Term; 3]> = base.iter().cloned().collect();
    assert_equal(&g, rules, &baseset, "after builtin-only build");
}

// [OPUS-4.8] Regression for review 1868 (Low): when fallback is forced by log:implies-family
// rules-as-data in the base (not by rule analysis), fallback_reason() must report the cause so
// the documented `None ⇔ counting active` contract holds; and it must clear when the data is gone.
#[test]
fn data_rule_fallback_reports_reason_and_clears() {
    // A qualifying counting rule, so any fallback is purely from the implies-as-data trigger.
    let rules = r#"@prefix : <http://ex/> .
{ ?x :p ?y } => { ?x :q ?y } .
"#;
    let base: Vec<[Term; 3]> = vec![[ex("a"), ex("p"), ex("b")]];
    let mut g = MaterializedN3Graph::new(rules, &base).unwrap();
    assert_eq!(g.mode(), N3Mode::Counting);
    assert!(g.fallback_reason().is_none(), "counting active ⇒ no reason");

    // Insert a log:implies triple AS DATA — forces fallback.
    let implies = iri("http://www.w3.org/2000/10/swap/log#implies");
    g.insert(&[[ex("r1"), implies.clone(), ex("r2")]]);
    assert_eq!(g.mode(), N3Mode::Fallback, "implies-as-data forces fallback");
    let reason = g.fallback_reason();
    assert!(reason.is_some(), "data-rule fallback must report a reason (1868 Low)");
    assert!(
        reason.unwrap().contains("implies"),
        "reason should name the implies-as-data cause, got {reason:?}"
    );

    // Remove it — counting resumes and the reason clears.
    g.delete(&[[ex("r1"), implies, ex("r2")]]);
    assert_eq!(g.mode(), N3Mode::Counting, "removing the data rule resumes counting");
    assert!(g.fallback_reason().is_none(), "reason must clear when counting resumes");
}
