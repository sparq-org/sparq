//! Shared manifest execution for the RDF 1.1 and RDF 1.2 syntax suites.

use crate::inference::report::{Outcome, TestResult};
use crate::rdf::{as_node, iri_to_path, MiniGraph};
use oxrdf::dataset::CanonicalizationAlgorithm;
use oxrdf::{Dataset, Term};
use sparq_core::Graph;
use std::path::Path;

const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const RDFT: &str = "http://www.w3.org/ns/rdftest#";

type Parse = fn(&str, &str) -> Result<Graph, String>;

pub(crate) fn run_manifest(
    manifest: &Path,
    base: Option<&str>,
    label: &str,
    stem: &str,
    parse: Parse,
) -> Result<Vec<(String, TestResult)>, String> {
    let manifest = manifest.canonicalize().map_err(|e| e.to_string())?;
    let g = MiniGraph::load(&manifest)?;
    let declared_base = g.triples.iter().find_map(|t| {
        (t.predicate.as_str() == format!("{MF}assumedTestBase"))
            .then_some(&t.object)
            .and_then(|o| match o {
                Term::NamedNode(n) => Some(n.as_str()),
                _ => None,
            })
    });
    let base = base
        .or(declared_base)
        .ok_or("manifest has no mf:assumedTestBase")?;
    let root = manifest.parent().ok_or("manifest has no directory")?;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for head in g
        .triples
        .iter()
        .filter(|t| t.predicate.as_str() == format!("{MF}entries"))
    {
        for entry in g.checked_list(&head.object)? {
            let node = as_node(&entry).ok_or("manifest entry is not a resource")?;
            if !seen.insert(node.clone()) {
                continue;
            }
            if g.objects(&node, &format!("{RDFT}approval")).iter().any(
                |o| matches!(o, Term::NamedNode(n) if n.as_str() == format!("{RDFT}Withdrawn")),
            ) {
                continue;
            }
            let Term::NamedNode(id) = entry else {
                return Err("test has no IRI".into());
            };
            let name = g
                .str_object(&node, &format!("{MF}name"))
                .unwrap_or_else(|| {
                    id.as_str()
                        .rsplit(['#', '/'])
                        .next()
                        .unwrap_or(id.as_str())
                        .into()
                });
            let file = |pred: &str| {
                g.object(&node, &format!("{MF}{pred}"))
                    .and_then(|t| match t {
                        Term::NamedNode(n) => iri_to_path(n.as_str()),
                        _ => None,
                    })
            };
            let kind = g
                .types_of(&node)
                .into_iter()
                .find_map(|ty| ty.strip_prefix(RDFT).map(str::to_owned))
                .unwrap_or_default();
            let outcome = if stem == "TestXML" && !cfg!(feature = "rdfxml") {
                Outcome::Fail("RDF/XML parser requires the rdfxml feature".into())
            } else {
                match file("action") {
                    Some(action) => {
                        let action_base = match action.strip_prefix(root) {
                            Ok(relative) => format!("{base}{}", relative.display()),
                            Err(_) => crate::rdf::file_iri(&action),
                        };
                        evaluate(
                            &action,
                            file("result").as_deref(),
                            &action_base,
                            kind.strip_prefix(stem).unwrap_or("unknown"),
                            parse,
                            matches!(stem, "TestNTriples" | "TestTurtle" | "TestXML"),
                        )
                    }
                    None => Outcome::Fail("missing or nonlocal mf:action".into()),
                }
            };
            out.push((
                crate::rdf::published_iri(id.as_str(), root, base),
                TestResult {
                    suite: label.into(),
                    name,
                    outcome,
                },
            ));
        }
    }
    if out.is_empty() {
        return Err(format!("{} has no active entries", manifest.display()));
    }
    Ok(out)
}

fn evaluate(
    action: &Path,
    result: Option<&Path>,
    base: &str,
    kind: &str,
    parse: Parse,
    triples: bool,
) -> Outcome {
    // A missing fixture or invalid UTF-8 is a runner failure, even for a negative test.
    let text = match std::fs::read_to_string(action) {
        Ok(t) => t,
        Err(e) => return Outcome::Fail(format!("read {}: {e}", action.display())),
    };
    let negative = matches!(kind, "NegativeSyntax" | "NegativeEval");
    if !matches!(
        kind,
        "PositiveSyntax" | "NegativeSyntax" | "Eval" | "NegativeEval" | "PositiveC14N"
    ) {
        return Outcome::Fail(format!("unhandled test kind {kind}"));
    }
    let parsed = match parse(&text, base) {
        Ok(q) if negative => {
            return Outcome::Fail(format!("negative test accepted ({} quads)", q.len()))
        }
        Ok(q) => q,
        Err(_) if negative => return Outcome::Pass,
        Err(e) => return Outcome::Fail(format!("action parse error: {e}")),
    };
    if kind == "PositiveSyntax" {
        return Outcome::Pass;
    }
    let Some(result) = result else {
        return Outcome::Fail("missing mf:result".into());
    };
    let expected = match std::fs::read(result) {
        Ok(b) => b,
        Err(e) => return Outcome::Fail(format!("read {}: {e}", result.display())),
    };
    // Exercise the shipping CLI/library writers over the loaded graph/dataset.
    // Do not sort lines or relabel blank nodes here: this is syntax C14N, not RDFC-1.0.
    let rendered = if triples {
        let triples = parsed
            .iter_ids()
            .map(|[s, p, o]| {
                Ok(oxrdf::Triple {
                    subject: parsed
                        .dict()
                        .term(s)
                        .try_into()
                        .map_err(|e| format!("{e}"))?,
                    predicate: parsed
                        .dict()
                        .term(p)
                        .try_into()
                        .map_err(|e| format!("{e}"))?,
                    object: parsed.dict().term(o),
                })
            })
            .collect::<Result<Vec<_>, String>>();
        match triples {
            Ok(triples) => sparq_engine::triples_to_ntriples(&triples),
            Err(e) => return Outcome::Fail(format!("writer input: {e}")),
        }
    } else {
        sparq_engine::serialize::graph_to_nquads(&parsed)
    };
    if kind == "PositiveC14N" {
        return if rendered.as_bytes() == expected {
            Outcome::Pass
        } else {
            Outcome::Fail("canonical syntax differs byte-for-byte".into())
        };
    }
    let dataset = |bytes: &[u8]| -> Result<Dataset, String> {
        let quads = oxttl::NQuadsParser::new()
            .for_slice(bytes)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut ds: Dataset = quads.iter().collect();
        // Includes blank nodes nested inside triple terms, under the SAME bijection.
        ds.canonicalize(CanonicalizationAlgorithm::Unstable);
        Ok(ds)
    };
    match (dataset(rendered.as_bytes()), dataset(&expected)) {
        (Ok(got), Ok(want)) if got == want => Outcome::Pass,
        (Ok(_), Ok(_)) => Outcome::Fail("parsed dataset not isomorphic to reference".into()),
        (Err(e), _) | (_, Err(e)) => Outcome::Fail(format!("comparison parse error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line_syntax::{self, LineSuite};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "sparq-earl-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn manifest(f: &Fixture, stem: &str, entries: &str) -> std::path::PathBuf {
        f.write("manifest.ttl", format!(r#"
            @prefix mf: <{MF}> . @prefix rdft: <{RDFT}> .
            @prefix : <https://example.test/manifest#> .
            :manifest a mf:Manifest; mf:assumedTestBase <https://example.test/data/>;
                mf:entries ( :ok :missing :unknown :withdrawn ) .
            :ok a rdft:{stem}Eval; mf:name "a display name"; mf:action <input>; mf:result <expected.nq> .
            :missing a rdft:{stem}NegativeSyntax .
            :unknown a rdft:NewTestKind; mf:action <input> .
            :withdrawn a rdft:{stem}PositiveSyntax; rdft:approval rdft:Withdrawn .
            :unlisted a rdft:{stem}PositiveSyntax; mf:action <input> .
            {entries}
        "#))
    }

    #[test]
    fn manifest_preserves_ids_bases_and_unrunnable_entries() {
        for stem in ["TestTurtle", "TestTrig"] {
            let f = Fixture::new();
            f.write("input", "<s> <p> <o> .");
            f.write("expected.nq", "<https://example.test/data/s> <https://example.test/data/p> <https://example.test/data/o> .\n");
            let path = manifest(&f, stem, "");
            let run = |base| {
                if stem == "TestTurtle" {
                    crate::turtle_suite::run_manifest(&path, base)
                } else {
                    line_syntax::run_manifest(LineSuite::TriG, &path, base)
                }
            };
            let rows = run(None).unwrap();
            assert_eq!(rows.len(), 3, "only active listed entries");
            assert_eq!(rows[0].0, "https://example.test/manifest#ok");
            assert_eq!(rows[0].1.name, "a display name");
            assert!(matches!(rows[0].1.outcome, Outcome::Pass));
            assert!(
                matches!(rows[1].1.outcome, Outcome::Fail(_)),
                "missing action is a failure"
            );
            assert!(
                matches!(rows[2].1.outcome, Outcome::Fail(_)),
                "unknown kind is a failure"
            );
            assert!(
                matches!(
                    run(Some("https://wrong.example/")).unwrap()[0].1.outcome,
                    Outcome::Fail(_)
                ),
                "explicit base override must be used"
            );
        }
    }

    #[test]
    fn rdfxml_uses_shipping_parser_and_isomorphic_eval() {
        let f = Fixture::new();
        f.write("input", r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:e="https://example.test/">
            <rdf:Description rdf:about="s"><e:p rdf:nodeID="a"/></rdf:Description>
            <rdf:Description rdf:nodeID="a"><e:p rdf:resource="o"/></rdf:Description></rdf:RDF>"#);
        f.write("expected.nq", "<https://example.test/data/s> <https://example.test/p> _:renamed .\n_:renamed <https://example.test/p> <https://example.test/data/o> .\n");
        let path = manifest(&f, "TestXML", "");
        let run = || line_syntax::run_manifest(LineSuite::RdfXml, &path, None).unwrap();
        let rows = run();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0, "https://example.test/manifest#ok");
        assert_eq!(
            matches!(rows[0].1.outcome, Outcome::Pass),
            cfg!(feature = "rdfxml")
        );
        assert!(
            matches!(rows[1].1.outcome, Outcome::Fail(_)),
            "missing action fails"
        );
        f.write("expected.nq", "<https://example.test/data/s> <https://example.test/p> _:renamed .\n_:renamed <https://example.test/p> <https://example.test/different> .\n");
        assert!(
            matches!(run()[0].1.outcome, Outcome::Fail(_)),
            "different graph fails"
        );
        f.write("input", "<not-closed");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("TestXMLEval", "TestXMLNegativeSyntax");
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            matches!(run()[0].1.outcome, Outcome::Pass),
            cfg!(feature = "rdfxml"),
            "disabled parser cannot pass negative syntax"
        );
    }

    #[test]
    fn negative_fixture_errors_are_failures() {
        let f = Fixture::new();
        for suite in [LineSuite::NTriples, LineSuite::NQuads, LineSuite::TriG] {
            let parse: Parse = match suite {
                LineSuite::RdfXml => unreachable!(),
                LineSuite::NTriples => {
                    |t, b| line_syntax::parse_sparq_graph(LineSuite::NTriples, t, b)
                }
                LineSuite::NQuads => |t, b| line_syntax::parse_sparq_graph(LineSuite::NQuads, t, b),
                LineSuite::TriG => |t, b| line_syntax::parse_sparq_graph(LineSuite::TriG, t, b),
            };
            for kind in ["NegativeSyntax", "NegativeEval"] {
                assert!(matches!(
                    evaluate(
                        &f.0.join("absent"),
                        None,
                        "https://example.test/",
                        kind,
                        parse,
                        suite == LineSuite::NTriples,
                    ),
                    Outcome::Fail(_)
                ));
                let bad_utf8 = f.write("bad-utf8", [0xff]);
                assert!(matches!(
                    evaluate(
                        &bad_utf8,
                        None,
                        "https://example.test/",
                        kind,
                        parse,
                        suite == LineSuite::NTriples
                    ),
                    Outcome::Fail(_)
                ));
                let invalid = f.write("invalid", "<s>");
                assert!(matches!(
                    evaluate(
                        &invalid,
                        None,
                        "https://example.test/",
                        kind,
                        parse,
                        suite == LineSuite::NTriples
                    ),
                    Outcome::Pass
                ));
            }
        }
    }

    #[test]
    fn canonical_form_is_byte_comparison() {
        for (suite, named) in [
            (LineSuite::NTriples, false),
            (LineSuite::NQuads, false),
            (LineSuite::NQuads, true),
        ] {
            let f = Fixture::new();
            let stem = if suite == LineSuite::NTriples {
                "TestNTriples"
            } else {
                "TestNQuads"
            };
            let graph = if named {
                " <https://example.test/g>"
            } else {
                ""
            };
            f.write("input", format!(r#"<https://example.test/s>  <https://example.test/p> "\u0041"@EN{graph} . # comment"#));
            let expected =
                format!("<https://example.test/s> <https://example.test/p> \"A\"@en{graph} .\n");
            f.write("expected.nq", &expected);
            let path = manifest(&f, stem, &format!(":ok a rdft:{stem}PositiveC14N ."));
            // Replace the Eval type so this entry exercises the C14N branch alone.
            let text = std::fs::read_to_string(&path).unwrap().replace(
                &format!("rdft:{stem}Eval"),
                &format!("rdft:{stem}PositiveC14N"),
            );
            std::fs::write(&path, text).unwrap();
            assert!(matches!(
                line_syntax::run_manifest(suite, &path, None).unwrap()[0]
                    .1
                    .outcome,
                Outcome::Pass
            ));
            f.write("expected.nq", expected.trim_end());
            assert!(
                matches!(
                    line_syntax::run_manifest(suite, &path, None).unwrap()[0]
                        .1
                        .outcome,
                    Outcome::Fail(_)
                ),
                "missing final LF must fail even for equivalent RDF"
            );
        }
    }

    #[test]
    fn eval_renames_nested_blank_nodes_in_one_dataset() {
        let f = Fixture::new();
        f.write("input", "_:a <https://example.test/p> <<( _:a <https://example.test/p> _:b )>> . _:b <https://example.test/p> _:a .");
        f.write("expected.nq", "_:x <https://example.test/p> <<( _:x <https://example.test/p> _:y )>> .\n_:y <https://example.test/p> _:x .\n");
        let path = manifest(&f, "TestTurtle", "");
        assert!(matches!(
            crate::turtle_suite::run_manifest(&path, None).unwrap()[0]
                .1
                .outcome,
            Outcome::Pass
        ));
        f.write("expected.nq", "_:x <https://example.test/p> <<( _:y <https://example.test/p> _:y )>> .\n_:y <https://example.test/p> _:x .\n");
        assert!(
            matches!(
                crate::turtle_suite::run_manifest(&path, None).unwrap()[0]
                    .1
                    .outcome,
                Outcome::Fail(_)
            ),
            "one bijection must cover both nested and top-level blank nodes"
        );
    }
}
