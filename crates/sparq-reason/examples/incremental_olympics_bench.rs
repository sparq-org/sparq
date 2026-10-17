//! Incremental-maintenance benchmark: insert AND delete deltas vs full re-materialization,
//! at olympics scale (RDFS + OWL 2 RL) and on a synthetic 1k-doc WAC pod (N3 counting fast
//! path). Methodology + results live in bench/inference/incremental-bench.md. Run:
//!
//!     cargo run -p sparq-reason --example incremental_olympics_bench --release [olympics.nt]
//!
//! The dataset defaults to bench/qlever-olympics/olympics.nt (gitignored; fetch with the
//! QLever harness in that directory). RDFS/OWL workloads synthesize a small TBox over the
//! dataset's real vocabulary (the data carries only 765 subClassOf triples of its own) —
//! see `olympics_tbox` for the exact axioms.

use rustc_hash::FxHashSet;
use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use sparq_reason::n3::Term;
use sparq_reason::{
    materialize_owl_rl, materialize_rdfs, MaterializedGraph, MaterializedN3Graph,
    MaterializedOwlGraph, N3Mode, OwlMode,
};
use std::time::Instant;

/// Deterministic xorshift64* (reproducible deltas).
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

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const OLY: &str = "http://wallscope.co.uk/ontology/olympics/";
const DBO: &str = "http://dbpedia.org/ontology/";
const FOAF: &str = "http://xmlns.com/foaf/0.1/";
const SYN: &str = "http://sparq.dev/bench/olympics#";

/// The synthesized TBox over the olympics vocabulary (documented in incremental-bench.md):
/// class chains above the data's real classes, domains/ranges for the heavy predicates,
/// subPropertyOf links, and — for the OWL workloads — inverseOf / equivalentClass, plus a
/// transitive `syn:locatedIn` for the fixpoint variant.
fn olympics_tbox(dict: &mut Dict, owl_features: bool, transitive: bool) -> Vec<[Id; 3]> {
    let mut t = Vec::new();
    let mut iri = |s: &str| dict.intern_iri(s);
    let sc = iri(&format!("{RDFS}subClassOf"));
    let sp = iri(&format!("{RDFS}subPropertyOf"));
    let dom = iri(&format!("{RDFS}domain"));
    let rng = iri(&format!("{RDFS}range"));
    let person = iri(&format!("{FOAF}Person"));
    let agent = iri(&format!("{DBO}Agent"));
    let entity = iri(&format!("{SYN}Entity"));
    let team = iri(&format!("{DBO}SportsTeam"));
    let org = iri(&format!("{DBO}Organisation"));
    let event = iri(&format!("{DBO}SportsEvent"));
    let ev = iri(&format!("{SYN}Event"));
    let result = iri(&format!("{SYN}Result"));
    let games = iri(&format!("{DBO}Olympics"));
    t.extend([
        [person, sc, agent],
        [agent, sc, entity],
        [team, sc, org],
        [org, sc, agent],
        [event, sc, ev],
        [ev, sc, entity],
        [games, sc, ev],
        [result, sc, entity],
    ]);
    let p_athlete = iri(&format!("{OLY}athlete"));
    let p_games = iri(&format!("{OLY}games"));
    let p_event = iri(&format!("{OLY}event"));
    let p_medal = iri(&format!("{OLY}medal"));
    let p_team = iri(&format!("{DBO}team"));
    let p_age = iri(&format!("{FOAF}age"));
    let p_gender = iri(&format!("{FOAF}gender"));
    let medal = iri(&format!("{SYN}Medal"));
    let affiliated = iri(&format!("{SYN}affiliatedWith"));
    let award = iri(&format!("{SYN}award"));
    t.extend([
        [p_athlete, dom, result],
        [p_athlete, rng, person],
        [p_games, dom, result],
        [p_games, rng, games],
        [p_event, dom, result],
        [p_event, rng, event],
        [p_medal, dom, result],
        [p_medal, rng, medal],
        [p_team, dom, person],
        [p_team, rng, team],
        [p_age, dom, person],
        [p_gender, dom, person],
        [p_team, sp, affiliated],
        [p_medal, sp, award],
    ]);
    if owl_features {
        let inv = iri(&format!("{OWL}inverseOf"));
        let eqc = iri(&format!("{OWL}equivalentClass"));
        let has_member = iri(&format!("{SYN}hasMember"));
        let human = iri(&format!("{SYN}Human"));
        t.push([p_team, inv, has_member]);
        t.push([person, eqc, human]);
    }
    if transitive {
        let ty = iri(RDF_TYPE);
        let trans = iri(&format!("{OWL}TransitiveProperty"));
        let located = iri(&format!("{SYN}locatedIn"));
        t.push([located, ty, trans]);
    }
    t
}

/// Synthetic chain facts for the transitive property (venue -> city -> region -> country ->
/// continent over 50 cities; the transitive subgraph stays small relative to the dataset,
/// which is the realistic shape).
fn located_chain(dict: &mut Dict) -> Vec<[Id; 3]> {
    let located = dict.intern_iri(&format!("{SYN}locatedIn"));
    let mut out = Vec::new();
    for c in 0..50 {
        let venue = dict.intern_iri(&format!("{SYN}venue{c}"));
        let city = dict.intern_iri(&format!("{SYN}city{c}"));
        let region = dict.intern_iri(&format!("{SYN}region{}", c % 10));
        let country = dict.intern_iri(&format!("{SYN}country{}", c % 5));
        let continent = dict.intern_iri(&format!("{SYN}continent{}", c % 3));
        out.extend([
            [venue, located, city],
            [city, located, region],
            [region, located, country],
            [country, located, continent],
        ]);
    }
    out
}

/// Fresh, never-seen ABox triples shaped like the dataset's own (new athletes with type,
/// age, team, and a result linking them).
fn fresh_inserts(dict: &mut Dict, n: usize, tag: &str) -> Vec<[Id; 3]> {
    let ty = dict.intern_iri(RDF_TYPE);
    let person = dict.intern_iri(&format!("{FOAF}Person"));
    let p_team = dict.intern_iri(&format!("{DBO}team"));
    let p_age = dict.intern_iri(&format!("{FOAF}age"));
    let p_athlete = dict.intern_iri(&format!("{OLY}athlete"));
    let mut out = Vec::with_capacity(n);
    let mut i = 0usize;
    while out.len() < n {
        let a = dict.intern_iri(&format!("{SYN}newAthlete_{tag}_{i}"));
        let r = dict.intern_iri(&format!("{SYN}newResult_{tag}_{i}"));
        let team = dict.intern_iri(&format!("{SYN}newTeam_{tag}_{}", i % 7));
        let age = dict.intern_lit(
            &format!("{}", 18 + i % 30),
            "http://www.w3.org/2001/XMLSchema#integer",
            None,
        );
        for f in [[a, ty, person], [a, p_team, team], [a, p_age, age], [r, p_athlete, a]] {
            if out.len() < n {
                out.push(f);
            }
        }
        i += 1;
    }
    out
}

// sq-x58ow (parent sq-6tykl.4): sample ONLY triples whose deletion stays on the
// pure-ABox incremental path. `is_rebuild_trigger` MUST mirror
// `MaterializedOwlGraph::triggers_rebuild` — not just the six TBox *predicates* but also
// axiom-typing triples (`p == rdf:type` && an axiom-class object like
// `owl:TransitiveProperty`) and any triple mentioning an occurrence-guarded id
// (`owl:Thing`/`owl:Nothing`/the XSD numeric tower). Excluding only the six TBox predicates
// (the prior behaviour) let the fixed-seed sampler draw the lone
// `[syn:locatedIn, rdf:type, owl:TransitiveProperty]` axiom triple on the fixpoint workload,
// which legitimately takes the rebuild path and trips the "ABox deltas must not rebuild"
// assert — forcing the bench to pin awkward draw-safe tier sizes. With the faithful guard the
// suite can use round tier sizes at any scale.
fn sample_deletes(
    base: &[[Id; 3]],
    rng: &mut Rng,
    n: usize,
    is_rebuild_trigger: &dyn Fn(&[Id; 3]) -> bool,
) -> Vec<[Id; 3]> {
    let mut out = Vec::with_capacity(n);
    let mut seen = rustc_hash::FxHashSet::default();
    while out.len() < n {
        let t = base[rng.below(base.len())];
        if is_rebuild_trigger(&t) {
            continue; // ABox deltas only: TBox / axiom-typing / guarded deltas rebuild.
        }
        if seen.insert(t) {
            out.push(t);
        }
    }
    out
}

fn bench_rdfs(base: &[[Id; 3]], dict: &mut Dict, is_rebuild_trigger: &dyn Fn(&[Id; 3]) -> bool) {
    println!("\n== RDFS (MaterializedGraph) ==");
    let t = Instant::now();
    let mut g = MaterializedGraph::new(dict, base);
    println!(
        "initial incremental build:       {:8.3}s   closure {} (base {})",
        secs(t),
        g.len(),
        base.len()
    );

    let t = Instant::now();
    let mut full: Vec<[Id; 3]> = base.to_vec();
    materialize_rdfs(dict, &mut full);
    let full_secs = secs(t);
    println!("full materialize_rdfs (v1 path): {full_secs:8.3}s   closure {}", full.len());

    let mut rng = Rng(0xBEEF_0001);
    for &n in &[1usize, 100, 10_000] {
        let ins = fresh_inserts(dict, n, &format!("rdfs{n}"));
        let t = Instant::now();
        g.insert(&ins);
        let ti = secs(t);
        let del = sample_deletes(base, &mut rng, n, is_rebuild_trigger);
        let t = Instant::now();
        g.delete(&del);
        let td = secs(t);
        // restore
        g.delete(&ins);
        g.insert(&del);
        println!(
            "delta {n:>6}: insert {ti:9.6}s ({:>9.0}x vs full)   delete {td:9.6}s ({:>9.0}x vs full)",
            full_secs / ti,
            full_secs / td
        );
    }
    assert_eq!(g.len(), full.iter().collect::<rustc_hash::FxHashSet<_>>().len());
}

fn bench_owl(
    base: &[[Id; 3]],
    dict: &mut Dict,
    is_rebuild_trigger: &dyn Fn(&[Id; 3]) -> bool,
    label: &str,
    expect: OwlMode,
) {
    println!("\n== OWL 2 RL (MaterializedOwlGraph, {label}) ==");
    let t = Instant::now();
    let mut g = MaterializedOwlGraph::new(dict, base);
    println!(
        "initial incremental build:       {:8.3}s   closure {} (base {}, mode {:?})",
        secs(t),
        g.len(),
        base.len(),
        g.mode()
    );
    assert_eq!(g.mode(), expect);

    let t = Instant::now();
    let mut full: Vec<[Id; 3]> = base.to_vec();
    materialize_owl_rl(dict, &mut full);
    let full_secs = secs(t);
    println!("full materialize_owl_rl:         {full_secs:8.3}s   closure {}", full.len());

    let mut rng = Rng(0xBEEF_0002);
    for &n in &[1usize, 100, 10_000] {
        let ins = fresh_inserts(dict, n, &format!("owl{label}{n}"));
        let t = Instant::now();
        g.insert(dict, &ins);
        let ti = secs(t);
        let del = sample_deletes(base, &mut rng, n, is_rebuild_trigger);
        let t = Instant::now();
        g.delete(dict, &del);
        let td = secs(t);
        g.delete(dict, &ins);
        g.insert(dict, &del);
        println!(
            "delta {n:>6}: insert {ti:9.6}s ({:>9.0}x vs full)   delete {td:9.6}s ({:>9.0}x vs full)",
            full_secs / ti,
            full_secs / td
        );
    }
    assert_eq!(g.full_rebuilds(), 0, "ABox deltas must not rebuild");
    assert_eq!(g.len(), full.iter().collect::<rustc_hash::FxHashSet<_>>().len());
}

// ---- N3 / WAC ------------------------------------------------------------------------------

fn iri(s: String) -> Term {
    Term::Iri(s)
}

/// A synthetic 1k-doc WAC pod, shaped like sparq-solid's loader output (solidx structural
/// facts + ACL-document facts): 200 containers x 5 docs under one root; the root ACL grants
/// the owner everything by default; every 10th container has its own ACL (public read via
/// agentClass + a group write grant + an origin-restricted grant).
fn wac_pod() -> Vec<[Term; 3]> {
    let solidx = |l: &str| iri(format!("https://sparq.dev/ns/solidx#{l}"));
    let acl = |l: &str| iri(format!("http://www.w3.org/ns/auth/acl#{l}"));
    let vcard = |l: &str| iri(format!("http://www.w3.org/2006/vcard/ns#{l}"));
    let foaf_agent = iri("http://xmlns.com/foaf/0.1/Agent".into());
    let ty = iri(RDF_TYPE.into());
    let b_true =
        Term::Lit("true".into(), "http://www.w3.org/2001/XMLSchema#boolean".into(), None);
    let pod = "https://pod.ex/";
    let res = |p: &str| iri(format!("{pod}{p}"));
    let alice = iri("https://alice.ex/profile#me".into());

    let mut f: Vec<[Term; 3]> = Vec::new();
    f.push([res(""), solidx("isResource"), b_true.clone()]);
    f.push([res(""), solidx("ownAcl"), res(".acl")]);
    let owner = iri(format!("{pod}.acl#owner"));
    f.push([owner.clone(), ty.clone(), acl("Authorization")]);
    f.push([owner.clone(), solidx("inDoc"), res(".acl")]);
    f.push([owner.clone(), acl("agent"), alice.clone()]);
    f.push([owner.clone(), acl("accessTo"), res("")]);
    f.push([owner.clone(), acl("default"), res("")]);
    for m in ["Read", "Write", "Control"] {
        f.push([owner.clone(), acl("mode"), acl(m)]);
    }
    // one group document
    let group = iri(format!("{pod}groups#team"));
    for k in 0..8 {
        f.push([group.clone(), vcard("hasMember"), iri(format!("https://member{k}.ex/#me"))]);
    }
    // PODN scales the pod (default 200 containers = the 1k-doc fixture shape); used to
    // verify the incremental build scales linearly (the n3_fire anchoring fix).
    let n_containers: usize =
        std::env::var("PODN").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    for c in 0..n_containers {
        let cont = format!("c{c}/");
        f.push([res(&cont), solidx("isResource"), b_true.clone()]);
        f.push([res(&cont), solidx("parent"), res("")]);
        for d in 0..5 {
            f.push([res(&format!("{cont}doc{d}")), solidx("isResource"), b_true.clone()]);
            f.push([res(&format!("{cont}doc{d}")), solidx("parent"), res(&cont)]);
        }
        if c % 10 == 0 {
            let acl_doc = res(&format!("{cont}.acl"));
            f.push([res(&cont), solidx("ownAcl"), acl_doc.clone()]);
            let pub_a = iri(format!("{pod}{cont}.acl#public"));
            f.push([pub_a.clone(), ty.clone(), acl("Authorization")]);
            f.push([pub_a.clone(), solidx("inDoc"), acl_doc.clone()]);
            f.push([pub_a.clone(), acl("agentClass"), foaf_agent.clone()]);
            f.push([pub_a.clone(), acl("accessTo"), res(&cont)]);
            f.push([pub_a.clone(), acl("default"), res(&cont)]);
            f.push([pub_a.clone(), acl("mode"), acl("Read")]);
            let grp_a = iri(format!("{pod}{cont}.acl#team"));
            f.push([grp_a.clone(), ty.clone(), acl("Authorization")]);
            f.push([grp_a.clone(), solidx("inDoc"), acl_doc.clone()]);
            f.push([grp_a.clone(), acl("agentGroup"), group.clone()]);
            f.push([grp_a.clone(), acl("default"), res(&cont)]);
            f.push([grp_a.clone(), acl("mode"), acl("Write")]);
            let app_a = iri(format!("{pod}{cont}.acl#app"));
            f.push([app_a.clone(), ty.clone(), acl("Authorization")]);
            f.push([app_a.clone(), solidx("inDoc"), acl_doc.clone()]);
            f.push([app_a.clone(), acl("agent"), alice.clone()]);
            f.push([app_a.clone(), acl("origin"), iri("https://app.ex".into())]);
            f.push([app_a.clone(), acl("default"), res(&cont)]);
            f.push([app_a.clone(), acl("mode"), acl("Read")]);
        }
    }
    f
}

fn n3_term_src(t: &Term) -> String {
    match t {
        Term::Iri(i) => format!("<{i}>"),
        Term::Lit(v, dt, None) if dt == "http://www.w3.org/2001/XMLSchema#string" => {
            format!("\"{v}\"")
        }
        Term::Lit(v, dt, None) => format!("\"{v}\"^^<{dt}>"),
        other => panic!("unexpected term in pod fixture: {other:?}"),
    }
}

fn bench_n3_wac() {
    println!("\n== N3 / WAC (MaterializedN3Graph, counting fast path) ==");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../sparq-solid/rules");
    let rules = format!(
        "{}\n{}",
        std::fs::read_to_string(format!("{dir}/common.n3")).expect("common.n3"),
        std::fs::read_to_string(format!("{dir}/wac.n3")).expect("wac.n3")
    );
    let pod = wac_pod();
    println!("pod: {} reasoner input facts (1,201 resources, 21 ACL docs)", pod.len());

    // Baseline: the v1 story — a full engine re-run of rules + facts on every ACL change.
    let src: String = {
        let mut s = rules.clone();
        s.push('\n');
        for f in &pod {
            s.push_str(&format!(
                "{} {} {} .\n",
                n3_term_src(&f[0]),
                n3_term_src(&f[1]),
                n3_term_src(&f[2])
            ));
        }
        s
    };
    let t = Instant::now();
    let closure = sparq_reason::reason_n3_terms(&src, None).expect("engine run");
    let full_secs = secs(t);
    println!(
        "full engine re-run (v1 story):   {:8.3}s   closure {}",
        full_secs,
        closure.facts.len()
    );

    let t = Instant::now();
    let mut g = MaterializedN3Graph::new(&rules, &pod).expect("rules parse");
    println!(
        "initial incremental build:       {:8.3}s   closure {} (mode {:?})",
        secs(t),
        g.len(),
        g.mode()
    );
    assert_eq!(g.mode(), N3Mode::Counting, "{:?}", g.fallback_reason());
    assert_eq!(g.len(), closure.facts.iter().collect::<rustc_hash::FxHashSet<_>>().len());

    let acl = |l: &str| iri(format!("http://www.w3.org/ns/auth/acl#{l}"));
    // 1-triple ACL edits (the live-pod hot path): grant + revoke a mode; add + remove an
    // agent.
    let owner = iri("https://pod.ex/.acl#owner".into());
    let edits: Vec<(&str, [Term; 3])> = vec![
        ("grant mode", [owner.clone(), acl("mode"), acl("Append")]),
        ("new agent", [owner.clone(), acl("agent"), iri("https://bob.ex/#me".into())]),
    ];
    for (label, t3) in &edits {
        let t = Instant::now();
        g.insert(std::slice::from_ref(t3));
        let ti = secs(t);
        let t = Instant::now();
        g.delete(std::slice::from_ref(t3));
        let td = secs(t);
        println!(
            "{label:<12} insert {ti:9.6}s ({:>9.0}x vs re-run)   delete {td:9.6}s ({:>9.0}x vs re-run)",
            full_secs / ti,
            full_secs / td
        );
    }
    // Adding a NEW own ACL to a container is an ownAcl (guard-predicate) delta -> the
    // documented rebuild path. Measure it honestly.
    let solidx = |l: &str| iri(format!("https://sparq.dev/ns/solidx#{l}"));
    let t3 = [
        iri("https://pod.ex/c3/".into()),
        solidx("ownAcl"),
        iri("https://pod.ex/c3/.acl".into()),
    ];
    let before = g.full_rebuilds();
    let t = Instant::now();
    g.insert(std::slice::from_ref(&t3));
    let tg = secs(t);
    println!(
        "ownAcl edit (guard pred) rebuild: {tg:8.3}s   (rebuilds {} -> {})",
        before,
        g.full_rebuilds()
    );
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/qlever-olympics/olympics.nt").into()
    });
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            println!("olympics: {path}");
            let t = Instant::now();
            let (mut dict, abox) = Graph::parse_to_triples(&text, "ntriples").expect("parse");
            println!("parsed {} triples in {:.1}s", abox.len(), secs(t));
            drop(text);

            let mut iri = |s: String| dict.intern_iri(&s);
            // The six TBox *predicates* whose mutation always rebuilds.
            let tbox_preds: FxHashSet<Id> = [
                format!("{RDFS}subClassOf"),
                format!("{RDFS}subPropertyOf"),
                format!("{RDFS}domain"),
                format!("{RDFS}range"),
                format!("{OWL}inverseOf"),
                format!("{OWL}equivalentClass"),
            ]
            .into_iter()
            .map(&mut iri)
            .collect();
            // sq-x58ow: the rest of `triggers_rebuild`'s surface — axiom-typing
            // (`p == rdf:type` && an axiom-class object) and occurrence-guarded ids.
            let ty = iri(RDF_TYPE.into());
            let axiom_objs: FxHashSet<Id> = [
                format!("{OWL}SymmetricProperty"),
                format!("{OWL}TransitiveProperty"),
                format!("{OWL}FunctionalProperty"),
                format!("{OWL}InverseFunctionalProperty"),
            ]
            .into_iter()
            .map(&mut iri)
            .collect();
            // Occurrence-guarded ids (owl:Thing/Nothing + the XSD numeric tower). None occur
            // as sampled term ids in this driver's synthetic data — typed literals intern
            // distinctly from their datatype IRI — but the guard mirrors the library exactly
            // so it stays correct if the fixtures ever grow such a triple.
            let special: FxHashSet<Id> = [format!("{OWL}Thing"), format!("{OWL}Nothing")]
                .into_iter()
                .map(&mut iri)
                .collect();
            let is_rebuild_trigger = |t: &[Id; 3]| -> bool {
                tbox_preds.contains(&t[1])
                    || (t[1] == ty && axiom_objs.contains(&t[2]))
                    || t.iter().any(|id| special.contains(id))
            };

            // RDFS: data + synthesized RDFS TBox.
            let mut base = abox.clone();
            base.extend(olympics_tbox(&mut dict, false, false));
            bench_rdfs(&base, &mut dict, &is_rebuild_trigger);

            // OWL monotone: + inverseOf/equivalentClass.
            let mut base = abox.clone();
            base.extend(olympics_tbox(&mut dict, true, false));
            bench_owl(&base, &mut dict, &is_rebuild_trigger, "mono", OwlMode::CountingMono);

            // OWL fixpoint: + a transitive locatedIn over a synthetic 200-edge chain.
            let mut base = abox;
            base.extend(olympics_tbox(&mut dict, true, true));
            base.extend(located_chain(&mut dict));
            bench_owl(&base, &mut dict, &is_rebuild_trigger, "fixpoint", OwlMode::CountingFixpoint);
        }
        Err(e) => {
            println!("olympics dataset not found ({path}: {e}) — skipping RDFS/OWL workloads");
        }
    }
    bench_n3_wac();
}
