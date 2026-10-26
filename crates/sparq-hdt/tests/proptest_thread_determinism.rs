//! Property tests for concurrent HDT dictionary decode and fixed-order remapping.

use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed};
use sparq_core::Graph;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Clone, Debug)]
enum Object {
    Iri(u8),
    Literal(u8),
}

fn object_strategy() -> impl Strategy<Value = Object> {
    prop_oneof![
        3 => (0u8..8).prop_map(Object::Iri),
        2 => (0u8..6).prop_map(Object::Literal),
    ]
}

fn triple_strategy() -> impl Strategy<Value = (u8, u8, Object)> {
    (0u8..8, 0u8..4, object_strategy())
}

fn render_nt(triples: &[(u8, u8, Object)]) -> String {
    triples
        .iter()
        .map(|(subject, predicate, object)| {
            let object = match object {
                // Reusing the subject namespace in object position populates HDT's
                // shared dictionary section; the small domains also make duplicates common.
                Object::Iri(index) => format!("<http://example.test/resource/{index}>"),
                Object::Literal(index) => format!("\"value-{index}\""),
            };
            format!(
                "<http://example.test/resource/{subject}> <http://example.test/predicate/{predicate}> {object} .\n"
            )
        })
        .collect()
}

fn write_hdt(nt: &str, directory: &Path) -> std::path::PathBuf {
    let nt_path = directory.join("source.nt");
    let hdt_path = directory.join("source.hdt");
    std::fs::write(&nt_path, nt).expect("write generated N-Triples fixture");

    let hdt = hdt::Hdt::read_nt(&nt_path).expect("build HDT fixture from N-Triples");
    let mut output = std::io::BufWriter::new(
        std::fs::File::create(&hdt_path).expect("create generated HDT fixture"),
    );
    hdt.write(&mut output).expect("write generated HDT fixture");
    hdt_path
}

fn triple_set(graph: &Graph) -> BTreeSet<[String; 3]> {
    let scan = graph.store().scan(&[None, None, None]);
    scan.rows
        .iter()
        .map(|row| {
            let [subject, predicate, object] = scan.to_spo(row);
            [
                graph.dict().term(subject).to_string(),
                graph.dict().term(predicate).to_string(),
                graph.dict().term(object).to_string(),
            ]
        })
        .collect()
}

fn load_hdt_in_pool(path: &Path, threads: usize) -> Graph {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("build scoped rayon pool")
        .install(|| sparq_hdt::load(path).expect("load generated HDT fixture"))
}

proptest! {
    #![proptest_config(Config {
        cases: 24,
        rng_seed: RngSeed::Fixed(0x5A17_D37E),
        ..Config::default()
    })]

    // sq-agrtc: this witnesses both the native-loader equivalence law and
    // invariance of concurrent section decode/fixed-order merge under pool width.
    #[test]
    fn concurrent_decode_is_equivalent_and_thread_deterministic(
        triples in prop::collection::vec(triple_strategy(), 1..24)
    ) {
        let nt = render_nt(&triples);
        let directory = tempfile::tempdir().expect("create unique fixture directory");
        let hdt_path = write_hdt(&nt, directory.path());

        let native = Graph::load_str(&nt, "ntriples").expect("load generated N-Triples");
        let one_thread = load_hdt_in_pool(&hdt_path, 1);
        let four_threads = load_hdt_in_pool(&hdt_path, 4);

        let expected = triple_set(&native);
        prop_assert_eq!(triple_set(&one_thread), expected.clone());
        prop_assert_eq!(triple_set(&four_threads), expected);
        prop_assert_eq!(triple_set(&one_thread), triple_set(&four_threads));
    }
}
