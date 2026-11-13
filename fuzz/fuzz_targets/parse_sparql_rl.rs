//! Exercise the opt-in SPARQL-RL parser with arbitrary UTF-8 source.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);
    let _ = sparq_shacl::sparql_rl::parse(&source, "http://example.org/rules.srl");
});
