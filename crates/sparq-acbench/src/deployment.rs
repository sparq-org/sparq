//! Independent-dimension corpus for the many-Pod AC-SPARQL study.
//!
//! The legacy benchmark generators intentionally remain available through their
//! [`crate::GenParams`] scale factor.  This module is the paper-specific complement:
//! Pod count, origin topology, document count, triples per document, hierarchy depth,
//! own-ACL placement, domain vocabulary, and audience mix are independent inputs.
//! That separation is load-bearing for a scaling experiment—changing the number of
//! unrelated Pods must not silently change the requested Pod's working set.
//!
//! The generator implements a deliberately narrow WAC common subset: owner, public,
//! and one named recipient.  It does not pretend that WAC, ACP, and ODRL are
//! interchangeable.  Every content document is retained separately and renders as
//! exactly one named graph; control documents and container graphs are separate.

use std::fmt::Write as _;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const LDP_CONTAINER: &str = "http://www.w3.org/ns/ldp#Container";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";

/// Domain vocabulary used for generated content.
///
/// The variants keep counts identical while changing predicates and local graph
/// shape.  They are structural synthetic scenarios, not samples from deployed Pods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentDomain {
    /// Profile, post, author, mention, tag, and time-shaped RDF.
    Social,
    /// Patient, observation, code, value, unit, and time-shaped RDF.
    Health,
}

/// How generated Pod roots are distributed over URI authorities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OriginTopology {
    /// Every Pod is a path below `https://pod.example/`.
    SharedOrigin,
    /// Every Pod has its own `https://pod-N.example/` authority.
    OriginPerPod,
}

/// Audience assigned to one content document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DocumentAudience {
    /// Readable by every requester, including an anonymous requester.
    Public,
    /// Readable only by the owning `WebID`.
    Private,
    /// Readable by the owner and the Pod's named recipient.
    Shared,
}

/// A requester's relationship to a generated Pod.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalClass {
    /// The `WebID` that owns the Pod with this zero-based index.
    Owner(u32),
    /// The named recipient configured by the Pod with this zero-based index.
    Recipient(u32),
    /// An authenticated `WebID` with no generated grant.
    Stranger,
    /// A request without an authenticated `WebID`.
    Anonymous,
}

/// Integer audience proportions in per-mille units.
///
/// Integer weights avoid platform-dependent floating-point boundary decisions in a
/// corpus whose bytes are expected to be deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentAudienceMix {
    /// Public share in per-mille units.
    pub public: u16,
    /// Owner-private share in per-mille units.
    pub private: u16,
    /// Named-recipient share in per-mille units.
    pub shared: u16,
}

impl DeploymentAudienceMix {
    /// The controlled study's default 10% public, 70% private, 20% shared mix.
    #[must_use]
    pub const fn controlled() -> Self {
        Self {
            public: 100,
            private: 700,
            shared: 200,
        }
    }

    /// Validate that the three non-negative integer components total 1,000.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error when the total is not exactly 1,000.
    pub fn validate(self) -> Result<(), String> {
        let total = u32::from(self.public) + u32::from(self.private) + u32::from(self.shared);
        if total == 1_000 {
            Ok(())
        } else {
            Err(format!(
                "deployment audience mix totals {total} per mille, expected 1000"
            ))
        }
    }
}

/// Independent inputs to [`generate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentParams {
    /// Root seed.  Each Pod/document derives its own sub-seed, so increasing `pods`
    /// leaves all already-present Pods byte-identical.
    pub seed: u64,
    /// Number of independent Pod roots.
    pub pods: u32,
    /// Content documents in each Pod (container/control documents are additional).
    pub documents_per_pod: u32,
    /// Exact generated content triples in each content document.
    pub triples_per_document: u32,
    /// Number of category-container levels from a Pod root to every content document.
    /// Valid range: 1–6.
    pub container_depth: u8,
    /// Fraction of content documents with an own ACL, in per-mille units.  Category
    /// defaults preserve the same decision when this placement factor changes.
    pub own_acl_coverage: u16,
    /// Public/private/named-recipient mix.
    pub audience_mix: DeploymentAudienceMix,
    /// URI authority layout.
    pub topology: OriginTopology,
    /// Content vocabulary/shape.
    pub domain: DeploymentDomain,
}

impl DeploymentParams {
    /// Small deterministic configuration for unit and local smoke tests.
    #[must_use]
    pub const fn smoke() -> Self {
        Self {
            seed: 42,
            pods: 2,
            documents_per_pod: 8,
            triples_per_document: 8,
            container_depth: 3,
            own_acl_coverage: 250,
            audience_mix: DeploymentAudienceMix::controlled(),
            topology: OriginTopology::SharedOrigin,
            domain: DeploymentDomain::Social,
        }
    }

    /// Validate bounds that keep all count arithmetic and hierarchy construction safe.
    ///
    /// # Errors
    ///
    /// Returns the first invalid field with its value and accepted range.
    pub fn validate(&self) -> Result<(), String> {
        if self.pods == 0 {
            return Err("deployment pods must be greater than zero".to_owned());
        }
        if self.documents_per_pod == 0 {
            return Err("documents_per_pod must be greater than zero".to_owned());
        }
        if self.triples_per_document == 0 {
            return Err("triples_per_document must be greater than zero".to_owned());
        }
        if !(1..=6).contains(&self.container_depth) {
            return Err(format!(
                "container_depth {} outside the supported range 1..=6",
                self.container_depth
            ));
        }
        if self.own_acl_coverage > 1_000 {
            return Err(format!(
                "own_acl_coverage {} exceeds 1000 per mille",
                self.own_acl_coverage
            ));
        }
        self.audience_mix.validate()?;
        let total_documents = u64::from(self.pods)
            .checked_mul(u64::from(self.documents_per_pod))
            .ok_or_else(|| "Pod × document count overflows u64".to_owned())?;
        let _ = usize::try_from(total_documents)
            .map_err(|_| "Pod × document count does not fit this platform".to_owned())?;
        let _ = total_documents
            .checked_mul(u64::from(self.triples_per_document))
            .ok_or_else(|| "total generated triple count overflows u64".to_owned())?;
        Ok(())
    }
}

/// One generated Pod and the identities used by the independent oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentPod {
    /// Zero-based stable Pod index.
    pub index: u32,
    /// Container IRI ending in `/`.
    pub root_iri: String,
    /// Owning `WebID`.
    pub owner_webid: String,
    /// Named recipient used by shared documents.
    pub recipient_webid: String,
}

/// One generated RDF content document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDocument {
    /// Zero-based owning Pod index.
    pub pod_index: u32,
    /// LDP resource IRI and named-graph IRI.
    pub iri: String,
    /// Audience from which the independent oracle derives visibility.
    pub audience: DocumentAudience,
    /// Whether an own ACL is emitted for this resource.
    pub has_own_acl: bool,
    /// N-Triples document body.  This is also valid Turtle for the in-memory LWS store.
    pub ntriples: String,
}

/// A generated container and its direct containment parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentContainer {
    /// Zero-based owning Pod index.
    pub pod_index: u32,
    /// Container IRI ending in `/`.
    pub iri: String,
    /// Direct parent container; `None` only for a per-authority Pod root.  Under the
    /// shared-origin topology each Pod root names the deployment root as parent.
    pub parent_iri: Option<String>,
}

/// One WAC control document rendered as N-Triples/Turtle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlDocument {
    /// Zero-based owning Pod index.
    pub pod_index: u32,
    /// ACL resource IRI, also used as its named graph IRI.
    pub iri: String,
    /// Resource or container governed by the ACL.
    pub governed_iri: String,
    /// WAC triples in N-Triples syntax (a Turtle subset).
    pub ntriples: String,
}

/// Complete deterministic deployment corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentCorpus {
    /// Validated parameters that produced this corpus.
    pub params: DeploymentParams,
    /// Stable Pod descriptors in index order.
    pub pods: Vec<DeploymentPod>,
    /// Stable container descriptors in generation order.
    pub containers: Vec<DeploymentContainer>,
    /// Content documents, ordered first by Pod and then local document index.
    pub documents: Vec<ContentDocument>,
    /// Root, category, and selected own-resource ACLs.
    pub control_documents: Vec<ControlDocument>,
}

/// One deterministic query in the paper's operator-family workload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkQuery {
    /// Stable identifier used by raw records and figures.
    pub id: &'static str,
    /// Operator family named in the frozen analysis protocol.
    pub family: &'static str,
    /// Smallest `triples_per_document` for which the required predicates exist.
    pub minimum_triples_per_document: u32,
    /// SPARQL 1.1 SELECT query text.
    pub sparql: String,
}

impl DeploymentCorpus {
    /// Whether `principal` may read `document` under the generator's WAC subset.
    ///
    /// This oracle reads only ownership and the assigned audience.  It does not invoke
    /// a policy compiler, N3 reasoner, SPARQL engine, or server authorization code.
    #[must_use]
    pub fn can_read(&self, principal: PrincipalClass, document: &ContentDocument) -> bool {
        match document.audience {
            DocumentAudience::Public => true,
            DocumentAudience::Private => principal == PrincipalClass::Owner(document.pod_index),
            DocumentAudience::Shared => {
                matches!(
                    principal,
                    PrincipalClass::Owner(i) | PrincipalClass::Recipient(i)
                        if i == document.pod_index
                )
            }
        }
    }

    /// Content documents readable by `principal`, preserving corpus order.
    #[must_use]
    pub fn readable_documents(&self, principal: PrincipalClass) -> Vec<&ContentDocument> {
        self.documents
            .iter()
            .filter(|document| self.can_read(principal, document))
            .collect()
    }

    /// Render content, container, and WAC documents as one deterministic N-Quads dataset.
    ///
    /// Each source document becomes exactly one named graph.  No triples are written to
    /// the default graph.  Container marker triples are structural and are not included
    /// in `triples_per_document`.
    #[must_use]
    pub fn dataset_nquads(&self) -> String {
        let content_lines = u64::from(self.params.pods)
            .checked_mul(u64::from(self.params.documents_per_pod))
            .and_then(|value| value.checked_mul(u64::from(self.params.triples_per_document)))
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        let mut output = String::with_capacity(content_lines.saturating_mul(120));
        for container in &self.containers {
            let triple = format!("<{}> <{RDF_TYPE}> <{LDP_CONTAINER}> .\n", container.iri);
            append_graph(&mut output, &triple, &container.iri);
        }
        for document in &self.documents {
            append_graph(&mut output, &document.ntriples, &document.iri);
        }
        for document in &self.control_documents {
            append_graph(&mut output, &document.ntriples, &document.iri);
        }
        output
    }

    /// Render one Pod's content, containers, and WAC documents as N-Quads.
    ///
    /// # Errors
    ///
    /// Returns an error when `pod_index` does not identify a generated Pod.
    pub fn pod_dataset_nquads(&self, pod_index: u32) -> Result<String, String> {
        if pod_index >= self.params.pods {
            return Err(format!(
                "Pod index {pod_index} outside generated range 0..{}",
                self.params.pods
            ));
        }
        let mut output = String::new();
        for container in self
            .containers
            .iter()
            .filter(|container| container.pod_index == pod_index)
        {
            let triple = format!("<{}> <{RDF_TYPE}> <{LDP_CONTAINER}> .\n", container.iri);
            append_graph(&mut output, &triple, &container.iri);
        }
        for document in self
            .documents
            .iter()
            .filter(|document| document.pod_index == pod_index)
        {
            append_graph(&mut output, &document.ntriples, &document.iri);
        }
        for document in self
            .control_documents
            .iter()
            .filter(|document| document.pod_index == pod_index)
        {
            append_graph(&mut output, &document.ntriples, &document.iri);
        }
        Ok(output)
    }

    /// Render only content readable by `principal` as a reference N-Quads dataset.
    ///
    /// The independent audience oracle selects documents before any engine call; this
    /// is the physical reference dataset used for exact result-multiset comparison.
    #[must_use]
    pub fn readable_content_nquads(&self, principal: PrincipalClass) -> String {
        let mut output = String::new();
        for document in self
            .documents
            .iter()
            .filter(|document| self.can_read(principal, document))
        {
            append_graph(&mut output, &document.ntriples, &document.iri);
        }
        output
    }

    /// Render one Pod's independently readable content as a reference N-Quads dataset.
    ///
    /// # Errors
    ///
    /// Returns an error when `pod_index` does not identify a generated Pod.
    pub fn pod_readable_content_nquads(
        &self,
        pod_index: u32,
        principal: PrincipalClass,
    ) -> Result<String, String> {
        if pod_index >= self.params.pods {
            return Err(format!(
                "Pod index {pod_index} outside generated range 0..{}",
                self.params.pods
            ));
        }
        let mut output = String::new();
        for document in self.documents.iter().filter(|document| {
            document.pod_index == pod_index && self.can_read(principal, document)
        }) {
            append_graph(&mut output, &document.ntriples, &document.iri);
        }
        Ok(output)
    }

    /// Build the fixed eight-family query workload for one generated Pod.
    ///
    /// Queries whose `minimum_triples_per_document` exceeds the corpus setting are
    /// deliberately returned too, so a runner can emit an explicit `inapplicable`
    /// record instead of silently substituting a different query.
    ///
    /// # Errors
    ///
    /// Returns an error when `pod_index` does not identify a generated Pod.
    pub fn benchmark_queries(&self, pod_index: u32) -> Result<Vec<BenchmarkQuery>, String> {
        let pod = self
            .pods
            .get(pod_index as usize)
            .ok_or_else(|| format!("Pod index {pod_index} is not present"))?;
        let target = self
            .documents
            .iter()
            .find(|document| document.pod_index == pod_index)
            .ok_or_else(|| format!("Pod {pod_index} has no content document"))?;
        let (class, principal, date, alternating_left, alternating_right, date_filter) =
            query_vocabulary(self.params.domain);
        let item = format!("{}#item", target.iri);

        Ok(vec![
            BenchmarkQuery {
                id: "q1-point",
                family: "graph-bound point lookup",
                minimum_triples_per_document: 1,
                sparql: format!(
                    "SELECT ?class WHERE {{ GRAPH <{}> {{ <{item}> a ?class }} }}",
                    target.iri
                ),
            },
            BenchmarkQuery {
                id: "q2-star",
                family: "selective star pattern",
                minimum_triples_per_document: 3,
                sparql: format!(
                    "SELECT ?g ?item ?date WHERE {{ GRAPH ?g {{ ?item a <{class}> ; <{principal}> <{}> ; <{date}> ?date . {date_filter} }} }}",
                    pod.owner_webid
                ),
            },
            BenchmarkQuery {
                id: "q3-join",
                family: "multiway join",
                minimum_triples_per_document: 8,
                sparql: format!(
                    "SELECT ?g ?item WHERE {{ GRAPH ?g {{ ?item a <{class}> ; <https://schema.org/isPartOf> ?g . ?g <https://schema.org/about> ?item }} }}"
                ),
            },
            BenchmarkQuery {
                id: "q4-optional",
                family: "OPTIONAL",
                minimum_triples_per_document: 5,
                sparql: format!(
                    "SELECT ?g ?item ?left ?right WHERE {{ GRAPH ?g {{ ?item a <{class}> . OPTIONAL {{ ?item <{alternating_left}> ?left }} OPTIONAL {{ ?item <{alternating_right}> ?right }} }} }}"
                ),
            },
            BenchmarkQuery {
                id: "q5-not-exists",
                family: "NOT EXISTS",
                minimum_triples_per_document: 5,
                sparql: format!(
                    "SELECT ?g ?item WHERE {{ GRAPH ?g {{ ?item a <{class}> . FILTER NOT EXISTS {{ ?item <{alternating_left}> ?value }} }} }}"
                ),
            },
            BenchmarkQuery {
                id: "q6-path",
                family: "bounded property path",
                minimum_triples_per_document: 8,
                sparql: format!(
                    "SELECT ?g ?item WHERE {{ GRAPH ?g {{ ?item a <{class}> ; <https://schema.org/isPartOf>/<https://schema.org/about> ?item }} }}"
                ),
            },
            BenchmarkQuery {
                id: "q7-aggregate",
                family: "aggregate/grouping",
                minimum_triples_per_document: 2,
                sparql: format!(
                    "SELECT ?principal (COUNT(?item) AS ?count) WHERE {{ GRAPH ?g {{ ?item a <{class}> ; <{principal}> ?principal }} }} GROUP BY ?principal"
                ),
            },
            BenchmarkQuery {
                id: "q8-graph-scan",
                family: "unbound GRAPH scan",
                minimum_triples_per_document: 1,
                sparql: format!("SELECT ?g ?item WHERE {{ GRAPH ?g {{ ?item a <{class}> }} }}"),
            },
        ])
    }
}

fn query_vocabulary(
    domain: DeploymentDomain,
) -> (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    match domain {
        DeploymentDomain::Social => (
            "https://schema.org/SocialMediaPosting",
            "https://schema.org/author",
            "https://schema.org/dateCreated",
            "https://schema.org/mentions",
            "https://schema.org/interactionStatistic",
            "FILTER(?date >= \"2026-10-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>)",
        ),
        DeploymentDomain::Health => (
            "https://schema.org/MedicalCondition",
            "https://schema.org/patient",
            "https://schema.org/dateRecorded",
            "https://schema.org/medicalCode",
            "https://schema.org/dosageForm",
            "FILTER(?date >= \"2025-10-01\"^^<http://www.w3.org/2001/XMLSchema#date>)",
        ),
    }
}

/// Generate a deterministic many-Pod corpus from independent parameters.
///
/// # Errors
///
/// Returns an error when [`DeploymentParams::validate`] fails or count arithmetic
/// cannot fit the current platform.
pub fn generate(params: &DeploymentParams) -> Result<DeploymentCorpus, String> {
    params.validate()?;
    let total_documents =
        usize::try_from(u64::from(params.pods) * u64::from(params.documents_per_pod))
            .map_err(|_| "Pod × document count does not fit this platform".to_owned())?;
    let mut pods = Vec::with_capacity(params.pods as usize);
    let mut containers = Vec::with_capacity(
        (params.pods as usize).saturating_mul(1 + 3 * usize::from(params.container_depth)),
    );
    let mut documents = Vec::with_capacity(total_documents);
    let mut control_documents = Vec::new();

    for pod_index in 0..params.pods {
        let root_iri = pod_root(params.topology, pod_index);
        let owner_webid = format!("{root_iri}profile/card#me");
        let recipient_webid = format!("https://identities.example/recipient-{pod_index}#me");
        let pod = DeploymentPod {
            index: pod_index,
            root_iri: root_iri.clone(),
            owner_webid: owner_webid.clone(),
            recipient_webid: recipient_webid.clone(),
        };
        pods.push(pod);

        let deployment_parent = match params.topology {
            OriginTopology::SharedOrigin => Some("https://pod.example/".to_owned()),
            OriginTopology::OriginPerPod => None,
        };
        containers.push(DeploymentContainer {
            pod_index,
            iri: root_iri.clone(),
            parent_iri: deployment_parent,
        });
        control_documents.push(container_acl(pod_index, &root_iri, &owner_webid, None));

        let mut leaves = Vec::with_capacity(3);
        for audience in [
            DocumentAudience::Public,
            DocumentAudience::Private,
            DocumentAudience::Shared,
        ] {
            let category = audience_segment(audience);
            let mut parent = root_iri.clone();
            let mut category_root = String::new();
            for level in 0..params.container_depth {
                let iri = if level == 0 {
                    format!("{root_iri}{category}/")
                } else {
                    format!("{parent}level-{level}/")
                };
                if level == 0 {
                    category_root.clone_from(&iri);
                }
                containers.push(DeploymentContainer {
                    pod_index,
                    iri: iri.clone(),
                    parent_iri: Some(parent),
                });
                parent = iri;
            }
            control_documents.push(container_acl(
                pod_index,
                &category_root,
                &owner_webid,
                Some((audience, &recipient_webid)),
            ));
            leaves.push((audience, parent));
        }

        for local_index in 0..params.documents_per_pod {
            let audience = assigned_audience(params, pod_index, local_index);
            let leaf = leaves
                .iter()
                .find_map(|(candidate, iri)| (*candidate == audience).then_some(iri))
                .expect("three audience leaves are generated above");
            let iri = format!("{leaf}document-{local_index}");
            let has_own_acl = assigned_own_acl(params, pod_index, local_index);
            let ntriples = content_document(params, pod_index, local_index, &iri, &owner_webid);
            documents.push(ContentDocument {
                pod_index,
                iri: iri.clone(),
                audience,
                has_own_acl,
                ntriples,
            });
            if has_own_acl {
                control_documents.push(resource_acl(
                    pod_index,
                    &iri,
                    &owner_webid,
                    &recipient_webid,
                    audience,
                ));
            }
        }
    }

    Ok(DeploymentCorpus {
        params: params.clone(),
        pods,
        containers,
        documents,
        control_documents,
    })
}

fn pod_root(topology: OriginTopology, pod_index: u32) -> String {
    match topology {
        OriginTopology::SharedOrigin => format!("https://pod.example/pods/{pod_index}/"),
        OriginTopology::OriginPerPod => format!("https://pod-{pod_index}.example/"),
    }
}

fn audience_segment(audience: DocumentAudience) -> &'static str {
    match audience {
        DocumentAudience::Public => "public",
        DocumentAudience::Private => "private",
        DocumentAudience::Shared => "shared",
    }
}

fn assigned_audience(
    params: &DeploymentParams,
    pod_index: u32,
    local_index: u32,
) -> DocumentAudience {
    let roll = (derived_u64(params.seed, pod_index, local_index, 0x41) % 1_000) as u16;
    if roll < params.audience_mix.public {
        DocumentAudience::Public
    } else if roll < params.audience_mix.public + params.audience_mix.private {
        DocumentAudience::Private
    } else {
        DocumentAudience::Shared
    }
}

fn assigned_own_acl(params: &DeploymentParams, pod_index: u32, local_index: u32) -> bool {
    (derived_u64(params.seed, pod_index, local_index, 0x93) % 1_000)
        < u64::from(params.own_acl_coverage)
}

fn derived_u64(seed: u64, pod_index: u32, local_index: u32, lane: u64) -> u64 {
    // SplitMix64's finalizer, applied to a coordinate-derived word.  There is no mutable
    // global PRNG stream, so adding later Pods cannot perturb an existing Pod's bytes.
    let mut z = seed
        ^ u64::from(pod_index).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ u64::from(local_index).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ lane.wrapping_mul(0x94d0_49bb_1331_11eb);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn content_document(
    params: &DeploymentParams,
    pod_index: u32,
    local_index: u32,
    iri: &str,
    owner_webid: &str,
) -> String {
    let mut output = String::with_capacity(params.triples_per_document as usize * 100);
    for triple_index in 0..params.triples_per_document {
        let value = derived_u64(
            params.seed,
            pod_index,
            local_index,
            u64::from(triple_index) + 0x100,
        );
        let (subject, predicate, object) = match params.domain {
            DeploymentDomain::Social => social_spo(
                triple_index,
                value,
                iri,
                owner_webid,
                pod_index,
                local_index,
            ),
            DeploymentDomain::Health => {
                health_spo(triple_index, value, iri, owner_webid, local_index)
            }
        };
        let _ = writeln!(output, "<{subject}> <{predicate}> {object} .");
    }
    output
}

fn social_spo(
    triple_index: u32,
    value: u64,
    iri: &str,
    owner: &str,
    pod_index: u32,
    local_index: u32,
) -> (String, &'static str, String) {
    let cycle = triple_index / 8;
    let item = if cycle == 0 {
        format!("{iri}#item")
    } else {
        format!("{iri}#item-{cycle}")
    };
    match triple_index % 8 {
        0 => (
            item,
            RDF_TYPE,
            "<https://schema.org/SocialMediaPosting>".to_owned(),
        ),
        1 => (item, "https://schema.org/author", format!("<{owner}>")),
        2 => (
            item,
            "https://schema.org/dateCreated",
            format!(
                "\"2026-{:02}-{:02}T{:02}:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>",
                1 + value % 12,
                1 + (value / 12) % 28,
                (value / 336) % 24
            ),
        ),
        3 => (
            item,
            "https://schema.org/text",
            format!("\"social-{pod_index}-{}\"", value % 10_000),
        ),
        4 if local_index.is_multiple_of(2) => (
            item,
            "https://schema.org/mentions",
            format!("<https://pod-{}.example/profile/card#me>", value % 10_000),
        ),
        4 => (
            item,
            "https://schema.org/interactionStatistic",
            format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#integer>",
                value % 500
            ),
        ),
        5 => (item, "https://schema.org/isPartOf", format!("<{iri}>")),
        6 => (
            iri.to_owned(),
            "https://schema.org/identifier",
            format!("\"social-{pod_index}-{local_index}-{cycle}\""),
        ),
        _ => (
            iri.to_owned(),
            "https://schema.org/about",
            format!("<{item}>"),
        ),
    }
}

fn health_spo(
    triple_index: u32,
    value: u64,
    iri: &str,
    owner: &str,
    local_index: u32,
) -> (String, &'static str, String) {
    let cycle = triple_index / 8;
    let item = if cycle == 0 {
        format!("{iri}#item")
    } else {
        format!("{iri}#item-{cycle}")
    };
    match triple_index % 8 {
        0 => (
            item,
            RDF_TYPE,
            "<https://schema.org/MedicalCondition>".to_owned(),
        ),
        1 => (item, "https://schema.org/patient", format!("<{owner}>")),
        2 => (
            item,
            "https://schema.org/dateRecorded",
            format!(
                "\"2025-{:02}-{:02}\"^^<http://www.w3.org/2001/XMLSchema#date>",
                1 + value % 12,
                1 + (value / 12) % 28
            ),
        ),
        3 if local_index.is_multiple_of(2) => (
            item,
            "https://schema.org/medicalCode",
            format!("<https://codes.example/condition/{}>", value % 512),
        ),
        3 => (
            item,
            "https://schema.org/dosageForm",
            format!("\"form-{}\"", value % 16),
        ),
        4 => (
            item,
            "https://schema.org/value",
            format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
                value % 250
            ),
        ),
        5 => (
            item,
            "https://schema.org/unitText",
            "\"synthetic-unit\"".to_owned(),
        ),
        6 => (item, "https://schema.org/isPartOf", format!("<{iri}>")),
        _ => (
            iri.to_owned(),
            "https://schema.org/about",
            format!("<{item}>"),
        ),
    }
}

fn container_acl(
    pod_index: u32,
    governed: &str,
    owner: &str,
    audience: Option<(DocumentAudience, &str)>,
) -> ControlDocument {
    let iri = format!("{governed}.acl");
    let mut body = authorization(&iri, "owner", governed, owner, true, false, true);
    if let Some((class, recipient)) = audience {
        match class {
            DocumentAudience::Public => {
                body.push_str(&public_authorization(&iri, governed, true));
            }
            DocumentAudience::Private => {}
            DocumentAudience::Shared => {
                body.push_str(&authorization(
                    &iri,
                    "recipient",
                    governed,
                    recipient,
                    true,
                    false,
                    false,
                ));
            }
        }
    }
    ControlDocument {
        pod_index,
        iri,
        governed_iri: governed.to_owned(),
        ntriples: body,
    }
}

fn resource_acl(
    pod_index: u32,
    governed: &str,
    owner: &str,
    recipient: &str,
    audience: DocumentAudience,
) -> ControlDocument {
    let iri = format!("{governed}.acl");
    let mut body = authorization(&iri, "owner", governed, owner, false, false, true);
    match audience {
        DocumentAudience::Public => body.push_str(&public_authorization(&iri, governed, false)),
        DocumentAudience::Private => {}
        DocumentAudience::Shared => body.push_str(&authorization(
            &iri,
            "recipient",
            governed,
            recipient,
            false,
            false,
            false,
        )),
    }
    ControlDocument {
        pod_index,
        iri,
        governed_iri: governed.to_owned(),
        ntriples: body,
    }
}

fn authorization(
    acl_iri: &str,
    fragment: &str,
    governed: &str,
    agent: &str,
    default_scope: bool,
    public: bool,
    grant_control: bool,
) -> String {
    let subject = format!("{acl_iri}#{fragment}");
    let mut body = String::new();
    let _ = writeln!(body, "<{subject}> <{RDF_TYPE}> <{ACL}Authorization> .");
    if public {
        let _ = writeln!(body, "<{subject}> <{ACL}agentClass> <{FOAF_AGENT}> .");
    } else {
        let _ = writeln!(body, "<{subject}> <{ACL}agent> <{agent}> .");
    }
    let _ = writeln!(body, "<{subject}> <{ACL}accessTo> <{governed}> .");
    if default_scope {
        let _ = writeln!(body, "<{subject}> <{ACL}default> <{governed}> .");
    }
    let _ = writeln!(body, "<{subject}> <{ACL}mode> <{ACL}Read> .");
    if grant_control {
        let _ = writeln!(body, "<{subject}> <{ACL}mode> <{ACL}Control> .");
    }
    body
}

fn public_authorization(acl_iri: &str, governed: &str, default_scope: bool) -> String {
    authorization(acl_iri, "public", governed, "", default_scope, true, false)
}

fn append_graph(output: &mut String, ntriples: &str, graph_iri: &str) {
    for line in ntriples
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let statement = line
            .strip_suffix('.')
            .expect("generator emits N-Triples statements ending in a dot")
            .trim_end();
        let _ = writeln!(output, "{statement} <{graph_iri}> .");
    }
}
