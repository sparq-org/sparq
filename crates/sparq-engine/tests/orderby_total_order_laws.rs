//! ORDER BY's comparator must be a total order over every term mix, including numerics
//! across the i128 tower boundary, ill-formed numeric lexicals, NaN, INF and -0. A cycle
//! (a < b < c < a) makes the sorted output depend on the input order, so every
//! permutation of random triples must sort to the same sequence. The pool holds no two
//! distinct terms with equal values (`1.5` and `"1.5"^^xsd:float` tie, and a stable sort
//! keeps tied terms in input order), so any order difference is a comparator defect.

use sparq_core::Graph;
use sparq_engine::query;

const POOL: &[&str] = &[
    // In-tower decimals whose scale alignment overflows i128, and a beyond-tower one
    // that shares their f64 image.
    "\"1.7014118346046923173168730371588410573\"^^xsd:decimal",
    "\"1.70141183460469231731687303715884105727\"^^xsd:decimal",
    "\"1.70141183460469231731687303715884105728\"^^xsd:decimal",
    "\"170141183460469231731687303715884105727\"^^xsd:integer",
    "\"170141183460469231731687303715884105728\"^^xsd:integer",
    "\"-170141183460469231731687303715884105729\"^^xsd:integer",
    "\"99999999999999999999999999999999999999999\"^^xsd:integer",
    "\"9007199254740993\"^^xsd:integer",
    "\"9007199254740992\"^^xsd:double",
    "2",
    "10",
    "-0.0e0",
    "\"NaN\"^^xsd:double",
    "\"INF\"^^xsd:double",
    "\"-INF\"^^xsd:double",
    "1.5",
    "\"11\u{a0}\"^^xsd:integer",
    "\" 7\"^^xsd:integer",
    "\"abc\"^^xsd:integer",
    "\"1.2.3\"^^xsd:decimal",
    "\"5\"",
    "\"5\"@en",
    "<http://ex/iri>",
    "\"2020-01-01T00:00:00Z\"^^xsd:dateTime",
    "true",
];

fn sorted(values: &[&str]) -> Vec<String> {
    let g = Graph::load_str("", "turtle").unwrap();
    let rows = values.iter().map(|v| format!("({v})")).collect::<Vec<_>>().join(" ");
    let q = format!(
        "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> \
         SELECT ?v WHERE {{ VALUES (?v) {{ {rows} }} }} ORDER BY ?v"
    );
    query(&g, &q)
        .unwrap()
        .rows
        .iter()
        .map(|r| r[0].as_ref().map(|t| t.to_string()).unwrap_or_default())
        .collect()
}

/// A small deterministic generator, so a failure reproduces.
struct Lcg(u64);
impl Lcg {
    fn pick(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n
    }
}

#[test]
fn every_permutation_of_random_triples_sorts_the_same() {
    let mut rng = Lcg(0x5eed);
    let perms = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    for _ in 0..400 {
        let t = [POOL[rng.pick(POOL.len())], POOL[rng.pick(POOL.len())], POOL[rng.pick(POOL.len())]];
        let first = sorted(&t);
        for p in &perms[1..] {
            let got = sorted(&p.map(|i| t[i]));
            assert_eq!(got, first, "input {:?} permuted {p:?}", t);
        }
    }
}

/// A and B fit the tower but their scales cannot align in i128; C is
/// beyond the tower. Exactly, B < C < A, in every input order.
#[test]
fn decimals_across_the_tower_boundary_order_by_exact_value() {
    let (a, b, c) = (POOL[0], POOL[1], POOL[2]);
    let want = sorted(&[b, c, a]);
    assert!(want[0].contains("105727\""), "{want:?}");
    assert!(want[1].contains("105728\""), "{want:?}");
    assert!(want[2].contains("10573\""), "{want:?}");
    for t in [[a, b, c], [a, c, b], [b, a, c], [c, a, b], [c, b, a]] {
        assert_eq!(sorted(&t), want, "{t:?}");
    }
}

/// MIN and MAX over the same three agree with the exact order in every input order.
#[test]
fn min_max_across_the_tower_boundary_use_exact_value() {
    let (a, b, c) = (POOL[0], POOL[1], POOL[2]);
    let g = Graph::load_str("", "turtle").unwrap();
    for t in [[a, b, c], [a, c, b], [b, a, c], [b, c, a], [c, a, b], [c, b, a]] {
        let rows = t.iter().map(|v| format!("({v})")).collect::<Vec<_>>().join(" ");
        let agg = |f: &str| {
            let q = format!(
                "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> \
                 SELECT ({f}(?v) AS ?m) WHERE {{ VALUES (?v) {{ {rows} }} }}"
            );
            query(&g, &q).unwrap().rows[0][0].as_ref().map(|t| t.to_string()).unwrap_or_default()
        };
        assert!(agg("MIN").contains("105727\""), "MIN {t:?}: {}", agg("MIN"));
        assert!(agg("MAX").contains("10573\""), "MAX {t:?}: {}", agg("MAX"));
    }
}

/// The same three as STORED triples (the id-level sort cells) order exactly too.
#[test]
fn stored_decimals_across_the_tower_boundary_order_by_exact_value() {
    let (a, b, c) = (POOL[0], POOL[1], POOL[2]);
    for t in [[a, b, c], [a, c, b], [b, a, c], [b, c, a], [c, a, b], [c, b, a]] {
        let mut ttl = String::from("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
        for (i, v) in t.iter().enumerate() {
            ttl.push_str(&format!("<http://ex/s{i}> <http://ex/v> {v} .\n"));
        }
        let g = Graph::load_str(&ttl, "turtle").unwrap();
        let got: Vec<String> = query(&g, "SELECT ?v WHERE { ?s <http://ex/v> ?v } ORDER BY ?v")
            .unwrap()
            .rows
            .iter()
            .map(|r| r[0].as_ref().map(|t| t.to_string()).unwrap_or_default())
            .collect();
        assert!(got[0].contains("105727\"") && got[1].contains("105728\"") && got[2].contains("10573\""), "{t:?}: {got:?}");
    }
}
