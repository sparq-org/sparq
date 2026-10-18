//! Differential guard for the PARALLEL (sharded) dictionary consolidation:
//! the same N-Triples document is loaded through every dict-build path —
//!
//!   1. fully serial streaming load (`load_reader`, the reference),
//!   2. sharded parallel in-memory load (`load_str` in a ≥2-thread pool),
//!   3. the 1-thread in-memory load (the serial-merge fallback path),
//!   4. out-of-core external build, sharded dict (`build_external_opts(.., true)`),
//!   5. out-of-core external build, serial dict (`build_external_opts(.., false)`),
//!
//! and every store must agree exactly: identical term-level triple sets, identical
//! dictionary sizes, and identical results for the whole qlever-synthetic bench query
//! suite. Term-ID ASSIGNMENT may differ between paths (the sharded dict groups ids by
//! hash shard); results must not.

use sparq_core::Graph;
use sparq_engine::query;

/// Deterministic synthetic data: the bench generator's social-graph shape (so the bench
/// queries return non-trivial results) plus the term-kind edge cases (language tags,
/// plain/typed/escaped literals, blank nodes, duplicate triples, a second namespace).
fn synthetic_nt(n: u32) -> String {
    let n_cities = (n / 10).max(1);
    let mut seed = 0x9E3779B97F4A7C15u64;
    let mut rng = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as u32
    };
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!(
            "<http://ex/n{i}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person> .\n"
        ));
        s.push_str(&format!("<http://ex/n{i}> <http://ex/name> \"name{i}\" .\n"));
        s.push_str(&format!(
            "<http://ex/n{i}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            20 + i % 80
        ));
        s.push_str(&format!("<http://ex/n{i}> <http://ex/city> <http://ex/c{}> .\n", i % n_cities));
        for _ in 0..4 {
            s.push_str(&format!("<http://ex/n{i}> <http://ex/follows> <http://ex/n{}> .\n", rng() % n));
        }
    }
    // Term-kind edge cases + an exact duplicate line (must dedup identically).
    s.push_str("<http://ex/n0> <http://ex/name> \"name0\" .\n");
    s.push_str("<http://ex/n0> <http://other.org/p> \"a \\\"q\\\" b\\nc \\\\ d \\u00e9\" .\n");
    s.push_str("<http://ex/n0> <http://ex/label> \"caf\\u00e9\"@fr .\n");
    s.push_str("<http://ex/n0> <http://ex/v> \"1.5\"^^<http://www.w3.org/2001/XMLSchema#decimal> .\n");
    s.push_str("_:b0 <http://ex/follows> <http://ex/n0> .\n");
    s.push_str("_:b0 <http://ex/name> \"blank zero\" .\n");
    s
}

/// The real bench suite (bench/qlever-synthetic/queries/*.rq), read from the repo.
fn bench_queries() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/qlever-synthetic/queries");
    let mut qs: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("bench query dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "rq").unwrap_or(false))
        .map(|p| {
            (p.file_stem().unwrap().to_string_lossy().into_owned(), std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    qs.sort();
    assert!(!qs.is_empty(), "bench query suite must not be empty");
    qs
}

/// Canonical, id-independent form of a query result: rows rendered to term strings,
/// sorted (SELECT order is not asserted by these bench queries).
fn canon_rows(g: &Graph, sparql: &str) -> Vec<String> {
    let r = query(g, sparql).expect("query");
    let mut rows: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|t| t.as_ref().map(|t| t.to_string()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();
    rows.sort();
    rows
}

/// Canonical term-level dump of the whole store.
fn canon_triples(g: &Graph) -> Vec<String> {
    let mut rows = canon_rows(g, "SELECT * WHERE { ?s ?p ?o }");
    rows.sort();
    rows
}

fn pool(n: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new().num_threads(n).build().expect("rayon pool")
}

#[test]
fn all_dict_build_paths_agree_on_bench_suite() {
    let nt = synthetic_nt(3000);

    // 1. Serial streaming reference.
    let serial = Graph::load_reader(nt.as_bytes(), "ntriples").expect("serial load");
    // 2. Sharded parallel in-memory (4 threads → the sharded consolidation path).
    let sharded_mem = pool(4).install(|| Graph::load_str(&nt, "ntriples")).expect("sharded load");
    // 3. 1-thread in-memory (the serial-merge fallback inside the parallel loader).
    let one_thread = pool(1).install(|| Graph::load_str(&nt, "ntriples")).expect("1-thread load");
    // 4 + 5. External (out-of-core) builds, sharded and serial dict.
    let base = std::env::temp_dir().join(format!("sparq_dictpar_diff_{}", std::process::id()));
    let (ext_sharded_dir, ext_serial_dir) = (base.join("sharded"), base.join("serial"));
    Graph::build_external_opts(nt.as_bytes(), "ntriples", &ext_sharded_dir, 4096, true).expect("ext sharded");
    Graph::build_external_opts(nt.as_bytes(), "ntriples", &ext_serial_dir, 4096, false).expect("ext serial");
    let ext_sharded = Graph::open(&ext_sharded_dir).expect("open ext sharded");
    let ext_serial = Graph::open(&ext_serial_dir).expect("open ext serial");

    let stores: Vec<(&str, &Graph)> = vec![
        ("serial-streaming", &serial),
        ("sharded-in-memory", &sharded_mem),
        ("one-thread-in-memory", &one_thread),
        ("external-sharded", &ext_sharded),
        ("external-serial", &ext_serial),
    ];

    // Identical sizes (same dedup, same distinct-term count) ...
    for (name, g) in &stores[1..] {
        assert_eq!(g.len(), serial.len(), "{name}: triple count differs from serial reference");
        assert_eq!(g.dict().len(), serial.dict().len(), "{name}: dict size differs from serial reference");
    }
    // ... identical term-level content ...
    let reference = canon_triples(&serial);
    for (name, g) in &stores[1..] {
        assert_eq!(canon_triples(g), reference, "{name}: term-level triples differ from serial reference");
    }
    // ... and identical results across the whole bench query suite.
    for (qname, sparql) in bench_queries() {
        let want = canon_rows(&serial, &sparql);
        for (name, g) in &stores[1..] {
            assert_eq!(canon_rows(g, &sparql), want, "{name}: query {qname} differs from serial reference");
        }
    }

    std::fs::remove_dir_all(&base).ok();
}

#[test]
fn sharded_in_memory_dict_supports_lookup_after_load() {
    // The sharded consolidation rebuilds the dict's lookup table (`build_table`) — a
    // constant in a WHERE pattern must still resolve (this is `lookup`, exercised via a
    // query with bound terms), and the count must match the serial path.
    let nt = synthetic_nt(500);
    let sharded = pool(4).install(|| Graph::load_str(&nt, "ntriples")).expect("sharded load");
    let serial = Graph::load_reader(nt.as_bytes(), "ntriples").expect("serial load");
    let q = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s a ex:Person . ?s ex:name \"name42\" }";
    let (a, b) = (canon_rows(&sharded, q), canon_rows(&serial, q));
    assert!(!a.is_empty(), "bound-constant query must match (lookup table present)");
    assert_eq!(a, b);
}
