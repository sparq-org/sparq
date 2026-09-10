//! Differential guarantee: query RESULTS are identical with the zk recorder
//! armed (`zk::install`) vs disarmed, across a fuzz slice spanning every plan
//! shape. The recorder must be a pure observer — the result-preserving plan
//! changes it forces (disabled COUNT/LIMIT pushdowns) must not alter outputs.
//!
//! The generator mirrors `sparq-bench`'s differential fuzzer (mixed-datatype
//! column, every category) so the cases stress the same plan surface, but the
//! oracle here is "zk-on == zk-off" rather than "sparq == Oxigraph" (that
//! cross-engine differential already runs in sparq-bench; the zk feature only
//! needs to prove it does not perturb results).
//!
//! [OPUS-4.8] written while Fable 5 unavailable — re-review when Fable returns.

#![cfg(feature = "zk")]

use sparq_engine::{query, zk, QueryResult};
use sparq_core::Graph;

/// Deterministic SplitMix64 (same as the sparq-bench fuzzer — reproducible).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

fn gen_graph(rng: &mut Rng) -> String {
    let n = 3 + rng.below(14) as usize;
    let mut s = String::from(
        "@prefix ex: <http://ex/> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n",
    );
    for i in 0..n {
        let subj = format!("ex:n{i}");
        if rng.chance(3, 4) {
            s.push_str(&format!("{subj} a ex:T .\n"));
        }
        if rng.chance(7, 8) {
            let age = rng.below(120);
            s.push_str(&format!("{subj} ex:age {age} .\n"));
        }
        if rng.chance(3, 4) {
            let v = match rng.below(7) {
                0 => format!("{}", rng.below(120)),
                1 => format!("\"{}\"^^xsd:int", rng.below(120)),
                2 => format!("\"{}.5\"^^xsd:decimal", rng.below(120)),
                3 => format!("\"-{}\"^^xsd:integer", rng.below(60)),
                4 => format!("\"s{}\"", rng.below(5)),
                5 => format!("\"0{}\"^^xsd:integer", 1 + rng.below(9)),
                _ => format!("ex:n{}", rng.below(n as u64)),
            };
            s.push_str(&format!("{subj} ex:val {v} .\n"));
        }
        if rng.chance(3, 4) {
            s.push_str(&format!("{subj} ex:name \"name{i}\" .\n"));
        }
        // ex:p edges (chains / triangles for WCOJ).
        let edges = rng.below(4);
        for _ in 0..edges {
            s.push_str(&format!("{subj} ex:p ex:n{} .\n", rng.below(n as u64)));
        }
    }
    s
}

fn gen_filter(rng: &mut Rng, var: &str) -> String {
    let op = ["<", "<=", ">", ">=", "=", "!="][rng.below(6) as usize];
    let t = rng.below(160) as i64 - 40;
    format!("FILTER({var} {op} {t})")
}

fn gen_query(rng: &mut Rng) -> String {
    let pfx = "PREFIX ex: <http://ex/>\nPREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";
    let cats = [
        "bgp", "filter", "optional", "union", "minus", "limit", "distinct", "order", "ask", "count",
        "graph_default",
    ];
    let cat = cats[rng.below(cats.len() as u64) as usize];
    let body = match cat {
        "bgp" => match rng.below(5) {
            0 => "?s ex:age ?a".to_string(),
            1 => "?s ex:p ?o . ?o ex:age ?a".to_string(),
            2 => "?s ex:age ?a . ?s ex:name ?n".to_string(),
            3 => "?s ex:p ?o . ?o ex:p ?t . ?t ex:p ?s".to_string(),
            _ => "?s ex:p ?o . ?o ex:name ?n . ?s ex:age ?a".to_string(),
        },
        "filter" => {
            let var = if rng.chance(1, 2) { "?a" } else { "?v" };
            let pat = if var == "?a" { "?s ex:age ?a" } else { "?s ex:val ?v" };
            let extra = if rng.chance(1, 2) { " . ?s ex:name ?n" } else { "" };
            format!("{pat}{extra} {}", gen_filter(rng, var))
        }
        "optional" => match rng.below(3) {
            0 => "?s ex:name ?n OPTIONAL { ?s ex:age ?a }".to_string(),
            1 => "?s ex:age ?a OPTIONAL { ?s ex:p ?o }".to_string(),
            _ => "?s ex:name ?n OPTIONAL { ?s ex:age ?a . FILTER(?a > 50) }".to_string(),
        },
        "union" => "{ ?s ex:age ?a } UNION { ?s ex:name ?a }".to_string(),
        "minus" => "?s ex:name ?n MINUS { ?s ex:age ?a }".to_string(),
        "limit" => {
            let k = rng.below(8);
            let off = rng.below(5);
            return format!("{pfx}SELECT * WHERE {{ ?s ex:p ?o . ?o ex:age ?a }} LIMIT {k} OFFSET {off}");
        }
        "distinct" => return format!("{pfx}SELECT DISTINCT ?a WHERE {{ ?s ex:age ?a }}"),
        "order" => {
            let k = 1 + rng.below(10);
            let dir = if rng.chance(1, 2) { "?a" } else { "DESC(?a)" };
            return format!("{pfx}SELECT ?a WHERE {{ ?s ex:age ?a }} ORDER BY {dir} LIMIT {k}");
        }
        "ask" => return format!("{pfx}ASK {{ ?s ex:age ?a {} }}", gen_filter(rng, "?a")),
        "count" => return format!("{pfx}SELECT (COUNT(?a) AS ?c) WHERE {{ ?s ex:age ?a }}"),
        _ => "?s ex:val ?v . ?s ex:name ?n".to_string(),
    };
    format!("{pfx}SELECT * WHERE {{ {body} }}")
}

/// Canonical representation of a result for comparison. For ORDER BY queries
/// the row SEQUENCE is significant, so it is preserved; otherwise rows are
/// compared as a sorted multiset (robust to incidental reordering).
fn canon(r: &QueryResult, ordered: bool) -> Vec<String> {
    let mut rows: Vec<String> = r.rows.iter().map(|row| format!("{row:?}")).collect();
    if !ordered {
        rows.sort();
    }
    rows
}

/// The differential: results must be identical with the recorder armed vs not.
fn assert_same(g: &Graph, q: &str) {
    // ORDER BY makes the row sequence load-bearing — do NOT sort it away, or a
    // zk-on/zk-off ordering regression would be masked.
    let ordered = q.contains("ORDER BY");
    let off = query(g, q);
    let on = {
        let _guard = zk::install();
        let r = query(g, q);
        let _ = zk::take();
        r
    };
    match (off, on) {
        (Ok(a), Ok(b)) => assert_eq!(
            canon(&a, ordered),
            canon(&b, ordered),
            "zk on/off result mismatch for query:\n{q}"
        ),
        (Err(_), Err(_)) => {}
        (a, b) => panic!("zk on/off error-status mismatch for {q}: off={a:?} on={b:?}"),
    }
}

#[test]
fn differential_zk_on_equals_off_10k() {
    // 10k cases across all categories (the brief's fuzz slice). Deterministic:
    // a failure prints the exact seed-derived query + graph.
    let n_cases: u64 = std::env::var("ZK_FUZZ_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    for seed in 0..n_cases {
        let mut rng = Rng::new(seed ^ 0x5a17_c0de_d00d_f00d);
        let turtle = gen_graph(&mut rng);
        let g = match Graph::load_str(&turtle, "turtle") {
            Ok(g) => g,
            Err(_) => continue,
        };
        let q = gen_query(&mut rng);
        assert_same(&g, &q);
    }
}
