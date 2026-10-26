//! Generated differential coverage for HDT ingestion against the native N-Triples loader.

use sparq_core::Graph;
use std::collections::BTreeSet;

/// Small deterministic generator kept local so this test adds no dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn index(&mut self, len: usize) -> usize {
        (self.next() as usize) % len
    }
}

fn generated_graph(rng: &mut Lcg, case: usize) -> String {
    const LITERALS: &[&str] = &[
        r#""plain""#,
        r#""unicode café — snowman ☃""#,
        r#""café"@fr"#,
        r#""42"^^<http://www.w3.org/2001/XMLSchema#integer>"#,
        r#""3.125"^^<http://www.w3.org/2001/XMLSchema#decimal>"#,
        r#""true"^^<http://www.w3.org/2001/XMLSchema#boolean>"#,
        r#""custom"^^<http://example.test/datatype/token>"#,
    ];

    let triple_count = 12 + rng.index(25);
    let mut nt = String::new();

    // Every case contains a typed literal, providing a stable one-triple regression witness.
    nt.push_str(&format!(
        "<http://example.test/s/{case}/typed> <http://example.test/p/value> \
         \"{case}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n"
    ));

    for row in 1..triple_count {
        let subject = rng.index(11);
        let predicate = rng.index(7);
        let object = if rng.index(3) == 0 {
            format!("<http://example.test/o/{case}/{}>", rng.index(13))
        } else {
            LITERALS[rng.index(LITERALS.len())].to_owned()
        };
        nt.push_str(&format!(
            "<http://example.test/s/{case}/{subject}> \
             <http://example.test/p/{predicate}> {object} .\n"
        ));
        // Deliberately repeat some statements: both loaders must agree on set semantics.
        if row % 9 == 0 {
            let last = nt
                .lines()
                .last()
                .expect("a triple was just generated")
                .to_owned();
            nt.push_str(&last);
            nt.push('\n');
        }
    }
    nt
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

#[test]
fn hdt_load_equals_ntriples_over_generated_corpus() {
    let mut rng = Lcg(0x5eed_d1ff_e2e5_71a1);

    for case in 0..24 {
        let nt = generated_graph(&mut rng, case);
        let dir = tempfile::tempdir().expect("creating unique corpus scratch directory");
        let nt_path = dir.path().join("source.nt");
        let hdt_path = dir.path().join("archive.hdt");
        std::fs::write(&nt_path, &nt).expect("writing generated N-Triples source");

        let archive = hdt::Hdt::read_nt(&nt_path).expect("building HDT from generated source");
        let mut output = std::io::BufWriter::new(
            std::fs::File::create(&hdt_path).expect("creating generated HDT archive"),
        );
        archive
            .write(&mut output)
            .expect("writing generated HDT archive");
        drop(output);

        let from_hdt = sparq_hdt::load(&hdt_path).expect("loading generated HDT archive");
        let from_nt = Graph::load_str(&nt, "ntriples").expect("loading generated N-Triples");

        assert_eq!(
            triple_set(&from_hdt),
            triple_set(&from_nt),
            "loaders differ for generated corpus case {case}:\n{nt}"
        );
    }
}
