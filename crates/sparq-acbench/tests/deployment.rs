//! Independent-dimension many-Pod corpus invariants.

use std::collections::BTreeSet;

use sparq_acbench::deployment::{
    generate, DeploymentAudienceMix, DeploymentDomain, DeploymentParams, DocumentAudience,
    OriginTopology, PrincipalClass,
};

fn params() -> DeploymentParams {
    DeploymentParams::smoke()
}

#[test]
fn validation_rejects_zero_counts_bad_depth_and_bad_mix() {
    let mut p = params();
    p.pods = 0;
    assert!(p.validate().is_err());
    p = params();
    p.documents_per_pod = 0;
    assert!(p.validate().is_err());
    p = params();
    p.triples_per_document = 0;
    assert!(p.validate().is_err());
    p = params();
    p.container_depth = 7;
    assert!(p.validate().is_err());
    p = params();
    p.own_acl_coverage = 1_001;
    assert!(p.validate().is_err());
    p = params();
    p.audience_mix = DeploymentAudienceMix {
        public: 100,
        private: 700,
        shared: 199,
    };
    assert!(p.validate().is_err());
    p = params();
    p.pods = u32::MAX;
    p.documents_per_pod = u32::MAX;
    p.triples_per_document = 2;
    assert!(
        p.validate().is_err(),
        "the total triple count must not wrap"
    );
}

#[test]
fn identical_params_produce_identical_bytes() {
    let a = generate(&params()).expect("valid corpus");
    let b = generate(&params()).expect("valid corpus");
    assert_eq!(a, b);
    assert_eq!(a.dataset_nquads(), b.dataset_nquads());
}

#[test]
fn increasing_pods_leaves_existing_pods_byte_identical() {
    let small = generate(&params()).expect("valid corpus");
    let mut larger_params = params();
    larger_params.pods += 5;
    let large = generate(&larger_params).expect("valid corpus");
    assert_eq!(small.pods, large.pods[..small.pods.len()]);
    assert_eq!(small.documents, large.documents[..small.documents.len()]);
    let small_controls: Vec<_> = small
        .control_documents
        .iter()
        .filter(|document| document.pod_index < small.params.pods)
        .collect();
    let large_controls: Vec<_> = large
        .control_documents
        .iter()
        .filter(|document| document.pod_index < small.params.pods)
        .collect();
    assert_eq!(small_controls, large_controls);
}

#[test]
fn counts_are_independent_and_every_document_has_exact_triples() {
    let mut p = params();
    p.pods = 3;
    p.documents_per_pod = 11;
    p.triples_per_document = 13;
    p.container_depth = 4;
    let corpus = generate(&p).expect("valid corpus");
    assert_eq!(corpus.pods.len(), 3);
    assert_eq!(corpus.documents.len(), 33);
    assert_eq!(corpus.containers.len(), 3 * (1 + 3 * 4));
    for document in &corpus.documents {
        assert_eq!(document.ntriples.lines().count(), 13);
        let unique: BTreeSet<_> = document.ntriples.lines().collect();
        assert_eq!(unique.len(), 13, "RDF statements must not collapse");
    }
}

#[test]
fn container_depth_is_both_containment_and_lexical_ancestry() {
    let mut p = params();
    p.pods = 1;
    p.container_depth = 6;
    let corpus = generate(&p).expect("valid corpus");
    for container in &corpus.containers {
        if let Some(parent) = &container.parent_iri {
            assert!(
                container.iri.starts_with(parent),
                "{} must be lexically below {}",
                container.iri,
                parent
            );
        }
    }
    for document in &corpus.documents {
        let category = match document.audience {
            DocumentAudience::Public => "public",
            DocumentAudience::Private => "private",
            DocumentAudience::Shared => "shared",
        };
        let expected_prefix = format!(
            "{}{category}/level-1/level-2/level-3/level-4/level-5/",
            corpus.pods[0].root_iri
        );
        assert!(document.iri.starts_with(&expected_prefix));
    }
}

#[test]
fn topology_changes_authorities_without_changing_local_shape() {
    let mut shared_params = params();
    shared_params.pods = 4;
    shared_params.topology = OriginTopology::SharedOrigin;
    let shared = generate(&shared_params).expect("valid shared topology");
    let shared_authorities: BTreeSet<_> = shared
        .pods
        .iter()
        .map(|pod| pod.root_iri.split('/').nth(2).expect("absolute IRI"))
        .collect();
    assert_eq!(shared_authorities.len(), 1);

    let mut separate_params = shared_params;
    separate_params.topology = OriginTopology::OriginPerPod;
    let separate = generate(&separate_params).expect("valid separate topology");
    let separate_authorities: BTreeSet<_> = separate
        .pods
        .iter()
        .map(|pod| pod.root_iri.split('/').nth(2).expect("absolute IRI"))
        .collect();
    assert_eq!(separate_authorities.len(), 4);
    assert_eq!(shared.documents.len(), separate.documents.len());
    assert!(shared
        .documents
        .iter()
        .zip(&separate.documents)
        .all(|(a, b)| a.audience == b.audience
            && a.ntriples.lines().count() == b.ntriples.lines().count()));
}

#[test]
fn own_acl_coverage_changes_placement_but_not_oracle_decisions() {
    let mut none_params = params();
    none_params.own_acl_coverage = 0;
    let none = generate(&none_params).expect("valid zero coverage");
    assert!(none.documents.iter().all(|document| !document.has_own_acl));

    let mut all_params = none_params;
    all_params.own_acl_coverage = 1_000;
    let all = generate(&all_params).expect("valid full coverage");
    assert!(all.documents.iter().all(|document| document.has_own_acl));
    assert!(all.control_documents.len() > none.control_documents.len());
    for (left, right) in none.documents.iter().zip(&all.documents) {
        assert_eq!(left.audience, right.audience);
        for principal in [
            PrincipalClass::Owner(left.pod_index),
            PrincipalClass::Recipient(left.pod_index),
            PrincipalClass::Stranger,
            PrincipalClass::Anonymous,
        ] {
            assert_eq!(
                none.can_read(principal, left),
                all.can_read(principal, right)
            );
        }
    }
}

#[test]
fn oracle_has_non_vacuous_allow_and_deny_cases() {
    let mut p = params();
    p.pods = 1;
    p.documents_per_pod = 3;
    p.audience_mix = DeploymentAudienceMix {
        public: 0,
        private: 0,
        shared: 1_000,
    };
    let shared = generate(&p).expect("valid all-shared corpus");
    assert!(shared
        .documents
        .iter()
        .all(|document| document.audience == DocumentAudience::Shared));
    assert_eq!(shared.readable_documents(PrincipalClass::Owner(0)).len(), 3);
    assert_eq!(
        shared
            .readable_documents(PrincipalClass::Recipient(0))
            .len(),
        3
    );
    assert!(shared
        .readable_documents(PrincipalClass::Owner(1))
        .is_empty());
    assert!(shared
        .readable_documents(PrincipalClass::Stranger)
        .is_empty());
    assert!(shared
        .readable_documents(PrincipalClass::Anonymous)
        .is_empty());

    p.audience_mix = DeploymentAudienceMix {
        public: 1_000,
        private: 0,
        shared: 0,
    };
    let public = generate(&p).expect("valid all-public corpus");
    assert_eq!(
        public.readable_documents(PrincipalClass::Anonymous).len(),
        3
    );
}

#[test]
fn every_source_document_renders_into_its_own_named_graph() {
    let corpus = generate(&params()).expect("valid corpus");
    let nquads = corpus.dataset_nquads();
    for document in &corpus.documents {
        let suffix = format!("<{}> .", document.iri);
        assert_eq!(
            nquads
                .lines()
                .filter(|line| line.ends_with(&suffix))
                .count(),
            corpus.params.triples_per_document as usize
        );
    }
    for control in &corpus.control_documents {
        let suffix = format!("<{}> .", control.iri);
        assert_eq!(
            nquads
                .lines()
                .filter(|line| line.ends_with(&suffix))
                .count(),
            control.ntriples.lines().count()
        );
    }
}

#[test]
fn domain_changes_predicates_not_controlled_counts() {
    let social = generate(&params()).expect("valid social corpus");
    let mut health_params = params();
    health_params.domain = DeploymentDomain::Health;
    let health = generate(&health_params).expect("valid health corpus");
    assert_eq!(social.documents.len(), health.documents.len());
    assert!(social.documents[0].ntriples.contains("SocialMediaPosting"));
    assert!(health.documents[0].ntriples.contains("MedicalCondition"));
    assert_eq!(
        social.documents[0].ntriples.lines().count(),
        health.documents[0].ntriples.lines().count()
    );
}

#[test]
fn pod_reference_contains_only_readable_content_from_that_pod() {
    let corpus = generate(&params()).expect("valid corpus");
    let reference = corpus
        .pod_readable_content_nquads(0, PrincipalClass::Anonymous)
        .expect("Pod zero exists");
    for document in &corpus.documents {
        let present = reference.contains(&format!("<{}> .", document.iri));
        assert_eq!(
            present,
            document.pod_index == 0 && document.audience == DocumentAudience::Public
        );
    }
    assert!(corpus
        .pod_readable_content_nquads(corpus.params.pods, PrincipalClass::Anonymous)
        .is_err());
}

#[test]
fn query_workload_has_eight_stable_families_and_declares_applicability() {
    for domain in [DeploymentDomain::Social, DeploymentDomain::Health] {
        let mut p = params();
        p.domain = domain;
        let corpus = generate(&p).expect("valid corpus");
        let queries = corpus.benchmark_queries(0).expect("Pod zero exists");
        assert_eq!(queries.len(), 8);
        assert_eq!(queries[0].id, "q1-point");
        assert_eq!(queries[7].id, "q8-graph-scan");
        assert!(queries
            .iter()
            .all(|query| !query.family.is_empty() && !query.sparql.is_empty()));
        assert!(queries
            .iter()
            .any(|query| query.minimum_triples_per_document == 8));
    }
}
