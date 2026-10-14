//! sq-v7li4 — metamorphic laws for the lean `K:` keyword layer.

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use sparq_terse::{legend, terse_to_sparql, TerseError, LEGEND_VERSION};

fn runner() -> TestRunner {
    TestRunner::new_with_rng(
        Config {
            cases: 256,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &[0x56; 32]),
    )
}

fn source(template: u8, names: &[&str]) -> String {
    let triples = names
        .iter()
        .enumerate()
        .map(|(index, name)| format!("<urn:s{index}> K:{name} <urn:o{index}> ."))
        .collect::<Vec<_>>()
        .join(" ");
    match template % 3 {
        0 => format!("SELECT * WHERE {{ {triples} }}"),
        1 => format!("ASK WHERE {{ {triples} }}"),
        _ => format!("CONSTRUCT {{ ?s <urn:copied> ?o }} WHERE {{ {triples} }}"),
    }
}

#[test]
fn legend_expansion_is_parseable_echoed_and_idempotent() {
    let entries = legend();
    let strategy = (0u8..3, prop::collection::vec(any::<bool>(), entries.len()))
        .prop_filter("select at least one legend keyword", |(_, selected)| {
            selected.iter().any(|selected| *selected)
        });

    runner()
        .run(&strategy, |(template, selected)| {
            let chosen = entries
                .iter()
                .zip(selected)
                .filter_map(|((name, iri), selected)| selected.then_some((*name, iri)))
                .collect::<Vec<_>>();
            let names = chosen.iter().map(|(name, _)| *name).collect::<Vec<_>>();

            let expansion = terse_to_sparql(&source(template, &names))?;
            prop_assert_eq!(expansion.keywords.len(), chosen.len());
            for (echo, (name, iri)) in expansion.keywords.iter().zip(&chosen) {
                prop_assert_eq!(echo.keyword.as_str(), *name);
                prop_assert_eq!(echo.iri.as_str(), iri.as_str());
                prop_assert_eq!(echo.legend_version.as_str(), LEGEND_VERSION);
            }

            let second = terse_to_sparql(&expansion.canonical_sparql)?;
            prop_assert_eq!(&second.canonical_sparql, &expansion.canonical_sparql);
            prop_assert!(second.keywords.is_empty());
            Ok(())
        })
        .unwrap();
}

#[test]
fn unknown_keywords_and_real_k_prefixes_fail_closed() {
    let entries = legend();
    runner()
        .run(&(any::<u32>(), 0usize..entries.len()), |(suffix, index)| {
            let unknown = format!("notInLegend{suffix}");
            let unknown_src = format!("ASK WHERE {{ <urn:s> K:{unknown} <urn:o> }}");
            match terse_to_sparql(&unknown_src) {
                Err(TerseError::UnknownKeyword {
                    keyword,
                    legend_version,
                    ..
                }) => {
                    prop_assert_eq!(keyword, unknown);
                    prop_assert_eq!(legend_version, LEGEND_VERSION);
                }
                other => prop_assert!(false, "expected UnknownKeyword, got {other:?}"),
            }

            let name = entries[index].0;
            let collision = format!(
                "PREFIX K: <urn:real-prefix:> SELECT * WHERE {{ <urn:s> K:{name} <urn:o> }}"
            );
            match terse_to_sparql(&collision) {
                Err(TerseError::KeywordPrefixCollision { keyword }) => {
                    prop_assert_eq!(keyword, name);
                }
                other => prop_assert!(false, "expected KeywordPrefixCollision, got {other:?}"),
            }
            Ok(())
        })
        .unwrap();
}
