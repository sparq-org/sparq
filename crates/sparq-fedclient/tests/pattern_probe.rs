//! Recall-safety and effectiveness witnesses for pattern-level probing. sq-fx5id.

#![cfg(feature = "pattern_probe")]

use sparq_core::Graph;
use sparq_engine::{json::to_sparql_json, query};
use sparq_fedclient::discovery::Fetcher;
use sparq_fedclient::{
    materialize_multi_source, select_sources_with_pattern_probes, solutions_equal, Endpoint,
    FederatedSource, PatternProbeConfig, PatternProbeSession, ProbeSource, Relation,
    SourceResolver, Transport,
};
use sparq_fedplan::{
    plan_bgp, Bgp, PatternSources, PlanOptions, PredPartition, SourceCandidate, SourceDescriptor,
    SourceId, Term, TriplePattern, Var,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct ProbeFetcher {
    requests: AtomicUsize,
}

impl ProbeFetcher {
    fn new() -> Self {
        Self {
            requests: AtomicUsize::new(0),
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Fetcher for ProbeFetcher {
    fn get(&self, url: &str, _accept: &str) -> Result<String, String> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        if url.contains("timeout.example") {
            return Err("simulated timeout".into());
        }
        if url.contains("empty.example") || url.contains("8.8.8.8") {
            return Ok(r#"{"boolean":false}"#.into());
        }
        if url.contains("ASK%20") {
            return Ok(r#"{"boolean":true}"#.into());
        }
        Ok(r#"{"head":{"vars":["s"]},"results":{"bindings":[{"s":{"type":"uri","value":"http://ex/a"}},{"s":{"type":"uri","value":"http://ex/b"}}]}}"#.into())
    }
}

fn iri(value: &str) -> Term {
    Term::Iri(value.into())
}

fn var(name: &str) -> Term {
    Term::Var(Var::new(name))
}

fn pattern() -> TriplePattern {
    TriplePattern::new(var("s"), iri("http://ex/p"), var("o"))
}

fn config(max_probes: usize) -> PatternProbeConfig {
    PatternProbeConfig {
        max_probes,
        cardinality_cap: 3,
        fallback_cardinality: 777.0,
    }
}

#[test]
fn definitive_false_prunes_but_timeout_keeps_and_count_replaces_fallback() {
    let fetcher = ProbeFetcher::new();
    let mut session = PatternProbeSession::new(&fetcher, config(8));
    let bgp = Bgp::new(vec![pattern()]);
    let selection = select_sources_with_pattern_probes(
        &bgp,
        &[
            ProbeSource {
                endpoint: "https://empty.example/sparql",
                descriptor: None,
            },
            ProbeSource {
                endpoint: "https://timeout.example/sparql",
                descriptor: None,
            },
            ProbeSource {
                endpoint: "https://live.example/sparql",
                descriptor: None,
            },
        ],
        &mut session,
    );

    assert_eq!(
        selection[0].candidates.len(),
        2,
        "only exact ASK false prunes"
    );
    assert_eq!(selection[0].candidates[0].source, 1);
    assert_eq!(selection[0].candidates[0].estimated_cardinality, 777.0);
    assert_eq!(selection[0].candidates[1].source, 2);
    assert_eq!(selection[0].candidates[1].estimated_cardinality, 2.0);
    assert_eq!(session.stats().requests_issued, 4);
    assert_eq!(fetcher.requests(), 4);
}

#[test]
fn duplicate_pattern_is_cached_and_budget_counts_requests() {
    let fetcher = ProbeFetcher::new();
    let mut session = PatternProbeSession::new(&fetcher, config(2));
    let bgp = Bgp::new(vec![pattern(), pattern()]);
    let selection = select_sources_with_pattern_probes(
        &bgp,
        &[ProbeSource {
            endpoint: "https://live.example/sparql",
            descriptor: None,
        }],
        &mut session,
    );
    assert_eq!(selection[0].candidates[0].estimated_cardinality, 2.0);
    assert_eq!(selection[1].candidates[0].estimated_cardinality, 2.0);
    assert_eq!(session.stats().requests_issued, 2);
    assert_eq!(session.stats().cache_hits, 1);
    assert_eq!(
        fetcher.requests(),
        2,
        "cache hit must not reissue per binding/pattern"
    );

    let mut exhausted = PatternProbeSession::new(&fetcher, config(1));
    let selected = select_sources_with_pattern_probes(
        &Bgp::new(vec![pattern()]),
        &[ProbeSource {
            endpoint: "https://live.example/sparql",
            descriptor: None,
        }],
        &mut exhausted,
    );
    assert_eq!(selected[0].candidates[0].estimated_cardinality, 777.0);
    assert_eq!(exhausted.stats().requests_issued, 1);
    assert!(exhausted.stats().budget_exhausted);
}

#[test]
fn served_void_cardinality_skips_live_probe() {
    let descriptor = SourceDescriptor::builder(SourceId::new("served"))
        .predicate(PredPartition {
            predicate: "http://ex/p".into(),
            triples: 23,
            distinct_subjects: 11,
            distinct_objects: 17,
        })
        .build();
    let fetcher = ProbeFetcher::new();
    let mut session = PatternProbeSession::new(&fetcher, config(8));
    let selection = select_sources_with_pattern_probes(
        &Bgp::new(vec![pattern()]),
        &[ProbeSource {
            endpoint: "https://live.example/sparql",
            descriptor: Some(&descriptor),
        }],
        &mut session,
    );
    assert_eq!(selection[0].candidates[0].estimated_cardinality, 23.0);
    assert_eq!(session.stats().requests_issued, 0);
    assert_eq!(fetcher.requests(), 0);
}

struct EngineTransport {
    graph: Arc<Graph>,
}

impl Transport for EngineTransport {
    fn fetch(&self, _endpoint: &str, query_text: &str) -> Result<String, String> {
        Ok(to_sparql_json(&query(&self.graph, query_text)?))
    }
}

fn descriptor(id: &str) -> SourceDescriptor {
    SourceDescriptor::builder(SourceId::new(id))
        .total_triples(100)
        .predicate(PredPartition {
            predicate: "http://ex/p".into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .predicate(PredPartition {
            predicate: "http://ex/q".into(),
            triples: 10,
            distinct_subjects: 5,
            distinct_objects: 5,
        })
        .build()
}

fn unprobed_selection(bgp: &Bgp) -> Vec<PatternSources> {
    bgp.patterns
        .iter()
        .enumerate()
        .map(|(pattern, _)| PatternSources {
            pattern,
            candidates: vec![
                SourceCandidate {
                    source: 0,
                    estimated_cardinality: 777.0,
                },
                SourceCandidate {
                    source: 1,
                    estimated_cardinality: 777.0,
                },
            ],
        })
        .collect()
}

fn execute(
    bgp: &Bgp,
    selection: &[PatternSources],
    descriptors: &[SourceDescriptor],
    endpoints: &[Endpoint],
) -> Relation {
    let plan = plan_bgp(bgp, selection, descriptors, &PlanOptions::default()).unwrap();
    let adapters: Vec<&dyn FederatedSource> = endpoints
        .iter()
        .map(|endpoint| endpoint as &dyn FederatedSource)
        .collect();
    let resolver = SourceResolver::new(bgp, &adapters);
    materialize_multi_source(&resolver, selection, &plan).unwrap()
}

fn assert_probe_preserves_multiset(bgp: Bgp) {
    let empty = Arc::new(Graph::new());
    let live = Arc::new(
        Graph::load_str(
            "@prefix ex: <http://ex/> . ex:a ex:p ex:b . ex:b ex:q ex:c .",
            "turtle",
        )
        .unwrap(),
    );
    let endpoints = vec![
        Endpoint::new(
            "https://8.8.8.8/sparql",
            Box::new(EngineTransport { graph: empty }),
        ),
        Endpoint::new(
            "https://1.1.1.1/sparql",
            Box::new(EngineTransport { graph: live }),
        ),
    ];
    let descriptors = vec![descriptor("empty"), descriptor("live")];
    let baseline = unprobed_selection(&bgp);

    let fetcher = ProbeFetcher::new();
    let mut session = PatternProbeSession::new(&fetcher, config(32));
    let probed = select_sources_with_pattern_probes(
        &bgp,
        &[
            ProbeSource {
                endpoint: "https://8.8.8.8/sparql",
                descriptor: None,
            },
            ProbeSource {
                endpoint: "https://1.1.1.1/sparql",
                descriptor: None,
            },
        ],
        &mut session,
    );

    assert!(probed
        .iter()
        .all(|sources| { sources.candidates.len() == 1 && sources.candidates[0].source == 1 }));
    let without_feature = execute(&bgp, &baseline, &descriptors, &endpoints);
    let with_feature = execute(&bgp, &probed, &descriptors, &endpoints);
    assert!(solutions_equal(
        &without_feature.vars,
        &without_feature.rows,
        &with_feature.vars,
        &with_feature.rows,
    ));
}

#[test]
fn probed_and_unprobed_results_match_across_federation_fixtures() {
    assert_probe_preserves_multiset(Bgp::new(vec![pattern()]));
    assert_probe_preserves_multiset(Bgp::new(vec![
        pattern(),
        TriplePattern::new(var("o"), iri("http://ex/q"), var("z")),
    ]));
}
