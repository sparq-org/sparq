//! [FABLE-5] sq-zgbso.3 — the IN-CRATE measurement instrument for the sq-zgbso.1 spike's
//! open question: what fraction of a WAC/ACP-shaped `reason_n3` materialize call is the
//! per-call FACT round-trip (id-level graph -> N3 text -> re-parse -> String-term
//! fixpoint -> re-intern; 3x for the stratified ACP pipeline) that the id-level compiled
//! path removes (design record `research/odrl-n3-compiled-rules.md` §1, items 1-2)?
//!
//! Both paths start from the SAME id-level facts (a `Dict` + `[[Id; 3]]`, exactly the
//! sq-zgbso.4 integration situation) over loader-shaped fixture facts at the sparq-solid
//! fixture scale (6×6×6 tree + 4 docs/leaf):
//!
//! * **text path** (the status quo per `materialize_wac`/`materialize_acp`): serialize
//!   the id facts to N3 text, then `reason_n3` (parse + string-term fixpoint + intern);
//!   ACP re-serializes each stratum's closure for the next.
//! * **id path**: bind the pre-compiled rules into the dictionary and run the compiled
//!   fixpoint straight over the ids (`bind` is INSIDE the timed region — sq-zgbso.4
//!   would pay it per call too).
//!
//! The two closures are asserted SET-EQUAL first (a full-scale differential), then each
//! path is timed best-of-N. **All wall-clock output is NON-canonical** (work-box only,
//! for bead/PR comments) — nothing here feeds docs, tests, or `bench/perf-baseline.json`.
//!
//! Run: `cargo run -p sparq-reason --features compiled-rules --release --example
//! compiled_rules_bench`

#[path = "../tests/common/mod.rs"]
mod fixture;

use fixture::{acp_facts, closure_to_n3, solid_rules, triples_as_strings, wac_facts, Scale};
use sparq_core::dict::{Dict, Id};
use sparq_reason::n3::compiled::{compile, compile_with_cycles, intern_facts, CompiledRuleSet};
use sparq_reason::{reason_n3, reason_n3_with_cycles, NegationCycles};
use std::time::Instant;

const ITERS: usize = 5;

fn best_of<F: FnMut() -> usize>(mut f: F) -> (f64, usize) {
    let mut best = f64::INFINITY;
    let mut n = 0;
    for _ in 0..ITERS {
        let t0 = Instant::now();
        n = f();
        best = best.min(t0.elapsed().as_secs_f64() * 1e3);
    }
    (best, n)
}

fn report(system: &str, text_ms: f64, id_ms: f64, closure: usize) {
    let removed = 100.0 * (text_ms - id_ms) / text_ms;
    println!(
        "{system}: text-path {text_ms:.1} ms | id-path {id_ms:.2} ms | ratio {:.0}x | \
         round-trip fraction removed {removed:.1}% | closure {closure} facts",
        text_ms / id_ms
    );
}

fn main() {
    println!(
        "compiled_rules_bench — NON-CANONICAL work-box timings (best of {ITERS}); \
         loader-shaped fixture facts at the sparq-solid fixture scale.\n"
    );
    let sc = Scale {
        tops: 6,
        mids: 6,
        leaves: 6,
        docs: 4,
    };

    // ---- WAC: one stratum ----------------------------------------------------------
    let facts = wac_facts(&sc);
    let rules_text = format!("{}\n{}", solid_rules("common.n3"), solid_rules("wac.n3"));
    let compiled = compile(&rules_text).expect("compile WAC rules");

    // Both paths start from id-level facts (the .4 integration shape).
    let mut dict = Dict::new();
    let fact_ids = intern_facts(&mut dict, &facts).expect("intern WAC facts");
    println!(
        "WAC input: {} facts, {} compiled rules",
        fact_ids.len(),
        compiled.n_rules()
    );

    // Differential FIRST (full-scale): the two closures must be set-equal.
    let text_set = {
        let txt = closure_to_n3(&dict, &fact_ids);
        let mut d = Dict::new();
        let ids = reason_n3(&mut d, &format!("{txt}\n{rules_text}")).expect("reason_n3 WAC");
        triples_as_strings(&d, &ids)
    };
    let id_set = {
        let mut d2 = Dict::new();
        let ids2 = intern_facts(&mut d2, &facts).expect("intern");
        let out = compiled.bind(&mut d2).eval(&mut d2, &ids2);
        triples_as_strings(&d2, &out)
    };
    fixture::assert_set_equal(&text_set, &id_set, "WAC full-scale differential");
    println!(
        "WAC differential: closures identical ({} facts)\n",
        id_set.len()
    );

    let (text_ms, n1) = best_of(|| {
        let txt = closure_to_n3(&dict, &fact_ids); // item 1a: ids -> N3 text
        let mut d = Dict::new();
        reason_n3(&mut d, &format!("{txt}\n{rules_text}"))
            .expect("reason_n3")
            .len()
    });
    let (id_ms, n2) = best_of(|| {
        let mut d = dict.clone();
        compiled.bind(&mut d).eval(&mut d, &fact_ids).len()
    });
    assert_eq!(n1, n2, "closure sizes must agree");
    report("WAC (1 stratum)", text_ms, id_ms, n2);

    // ---- ACP: three strata ---------------------------------------------------------
    let facts = acp_facts(&sc);
    let common_rules = solid_rules("common.n3");
    let (a, b, c) = (
        solid_rules("acp-a.n3"),
        solid_rules("acp-b.n3"),
        solid_rules("acp-c.n3"),
    );
    let ra = compile(&format!("{common_rules}\n{a}")).expect("compile acp-a");
    let rb = compile(&b).expect("compile acp-b");
    // acp-c.n3 concludes a variable predicate next to store-scoped negation, so it cannot be
    // stratified yet; sparq-solid runs it single-pass by explicit opt-in, and so does this bench.
    let rc = compile_with_cycles(&c, NegationCycles::SinglePass).expect("compile acp-c");

    let mut dict = Dict::new();
    let fact_ids = intern_facts(&mut dict, &facts).expect("intern ACP facts");
    println!(
        "\nACP input: {} facts, {}+{}+{} compiled rules",
        fact_ids.len(),
        ra.n_rules(),
        rb.n_rules(),
        rc.n_rules()
    );

    let text_path = |dict: &Dict, fact_ids: &[[Id; 3]]| -> (Dict, Vec<[Id; 3]>) {
        let txt = closure_to_n3(dict, fact_ids);
        let mut d1 = Dict::new();
        let c1 = reason_n3(&mut d1, &format!("{txt}\n{common_rules}\n{a}")).expect("acp-a");
        let f1 = closure_to_n3(&d1, &c1);
        let mut d2 = Dict::new();
        let c2 = reason_n3(&mut d2, &format!("{f1}\n{b}")).expect("acp-b");
        let f2 = closure_to_n3(&d2, &c2);
        let mut d3 = Dict::new();
        let c3 = reason_n3_with_cycles(&mut d3, &format!("{f2}\n{c}"), NegationCycles::SinglePass)
            .expect("acp-c");
        (d3, c3)
    };
    let id_path = |base: &Dict,
                   fact_ids: &[[Id; 3]],
                   ra: &CompiledRuleSet,
                   rb: &CompiledRuleSet,
                   rc: &CompiledRuleSet|
     -> (Dict, Vec<[Id; 3]>) {
        let mut d = base.clone();
        let s1 = ra.bind(&mut d).eval(&mut d, fact_ids);
        let s2 = rb.bind(&mut d).eval(&mut d, &s1);
        let s3 = rc.bind(&mut d).eval(&mut d, &s2);
        (d, s3)
    };

    let (td, tc) = text_path(&dict, &fact_ids);
    let (cd, cc) = id_path(&dict, &fact_ids, &ra, &rb, &rc);
    fixture::assert_set_equal(
        &triples_as_strings(&td, &tc),
        &triples_as_strings(&cd, &cc),
        "ACP full-scale differential",
    );
    println!(
        "ACP differential: closures identical ({} facts)\n",
        cc.len()
    );

    let (text_ms, n1) = best_of(|| text_path(&dict, &fact_ids).1.len());
    let (id_ms, n2) = best_of(|| id_path(&dict, &fact_ids, &ra, &rb, &rc).1.len());
    assert_eq!(n1, n2, "closure sizes must agree");
    report("ACP (3 strata)", text_ms, id_ms, n2);

    // Rule-compile cost for context (the part sq-zgbso.4 would move to build time).
    let t0 = Instant::now();
    let _ = compile(&format!("{common_rules}\n{a}")).unwrap();
    println!(
        "\ncontext: compiling common+acp-a from text takes {:.2} ms (once per process; \
         sq-zgbso.4 moves it to build time)",
        t0.elapsed().as_secs_f64() * 1e3
    );
    println!("\nNON-CANONICAL: work-box measurements for bead/PR reporting only.");
}
