//! Exact WAC parity for the independent-dimension paper corpus.

use std::collections::BTreeSet;

use oxrdf::NamedNode;
use sparq_acbench::deployment::{
    generate, DeploymentAudienceMix, DeploymentDomain, DeploymentParams, PrincipalClass,
};
use sparq_engine::QueryResult;
use sparq_solid::{Mode, PodStore, Session};

fn all_audiences_params() -> DeploymentParams {
    DeploymentParams {
        pods: 1,
        documents_per_pod: 30,
        triples_per_document: 8,
        own_acl_coverage: 500,
        audience_mix: DeploymentAudienceMix {
            public: 300,
            private: 400,
            shared: 300,
        },
        ..DeploymentParams::smoke()
    }
}

fn row_multiset(result: &QueryResult) -> Vec<String> {
    let mut rows: Vec<_> = result
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| {
                    cell.as_ref()
                        .map_or_else(|| "unbound".to_owned(), ToString::to_string)
                })
                .collect::<Vec<_>>()
                .join("\u{1f}")
        })
        .collect();
    rows.sort_unstable();
    rows
}

#[test]
fn generated_wac_decisions_equal_the_independent_oracle() {
    let corpus = generate(&all_audiences_params()).expect("valid corpus");
    let graph = sparq_core::Graph::load_dataset(
        &corpus.pod_dataset_nquads(0).expect("Pod zero exists"),
        "nquads",
    )
    .expect("generated N-Quads parse");
    let mut store = PodStore::new(graph);
    store.materialize_wac().expect("generated WAC materializes");
    let pod = &corpus.pods[0];
    let cases = [
        (
            PrincipalClass::Owner(0),
            Session {
                agent: Some(&pod.owner_webid),
                client: None,
                issuer: None,
                now: None,
            },
        ),
        (
            PrincipalClass::Recipient(0),
            Session {
                agent: Some(&pod.recipient_webid),
                client: None,
                issuer: None,
                now: None,
            },
        ),
        (
            PrincipalClass::Stranger,
            Session {
                agent: Some("https://identities.example/stranger#me"),
                client: None,
                issuer: None,
                now: None,
            },
        ),
        (PrincipalClass::Anonymous, Session::default()),
    ];

    let content_iris: BTreeSet<_> = corpus
        .documents
        .iter()
        .map(|document| document.iri.as_str())
        .collect();
    for (principal, session) in cases {
        let accessible = store.accessible(&session, Mode::Read);
        let actual: BTreeSet<_> = accessible
            .iter()
            .map(NamedNode::as_str)
            .filter(|iri| content_iris.contains(iri))
            .collect();
        let expected: BTreeSet<_> = corpus
            .readable_documents(principal)
            .into_iter()
            .map(|document| document.iri.as_str())
            .collect();
        assert_eq!(actual, expected, "WAC/oracle mismatch for {principal:?}");

        let type_iri = "https://schema.org/SocialMediaPosting";
        let query = format!("SELECT ?g WHERE {{ GRAPH ?g {{ ?s a <{type_iri}> }} }} ORDER BY ?g");
        let rows = store
            .query_as(&session, Mode::Read, &query)
            .expect("authorized query succeeds");
        let result_graphs: BTreeSet<_> = rows
            .rows
            .iter()
            .map(|row| {
                row[0]
                    .as_ref()
                    .expect("?g is bound")
                    .to_string()
                    .trim_start_matches('<')
                    .trim_end_matches('>')
                    .to_owned()
            })
            .collect();
        let expected_graphs: BTreeSet<_> = expected.iter().map(|iri| (*iri).to_owned()).collect();
        assert_eq!(
            result_graphs, expected_graphs,
            "query result mismatch for {principal:?}"
        );
    }
}

#[test]
fn zero_and_full_own_acl_coverage_preserve_decisions() {
    let mut without_params = all_audiences_params();
    without_params.own_acl_coverage = 0;
    let without = generate(&without_params).expect("valid zero-coverage corpus");
    let mut with_params = without_params;
    with_params.own_acl_coverage = 1_000;
    let with = generate(&with_params).expect("valid full-coverage corpus");

    for corpus in [&without, &with] {
        let graph = sparq_core::Graph::load_dataset(
            &corpus.pod_dataset_nquads(0).expect("Pod zero exists"),
            "nquads",
        )
        .expect("generated N-Quads parse");
        let mut store = PodStore::new(graph);
        store.materialize_wac().expect("generated WAC materializes");
        let owner = Session {
            agent: Some(&corpus.pods[0].owner_webid),
            client: None,
            issuer: None,
            now: None,
        };
        for document in &corpus.documents {
            assert!(
                store.decide(&owner, &document.iri, Mode::Read).allow,
                "owner must read {}",
                document.iri
            );
        }
    }
}

#[test]
fn every_query_family_matches_a_physically_filtered_reference_bag() {
    for domain in [DeploymentDomain::Social, DeploymentDomain::Health] {
        let mut params = all_audiences_params();
        params.domain = domain;
        let corpus = generate(&params).expect("valid corpus");
        let graph = sparq_core::Graph::load_dataset(
            &corpus.pod_dataset_nquads(0).expect("Pod zero exists"),
            "nquads",
        )
        .expect("generated N-Quads parse");
        let mut store = PodStore::new(graph);
        store.materialize_wac().expect("generated WAC materializes");
        let pod = &corpus.pods[0];
        let cases = [
            (
                PrincipalClass::Owner(0),
                Session {
                    agent: Some(pod.owner_webid.as_str()),
                    client: None,
                    issuer: None,
                    now: None,
                },
            ),
            (
                PrincipalClass::Recipient(0),
                Session {
                    agent: Some(pod.recipient_webid.as_str()),
                    client: None,
                    issuer: None,
                    now: None,
                },
            ),
            (
                PrincipalClass::Stranger,
                Session {
                    agent: Some("https://identities.example/stranger#me"),
                    client: None,
                    issuer: None,
                    now: None,
                },
            ),
            (PrincipalClass::Anonymous, Session::default()),
        ];
        let queries = corpus.benchmark_queries(0).expect("Pod zero exists");
        for (principal, session) in cases {
            let reference = sparq_core::Graph::load_dataset(
                &corpus
                    .pod_readable_content_nquads(0, principal)
                    .expect("Pod zero exists"),
                "nquads",
            )
            .expect("reference N-Quads parse");
            for query in &queries {
                let actual = store
                    .query_as(&session, Mode::Read, &query.sparql)
                    .unwrap_or_else(|error| panic!("{} failed: {error}", query.id));
                let expected = sparq_engine::query(&reference, &query.sparql)
                    .unwrap_or_else(|error| panic!("reference {} failed: {error}", query.id));
                assert_eq!(actual.vars, expected.vars, "{} variables", query.id);
                assert_eq!(
                    row_multiset(&actual),
                    row_multiset(&expected),
                    "{domain:?}/{principal:?}/{} result bag",
                    query.id
                );
            }
        }
    }
}

#[test]
fn selective_star_is_non_empty_for_every_primary_seed() {
    for domain in [DeploymentDomain::Social, DeploymentDomain::Health] {
        for seed in [17, 42, 101, 314, 2718] {
            let params = DeploymentParams {
                seed,
                pods: 1,
                documents_per_pod: 16,
                triples_per_document: 8,
                audience_mix: DeploymentAudienceMix {
                    public: 0,
                    private: 1_000,
                    shared: 0,
                },
                domain,
                ..DeploymentParams::smoke()
            };
            let corpus = generate(&params).expect("valid primary corpus");
            let reference = sparq_core::Graph::load_dataset(
                &corpus
                    .pod_readable_content_nquads(0, PrincipalClass::Owner(0))
                    .expect("Pod zero exists"),
                "nquads",
            )
            .expect("reference N-Quads parse");
            let query = &corpus.benchmark_queries(0).expect("Pod zero exists")[1];
            let rows = sparq_engine::query(&reference, &query.sparql)
                .expect("selective-star query succeeds")
                .rows
                .len();
            assert!(
                (3..=5).contains(&rows),
                "{domain:?} seed {seed} selected {rows} rows"
            );
        }
    }
}

#[test]
fn replacing_a_generated_own_acl_revokes_the_recipient_immediately() {
    let mut params = all_audiences_params();
    params.own_acl_coverage = 1_000;
    params.audience_mix = DeploymentAudienceMix {
        public: 0,
        private: 0,
        shared: 1_000,
    };
    let corpus = generate(&params).expect("valid all-shared corpus");
    let graph = sparq_core::Graph::load_dataset(
        &corpus.pod_dataset_nquads(0).expect("Pod zero exists"),
        "nquads",
    )
    .expect("generated N-Quads parse");
    let mut store = PodStore::new(graph);
    store.materialize_wac().expect("generated WAC materializes");
    let pod = &corpus.pods[0];
    let recipient = Session {
        agent: Some(&pod.recipient_webid),
        client: None,
        issuer: None,
        now: None,
    };
    let owner = Session {
        agent: Some(&pod.owner_webid),
        client: None,
        issuer: None,
        now: None,
    };
    let document = corpus
        .documents
        .first()
        .expect("generated corpus has content");
    assert!(store.decide(&recipient, &document.iri, Mode::Read).allow);

    let acl_iri = format!("{}.acl", document.iri);
    let subject = format!("{acl_iri}#owner-only");
    let replacement = format!(
        "<{subject}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> .\n\
         <{subject}> <http://www.w3.org/ns/auth/acl#agent> <{}> .\n\
         <{subject}> <http://www.w3.org/ns/auth/acl#accessTo> <{}> .\n\
         <{subject}> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> .\n",
        pod.owner_webid, document.iri
    );
    store
        .put_acl(&acl_iri, &replacement, "ntriples")
        .expect("valid own ACL replacement");

    assert!(store.decide(&owner, &document.iri, Mode::Read).allow);
    assert!(!store.decide(&recipient, &document.iri, Mode::Read).allow);
    let query = format!(
        "SELECT ?item WHERE {{ GRAPH <{}> {{ ?item a <https://schema.org/SocialMediaPosting> }} }}",
        document.iri
    );
    assert_eq!(
        store
            .query_as(&recipient, Mode::Read, &query)
            .expect("revoked query remains valid")
            .rows
            .len(),
        0
    );
}
