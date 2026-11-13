//! Exercise SPARQL-RL syntax, preparation and bounded inference with arbitrary UTF-8.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);
    let base = "http://example.org/rules.srl";
    if sparq_shacl::sparql_rl::parse(&source, base).is_ok() && source.len() <= 16 * 1024 {
        let _ = sparq_shacl::sparql_rl::check_well_formed(&source, base);
        let _ = sparq_shacl::sparql_rl::stratify(&source, base);
        let limits = sparq_shacl::sparql_rl::Limits {
            max_rules: 16,
            max_dependency_checks: 1024,
            max_iterations: 8,
            max_queries: 64,
            max_rows: 128,
            max_inferred_triples: 128,
            max_inferred_bytes: 64 * 1024,
            max_construct_triples: 128,
            max_construct_bytes: 64 * 1024,
            max_query_bytes: 64 * 1024,
            timeout: std::time::Duration::from_millis(100),
        };
        let _ = sparq_shacl::sparql_rl::infer_with_limits(
            &sparq_core::Graph::default(),
            &source,
            base,
            &limits,
        );
    }
});
