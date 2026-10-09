use super::*;

// (sq-98w7z.1) Used only by the `regex`-gated REGEX/REPLACE tests, so gate the
// helper too — otherwise it is dead code under `--no-default-features` (feature-OFF clippy).
#[cfg(feature = "regex")]
fn names_graph() -> Graph {
    Graph::load_str(
        "@prefix : <http://ex/> .\n\
             :a :name \"Alice\" . :b :name \"bob\" . :c :name \"Carol123\" .\n",
        "turtle",
    )
    .unwrap()
}

fn wide_graph(rows: usize) -> Graph {
    let ttl: String =
        (0..rows).map(|i| format!("<http://ex/s{i}> <http://ex/p> <http://ex/o> .\n")).collect();
    Graph::load_str(&ttl, "turtle").unwrap()
}

// ---- (A) REGEX / REPLACE compile memo ---------------------------------------

/// Result-equivalence differential: over a mixed (pattern, flags) set — valid,
/// invalid pattern, unknown flag, each of i/s/m/x/q — the memoised regex must
/// behave IDENTICALLY to a fresh `build_regex` compile, on both the first
/// (miss) and second (hit) lookup, including the `None` outcome that maps to
/// `Value::Error` per row.
#[cfg(feature = "regex")]
#[test]
fn regex_cache_fresh_equivalence_differential() {
    let cases: &[(&str, &str)] = &[
        ("^[A-Z]", ""),
        ("bob", "i"),
        ("a.c", "s"),
        ("^b", "m"),
        ("a b", "x"),
        ("a.c", "q"),  // literal mode: the dot matches only a literal dot
        ("A.C", "qi"), // q combined with i keeps case-insensitivity
        ("[", ""),     // invalid pattern -> None
        ("a", "z"),    // unknown flag -> None
        ("(unclosed", "i"),
    ];
    let texts = ["Alice", "bob", "BOB", "a.c", "aXc", "abc\nbcd", "a b"];
    for (pat, flags) in cases {
        let fresh = build_regex(pat, flags);
        for round in 0..2 {
            let cached = regex_cache::get(pat, flags);
            assert_eq!(
                cached.is_some(),
                fresh.is_some(),
                "compile outcome diverged for ({pat:?}, {flags:?}) round {round}"
            );
            if let (Some(c), Some(f)) = (&cached, &fresh) {
                for t in &texts {
                    assert_eq!(c.is_match(t), f.is_match(t), "is_match diverged: ({pat:?}, {flags:?}) on {t:?}");
                    assert_eq!(
                        c.replace_all(t, "_").into_owned(),
                        f.replace_all(t, "_").into_owned(),
                        "replace_all diverged: ({pat:?}, {flags:?}) on {t:?}"
                    );
                }
            }
        }
    }
}

/// Compile-once counter (NON-VACUOUS: reverting the memo lookup in
/// `regex_cache::get` makes the first delta 100, one per row): a
/// constant-pattern FILTER over 100 rows compiles once; a SECOND execution
/// reusing the pattern compiles zero times (the key is query-independent);
/// REPLACE adds one compile for its distinct pattern; an INVALID pattern
/// memoises its failure once while still erroring per row (FILTER drops every
/// row, BIND leaves every row with the variable unbound — the pre-memo
/// semantics).
#[cfg(feature = "regex")]
#[test]
fn regex_compiles_once_not_per_row() {
    let ttl: String =
        (0..100).map(|i| format!("<http://ex/s{i}> <http://ex/name> \"name{i}A\" .\n")).collect();
    let g = Graph::load_str(&ttl, "turtle").unwrap();
    let filter_q = "SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(REGEX(?n, \"e9[0-9]A$\")) }";

    let c0 = regex_cache::compile_count();
    assert_eq!(crate::query(&g, filter_q).unwrap().len(), 10); // name90A..name99A
    let c1 = regex_cache::compile_count();
    assert_eq!(c1 - c0, 1, "a constant-pattern FILTER over 100 rows must compile ONCE, not per row");

    assert_eq!(crate::query(&g, filter_q).unwrap().len(), 10);
    assert_eq!(regex_cache::compile_count(), c1, "a second execution reuses the memoised compile");

    assert_eq!(
        crate::query(&g, "SELECT ?r WHERE { ?s <http://ex/name> ?n BIND(REPLACE(?n, \"[0-9]+\", \"#\") AS ?r) }")
            .unwrap()
            .len(),
        100
    );
    assert_eq!(regex_cache::compile_count(), c1 + 1, "REPLACE compiles its distinct pattern once");

    // Invalid pattern: the FAILURE is memoised (one compile attempt), and the
    // per-row semantics stay exactly the pre-memo ones.
    assert_eq!(
        crate::query(&g, "SELECT ?n WHERE { ?s <http://ex/name> ?n FILTER(REGEX(?n, \"[\")) }").unwrap().len(),
        0,
        "invalid pattern: type error per row -> FILTER drops every row"
    );
    assert_eq!(regex_cache::compile_count(), c1 + 2);
    let r = crate::query(&g, "SELECT ?r WHERE { ?s <http://ex/name> ?n BIND(REPLACE(?n, \"[\", \"X\") AS ?r) }")
        .unwrap();
    assert_eq!(r.len(), 100, "invalid pattern under BIND keeps rows");
    assert!(r.rows.iter().all(|row| row[0].is_none()), "…with ?r unbound on every row");
    assert_eq!(regex_cache::compile_count(), c1 + 2, "the memoised failure is reused, not recompiled");
}

/// Overflowing the memo cap stays bounded and correct (the memo clears and
/// keeps compiling — degraded, never wrong).
#[cfg(feature = "regex")]
#[test]
fn regex_cache_cap_overflow_stays_correct() {
    for i in 0..131 {
        let pat = format!("^p{i}$");
        let re = regex_cache::get(&pat, "").expect("valid pattern");
        assert!(re.is_match(&format!("p{i}")));
        assert!(!re.is_match("nope"));
    }
}

/// EXISTS-nested REGEX re-enters the memo on the SAME thread in the middle of
/// the outer FILTER's row loop. The `(pattern, flags)` key is independent of
/// any `Bindings` layout (unlike a column-index thread-local), so the
/// re-entry can only hit the memo correctly: both patterns coexist and each
/// compiles exactly once despite the interleaving.
#[cfg(feature = "regex")]
#[test]
fn regex_cache_exists_reentry() {
    let g = names_graph();
    let c0 = regex_cache::compile_count();
    let q = "SELECT ?n WHERE { ?s <http://ex/name> ?n \
                 FILTER(REGEX(?n, \"^[A-Za-z]+[0-9]+$\") && EXISTS { ?s <http://ex/name> ?n2 FILTER(REGEX(?n2, \"^[A-Z]\")) }) }";
    assert_eq!(crate::query(&g, q).unwrap().len(), 1); // Carol123
    assert_eq!(
        regex_cache::compile_count() - c0,
        2,
        "outer + EXISTS-nested pattern compile once each across the re-entry"
    );
}

// ---- (B) RAND() thread-local PRNG -------------------------------------------

/// RAND() stays non-deterministic per SPARQL: every call yields a fresh
/// `xsd:double` in [0, 1), and two separate executions do not repeat.
#[test]
fn rand_fresh_per_call_in_unit_range() {
    let g = wide_graph(100);
    let q = "SELECT ?r WHERE { ?s <http://ex/p> ?o BIND(RAND() AS ?r) }";
    let r = crate::query(&g, q).unwrap();
    assert_eq!(r.len(), 100);
    let mut seen = std::collections::HashSet::new();
    for row in &r.rows {
        let Some(Term::Literal(l)) = &row[0] else { panic!("RAND() must bind a literal") };
        assert_eq!(l.datatype(), xsd::DOUBLE);
        let v: f64 = l.value().parse().expect("xsd:double lexical");
        assert!((0.0..1.0).contains(&v), "RAND() out of [0,1): {v}");
        seen.insert(l.value().to_string());
    }
    // splitmix64 collides over 100 draws with probability ~2^-46; a constant
    // or repeating stream (the failure mode this guards) collapses `seen`.
    assert!(seen.len() >= 99, "RAND() must draw fresh per call; got {} distinct of 100", seen.len());
    // Distinct executions draw from fresh state too (collision odds ~2^-53).
    let one = |q: &str| crate::query(&g, q).unwrap().rows[0][0].clone();
    assert_ne!(
        one("SELECT ?r WHERE { ?s <http://ex/p> ?o BIND(RAND() AS ?r) } LIMIT 1"),
        one("SELECT ?r WHERE { ?s <http://ex/p> ?o BIND(RAND() AS ?r) } LIMIT 1"),
        "RAND() must not repeat across executions"
    );
}

/// DENY-list guard (the memoisation-policy block in `eval_function_inner`):
/// every non-deterministic builtin — UUID(), STRUUID(), no-arg BNODE() —
/// must yield a FRESH value on every call. A value-keyed memo wired around
/// any of them (they take zero arguments, so a memo collapses ALL calls to
/// one value) fails this immediately: all 50 rows would bind one term.
#[test]
fn nondeterministic_builtins_fresh_per_call_never_memoised() {
    let g = wide_graph(50);
    for (expr, what) in
        [("UUID()", "UUID"), ("STRUUID()", "STRUUID"), ("BNODE()", "no-arg BNODE")]
    {
        let q = format!("SELECT ?v WHERE {{ ?s <http://ex/p> ?o BIND({expr} AS ?v) }}");
        let r = crate::query(&g, &q).unwrap();
        assert_eq!(r.len(), 50);
        let distinct: std::collections::HashSet<String> =
            r.rows.iter().map(|row| row[0].clone().unwrap().to_string()).collect();
        assert_eq!(distinct.len(), 50, "{what} must be fresh per call, never memoised");
    }
}

// ---- (C) NOW() query-constancy (SPARQL 1.1 §17.4.5.1) -----------------------

/// Deterministic guard mechanics (no clock dependence): outermost-wins
/// nesting, worker-guard restore, clear on the OUTERMOST drop only.
#[test]
fn query_now_scope_mechanics() {
    assert_eq!(query_now::snapshot(), None);
    let g1 = query_now::scope();
    let v = query_now::snapshot().expect("outermost scope samples the clock");
    {
        let g2 = query_now::scope(); // nested (sub-select / EXISTS re-entry)
        assert_eq!(query_now::snapshot(), Some(v), "nested scope keeps the outer instant");
        drop(g2);
        assert_eq!(query_now::snapshot(), Some(v), "nested drop must NOT clear the outer instant");
    }
    {
        let w = query_now::worker_install(Some(v + 999));
        assert_eq!(query_now::snapshot(), Some(v + 999));
        drop(w);
        assert_eq!(query_now::snapshot(), Some(v), "worker guard restores the previous instant");
    }
    drop(g1);
    assert_eq!(query_now::snapshot(), None, "outermost drop clears: the next execution re-samples");
}

/// A sleeping extension function forces a REAL ≥1 s clock advance INSIDE one
/// execution between two NOW() evaluations — they must still be equal (before
/// this fix each evaluation re-read the clock, so the seconds differed and
/// this test fails). A separate execution afterwards necessarily lands in a
/// LATER second, proving the pinned instant does not leak across executions.
#[test]
fn now_constant_within_execution_fresh_across_executions() {
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let mut reg = crate::FunctionRegistry::new();
    reg.register("http://test/sleep", |_args: &[Term]| {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        Ok(Term::Literal(Literal::new_typed_literal("true", xsd::BOOLEAN)))
    });
    let q = "SELECT ?a ?b WHERE { ?s ?p ?o BIND(NOW() AS ?a) \
                 BIND(<http://test/sleep>() AS ?x) BIND(NOW() AS ?b) }";
    let r = crate::query_with_functions(&g, q, &reg).unwrap();
    assert_eq!(r.len(), 1);
    let (a, b) = (r.rows[0][0].clone().unwrap(), r.rows[0][1].clone().unwrap());
    assert_eq!(a, b, "NOW() must be constant across a >=1s in-execution clock advance");
    // This execution starts >=1.2s after the first one PINNED its instant, so
    // its whole-second lexical is strictly later — a stale thread-local NOW
    // reused across queries would return `a` here.
    let r2 = crate::query(&g, "SELECT ?n WHERE { ?s ?p ?o BIND(NOW() AS ?n) }").unwrap();
    assert_ne!(r2.rows[0][0].clone().unwrap(), a, "a NEW execution must sample a NEW instant");
}

/// EXISTS re-entry across a REAL clock tick. The EXISTS body sleeps >=1.2s and
/// evaluates `NOW()` inside its own FILTER expression path, so the nested
/// evaluation re-enters expression handling ON THE SAME THREAD while the outer
/// execution's instant is still pinned. `?a` (`NOW()` from BEFORE the EXISTS)
/// must therefore equal `?b` (`NOW()` from AFTER the EXISTS returns, >=1.2s of
/// real time later): a single pinned instant survives the whole
/// EXISTS-inclusive execution. Pre-fix, `?a` and `?b` re-read the clock and
/// land in different whole seconds.
///
/// The EXISTS is CORRELATED on `?s` (a shared BOUND variable), so it runs the
/// per-row `eval_graph_pattern` re-entry rather than the uncorrelated ASK fast
/// path — the branch that actually re-enters expression evaluation for the
/// inner `NOW()`. An outer-only variable is deliberately NOT referenced inside
/// the EXISTS (spargebra does not substitute it into the inner scope).
#[test]
fn now_exists_reentry_same_instant() {
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let mut reg = crate::FunctionRegistry::new();
    reg.register("http://test/sleep", |_args: &[Term]| {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        Ok(Term::Literal(Literal::new_typed_literal("true", xsd::BOOLEAN)))
    });
    let q = "SELECT ?a ?b WHERE { ?s ?p ?o BIND(NOW() AS ?a) \
                 FILTER EXISTS { ?s ?p ?o2 BIND(<http://test/sleep>() AS ?x) FILTER(STR(NOW()) = STR(NOW())) } \
                 BIND(NOW() AS ?b) }";
    let r = crate::query_with_functions(&g, q, &reg).unwrap();
    assert_eq!(r.len(), 1);
    let (a, b) = (r.rows[0][0].clone().unwrap(), r.rows[0][1].clone().unwrap());
    assert_eq!(a, b, "NOW() before and after a >=1.2s EXISTS re-entry must be the SAME pinned instant");
}

/// Parallel-threshold row count: a single NOW() instant across ALL rows AND a
/// zero un-scoped-fallback delta. The fallback counter is what makes this
/// non-vacuous at whole-second clock granularity: without the rayon
/// `worker_install` wiring the workers' evaluations fall back to fresh clock
/// samples and the counter goes hot even when every sample lands in the same
/// second.
// (sq-98w7z.1) Gated on `parallel`: the test's whole point is the rayon
// `worker_install` NOW-snapshot wiring, and it references `PAR_THRESHOLD` — both of which only
// exist under the `parallel` feature. Without it there is no worker path to guard and the
// constant reference would not compile (feature-OFF clippy caught this).
#[cfg(feature = "parallel")]
#[test]
fn now_parallel_rows_single_instant_no_fallback() {
    use std::sync::atomic::Ordering;
    // Just past PAR_THRESHOLD: the FILTER/BIND parallel branches activate on a
    // `>= PAR_THRESHOLD` row count (independent of per-row cost), so a thin
    // over-threshold graph exercises the exact rayon worker paths this test
    // guards while staying cheap on a contended box. `+ 200` keeps a healthy
    // margin so the branch fires deterministically.
    let rows = PAR_THRESHOLD + 200;
    let g = wide_graph(rows);
    let f0 = query_now::UNSCOPED_FALLBACKS.load(Ordering::Relaxed);
    let q = "SELECT ?n WHERE { ?s <http://ex/p> ?o BIND(NOW() AS ?n) FILTER(STR(?n) = STR(NOW())) }";
    let r = crate::query(&g, q).unwrap();
    assert_eq!(r.len(), rows, "every row's FILTER NOW() must equal its BIND NOW()");
    let mut distinct = std::collections::HashSet::new();
    for row in &r.rows {
        distinct.insert(row[0].clone().unwrap().to_string());
    }
    assert_eq!(distinct.len(), 1, "NOW() must be one instant across all rows");
    assert_eq!(
        query_now::UNSCOPED_FALLBACKS.load(Ordering::Relaxed) - f0,
        0,
        "no evaluation path may fall back to an un-pinned clock sample"
    );
}
