
### complete comparison helpers — crates/sparq-bench/src/update_fuzz.rs:761

```rust
/// Whether a snapshot mentions a blank node, i.e. whether sorted N-Quads has stopped
/// being a canonical form and RDFC-1.0 relabelling is required.
///
/// Deliberately a substring test. It cannot produce a FALSE NEGATIVE — every blank
/// node renders as `_:label` — and a false positive (a literal containing the text
/// `_:`) would only route a blank-node-free snapshot through the canonicalizer, which
/// on such input is just a sort-and-deduplicate. The generator emits no such literal
/// today; the check stays conservative so that if one is ever added, the comparator
/// degrades in the safe direction.
fn mentions_blank_node(lines: &[String]) -> bool {
    lines.iter().any(|l| l.contains("_:"))
}

/// Re-parses N-Quads lines back into `oxrdf` quads (the input side of every
/// structural rewrite below).
fn parse_lines(lines: &[String], what: &str) -> Result<Vec<Quad>, String> {
    let doc = {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    };
    sparq_canon::parse_nquads(&doc)
        .map_err(|e| format!("{}: N-Quads re-parse failed ({})", what, e))
}

/// The comparable form of one snapshot.
///
/// With `relabel` false (neither side mentions a blank node) the sorted lines ARE the
/// canonical form and are returned untouched — the strongest, byte-level compare.
/// With it true, both sides are relabelled to their RDFC-1.0 canonical form so the
/// comparison decides RDF ISOMORPHISM. The constrained `*_ground_terms` profile is
/// used deliberately: it is exactly RDFC-1.0 with triple terms as opaque constants and
/// fails closed on a blank node nested inside a triple term, so the non-standard
/// nested-bnode descent is never reachable from this harness.
fn comparable(lines: &[String], relabel: bool, what: &str) -> Result<Vec<String>, String> {
    if !relabel {
        return Ok(lines.to_vec());
    }
    let quads = parse_lines(lines, what)?;
    let canon = sparq_canon::canonicalize_rdf12_ground_terms(&quads)
        .map_err(|e| format!("{}: RDFC-1.0 canonicalization failed ({})", what, e))?;
    let mut out: Vec<String> = canon.lines().map(str::to_string).collect();
    out.sort();
    Ok(out)
}

/// Rewrites `t` the way Oxigraph's storage layer does: an `xsd:integer` literal whose
/// value fits the native integer it decodes to is re-rendered in canonical lexical
/// form. Structural, never string surgery — and it descends into triple terms, which
/// Oxigraph's recursive term encoder also normalizes.
fn oxigraph_normalized_term(t: &Term) -> Term {
    match t {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => match l.value().parse::<i64>() {
            Ok(v) if v.to_string() != l.value() => {
                Term::Literal(Literal::new_typed_literal(v.to_string(), xsd::INTEGER))
            }
            _ => t.clone(),
        },
        Term::Triple(inner) => Term::Triple(Box::new(Triple::new(
            inner.subject.clone(),
            inner.predicate.clone(),
            oxigraph_normalized_term(&inner.object),
        ))),
        _ => t.clone(),
    }
}

// [GPT-6 ASTRA] Quad uniqueness alone cannot detect colliding terms at different
// predicates/graphs. Check every integer leaf, including nested triple objects.
fn record_lexical_terms(
    term: &Term,
    integers: &mut BTreeMap<i64, String>,
    blank_nodes: &mut BTreeSet<String>,
) -> Result<(), String> {
    match term {
        Term::Literal(l) if l.datatype() == xsd::INTEGER => {
            if let Ok(value) = l.value().parse::<i64>() {
                match integers.insert(value, l.value().to_string()) {
                    Some(previous) if previous != l.value() => {
                        return Err(format!(
                            "integer normalization is not term-injective: {previous:?} and {:?}",
                            l.value()
                        ));
                    }
                    _ => {}
                }
            }
        }
        Term::BlankNode(b) => {
            blank_nodes.insert(b.as_str().to_string());
        }
        Term::Triple(t) => {
            if let oxrdf::NamedOrBlankNode::BlankNode(b) = &t.subject {
                blank_nodes.insert(b.as_str().to_string());
            }
            record_lexical_terms(&t.object, integers, blank_nodes)?;
        }
        _ => {}
    }
    Ok(())
}

struct NormalizedSnapshot {
    lines: Vec<String>,
    blank_nodes: usize,
}

/// Normalizes integer spellings without merging terms or rows.
fn oxigraph_normalized(lines: &[String], what: &str) -> Result<NormalizedSnapshot, String> {
    let quads = parse_lines(lines, what)?;
    if quads.len() != lines.len() {
        return Err(format!("{what}: parsing changed the raw row count"));
    }
    let mut integers = BTreeMap::new();
    let mut blank_nodes = BTreeSet::new();
    for q in &quads {
        record_lexical_terms(&q.object, &mut integers, &mut blank_nodes)?;
        if let oxrdf::NamedOrBlankNode::BlankNode(b) = &q.subject {
            blank_nodes.insert(b.as_str().to_string());
        }
        if let oxrdf::GraphName::BlankNode(b) = &q.graph_name {
            blank_nodes.insert(b.as_str().to_string());
        }
    }
    let mut out: Vec<String> = quads
        .iter()
        .map(|q| {
            let graph = match &q.graph_name {
                oxrdf::GraphName::DefaultGraph => None,
                g => Some(g.to_string()),
            };
            nquads_line(
                &q.subject.to_string(),
                &q.predicate.to_string(),
                &oxigraph_normalized_term(&q.object).to_string(),
                graph.as_deref(),
            )
        })
        .collect();
    out.sort();
    // The fixed probes project whole quads, so duplicates are not legitimate
    // projection multiplicity. Reject before canon's set conversion can hide them.
    if out.windows(2).any(|rows| rows[0] == rows[1]) {
        return Err(format!(
            "{what}: integer normalization merges or duplicates rows"
        ));
    }
    Ok(NormalizedSnapshot {
        lines: out,
        blank_nodes: blank_nodes.len(),
    })
}

/// The lines on exactly one side — the human-readable core of a divergence report.
fn one_sided(label_a: &str, a: &[String], label_b: &str, b: &[String]) -> String {
    let bset: std::collections::BTreeSet<&String> = b.iter().collect();
    let aset: std::collections::BTreeSet<&String> = a.iter().collect();
    let mut s = String::new();
    s.push_str(&format!("only in {}:\n", label_a));
    for l in a.iter().filter(|l| !bset.contains(l)) {
        s.push_str(&format!("  {}\n", l));
    }
    s.push_str(&format!("only in {}:\n", label_b));
    for l in b.iter().filter(|l| !aset.contains(l)) {
        s.push_str(&format!("  {}\n", l));
    }
    s
}

/// The outcome of comparing two snapshots.
enum Verdict {
    /// Byte-identical, or RDF-isomorphic when blank nodes are in play.
    Same,
    /// Absorbed by the adjudicated `update-oxigraph-integer-lexical-canonicalization`
    /// class: the two datasets agree exactly once Oxigraph's numeric normalization is
    /// re-derived on both sides.
    AdjudicatedIntegerLexical,
    /// A real divergence (the string is the report body).
    Differs(String),
}

/// Compares two snapshots under the canonical form the module docs describe.
///
/// `allow_integer_lexical` is set ONLY for a sparq-vs-Oxigraph compare, and only when
/// the allowlist enables the class. The sparq-vs-sparq compare passes false: both
/// sides are sparq, so any lexical disagreement between them is a real bug.
fn compare(
    label_a: &str,
    a: &[String],
    label_b: &str,
    b: &[String],
    allow_integer_lexical: bool,
) -> Verdict {
    let relabel = mentions_blank_node(a) || mentions_blank_node(b);
    let (ca, cb) = match (
        comparable(a, relabel, label_a),
        comparable(b, relabel, label_b),
    ) {
        (Ok(ca), Ok(cb)) => (ca, cb),
        (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
    };
    if ca == cb {
        // Canonicalization deduplicates, so a duplicate quad on one side alone would
        // survive the compare — check the raw counts to keep that failure visible.
        if a.len() != b.len() {
            return Verdict::Differs(format!(
                "datasets are isomorphic but the raw quad COUNTS differ \
                 ({} {} vs {} {}) — one side is yielding a duplicate quad",
                label_a,
                a.len(),
                label_b,
                b.len()
            ));
        }
        return Verdict::Same;
    }
    if allow_integer_lexical {
        if a.len() != b.len() {
            return Verdict::Differs(format!(
                "integer-lexical adjudication refused: raw row counts differ\n{}",
                one_sided(label_a, &ca, label_b, &cb)
            ));
        }
        let normalized = (
            oxigraph_normalized(a, label_a),
            oxigraph_normalized(b, label_b),
        );
        match normalized {
            (Ok(na), Ok(nb)) => {
                if na.blank_nodes != nb.blank_nodes {
                    return Verdict::Differs(format!(
                        "integer-lexical adjudication refused: blank-node counts differ ({} vs {})\n{}",
                        na.blank_nodes,
                        nb.blank_nodes,
                        one_sided(label_a, &ca, label_b, &cb)
                    ));
                }
                match (
                    comparable(&na.lines, relabel, label_a),
                    comparable(&nb.lines, relabel, label_b),
                ) {
                    (Ok(na), Ok(nb)) if na == nb => return Verdict::AdjudicatedIntegerLexical,
                    (Err(e), _) | (_, Err(e)) => {
                        return Verdict::Differs(format!(
                            "integer-lexical adjudication refused: {e}\n{}",
                            one_sided(label_a, &ca, label_b, &cb)
                        ));
                    }
                    _ => {}
                }
            }
            (Err(e), _) | (_, Err(e)) => {
                return Verdict::Differs(format!(
                    "integer-lexical adjudication refused: {e}\n{}",
                    one_sided(label_a, &ca, label_b, &cb)
                ));
            }
        }
    }
    Verdict::Differs(one_sided(label_a, &ca, label_b, &cb))
}
```

### nquads_line — crates/sparq-bench/src/update_fuzz.rs:707

```rust
fn nquads_line(subject: &str, predicate: &str, object: &str, graph: Option<&str>) -> String {
    match graph {
        Some(g) => format!("{} {} {} {} .", subject, predicate, object, g),
        None => format!("{} {} {} .", subject, predicate, object),
    }
}

/// A sparq `Graph` (default graph + named graphs) as SORTED N-Quads lines.
/// Duplicate lines are NOT collapsed — see the raw-count check in [`compare`].
```

### PROBES — crates/sparq-bench/src/update_fuzz.rs:1030

```rust
const PROBES: &[(&str, &[&str])] = &[
    ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
    (
        "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
        &["s", "p", "o", "g"],
    ),
];
```

### production apply_sequence — crates/sparq-bench/src/update_fuzz.rs:1179

```rust
fn apply_sequence(
    seed: u64,
    ops: &[Op],
    sandbox: Option<&LoadSandbox>,
    inject_divergence_at: Option<usize>,
    allow: &UpdateDivergenceAllowlist,
) -> Result<SeedOutcome, String> {
    let mut g_rebuild = Graph::new();
    let mut g_inplace = Graph::new();
    let store = Store::new().map_err(|e| format!("oxigraph store init: {}", e))?;
    let mut adjudicated_integer_lexical = 0u64;
    let mut isomorphism_compares = 0u64;

    let fail = |step: usize, op: &Op, detail: String| -> String {
        format!(
            "step={} of {}\nop: {}\n{}\nrepro: cargo run -p sparq-bench --release -- \
             update-fuzz --seed-start {} --seed-count 1\n--- full sequence ---\n{}",
            step,
            ops.len(),
            op.sparq,
            detail,
            seed,
            ops.iter()
                .enumerate()
                .map(|(i, o)| match &o.oxi {
                    Some(x) if *x != o.sparq =>
                        format!("[{}] {}\n     (reference engine ran: {})", i, o.sparq, x),
                    Some(_) => format!("[{}] {}", i, o.sparq),
                    None => format!("[{}] {}\n     (reference engine ran: nothing)", i, o.sparq),
                })
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    for (i, op) in ops.iter().enumerate() {
        // A LOAD's document must exist before the request runs.
        if let (Some(doc), Some(s)) = (&op.doc, sandbox) {
            s.write(doc).map_err(|e| fail(i, op, e))?;
        }

        // Apply to all three implementations. Every generated op is inside the
        // supported deterministic subset, so an error from ANY engine is itself a
        // divergence (strict — there is no unsupported-skip in this harness).
        g_rebuild = sparq_engine::update(&g_rebuild, &op.sparq)
            .map_err(|e| fail(i, op, format!("sparq update (rebuild path) error: {}", e)))?;
        sparq_engine::update_in_place(&mut g_inplace, &op.sparq)
            .map_err(|e| fail(i, op, format!("sparq update_in_place error: {}", e)))?;
        if let Some(oxi) = &op.oxi {
            store
                .update(oxi.as_str())
                .map_err(|e| fail(i, op, format!("oxigraph update error: {}", e)))?;
        }

        if inject_divergence_at == Some(i) {
            use oxigraph::model::{GraphName, NamedNode, Quad};
            let n = |s: &str| NamedNode::new(s).expect("valid IRI");
            let marker = Quad::new(
                n("http://ex/injected"),
                n("http://ex/injected"),
                n("http://ex/injected"),
                GraphName::DefaultGraph,
            );
            store
                .insert(&marker)
                .map_err(|e| format!("marker insert failed: {}", e))?;
        }

        // (a) Canonical dataset equality. Localizes a divergence to this exact step.
        // sparq-vs-sparq FIRST and STRICT (no adjudication): the two sparq paths must
        // agree with each other whatever the reference engine does.
        let nq_rebuild = sparq_nquads(&g_rebuild);
        let nq_inplace = sparq_nquads(&g_inplace);
        let nq_oxi = oxi_nquads(&store).map_err(|e| fail(i, op, e))?;
        if mentions_blank_node(&nq_rebuild) || mentions_blank_node(&nq_oxi) {
            isomorphism_compares += 1;
        }
        if let Verdict::Differs(detail) = compare(
            "sparq(rebuild)",
            &nq_rebuild,
            "sparq(in-place)",
            &nq_inplace,
            false,
        ) {
            return Err(fail(
                i,
                op,
                format!(
                    "canonical dataset differs BETWEEN SPARQ'S OWN UPDATE PATHS\n{}",
                    detail
                ),
            ));
        }
        for (label, nq) in [
            ("sparq(rebuild)", &nq_rebuild),
            ("sparq(in-place)", &nq_inplace),
        ] {
            match compare(label, nq, "oxigraph", &nq_oxi, allow.integer_lexical) {
                Verdict::Same => {}
                Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
                Verdict::Differs(detail) => {
                    return Err(fail(
                        i,
                        op,
                        format!(
                            "canonical dataset differs ({} vs oxigraph)\n{}",
                            label, detail
                        ),
                    ));
                }
            }
        }

        // (b) Probe SELECTs — the query-path view of the updated store, full sorted
        // binding sets (never counts). Checked for BOTH sparq graphs: the in-place
        // one reads through the live delta overlay.
        for (probe, vars) in PROBES {
            let oxi = oxi_probe(&store, probe, vars)
                .map_err(|e| fail(i, op, format!("oxigraph probe error: {}", e)))?;
            for (label, g) in [("rebuild", &g_rebuild), ("in-place", &g_inplace)] {
                let sparq = sparq_probe(g, probe).map_err(|e| fail(i, op, e))?;
                match compare("sparq", &sparq, "oxigraph", &oxi, allow.integer_lexical) {
                    Verdict::Same => {}
                    Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
                    Verdict::Differs(detail) => {
                        return Err(fail(
                            i,
                            op,
                            format!(
                                "probe {:?} binding set differs (sparq {} vs oxigraph)\n{}",
                                probe, label, detail
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(SeedOutcome {
        ops: ops.len() as u64,
        adjudicated_integer_lexical,
        isomorphism_compares,
    })
}
```

### complete allowlist — crates/sparq-bench/src/update_fuzz.rs:1326

```rust
/// The adjudicated `update-*` divergence classes this comparator has detectors for.
/// The registry is the same `bench/differential-divergences.json` the query fuzzer
/// consumes. A listed `update-*` class WITHOUT a detector here stays strict (loud
/// warning), mirroring the fail-toward-flagging posture of `fuzz.rs`.
struct UpdateDivergenceAllowlist {
    /// `update-oxigraph-integer-lexical-canonicalization` — Oxigraph's storage layer
    /// collapses `xsd:integer` lexical forms. Not a blind skip: absorbed only when
    /// re-deriving that normalization on both sides makes the datasets agree exactly.
    integer_lexical: bool,
    /// Where the allowlist was loaded from, and its posture (for the summary line).
    state: String,
}

/// The class id this comparator has a detector for.
const INTEGER_LEXICAL_CLASS: &str = "update-oxigraph-integer-lexical-canonicalization";

impl UpdateDivergenceAllowlist {
    fn strict(path: &str, why: &str) -> Self {
        UpdateDivergenceAllowlist {
            integer_lexical: false,
            state: format!("({}): {} — STRICT (every divergence fails)", path, why),
        }
    }

    /// Load from `SPARQ_FUZZ_DIVERGENCES` (a CI/agent override), else the committed
    /// repo default resolved relative to this crate's manifest (works from any cwd).
    fn load() -> Self {
        let path = std::env::var("SPARQ_FUZZ_DIVERGENCES").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../bench/differential-divergences.json"
            )
            .to_string()
        });
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_json(&s, &path),
            Err(e) => Self::strict(&path, &format!("unreadable ({})", e)),
        }
    }

    /// Parse the allowlist JSON. An unknown `update-*` class id is IGNORED with a loud
    /// warning (fail-STRICT: the comparator has no detector for it, so that class keeps
    /// failing rather than being silently "absorbed" by nothing); malformed JSON is
    /// also strict.
    fn from_json(s: &str, path: &str) -> Self {
        let v: serde_json::Value = match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => return Self::strict(path, &format!("invalid JSON ({})", e)),
        };
        let ids: Vec<String> = v["classes"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|c| c["id"].as_str())
            .filter(|id| id.starts_with("update-"))
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return Self::strict(path, "no adjudicated `update-*` classes");
        }
        let mut out = UpdateDivergenceAllowlist {
            integer_lexical: false,
            state: String::new(),
        };
        let mut enabled = Vec::new();
        for id in &ids {
            if id == INTEGER_LEXICAL_CLASS {
                out.integer_lexical = true;
                enabled.push(id.clone());
            } else {
                eprintln!(
                    "warning: divergence allowlist lists update class {:?} but update-fuzz has \
                     no detector for it — that class stays STRICT",
                    id
                );
            }
        }
        out.state = format!(
            "({}): adjudicated classes enabled {:?}; every other divergence fails",
            path, enabled
        );
        out
    }
}
```

### test allowlist helper — crates/sparq-bench/src/update_fuzz.rs:1475

```rust
    fn allowlist() -> UpdateDivergenceAllowlist {
        UpdateDivergenceAllowlist::load()
    }
```

### all new and changed semantic tests — crates/sparq-bench/src/update_fuzz.rs:1972

```rust
    // [GPT-6 ASTRA] These checks use actual generated operations and public updates;
    // a persistent map also catches collisions across different graphs or steps.
    #[test]
    fn generated_integer_domain_is_injective_including_load() {
        // [GPT-6 ASTRA] Independent test boundaries: do not derive the expected
        // domain from the production constant or rely on a sampled collision.
        fn assert_integer_pool(term: &Term) {
            match term {
                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
                    let value = l
                        .value()
                        .parse::<u64>()
                        .expect("nonnegative generated integer");
                    if value < 20 {
                        assert_eq!(l.value(), value.to_string(), "canonical pool spelling");
                    } else {
                        assert!((20..80).contains(&value), "noncanonical pool: {value}");
                        assert_ne!(l.value(), value.to_string(), "noncanonical pool spelling");
                    }
                }
                Term::Triple(t) => assert_integer_pool(&t.object),
                _ => {}
            }
        }
        assert_eq!(CANONICAL_INTEGER_VALUES, 20);
        let mut noncanonical = 0;
        let mut loads = 0;
        let mut load_integers = 0;
        let mut nested = 0;
        for seed in 0..400 {
            let ops = gen_sequence(&mut Rng::new(seed));
            let sandbox = LoadSandbox::new().expect("sandbox");
            sparq_engine::with_load_base(sandbox.path(), || {
                let mut graph = Graph::new();
                let mut integers = BTreeMap::new();
                let mut bnodes = BTreeSet::new();
                for op in &ops {
                    if let Some(doc) = &op.doc {
                        sandbox.write(doc).expect("LOAD document");
                        let rows: Vec<_> = doc.content.lines().map(str::to_string).collect();
                        for q in parse_lines(&rows, "LOAD document").expect("valid document") {
                            assert_integer_pool(&q.object);
                            match &q.object {
                                Term::Literal(l) if l.datatype() == xsd::INTEGER => {
                                    let value = l.value().parse::<u64>().unwrap();
                                    assert!(
                                        value < 20,
                                        "LOAD integer must stay canonical: {value}"
                                    );
                                    assert_eq!(value.to_string(), l.value());
                                    load_integers += 1;
                                }
                                _ => {}
                            }
                            record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                                .expect("LOAD shares the injective integer domain");
                        }
                        let mirror = sparq_engine::update(&Graph::new(), op.oxi.as_ref().unwrap())
                            .expect("reference INSERT mirror");
                        let loaded =
                            sparq_engine::update(&Graph::new(), &op.sparq).expect("isolated LOAD");
                        assert_eq!(sparq_nquads(&mirror), sparq_nquads(&loaded));
                        loads += 1;
                    }
                    graph = sparq_engine::update(&graph, &op.sparq)
                        .unwrap_or_else(|e| panic!("seed={seed} update={} failed: {e}", op.sparq));
                    for q in parse_lines(&sparq_nquads(&graph), "generated state").unwrap() {
                        assert_integer_pool(&q.object);
                        nested += usize::from(matches!(&q.object, Term::Triple(_)));
                        record_lexical_terms(&q.object, &mut integers, &mut bnodes)
                            .unwrap_or_else(|e| panic!("seed={seed}: {e}"));
                    }
                }
                noncanonical += integers
                    .iter()
                    .filter(|(v, lexical)| v.to_string() != **lexical)
                    .count();
            });
        }
        assert!(noncanonical > 0 && loads > 0 && load_integers > 0 && nested > 0);
        eprintln!(
            "injective corpus: noncanonical={noncanonical}, LOAD={loads}, triple terms={nested}"
        );
    }

    #[test]
    fn integer_pools_keep_draw_counts_and_all_spelling_families() {
        let mut seen = BTreeMap::new();
        let mut styles = BTreeSet::new();
        for seed in 0..400 {
            let mut rng = Rng::new(seed);
            let mut expected = Rng::new(seed);
            expected.below(20);
            expected.below(3);
            let term = gen_noncanonical_integer(&mut rng);
            assert_eq!(rng.next(), expected.next(), "exactly two RNG draws");
            let rows = vec![format!("<http://ex/s> <http://ex/p> {term} .")];
            let q = parse_lines(&rows, "generated literal")
                .unwrap()
                .pop()
                .unwrap();
            record_lexical_terms(&q.object, &mut seen, &mut BTreeSet::new()).unwrap();
            if let Term::Literal(l) = q.object {
                let v = l.value().parse::<u64>().unwrap();
                assert!((20..80).contains(&v));
                assert_ne!(v.to_string(), l.value());
                styles.insert(if l.value().starts_with('+') {
                    0
                } else if l.value().starts_with("00") {
                    1
                } else {
                    2
                });
            }
        }
        assert_eq!(styles.len(), 3);
        assert_eq!(seen.len(), 60, "all disjoint values reached");
    }

    #[test]
    fn lexical_collisions_across_contexts_and_nested_terms_fail() {
        for second in [
            "<http://ex/s> <http://ex/q> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            "<http://ex/s> <http://ex/q> <<( <http://ex/a> <http://ex/p> <<( <http://ex/b> <http://ex/r> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> )>> .",
        ] {
            let a = vec![
                format!("<http://ex/s> <http://ex/p> \"020\"^^{XSD_INTEGER} ."),
                second.to_string(),
            ];
            let b: Vec<_> = a.iter().map(|s| s.replace("\"020\"", "\"20\"")).collect();
            match compare("a", &a, "b", &b, true) {
                Verdict::Differs(e) => assert!(e.contains("not term-injective"), "{e}"),
                _ => panic!("different contexts must not hide a term collision"),
            }
        }
    }

    #[test]
    fn lexical_adjudication_preserves_rows_and_blank_node_structure() {
        // The blank-node case reaches canon's set conversion; without it the
        // ordinary vector comparison alone rejects the differing multiplicities.
        for subject in ["<http://ex/s>", "_:x"] {
            let p = format!("{subject} <http://ex/p> \"020\"^^{XSD_INTEGER} .");
            let q = format!("{subject} <http://ex/q> \"021\"^^{XSD_INTEGER} .");
            let a = vec![p.clone(), p.clone(), q.clone()];
            let b = vec![
                p.replace("020", "20"),
                q.replace("021", "21"),
                q.replace("021", "21"),
            ];
            assert!(
                matches!(compare("a", &a, "b", &b, true), Verdict::Differs(_)),
                "equal totals cannot hide duplicate redistribution for {subject}"
            );
        }
        let a = vec![
            format!("_:a <http://ex/p> \"020\"^^{XSD_INTEGER} ."),
            "_:a <http://ex/q> <http://ex/o> .".into(),
        ];
        let good = vec![
            format!("_:x <http://ex/p> \"20\"^^{XSD_INTEGER} ."),
            "_:x <http://ex/q> <http://ex/o> .".into(),
        ];
        assert!(matches!(
            compare("a", &a, "b", &good, true),
            Verdict::AdjudicatedIntegerLexical
        ));
        let split = vec![good[0].clone(), "_:y <http://ex/q> <http://ex/o> .".into()];
        match compare("a", &a, "b", &split, true) {
            Verdict::Differs(detail) => {
                assert!(
                    detail.contains("blank-node counts differ (1 vs 2)"),
                    "{detail}"
                );
                assert!(detail.contains("only in a:"), "{detail}");
                assert!(detail.contains("only in b:"), "{detail}");
                assert!(
                    detail.contains("\"020\""),
                    "original lexical missing: {detail}"
                );
                assert!(
                    detail.contains("\"20\""),
                    "reference lexical missing: {detail}"
                );
            }
            _ => panic!("different blank-node counts must fail with original dataset details"),
        }
        let extra = vec![good[0].clone(), "_:x <http://ex/r> <http://ex/o> .".into()];
        assert!(
            matches!(compare("a", &a, "b", &extra, true), Verdict::Differs(_)),
            "same node/row counts do not license a changed predicate"
        );
        for (left, right) in [
            (
                "\"20.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                "\"20\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
            ),
            ("\"x\"@en", "\"x\"@fr"),
        ] {
            let left = vec![format!("<http://ex/s> <http://ex/p> {left} .")];
            let right = vec![format!("<http://ex/s> <http://ex/p> {right} .")];
            assert!(matches!(
                compare("a", &left, "b", &right, true),
                Verdict::Differs(_)
            ));
        }
    }

    #[test]
    fn nested_noncanonical_terms_remain_exact_in_sparq() {
        let expected = vec![format!(
            "<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <<( <http://ex/b> <http://ex/r> \"020\"^^{XSD_INTEGER} )>> )>> ."
        )];
        let op = format!("INSERT DATA {{ {} }}", expected[0]);
        let rebuilt = sparq_engine::update(&Graph::new(), &op).unwrap();
        let mut inplace = Graph::new();
        sparq_engine::update_in_place(&mut inplace, &op).unwrap();
        for graph in [&rebuilt, &inplace] {
            assert_eq!(sparq_nquads(graph), expected);
            assert_eq!(sparq_probe(graph, PROBES[0].0).unwrap(), expected);
        }
        let canonical: Vec<_> = expected.iter().map(|s| s.replace("020", "20")).collect();
        assert!(matches!(
            compare("sparq", &expected, "reference", &canonical, true),
            Verdict::AdjudicatedIntegerLexical
        ));
        assert!(matches!(
            compare("sparq", &expected, "other sparq", &canonical, false),
            Verdict::Differs(_)
        ));
    }

    #[test]
    fn injected_marker_fails_during_actual_lexical_adjudication() {
        let ops = vec![
            Op::shared(format!(
                "INSERT DATA {{ <http://ex/s> <http://ex/p> \"020\"^^{XSD_INTEGER} }}"
            )),
            Op::shared("INSERT { ?s <http://ex/q> _:bt } WHERE { ?s <http://ex/p> ?o }".into()),
        ];
        let allow = UpdateDivergenceAllowlist {
            integer_lexical: true,
            state: "hermetic marker-adjudication test".into(),
        };
        let positive = apply_sequence(0, &ops, None, None, &allow).unwrap();
        assert_eq!(positive.ops, 2);
        assert!(positive.adjudicated_integer_lexical > 0);
        let negative = apply_sequence(0, &ops, None, Some(1), &allow).unwrap_err();
        assert!(negative.contains("step=1") && negative.contains("http://ex/injected"));
    }

    // [GPT-6 ASTRA] Literal historical input, not regenerated after narrowing the
    // reference corpus. The reference cannot represent this lexical/cardinality case.
    #[test]
    fn historical_4141222487_preserves_lexicals_and_fresh_nodes() {
        let ops = [
            r#"INSERT DATA { GRAPH <http://ex/g0> { <http://ex/s4> <http://ex/p0> "lit2" . } <http://ex/s2> <http://ex/p3> <http://ex/o5> . <http://ex/s4> <http://ex/p3> 7 . <http://ex/s0> <http://ex/p2> "lit1" . <http://ex/s4> <http://ex/p0> <<( <http://ex/s2> <http://ex/p0> "tag0"@en )>> . }"#,
            r#"INSERT DATA { <http://ex/s5> <http://ex/p2> <<( <http://ex/s4> <http://ex/p2> 9 )>> . <http://ex/s1> <http://ex/p3> 6 . GRAPH <http://ex/g2> { <http://ex/s3> <http://ex/p0> 1 . } GRAPH <http://ex/g1> { <http://ex/s5> <http://ex/p0> "lit1" . } <http://ex/s2> <http://ex/p0> "lit1" . } ;
DELETE DATA { <http://ex/s2> <http://ex/p0> <http://ex/o2> . <http://ex/s4> <http://ex/p1> 18 . <http://ex/s0> <http://ex/p1> 8 . }"#,
            r#"DELETE DATA { <http://ex/s4> <http://ex/p0> <<( <http://ex/s2> <http://ex/p0> "tag0"@en )>> . <http://ex/s4> <http://ex/p3> 7 . <http://ex/s5> <http://ex/p2> <<( <http://ex/s4> <http://ex/p2> 9 )>> . }"#,
            r#"INSERT { ?s <http://ex/p1> <<( ?s ?p ?o )>> } WHERE { ?s ?p ?o FILTER(!isBlank(?s) && !isBlank(?o)) }"#,
            r#"INSERT DATA { <http://ex/s3> <http://ex/p1> "lit1" . <http://ex/s5> <http://ex/p2> <http://ex/o4> . <http://ex/s2> <http://ex/p1> 8 . <http://ex/s2> <http://ex/p1> "008"^^<http://www.w3.org/2001/XMLSchema#integer> . }"#,
            r#"LOAD SILENT <file://doc1.nt>"#,
            r#"DELETE DATA { <http://ex/s0> <http://ex/p2> "lit1" . }"#,
            r#"CREATE SILENT GRAPH <http://ex/g0>"#,
            r#"INSERT { ?s <http://ex/p0> _:bt } WHERE { ?s <http://ex/p1> ?o }"#,
            r#"ADD SILENT GRAPH <http://ex/g0> TO GRAPH <http://ex/g2>"#,
        ];
        let sandbox = LoadSandbox::new().unwrap();
        sandbox.write(&LoadDoc {
            name: "doc1.nt".into(),
            content: "<http://ex/s5> <http://ex/p1> <http://ex/o3> .\n<http://ex/s0> <http://ex/p0> <http://ex/o2> .\n<http://ex/s4> <http://ex/p1> <http://ex/o5> .\n".into(),
        }).unwrap();
        sparq_engine::with_load_base(sandbox.path(), || {
            let mut rebuilt = Graph::new();
            let mut inplace = Graph::new();
            for (step, op) in ops.iter().enumerate() {
                rebuilt = sparq_engine::update(&rebuilt, op).unwrap();
                sparq_engine::update_in_place(&mut inplace, op).unwrap();
                let a = sparq_nquads(&rebuilt);
                let b = sparq_nquads(&inplace);
                assert!(
                    matches!(compare("rebuild", &a, "inplace", &b, false), Verdict::Same),
                    "internal dataset mismatch at {step}"
                );
                for (probe, _) in PROBES {
                    let a = sparq_probe(&rebuilt, probe).unwrap();
                    let b = sparq_probe(&inplace, probe).unwrap();
                    assert!(
                        matches!(
                            compare("rebuild probe", &a, "inplace probe", &b, false),
                            Verdict::Same
                        ),
                        "internal full-row mismatch at {step}"
                    );
                }
                if step == 8 {
                    for graph in [&rebuilt, &inplace] {
                        let quads = parse_lines(&sparq_nquads(graph), "historical state").unwrap();
                        let fresh: Vec<_> = quads
                            .iter()
                            .filter(|q| {
                                q.predicate.as_str() == "http://ex/p0"
                                    && matches!(&q.object, Term::BlankNode(_))
                            })
                            .collect();
                        assert_eq!(fresh.len(), 9);
                        assert_eq!(
                            fresh
                                .iter()
                                .map(|q| q.object.to_string())
                                .collect::<BTreeSet<_>>()
                                .len(),
                            9
                        );
                        assert_eq!(
                            fresh
                                .iter()
                                .filter(|q| q.subject.to_string() == "<http://ex/s2>")
                                .count(),
                            4
                        );
                        let rows = sparq_engine::query(
                            graph,
                            "SELECT ?s ?o WHERE { ?s <http://ex/p1> ?o }",
                        )
                        .unwrap();
                        assert_eq!(rows.rows.len(), 9);
                        let s2: Vec<_> = quads
                            .iter()
                            .filter(|q| {
                                q.subject.to_string() == "<http://ex/s2>"
                                    && q.predicate.as_str() == "http://ex/p1"
                            })
                            .map(|q| q.object.to_string())
                            .collect();
                        assert!(s2.contains(&format!("\"8\"^^{XSD_INTEGER}")));
                        assert!(s2.contains(&format!("\"008\"^^{XSD_INTEGER}")));
                    }
                    eprintln!(
                        "historical index8: 9 solutions, 9 fresh blank nodes, 4 for s2; both lexical terms retained"
                    );
                }
            }
            let g2: Vec<_> = sparq_nquads(&rebuilt)
                .into_iter()
                .filter(|q| q.ends_with("<http://ex/g2> ."))
                .collect();
            assert_eq!(
                g2,
                vec![
                    format!("<http://ex/s3> <http://ex/p0> \"1\"^^{XSD_INTEGER} <http://ex/g2> ."),
                    "<http://ex/s4> <http://ex/p0> \"lit2\" <http://ex/g2> .".to_string(),
                ],
                "final index9 ADD preserved existing g2 and copied g0"
            );
            eprintln!(
                "historical index9: ADD completed; all 10 original requests checked internally"
            );
        });
    }
}
```

### generator integer pool — crates/sparq-bench/src/update_fuzz.rs:166

```rust
const CANONICAL_INTEGER_VALUES: u64 = 20;

fn gen_canonical_integer(rng: &mut Rng) -> u64 {
    rng.below(CANONICAL_INTEGER_VALUES)
}

// ── deterministic RNG ────────────────────────────────────────────────────────────

/// Deterministic SplitMix64 — no clock/entropy, so every case is reproducible from
/// its seed. (Same generator as `fuzz.rs`; duplicated because the bead keeps this
/// file's edits disjoint from the query fuzzer's.)
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

// ── generator ────────────────────────────────────────────────────────────────────
//
// Small term pools so random DELETE DATA / WHERE conditions collide with previously
// inserted data at high probability (a delete that never matches exercises nothing).
```
