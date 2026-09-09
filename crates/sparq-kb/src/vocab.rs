//! # `pkg:` vocabulary — the Project-Knowledge-Graph terms as Rust constants
//!
//! The `pkg:` IRIs of the dogfooding design (`research/dogfooding-sparq-knowledge-graph.md`
//! §2.3) as constants, mirroring the pattern of `crates/sparq-trust/src/vocab.rs`.
//!
//! The canonical, machine-readable form of this vocabulary is
//! [`ontology/pkg/pkg.ttl`](https://github.com/sparq-org/sparq/blob/main/crates/sparq-kb/ontology/pkg/pkg.ttl)
//! (Turtle, with `rdfs:label`/`rdfs:comment`/`rdfs:domain`/`rdfs:range`), the SHACL
//! write-time guardrails are in
//! [`shapes/pkg.shapes.ttl`](https://github.com/sparq-org/sparq/blob/main/crates/sparq-kb/shapes/pkg.shapes.ttl),
//! and the reuse + alignment-verification record is in `ontology/pkg/PROVENANCE.md`.
//! The `ttl_pins_match_rust_constants` test below keeps the published Turtle and
//! these Rust constants from drifting (exactly the `sparq-trust` discipline).
//!
//! Reuse-first: PKG mints almost NO net-new bespoke terms. The genuinely net-new ones
//! are [`EXPLORED_STATUS`], [`FOLLOW_UP_PRIORITY`], [`CONFIDENCE`],
//! [`COULD_BE_MERGED_WITH`], and the [`DEPENDS_ON`]/[`BLOCKED_BY`] `owl:inverseOf`
//! pair; everything else reuses PROV-O, SKOS, DCAT, DQV, FaBiO/FRBR/DC, CiTO,
//! schema.org, nanopublications, and the vendored `zkp-sparql` `sig-impl:`/`secx:`
//! pattern. The DQV quality model (sq-2489d.3) adds NO net-new bespoke predicate: the
//! [`EPISTEMIC_WEIGHT_DIMENSION`]/[`EPISTEMIC_BASIS_DIMENSION`] and
//! [`CONFIDENCE_MEASUREMENT`]/[`SOURCE_RELIABILITY_MEASUREMENT`] constants name
//! INSTANCES of the standard DQV classes (`dqv:Dimension` / `dqv:Metric`) — exactly as
//! [`OPEN`] etc. name `skos:Concept` instances — and the technique-relation graph leans
//! on the established `cito:extends` / `cito:usesMethodIn`. See PROVENANCE.md.
//!
//! [OPUS-4.8] sq-2m6zm.1 (epic sq-2m6zm); design record PR #1063. Flag for re-review
//! when Fable returns. 🤖 SPARQ agent — dogfooding sparq as a project knowledge graph.

/// The `pkg:` namespace base (sparq-local; a standardisation pass would rehome the
/// net-new terms — consistent with `trust:` / `zk:`).
pub const PKG_NS: &str = "https://sparq.dev/ns/pkg#";

// --- classes ---------------------------------------------------------------

/// `pkg:Source` — a bibliographic/documentary source (`fabio:Expression` +
/// `dcat:CatalogRecord`).
pub const SOURCE: &str = "https://sparq.dev/ns/pkg#Source";
/// `pkg:Document` — a concrete document realising a Source (`fabio:Expression`).
pub const DOCUMENT: &str = "https://sparq.dev/ns/pkg#Document";
/// `pkg:MachineAgent` — a non-human software agent (an extraction agent) a machine-
/// extracted Finding is `prov:wasAttributedTo` (`prov:SoftwareAgent`). The hook the
/// literature-tier SHACL shapes target (sq-2489d.5).
pub const MACHINE_AGENT: &str = "https://sparq.dev/ns/pkg#MachineAgent";
/// `pkg:Finding` — a reified discovery/claim (generalises `sig-impl:Assertion`).
pub const FINDING: &str = "https://sparq.dev/ns/pkg#Finding";
/// `pkg:Technique` — an algorithm/technique/method (`skos:Concept`).
pub const TECHNIQUE: &str = "https://sparq.dev/ns/pkg#Technique";
/// `pkg:Task` — the RDF projection of a `bd` issue (`schema:Action`).
pub const TASK: &str = "https://sparq.dev/ns/pkg#Task";

// --- concept schemes -------------------------------------------------------

/// `pkg:ExplorationStatus` — the source explored-status SKOS scheme.
pub const EXPLORATION_STATUS: &str = "https://sparq.dev/ns/pkg#ExplorationStatus";
/// `pkg:Topics` — the project topics SKOS scheme.
pub const TOPICS: &str = "https://sparq.dev/ns/pkg#Topics";
/// `pkg:Statuses` — the task-status SKOS scheme.
pub const STATUSES: &str = "https://sparq.dev/ns/pkg#Statuses";
/// `pkg:IssueTypes` — the bd issue-type SKOS scheme.
pub const ISSUE_TYPES: &str = "https://sparq.dev/ns/pkg#IssueTypes";
/// `pkg:Surfaces` — the project-surface SKOS scheme (mirrors `bd` `area:*`).
pub const SURFACES: &str = "https://sparq.dev/ns/pkg#Surfaces";

// --- properties (Finding) --------------------------------------------------

/// `pkg:about` — what a Finding is about (`rdfs:subPropertyOf dcterms:subject`).
pub const ABOUT: &str = "https://sparq.dev/ns/pkg#about";
/// `pkg:verdict` — the Finding verdict (`rdfs:subPropertyOf sig-impl:verdict`).
pub const VERDICT: &str = "https://sparq.dev/ns/pkg#verdict";
/// `pkg:confidence` — **NET-NEW** numeric confidence 0..1 (Finding or Source);
/// `rdfs:subPropertyOf dqv:value` of a named `dqv:QualityMeasurement` (sq-2489d.3).
pub const CONFIDENCE: &str = "https://sparq.dev/ns/pkg#confidence";
/// `pkg:assurance` — epistemic basis (reuses `secx:Proven`/`Claimed`/`Conjectured`);
/// also a `dqv:Metric` in [`EPISTEMIC_BASIS_DIMENSION`] (sq-2489d.3).
pub const ASSURANCE: &str = "https://sparq.dev/ns/pkg#assurance";
/// `pkg:discoveredFrom` — `rdfs:subPropertyOf prov:wasDerivedFrom`.
pub const DISCOVERED_FROM: &str = "https://sparq.dev/ns/pkg#discoveredFrom";

// --- properties (Source) ---------------------------------------------------

/// `pkg:exploredStatus` — **NET-NEW** the explored-vs-unexplored axis.
pub const EXPLORED_STATUS: &str = "https://sparq.dev/ns/pkg#exploredStatus";
/// `pkg:followUpPriority` — **NET-NEW** targeted-follow-up ordering integer.
pub const FOLLOW_UP_PRIORITY: &str = "https://sparq.dev/ns/pkg#followUpPriority";

// --- properties (Technique) ------------------------------------------------

/// `pkg:implementedBy` — the code implementing a Technique (`schema:SoftwareSourceCode`).
pub const IMPLEMENTED_BY: &str = "https://sparq.dev/ns/pkg#implementedBy";
/// `pkg:couldBeMergedWith` — **NET-NEW** symmetric merge hint between Techniques.
pub const COULD_BE_MERGED_WITH: &str = "https://sparq.dev/ns/pkg#couldBeMergedWith";

// --- properties (Task) -----------------------------------------------------

/// `pkg:dependsOn` — **NET-NEW** a task waits on its dependency (`owl:inverseOf
/// pkg:blockedBy`). There is deliberately NO `pkg:blocks` (§2.2).
pub const DEPENDS_ON: &str = "https://sparq.dev/ns/pkg#dependsOn";
/// `pkg:blockedBy` — **NET-NEW** the OWL inverse of `pkg:dependsOn` (§2.2).
pub const BLOCKED_BY: &str = "https://sparq.dev/ns/pkg#blockedBy";
/// `pkg:status` — the task status (a `pkg:Statuses` concept).
pub const STATUS: &str = "https://sparq.dev/ns/pkg#status";
/// `pkg:issueType` — the bd issue_type (a `pkg:IssueTypes` concept).
pub const ISSUE_TYPE: &str = "https://sparq.dev/ns/pkg#issueType";
/// `pkg:priority` — the bd priority integer (TaskShape bounds it to 0..4).
pub const PRIORITY: &str = "https://sparq.dev/ns/pkg#priority";
/// `pkg:surface` — the project surface a Task touches (a `pkg:Surfaces` concept).
pub const SURFACE: &str = "https://sparq.dev/ns/pkg#surface";
/// `pkg:label` — a free-form bd label string.
pub const LABEL: &str = "https://sparq.dev/ns/pkg#label";

// --- explored-status individuals -------------------------------------------

/// `pkg:Unexplored`.
pub const UNEXPLORED: &str = "https://sparq.dev/ns/pkg#Unexplored";
/// `pkg:Exploring`.
pub const EXPLORING: &str = "https://sparq.dev/ns/pkg#Exploring";
/// `pkg:Explored`.
pub const EXPLORED: &str = "https://sparq.dev/ns/pkg#Explored";
/// `pkg:DeadEnd`.
pub const DEAD_END: &str = "https://sparq.dev/ns/pkg#DeadEnd";

// --- task-status individuals -----------------------------------------------

/// `pkg:Open`.
pub const OPEN: &str = "https://sparq.dev/ns/pkg#Open";
/// `pkg:InProgress`.
pub const IN_PROGRESS: &str = "https://sparq.dev/ns/pkg#InProgress";
/// `pkg:Blocked`.
pub const BLOCKED: &str = "https://sparq.dev/ns/pkg#Blocked";
/// `pkg:Deferred`.
pub const DEFERRED: &str = "https://sparq.dev/ns/pkg#Deferred";
/// `pkg:Closed`.
pub const CLOSED: &str = "https://sparq.dev/ns/pkg#Closed";

// --- issue-type individuals ------------------------------------------------

/// `pkg:Bug`.
pub const BUG: &str = "https://sparq.dev/ns/pkg#Bug";
/// `pkg:Feature`.
pub const FEATURE: &str = "https://sparq.dev/ns/pkg#Feature";
/// `pkg:Chore`.
pub const CHORE: &str = "https://sparq.dev/ns/pkg#Chore";
/// `pkg:Decision`.
pub const DECISION: &str = "https://sparq.dev/ns/pkg#Decision";
/// `pkg:Milestone`.
pub const MILESTONE: &str = "https://sparq.dev/ns/pkg#Milestone";
/// `pkg:Spike`.
pub const SPIKE: &str = "https://sparq.dev/ns/pkg#Spike";
/// `pkg:Epic`.
pub const EPIC: &str = "https://sparq.dev/ns/pkg#Epic";

// --- DQV quality-model individuals (sq-2489d.3) ----------------------------
// Instances of the standard DQV classes (`dqv:Dimension` / `dqv:Metric`); they name
// the quality axes that `pkg:confidence` / `pkg:assurance` shorthand. NOT net-new
// bespoke predicates — they reuse the DQV model.

/// `pkg:EpistemicWeightDimension` — the `dqv:Dimension` of the numeric 0..1 confidence
/// axis (Finding belief / Source reliability).
pub const EPISTEMIC_WEIGHT_DIMENSION: &str = "https://sparq.dev/ns/pkg#EpistemicWeightDimension";
/// `pkg:EpistemicBasisDimension` — the `dqv:Dimension` of the categorical assurance
/// axis (`pkg:assurance` is a `dqv:Metric` in it).
pub const EPISTEMIC_BASIS_DIMENSION: &str = "https://sparq.dev/ns/pkg#EpistemicBasisDimension";
/// `pkg:ConfidenceMeasurement` — the `dqv:Metric` a Finding's `pkg:confidence` measures
/// (in [`EPISTEMIC_WEIGHT_DIMENSION`]).
pub const CONFIDENCE_MEASUREMENT: &str = "https://sparq.dev/ns/pkg#ConfidenceMeasurement";
/// `pkg:SourceReliabilityMeasurement` — the `dqv:Metric` a Source's `pkg:confidence`
/// measures (in [`EPISTEMIC_WEIGHT_DIMENSION`]).
pub const SOURCE_RELIABILITY_MEASUREMENT: &str =
    "https://sparq.dev/ns/pkg#SourceReliabilityMeasurement";

// --- tier named-graph IRIs (sq-tzars.7) ------------------------------------
// The named-graph IRIs for the tiered KB emission (`research/research-kb-program.md`
// decisions 5–6). Tier separation v1 = per-tier Turtle ARTIFACTS, NOT in-store named
// graphs (`sparq-kb` loads one in-memory Graph per query today; in-store separation is
// future work and is NOT claimed). These IRIs NAME each tier's artifact so a downstream
// loader/exporter (sq-tzars.8) keeps the tiers queryable-apart. They live in the
// `pkg/graph#` sub-namespace (mirroring the `pkg/agent#` MachineAgent IRI and the
// `pkg/kb#` instance namespace) — they are GRAPH identifiers, NOT `pkg:` vocabulary
// terms, so they are byte-pinned by `tier_graph_iris_are_pinned` below and are
// deliberately NOT in `ALL` (which pins the `pkg:` vocabulary against `pkg.ttl`).
// [SONNET-4.6] sq-tzars.7 🤖 SPARQ agent — provenance-driven GenAI KB.

/// The `pkg/graph#` sub-namespace holding the per-tier named-graph IRIs.
pub const GRAPH_NS: &str = "https://sparq.dev/ns/pkg/graph#";

/// Tier graph IRI: the HAND-AUTHORED tier — the `ingest_pkg.py` projector output (the
/// high-accuracy, human-curated PKG). Stamped on that output so it is queryable-apart.
pub const TIER_HAND_AUTHORED_GRAPH: &str = "https://sparq.dev/ns/pkg/graph#hand-authored";

/// Tier graph IRI: the MACHINE tier — the literature pipeline output for sources whose
/// licence is KNOWN and redistributable (full findings + PROV-O lineage; publishable).
pub const TIER_MACHINE_GRAPH: &str = "https://sparq.dev/ns/pkg/graph#machine";

/// Tier graph IRI: the LICENSE-RESTRICTED tier — machine findings whose source licence is
/// unknown/absent/non-redistributable (**fail-closed**). The full findings stay in this
/// PRIVATE artifact; only a metadata-only projection (no abstract-derived text) is public.
pub const TIER_LICENSE_RESTRICTED_GRAPH: &str =
    "https://sparq.dev/ns/pkg/graph#license-restricted";

/// Every tier named-graph IRI, in a stable order. Byte-pinned by
/// `tier_graph_iris_are_pinned`; deliberately disjoint from [`ALL`].
pub const TIER_GRAPHS: &[&str] = &[
    TIER_HAND_AUTHORED_GRAPH,
    TIER_MACHINE_GRAPH,
    TIER_LICENSE_RESTRICTED_GRAPH,
];

/// Every `pkg:` IRI this module names — the full set the drift guard pins against
/// `pkg.ttl`. Keep in sync when adding/removing a constant.
pub const ALL: &[&str] = &[
    SOURCE,
    DOCUMENT,
    MACHINE_AGENT,
    FINDING,
    TECHNIQUE,
    TASK,
    EXPLORATION_STATUS,
    TOPICS,
    STATUSES,
    ISSUE_TYPES,
    SURFACES,
    ABOUT,
    VERDICT,
    CONFIDENCE,
    ASSURANCE,
    DISCOVERED_FROM,
    EXPLORED_STATUS,
    FOLLOW_UP_PRIORITY,
    IMPLEMENTED_BY,
    COULD_BE_MERGED_WITH,
    DEPENDS_ON,
    BLOCKED_BY,
    STATUS,
    ISSUE_TYPE,
    PRIORITY,
    SURFACE,
    LABEL,
    UNEXPLORED,
    EXPLORING,
    EXPLORED,
    DEAD_END,
    OPEN,
    IN_PROGRESS,
    BLOCKED,
    DEFERRED,
    CLOSED,
    BUG,
    FEATURE,
    CHORE,
    DECISION,
    MILESTONE,
    SPIKE,
    EPIC,
    EPISTEMIC_WEIGHT_DIMENSION,
    EPISTEMIC_BASIS_DIMENSION,
    CONFIDENCE_MEASUREMENT,
    SOURCE_RELIABILITY_MEASUREMENT,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// No-drift guard: every `pkg:` IRI these Rust constants name MUST appear (as a
    /// subject, `pkg:LocalName ...`) in the published machine-readable vocabulary
    /// `ontology/pkg/pkg.ttl`, and every `pkg:LocalName a ...` declaration in the
    /// Turtle must be backed by a Rust constant. If a term is renamed/added in one
    /// and not the other this test fails — the published Turtle the maintainer reads
    /// and the constants downstream code uses cannot silently diverge (mirrors the
    /// `sparq-trust` `ttl_pins_match_rust_constants` discipline). [OPUS-4.8]
    #[test]
    fn ttl_pins_match_rust_constants() {
        const TTL: &str = include_str!("../ontology/pkg/pkg.ttl");

        // Every constant must be declared in the Turtle (as `pkg:Local ...`).
        for &iri in ALL {
            let local = iri
                .strip_prefix(PKG_NS)
                .expect("every constant is in the pkg: namespace");
            // A term appears as a subject `pkg:Local a|;|...` — match the `pkg:Local `
            // token (trailing space) so `pkg:Open` does not match `pkg:OpenFoo`.
            let token = format!("pkg:{local} ");
            assert!(
                TTL.contains(&token),
                "pkg.ttl is missing a declaration for `pkg:{local}` ({iri}) — \
                 the published vocabulary drifted from the Rust constants (sq-2m6zm.1)",
            );
        }

        // And every `pkg:LocalName a` declaration in the Turtle must be backed by a
        // Rust constant. Scan declaration lines: a term-declaration line looks like
        // `pkg:Foo a ...` at (trimmed) line start. Skip the `pkg: a owl:Ontology`
        // metadata line (bare namespace, no local name).
        let declared: Vec<&str> = TTL
            .lines()
            .filter_map(|line| {
                let t = line.trim_start();
                let rest = t.strip_prefix("pkg:")?;
                if rest.starts_with(char::is_whitespace) {
                    return None; // `pkg: a owl:Ontology` metadata line
                }
                let name = rest.split_whitespace().next()?;
                if rest[name.len()..].trim_start().starts_with("a ") {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "no pkg: term declarations found in pkg.ttl — parser/format drift",
        );
        for local in declared {
            let full = format!("{PKG_NS}{local}");
            assert!(
                ALL.contains(&full.as_str()),
                "pkg.ttl declares `pkg:{local}` but no Rust constant names it — \
                 add the constant to vocab.rs or remove the term (sq-2m6zm.1)",
            );
        }
    }

    /// Byte-pin the tier named-graph IRIs (sq-tzars.7). The per-tier ARTIFACTS and the
    /// tier-aware export script (sq-tzars.8) key off these EXACT strings, so a rename must
    /// be a conscious, test-breaking change. The tier graph IRIs are graph identifiers,
    /// NOT `pkg:` vocabulary terms, so they must stay OUT of [`ALL`] (which is pinned
    /// against `pkg.ttl`) — that separation is asserted here too. [SONNET-4.6]
    #[test]
    fn tier_graph_iris_are_pinned() {
        assert_eq!(GRAPH_NS, "https://sparq.dev/ns/pkg/graph#");
        assert_eq!(
            TIER_HAND_AUTHORED_GRAPH,
            "https://sparq.dev/ns/pkg/graph#hand-authored"
        );
        assert_eq!(TIER_MACHINE_GRAPH, "https://sparq.dev/ns/pkg/graph#machine");
        assert_eq!(
            TIER_LICENSE_RESTRICTED_GRAPH,
            "https://sparq.dev/ns/pkg/graph#license-restricted"
        );
        // Every tier graph IRI is in the `pkg/graph#` namespace.
        for &g in TIER_GRAPHS {
            assert!(
                g.starts_with(GRAPH_NS),
                "tier graph IRI must be in the pkg/graph# namespace: {}",
                g
            );
        }
        // The three tier graph IRIs are distinct.
        let set: std::collections::HashSet<&str> = TIER_GRAPHS.iter().copied().collect();
        assert_eq!(set.len(), TIER_GRAPHS.len(), "tier graph IRIs must be distinct");
        // Exactly one tier per emitted statement starts here: the tier graph IRIs are NOT
        // `pkg:` vocabulary terms, so none may leak into the pkg.ttl-pinned ALL set.
        for &g in TIER_GRAPHS {
            assert!(
                !ALL.contains(&g),
                "a tier graph IRI must not be in the pkg: vocab set ALL: {}",
                g
            );
        }
    }
}
