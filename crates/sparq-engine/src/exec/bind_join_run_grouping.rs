use super::*;
use oxrdf::NamedNode;

// Thread-local observation keeps parallel tests independent and compiles out of
// production. Small reachability fixtures execute on the calling thread.
std::thread_local! {
    static OBSERVED: std::cell::Cell<(usize, usize, usize)> = const {
        std::cell::Cell::new((0, 0, 0))
    };
}

pub(super) fn observe_strategy(presorted: bool) {
    OBSERVED.with(|state| {
        let (runs, hashed, scans) = state.get();
        state.set((runs + usize::from(presorted), hashed + usize::from(!presorted), scans));
    });
}

pub(super) fn observe_scan() {
    OBSERVED.with(|state| {
        let (runs, hashed, scans) = state.get();
        state.set((runs, hashed, scans + 1));
    });
}

fn take_observed() -> (usize, usize, usize) {
    OBSERVED.with(|state| state.replace((0, 0, 0)))
}

/// `ex:n2` carries TWO ages, so one run must fan out to two matches.
const TTL: &str = "@prefix ex: <http://ex/> .\n\
        ex:n1 ex:age 10 .\n\
        ex:n2 ex:age 20 .\n\
        ex:n2 ex:age 21 .\n\
        ex:n3 ex:age 30 .\n";

fn id(g: &Graph, iri: &str) -> Id {
    g.dict.lookup(&Term::NamedNode(NamedNode::new(iri).unwrap()))
}

/// Runs `bind_join` of `result` (one column `?k`, subject position) with
/// `?k ex:age ?v`, returning the output `(k, v)` pairs in emission order.
fn run(g: &Graph, rows: Vec<Row>, sorted_by: Option<Variable>) -> Vec<(Id, Id)> {
    let k = Variable::new("k").unwrap();
    let v = Variable::new("v").unwrap();
    let result = Bindings { vars: vec![k.clone()], rows, sorted_by };
    let id_pat: IdPattern = [None, Some(id(g, "http://ex/age")), None];
    let pos_vars = [Some(k), None, Some(v)];
    let out = bind_join(g, result, &id_pat, &pos_vars, 0, 0, None);
    assert_eq!(out.vars.len(), 2, "join var + the new object var");
    assert!(out.sorted_by.is_none(), "preserve the existing output metadata contract");
    out.rows.iter().map(|r| (r[0], r[1])).collect()
}

/// The expected bag: every result row paired with every `ex:age` object of its subject.
fn expected(g: &Graph, subjects: &[Id]) -> Vec<(Id, Id)> {
    let ages: Vec<(Id, Id)> = ["http://ex/n1", "http://ex/n2", "http://ex/n2", "http://ex/n3"]
        .iter()
        .zip(["10", "20", "21", "30"])
        .map(|(s, a)| {
            (id(g, s), g.dict.lookup_lit(a, "http://www.w3.org/2001/XMLSchema#integer", None))
        })
        .collect();
    let mut out: Vec<(Id, Id)> =
        subjects.iter().flat_map(|&s| ages.iter().filter(move |(a, _)| *a == s).copied()).collect();
    out.sort_unstable();
    out
}

#[test]
fn scrambled_result_hash_grouping_preserves_the_bag() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    // Scrambled join values, with `n1` DUPLICATED: the duplicate must be emitted twice
    // (one group, two result rows) — this is the multiplicity a bag join owes.
    let subjects = [id(&g, "http://ex/n3"), id(&g, "http://ex/n1"), id(&g, "http://ex/n2"), id(&g, "http://ex/n1")];
    let rows: Vec<Row> = subjects.iter().map(|&s| Row::from_slice(&[s])).collect();
    take_observed();
    let got = run(&g, rows, None);
    assert_eq!(take_observed(), (0, 1, 3));

    assert_eq!(got.len(), 5, "n1 twice, n2 twice (two ages), n3 once: {:?}", got);
    let mut sorted = got.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, expected(&g, &subjects), "output bag must match the per-row expansion");

}

#[test]
fn presorted_result_takes_the_no_sort_path_with_the_same_bag() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    // Verified ascending on `?k`: run grouping must preserve the same bag.
    let mut subjects = [id(&g, "http://ex/n3"), id(&g, "http://ex/n1"), id(&g, "http://ex/n2"), id(&g, "http://ex/n1")];
    subjects.sort_unstable();
    let rows: Vec<Row> = subjects.iter().map(|&s| Row::from_slice(&[s])).collect();
    take_observed();
    let got = run(&g, rows, Some(Variable::new("k").unwrap()));
    assert_eq!(take_observed(), (1, 0, 3));

    let mut sorted = got.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, expected(&g, &subjects));
    assert!(got.windows(2).all(|w| w[0].0 <= w[1].0), "{:?}", got);
}

#[test]
fn stale_sorted_by_uses_hash_grouping_and_scans_each_key_once() {
    let g = Graph::load_str(TTL, "turtle").unwrap();
    let mut keys = [id(&g, "http://ex/n1"), id(&g, "http://ex/n2")];
    keys.sort_unstable();
    // Alternation guarantees stale metadata regardless of dictionary-id assignment.
    // Repeated matches must not be accumulated once per split run by zk tracing.
    let subjects: Vec<Id> = (0..128).map(|i| keys[i % 2]).collect();
    let rows = subjects.iter().map(|&s| Row::from_slice(&[s])).collect();
    take_observed();
    let mut got = run(&g, rows, Some(Variable::new("k").unwrap()));
    assert_eq!(take_observed(), (0, 1, 2));
    got.sort_unstable();
    assert_eq!(got, expected(&g, &subjects));
}

const SORTED_QUERY: &str = "PREFIX ex: <http://ex/> SELECT ?key ?rank ?value WHERE {
        ?key ex:seed ?rank . ?key ex:payload ?value . }";
const HASH_QUERY: &str = "PREFIX ex: <http://ex/> SELECT ?key ?rank ?value WHERE {
        ?key ex:seed ?rank . ?key ex:payload ?value . FILTER(?rank >= 0) }";

fn query_graph() -> Graph {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    for key in 0..1024 {
        ttl.push_str(&format!("ex:k{key} ex:payload {}, {} .\n", 2 * key, 2 * key + 1));
        if key < 32 {
            // Rank order differs from key order; the filter chooses the rank scan.
            ttl.push_str(&format!("ex:k{key} ex:seed {} .\n", 31 - key));
        }
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

fn query_bag(result: crate::QueryResult) -> Vec<String> {
    let mut bag: Vec<String> = result.rows.iter().map(|row| format!("{row:?}")).collect();
    bag.sort_unstable();
    bag
}

#[test]
fn sparql_seed_scan_reaches_verified_runs_and_filter_scan_reaches_hash_fallback() {
    let g = query_graph();
    take_observed();
    let sorted = crate::query(&g, SORTED_QUERY).unwrap();
    assert_eq!(take_observed(), (1, 0, 32), "shared-key seed scan must reach run grouping");
    assert_eq!(sorted.rows.len(), 64, "each seed has two payload matches");
    take_observed();
    let hashed = crate::query(&g, HASH_QUERY).unwrap();
    assert_eq!(take_observed(), (0, 1, 32), "rank-filter seed scan must reach hash grouping");
    assert_eq!(query_bag(sorted), query_bag(hashed));
}

#[test]
fn sparql_join_budget_fails_instead_of_returning_a_partial_bag() {
    let g = query_graph();
    let limit = crate::QueryBudget { max_rows: Some(40), ..crate::QueryBudget::unlimited() };
    for (query, expected_path) in [(SORTED_QUERY, (1, 0)), (HASH_QUERY, (0, 1))] {
        take_observed();
        let err = crate::query_with_budget(&g, query, &limit).unwrap_err();
        assert_eq!(err, "query budget exceeded (max-rows)");
        let (runs, hashed, _) = take_observed();
        assert_eq!((runs, hashed), expected_path, "the seed fits; join fan-out trips the budget");
        assert_eq!(crate::query(&g, query).unwrap().rows.len(), 64, "budget state must not leak");
    }
}

// ---------------------------------------------------------------------------------
// Randomized differential vs a grouping-free reference join.
// ---------------------------------------------------------------------------------

/// Which output the faulty reference drops. Each variant is a way the run grouping
/// could silently lose bag cardinality, and the differential MUST go red against it —
/// otherwise the comparison below proves nothing.
#[derive(Clone, Copy)]
enum Fault {
    /// Honest reference.
    None,
    /// Emit a join value's matches for its FIRST result row only — loses the
    /// multiplicity a duplicated result row owes.
    DropDuplicateRows,
    /// Skip one whole join value — loses a group.
    DropOneGroup,
    /// Keep one match per result row — loses the multi-match fan-out.
    DropExtraMatches,
}

/// Reference join with NEITHER grouping strategy: one full scan of the pattern, then a
/// nested loop over the result rows. Independent of both the old `FxHashMap` grouping
/// and the new sorted-run walk, so agreement with it is real evidence about the bag. It
/// also reaches the store differently — ONE unbound-subject scan, against `bind_join`'s
/// per-value BOUND binary-search ranges — so that probe is cross-checked as well.
/// Rows are `[payload, k, v]`, returned sorted so callers compare MULTISETS rather than
/// emission order.
fn naive_bind_join(g: &Graph, rows: &[Row], age: Id, fault: Fault) -> Vec<[Id; 3]> {
    let pat: IdPattern = [None, Some(age), None];
    let scan = g.store.scan(&pat);
    let triples: Vec<[Id; 3]> = scan.rows.iter().map(|r| scan.to_spo(r)).collect();
    let mut out: Vec<[Id; 3]> = Vec::new();
    let mut emitted_keys: Vec<Id> = Vec::new();
    let mut victim: Option<Id> = None;
    for row in rows {
        let key = row[1];
        let mut objs: Vec<Id> =
            triples.iter().filter(|t| t[0] == key).map(|t| t[2]).collect();
        if objs.is_empty() {
            continue;
        }
        match fault {
            Fault::None => {}
            Fault::DropDuplicateRows => {
                if emitted_keys.contains(&key) {
                    continue;
                }
                emitted_keys.push(key);
            }
            Fault::DropOneGroup => {
                if *victim.get_or_insert(key) == key {
                    continue;
                }
            }
            Fault::DropExtraMatches => objs.truncate(1),
        }
        objs.sort_unstable();
        out.extend(objs.into_iter().map(|v| [row[0], key, v]));
    }
    out.sort_unstable();
    out
}

/// `bind_join` of a two-column result `(?p, ?k)` — the join column is NOT column 0, so
/// the run walk's index arithmetic is exercised too — with `?k ex:age ?v`. Returns the
/// output rows sorted, for multiset comparison.
fn run_multiset(g: &Graph, rows: Vec<Row>, age: Id, sorted: bool) -> Vec<[Id; 3]> {
    let p = Variable::new("p").unwrap();
    let k = Variable::new("k").unwrap();
    let v = Variable::new("v").unwrap();
    let sorted_by = sorted.then(|| k.clone());
    let result = Bindings { vars: vec![p, k.clone()], rows, sorted_by };
    let id_pat: IdPattern = [None, Some(age), None];
    let pos_vars = [Some(k), None, Some(v)];
    let out = bind_join(g, result, &id_pat, &pos_vars, 1, 0, None);
    let mut got: Vec<[Id; 3]> = out.rows.iter().map(|r| [r[0], r[1], r[2]]).collect();
    got.sort_unstable();
    got
}

/// xorshift64 — a deterministic generator, so a failure reproduces from its seed
/// (the workspace has no test-only `rand` dependency, and CI must not flake).
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

/// The result-equivalence obligation for the run-grouping rewrite: over randomized
/// graphs and result bags, `bind_join` must agree as a MULTISET with the grouping-free
/// reference — on the hash fallback branch (`sorted_by: None`) and on the presorted branch
/// (truthfully sorted rows plus matching `sorted_by`) alike.
///
/// The generated domain spans what the finding asks for: empty graphs and empty result
/// bags; subjects with 0 / 1 / 2-3 `ex:age` objects (no match, singleton, multi-match
/// fan-out); a payload column drawn from a tiny pool so IDENTICAL duplicate rows occur
/// and their multiplicity is checked; mostly-singleton high-NDV groups; real subject ids
/// that have no `ex:age` triple; and `NO_ID` join values from rows naming a subject the
/// graph never mentions.
#[test]
fn randomized_differential_vs_naive_reference() {
    // Per-fault detection counts: a control that never fires would make the
    // differential vacuous, so each must be exercised at least once.
    let mut caught = [0usize; 3];
    let mut total_out = 0usize;
    let mut stale_cases = 0usize;
    for seed in 1u64..=64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let n_subj = rng.below(12) as usize; // 0 → an empty graph
        let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
        for s in 0..n_subj {
            // 0 ages → a real id with no match; 2-3 → the multi-match fan-out.
            for a in 0..rng.below(4) {
                ttl.push_str(&format!("ex:n{} ex:age {} .\n", s, 100 * s as u64 + a));
            }
            // Keeps `ex:n{s}` in the dictionary even with zero ages.
            ttl.push_str(&format!("ex:n{} ex:name \"n{}\" .\n", s, s));
        }
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        let age = id(&g, "http://ex/age");

        let n_rows = rng.below(20) as usize; // 0 → an empty result bag
        let pool = (n_subj + 3) as u64; // the last 3 are absent → NO_ID join values
        let mut rows: Vec<Row> = Vec::new();
        for _ in 0..n_rows {
            let s = rng.below(pool);
            let key = id(&g, &format!("http://ex/n{}", s));
            // Payload from a 3-value pool, so identical rows repeat.
            rows.push(Row::from_slice(&[rng.below(3) as Id + 1, key]));
        }

        let want = naive_bind_join(&g, &rows, age, Fault::None);
        total_out += want.len();

        // The hash fallback branch: scrambled rows, no sortedness claimed.
        let got = run_multiset(&g, rows.clone(), age, false);
        assert_eq!(got, want, "hash branch, seed {}", seed);

        // Claimed sortedness over the original random bag must be checked. Stale
        // candidates fall back and still scan each distinct key only once.
        let is_sorted = rows.windows(2).all(|pair| pair[0][1] <= pair[1][1]);
        let distinct = rows.iter().map(|row| row[1]).collect::<std::collections::BTreeSet<_>>().len();
        take_observed();
        assert_eq!(run_multiset(&g, rows.clone(), age, true), want, "claimed-sorted branch, seed {}", seed);
        assert_eq!(take_observed(), (usize::from(is_sorted), usize::from(!is_sorted), distinct));
        stale_cases += usize::from(!is_sorted);

        // The no-sort branch: truthfully sorted rows WITH matching metadata. The
        // reference is row-order-independent, so the same `want` applies.
        let mut asc = rows.clone();
        asc.sort_by_key(|r| r[1]);
        assert_eq!(run_multiset(&g, asc, age, true), want, "presorted branch, seed {}", seed);

        // Controls: a reference that drops a duplicate row, a group, or an extra match
        // must be REJECTED by the very comparison that just passed.
        for (i, fault) in
            [Fault::DropDuplicateRows, Fault::DropOneGroup, Fault::DropExtraMatches]
                .into_iter()
                .enumerate()
        {
            let faulty = naive_bind_join(&g, &rows, age, fault);
            if faulty != want {
                assert_ne!(got, faulty, "differential must reject fault {}, seed {}", i, seed);
                caught[i] += 1;
            }
        }
    }
    assert!(total_out > 0, "the generated corpus produced no output rows at all");
    assert!(stale_cases > 0, "the randomized corpus must exercise stale metadata");
    for (i, n) in caught.iter().enumerate() {
        assert!(*n > 0, "control fault {} never fired — the differential is vacuous", i);
    }
}
