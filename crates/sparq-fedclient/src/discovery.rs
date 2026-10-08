//! Capability discovery (design §4.1) — **Phase 1**.
//!
//! Given a remote SPARQL endpoint this module fetches the two discovery documents a
//! `sparq-server` (and any standards-compliant endpoint) publishes, and distils them into
//! the planner's two inputs:
//!
//!   * a [`Capability`](crate::discovery::Capability) — *what the source can do* (full SPARQL
//!     1.1? which result formats? bind-join via VALUES? aggregates / property-paths /
//!     ORDER+LIMIT pushable?), parsed from the **SPARQL Service Description** (`GET <endpoint>`
//!     with no `query` parameter, per
//!     [SD §2](https://www.w3.org/TR/sparql11-service-description/)); and
//!   * an optional [`SourceDescriptor`](sparq_fedplan::SourceDescriptor) — the source's
//!     *statistics* (VoID counts +
//!     per-predicate / per-class partitions + the `scs:` characteristic sets), parsed from
//!     `GET <base>/.well-known/void`.
//!
//! ## What is net-new here, and what is reused
//!
//! The **VoID + `scs:` consumer already exists** end-to-end: the server writes it with
//! `sparq_introspect::Introspection::to_void_with_cs` and the planner reads it with
//! [`SourceDescriptor::from_void_nt`](sparq_fedplan::SourceDescriptor::from_void_nt). This
//! module **REUSES** that seam verbatim for the statistics half — it does NOT re-parse VoID.
//!
//! The **client-side Service-Description parser is the one genuinely new parser this layer
//! needs** (the server has only the *writer*, `sparq-server/src/descriptors.rs::sd_ntriples`).
//! [`parse_service_description`](crate::discovery::parse_service_description) is the exact
//! consumer of that writer's output, and the `sd_round_trip_against_served_fixture` test
//! round-trips a served-shape fixture through it.
//!
//! ## ASK-probe fallback
//!
//! When an endpoint publishes **no** SD and **no** VoID (a bare SPARQL endpoint), discovery
//! falls back to a FedX-style **`ASK { ... }` probe**: a source that answers an `ASK`
//! affirmatively is a full SPARQL endpoint, so a conservative "full SPARQL, VALUES bind-join,
//! no advertised stats" capability is assumed. This is the recall-safe default — it never
//! *claims* a feature the SD would have to confirm beyond core SELECT/ASK evaluation, and it
//! never invents statistics (the returned [`SourceDescriptor`](sparq_fedplan::SourceDescriptor)
//! is `None`, so the planner uses its uniform fallback).
//!
//! The separately default-OFF `pattern_probe` feature adds `PatternProbeSession`: bounded,
//! per-query ASK + capped-SELECT observations for source-pattern pairs whose VoID cardinality is
//! missing. It reuses the same [`Fetcher`](crate::discovery::Fetcher) policy seam, caches every
//! outcome, and fails open on
//! timeout/error/budget exhaustion. Only a well-formed ASK `false` prunes; successful row counts
//! replace fallback estimates and otherwise affect ranking/join order only. [GPT-5.6] sq-fx5id.
//!
//! ## SSRF caution on every fetch
//!
//! Discovery makes outbound HTTP GETs to a *caller-supplied* endpoint URL — a textbook SSRF
//! sink (threat-model B4). The production [`HttpFetcher`](crate::discovery::HttpFetcher)
//! therefore installs the **same default-deny egress filter the engine's SERVICE transport
//! uses**: it resolves the host itself and refuses to dial loopback / RFC1918 / link-local
//! (incl. the `169.254.169.254` cloud-metadata IP) / unique-local / CGNAT addresses, vetting
//! the *resolved* IP just before connect so a DNS-rebind cannot smuggle an internal address
//! through. The IP classifier (`is_forbidden_ip`) mirrors
//! `sparq_engine::service::is_forbidden_ip` (which is `pub(crate)` there, so it is
//! re-derived here rather than imported); an embedder may relax the policy per-host with
//! `HttpFetcher::allow_private_host`.
//!
//! The network is behind the [`Fetcher`](crate::discovery::Fetcher) trait seam, so the parser
//! + orchestration are tested with an in-memory [`MapFetcher`](crate::discovery::MapFetcher)
//! and pay **zero** network in CI.
//
// [OPUS-4.8] sq-nfxl (epic sq-dnko): Phase-1 discovery client + Service-Description parser.
// Net-new: the SD->Capability parser + the SSRF-guarded fetch seam + the ASK fallback.
// Reused: SourceDescriptor::from_void_nt for the VoID+scs statistics half. Flagged for
// Fable re-review when available.

use crate::source::pct_encode;
use sparq_fedplan::{SourceDescriptor, SourceId};

// ─── Vocabulary IRIs (must match sparq-server/src/descriptors.rs exactly) ────────────

/// `sd:` — the SPARQL 1.1 Service Description vocabulary namespace.
const SD_NS: &str = "http://www.w3.org/ns/sparql-service-description#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// `sd:SPARQL10Query` / `sd:SPARQL11Query` — the query-language feature IRIs.
const SD_SPARQL10_QUERY: &str = "http://www.w3.org/ns/sparql-service-description#SPARQL10Query";
const SD_SPARQL11_QUERY: &str = "http://www.w3.org/ns/sparql-service-description#SPARQL11Query";
/// `sd:BasicFederatedQuery` — the SERVICE-clause feature (the server advertises it only
/// when built with the `service` feature). We surface it as [`Capability::federated_query`].
const SD_BASIC_FEDERATED_QUERY: &str =
    "http://www.w3.org/ns/sparql-service-description#BasicFederatedQuery";
/// [OPUS-4.8] sq-yyy3: the sparq PROV-O **data-lineage** extension feature IRI a `sparq-server`
/// advertises via `sd:feature` when it serves W3C PROV-O lineage for the data it derives. We
/// surface it as [`Capability::provenance_lineage`] so a federation client can prefer (or record)
/// a source that can answer "where did this come from?". An SD that omits it yields the
/// recall-safe `false` default; it is never inferred from anything but this exact IRI.
const PROV_LINEAGE_FEATURE_IRI: &str = "http://sparq.dev/ns/prov#lineage";

/// [OPUS-4.8] sq-ym6kf: the `sd:supportedVersion` predicate — the SPARQL 1.2 Service Description
/// (ED) move that splits the version-agnostic LANGUAGE (`sd:SPARQLQuery`/`sd:SPARQLUpdate`) from
/// the language VERSION. A `sparq-server` (sq-2msb / gh-917) advertises the SPARQL language
/// versions it conformance-verifies via `sd:supportedVersion <sparql:version-*>`. The CLIENT
/// reads them into [`Capability::sparql_versions`].
const SD_SUPPORTED_VERSION: &str =
    "http://www.w3.org/ns/sparql-service-description#supportedVersion";

/// [OPUS-4.8] sq-ym6kf: the `sparql:` namespace (`http://www.w3.org/ns/sparql#`) the
/// `sparql:version-*` version IRIs live in. MUST match `sparq-server/src/descriptors.rs`
/// (the writer this parser consumes).
const SPARQL_NS: &str = "http://www.w3.org/ns/sparql#";

// ─── The capability model (§4.1) ─────────────────────────────────────────────────────
//
// NOTE on module ownership: the design names `Capability` / `Interface` / `BindJoin` etc. as
// living in `source.rs`. A SIBLING agent owns that module in this same crate, so to keep our
// edits non-overlapping (trivial merge) the discovery-specific capability model is defined
// HERE and re-exported from `discovery`. When the source-abstraction phase lands it can
// re-export or absorb these — they are deliberately self-contained.

/// The wire interface a source speaks. This Phase-1 slice discovers SPARQL `Endpoint`s; the
/// brTPF / TPF / local variants are named so the planner bridge (later phases) can match on a
/// stable enum, but discovery only ever *produces* `Endpoint` here (an ASK-reachable SPARQL
/// endpoint) — brTPF/TPF detection (Hydra control vocabulary) is a later phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    /// A full SPARQL 1.1 protocol endpoint.
    Endpoint,
    /// A bindings-restricted Triple Pattern Fragments server (brTPF).
    BrTpf,
    /// A plain Triple Pattern Fragments server (TPF).
    Tpf,
    /// The in-process `sparq-engine` over a local `Graph`.
    LocalEngine,
}

/// The highest SPARQL query language the source advertises (`sd:supportedLanguage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparqlVersion {
    /// `sd:SPARQL10Query`.
    Sparql10,
    /// `sd:SPARQL11Query`.
    Sparql11,
}

/// [OPUS-4.8] sq-ym6kf: one SPARQL language VERSION a source advertises via the SPARQL 1.2
/// Service Description (ED) `sd:supportedVersion <sparql:version-*>` predicate — the
/// version-agnostic-language / explicit-version split sq-2msb's server side emits.
///
/// These are the `sparql:` (`http://www.w3.org/ns/sparql#`) version IRIs the SD ED defines.
/// **[`SparqlLanguageVersion::Full12`]** (`sparql:version-1.2`) is the one a 1.2-aware
/// federation client checks for to decide a source supports the **full** SPARQL 1.2 / RDF 1.2
/// surface — **triple terms** and **`dir`-tagged (base-direction) literals** — *without
/// probing*, which is the discovery use case sq-2msb's server-side advertisement enables.
/// **[`SparqlLanguageVersion::Basic12`]** (`sparql:version-1.2-basic`) is the narrower
/// SPARQL-1.2-Query-with-RDF-1.2-BASIC profile; an [`SparqlLanguageVersion::Other`] carries any
/// future / unrecognised `sparql:version-*` IRI verbatim so it round-trips rather than being
/// silently dropped.
///
/// HONEST BOUNDARY (load-bearing): **absence** of `sd:supportedVersion` from a source's SD means
/// the version set is **UNKNOWN, not unsupported** — older endpoints predate the SD ED's
/// `sd:supportedVersion` term entirely. [`Capability::sparql_versions`] is therefore an empty
/// `Vec` in that case (distinguished from "advertises a non-empty set that omits 1.2"), and
/// [`Capability::advertises_full_sparql_1_2`] returns `false` only because the source made **no
/// claim** — never read it as a positive "does not support 1.2".
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SparqlLanguageVersion {
    /// `sparql:version-1.0`.
    V10,
    /// `sparql:version-1.1`.
    V11,
    /// `sparql:version-1.2-basic` — SPARQL 1.2 Query with RDF 1.2 **BASIC** conformance.
    Basic12,
    /// `sparql:version-1.2` — **full** SPARQL 1.2 / RDF 1.2 (triple terms + `dir`-tagged literals).
    Full12,
    /// Any other / future `sparql:version-*` IRI, carried verbatim (the part after the `sparql:`
    /// namespace, e.g. `"version-2.0"`), so a client can surface it without this enum being a
    /// closed set.
    Other(String),
}

impl SparqlLanguageVersion {
    /// Parses a `sparql:version-*` named-node IRI (full IRI string) into a
    /// [`SparqlLanguageVersion`]. Returns `None` for an IRI outside the `sparql:`
    /// (`http://www.w3.org/ns/sparql#`) namespace or whose local name is not a `version-*` term.
    pub fn from_version_iri(iri: &str) -> Option<SparqlLanguageVersion> {
        let local = iri.strip_prefix(SPARQL_NS)?;
        if !local.starts_with("version-") {
            return None;
        }
        Some(match local {
            "version-1.0" => SparqlLanguageVersion::V10,
            "version-1.1" => SparqlLanguageVersion::V11,
            "version-1.2-basic" => SparqlLanguageVersion::Basic12,
            "version-1.2" => SparqlLanguageVersion::Full12,
            other => SparqlLanguageVersion::Other(other.to_string()),
        })
    }

    /// The full `sparql:version-*` IRI this version denotes (the inverse of
    /// [`from_version_iri`](SparqlLanguageVersion::from_version_iri); round-trips losslessly).
    pub fn iri(&self) -> String {
        let local = match self {
            SparqlLanguageVersion::V10 => "version-1.0",
            SparqlLanguageVersion::V11 => "version-1.1",
            SparqlLanguageVersion::Basic12 => "version-1.2-basic",
            SparqlLanguageVersion::Full12 => "version-1.2",
            SparqlLanguageVersion::Other(local) => local,
        };
        format!("{SPARQL_NS}{local}")
    }
}

/// How much of a `FILTER` a source can evaluate remotely. Phase 1 reads only the *language*
/// (SD has no per-filter vocabulary), so it is set coarsely from the SPARQL version: a full
/// SPARQL 1.1 endpoint gets [`FilterClass::Full`]. Per-operator narrowing (e.g. a brTPF
/// server that evaluates no FILTER) lands with the pushdown phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterClass {
    /// No FILTER can be pushed (e.g. plain TPF) — everything is filtered locally.
    None,
    /// The full SPARQL 1.1 FILTER expression grammar evaluates remotely.
    Full,
}

/// The bind-join mode the source supports — the block-at-a-time pushdown lever (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindJoin {
    /// A SPARQL endpoint: send a `VALUES` block of bindings in one sub-query.
    Values,
    /// A brTPF server: at most `n` mappings per request (`hydra:maxMpR`-style).
    MaxMpR(u32),
    /// No bind pushdown (plain TPF / unknown): the planner keeps the join local.
    None,
}

/// A result/RDF media type the source can RETURN (`sd:resultFormat`), as the W3C
/// `<https://www.w3.org/ns/formats/...>` format IRI. Kept as the raw IRI string so the set is
/// loss-lessly round-trippable; convenience predicates ([`MediaType::is_sparql_results_json`])
/// cover the formats the client actually negotiates.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MediaType(pub String);

impl MediaType {
    /// The W3C format IRI.
    pub fn iri(&self) -> &str {
        &self.0
    }
    /// Whether this is `SPARQL_Results_JSON` — the format the client's SRJ transport reads.
    pub fn is_sparql_results_json(&self) -> bool {
        self.0 == "http://www.w3.org/ns/formats/SPARQL_Results_JSON"
    }
}

/// What a source can do — the planner's capability input (design §4.1). Far richer than a
/// coarse "service-description? / totalItems?" runtime check: each field is a statically
/// resolved fact about the source, so the pushdown phase can decide *per operator* what to
/// send remotely without another round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// The wire interface (always [`Interface::Endpoint`] from Phase-1 discovery).
    pub interface: Interface,
    /// The SPARQL query language advertised (`sd:supportedLanguage`), if any.
    pub sparql_version: Option<SparqlVersion>,
    /// [OPUS-4.8] sq-ym6kf: the SPARQL language VERSIONS the source advertises via the SPARQL 1.2
    /// SD `sd:supportedVersion <sparql:version-*>` predicate (sorted + de-duplicated). This is the
    /// CLIENT consumer of the server-side advertisement sq-2msb added: a 1.2-aware federation
    /// client reads it to detect full-1.2 (triple-term / `dir`-lang) support **without probing**.
    ///
    /// HONEST BOUNDARY: an **empty** `Vec` means the source published **no** `sd:supportedVersion`
    /// — its version posture is **UNKNOWN** (older endpoints predate the term), NOT a claim of
    /// "no version supported". Use [`Capability::advertises_full_sparql_1_2`] (which is `false` for
    /// "unknown") and treat absence as "probe / assume the conservative path", never as a negative.
    pub sparql_versions: Vec<SparqlLanguageVersion>,
    /// Which FILTER expressions evaluate remotely.
    pub pushable_filters: FilterClass,
    /// The bind-join mode (VALUES / maxMpR(n) / none).
    pub bind_join: BindJoin,
    /// Whether SPARQL aggregates (`GROUP BY` / `COUNT` …) can be pushed.
    pub aggregates: bool,
    /// Whether property paths can be pushed.
    pub property_paths: bool,
    /// Whether `ORDER BY` / `LIMIT` can be pushed.
    pub order_limit: bool,
    /// Whether the source advertises `sd:SPARQL11Update`.
    pub update: bool,
    /// Whether the source advertises `sd:BasicFederatedQuery` (a SERVICE-capable endpoint).
    pub federated_query: bool,
    /// [OPUS-4.8] sq-yyy3: whether the source advertises the sparq PROV-O data-lineage feature
    /// (`sd:feature <http://sparq.dev/ns/prov#lineage>`) — it can serve W3C PROV-O lineage for the
    /// data it derives. Purely informational for the planner (it constrains no pushdown), so it is
    /// the recall-safe `false` default unless the SD names the exact lineage feature IRI.
    pub provenance_lineage: bool,
    /// The result formats the source can return (`sd:resultFormat`), sorted + de-duplicated.
    pub result_formats: Vec<MediaType>,
}

impl Capability {
    /// The conservative capability assumed for an endpoint that **answers an `ASK` but
    /// publishes no Service Description** — the FedX-style fallback (§4.1). It claims full
    /// SPARQL 1.1 with VALUES bind-join (every SPARQL endpoint supports `VALUES`) and the
    /// SPARQL-Results-JSON result format (the protocol's mandatory format), but advertises
    /// **no** aggregates / property-paths / ORDER+LIMIT *push* claims beyond core evaluation —
    /// recall-safe: if a push turns out unsupported the planner keeps that operator local, it
    /// never produces a wrong answer. No statistics are invented (the caller pairs this with
    /// `None` for the [`SourceDescriptor`]).
    pub fn ask_fallback() -> Capability {
        Capability {
            interface: Interface::Endpoint,
            sparql_version: Some(SparqlVersion::Sparql11),
            // [OPUS-4.8] sq-ym6kf: an ASK-only endpoint published no SD, so it advertises NO
            // `sd:supportedVersion` — its version posture is UNKNOWN (empty), never an over-claim.
            sparql_versions: Vec::new(),
            pushable_filters: FilterClass::Full,
            bind_join: BindJoin::Values,
            aggregates: false,
            property_paths: false,
            order_limit: false,
            update: false,
            federated_query: false,
            // [OPUS-4.8] sq-yyy3: an ASK-only endpoint published no SD, so it makes no lineage
            // claim — recall-safe `false`.
            provenance_lineage: false,
            result_formats: vec![MediaType(
                "http://www.w3.org/ns/formats/SPARQL_Results_JSON".to_string(),
            )],
        }
    }

    /// Whether the source can return SPARQL-Results-JSON (the client's SRJ transport format).
    pub fn returns_sparql_results_json(&self) -> bool {
        self.result_formats
            .iter()
            .any(MediaType::is_sparql_results_json)
    }

    /// [OPUS-4.8] sq-ym6kf: whether the source **advertises** the FULL SPARQL 1.2 / RDF 1.2
    /// language version (`sd:supportedVersion <sparql:version-1.2>`) — i.e. triple terms and
    /// `dir`-tagged literals — so a 1.2-aware federation client can push those constructs to it
    /// without first probing.
    ///
    /// HONEST BOUNDARY: this is `false` when the source advertises a non-empty version set that
    /// omits `version-1.2` AND when the source advertises **nothing** at all (an older endpoint
    /// that predates `sd:supportedVersion`). The two cases are distinguished by
    /// [`Capability::advertises_sparql_versions`] (which is `false` only in the latter), so a
    /// caller can tell "explicitly not 1.2" from "unknown — probe or assume conservatively".
    /// `version-1.2-basic` alone does NOT satisfy this (it is the narrower RDF-1.2-BASIC profile).
    pub fn advertises_full_sparql_1_2(&self) -> bool {
        self.sparql_versions
            .contains(&SparqlLanguageVersion::Full12)
    }

    /// [OPUS-4.8] sq-ym6kf: whether the source advertised **any** `sd:supportedVersion` at all.
    /// `false` means its language-version posture is **UNKNOWN** (the source predates the SPARQL
    /// 1.2 SD term, or simply omitted it) — never read `false` as "no version supported".
    pub fn advertises_sparql_versions(&self) -> bool {
        !self.sparql_versions.is_empty()
    }
}

// ─── The Service-Description parser (THE net-new parser) ──────────────────────────────

/// Parses a SPARQL **Service Description** document (the N-Triples
/// `sparq-server/src/descriptors.rs::sd_ntriples` writes, or any standards-compliant SD) into
/// a [`Capability`].
///
/// Recognised triples (subject = the `sd:Service`):
///   * `sd:supportedLanguage` → [`Capability::sparql_version`] (SPARQL11 wins over 10) and
///     the [`Capability::update`] flag (`sd:SPARQL11Update`);
///   * `sd:supportedVersion <sparql:version-*>` → [`Capability::sparql_versions`] (sorted +
///     de-duplicated) — the SPARQL 1.2 SD language-VERSION advertisement (sq-ym6kf, consuming
///     the server side sq-2msb added). Absence ⇒ empty `Vec` ⇒ version posture **UNKNOWN**;
///   * `sd:resultFormat` → [`Capability::result_formats`] (sorted + de-duplicated);
///   * `sd:feature sd:BasicFederatedQuery` → [`Capability::federated_query`];
///   * `sd:feature <http://sparq.dev/ns/prov#lineage>` → [`Capability::provenance_lineage`]
///     (the sparq PROV-O data-lineage extension, sq-yyy3).
///
/// `pushable_filters` is set [`FilterClass::Full`] for a SPARQL 1.1 endpoint (SD has no
/// per-filter vocabulary; the pushdown phase narrows it). `bind_join` is [`BindJoin::Values`]
/// for a SPARQL endpoint (every endpoint evaluates a `VALUES` block).
///
/// Returns `Err` only on malformed N-Triples; an SD that simply omits a feature yields the
/// recall-safe "not advertised ⇒ not pushed" default for that field. Returns `Ok(None)` when
/// the document contains **no `sd:Service`** (so the caller falls back to an ASK probe).
pub fn parse_service_description(nt: &str) -> Result<Option<Capability>, String> {
    use oxrdf::{NamedOrBlankNode, Term as OxTerm};
    let sd = |local: &str| format!("{SD_NS}{local}");

    // Per-subject predicate→objects, so we can find the sd:Service subject and read its props.
    // The SD is small (a fixed shape), so a flat scan into a per-subject map is cheap.
    use rustc_hash::FxHashMap;
    let mut by_subject: FxHashMap<String, FxHashMap<String, Vec<OxTerm>>> = FxHashMap::default();
    // Subjects explicitly typed sd:Service.
    let mut service_subjects: Vec<String> = Vec::new();

    for r in oxttl::NTriplesParser::new().for_slice(nt.as_bytes()) {
        let t = r.map_err(|e| format!("malformed service-description N-Triples: {e}"))?;
        let subj = match &t.subject {
            NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
        };
        let pred = t.predicate.as_str().to_string();
        if pred == RDF_TYPE && iri_eq(&t.object, &sd("Service")) {
            service_subjects.push(subj.clone());
        }
        by_subject
            .entry(subj)
            .or_default()
            .entry(pred)
            .or_default()
            .push(t.object.clone());
    }

    // No sd:Service ⇒ this is not a Service Description (caller falls back to ASK).
    let Some(service) = service_subjects.first() else {
        return Ok(None);
    };
    let props = by_subject.get(service).expect("service subject collected");

    // --- supportedLanguage → version (+ update flag).
    let mut version: Option<SparqlVersion> = None;
    let mut update = false;
    if let Some(langs) = props.get(&sd("supportedLanguage")) {
        for l in langs {
            match l {
                OxTerm::NamedNode(n) if n.as_str() == SD_SPARQL11_QUERY => {
                    version = Some(SparqlVersion::Sparql11);
                }
                OxTerm::NamedNode(n)
                    if n.as_str() == SD_SPARQL10_QUERY
                        && version != Some(SparqlVersion::Sparql11) =>
                {
                    version = Some(SparqlVersion::Sparql10);
                }
                OxTerm::NamedNode(n)
                    if n.as_str()
                        == "http://www.w3.org/ns/sparql-service-description#SPARQL11Update" =>
                {
                    update = true;
                }
                _ => {}
            }
        }
    }

    // --- resultFormat → media types (sorted + de-duplicated for a deterministic set).
    let mut result_formats: Vec<MediaType> = props
        .get(&sd("resultFormat"))
        .into_iter()
        .flatten()
        .filter_map(|t| match t {
            OxTerm::NamedNode(n) => Some(MediaType(n.as_str().to_string())),
            _ => None,
        })
        .collect();
    result_formats.sort();
    result_formats.dedup();

    // --- [OPUS-4.8] sq-ym6kf: sd:supportedVersion → the SPARQL language VERSIONS the source
    // advertises (the SPARQL 1.2 SD predicate sq-2msb's server side emits). Only `sparql:version-*`
    // named-node objects are recognised; a non-`sparql:` IRI / literal / blank node is ignored, and
    // ABSENCE leaves the Vec empty (version posture UNKNOWN, never read as "no version supported").
    let mut sparql_versions: Vec<SparqlLanguageVersion> = props
        .get(SD_SUPPORTED_VERSION)
        .into_iter()
        .flatten()
        .filter_map(|t| match t {
            OxTerm::NamedNode(n) => SparqlLanguageVersion::from_version_iri(n.as_str()),
            _ => None,
        })
        .collect();
    sparql_versions.sort();
    sparql_versions.dedup();

    // --- sd:feature IRIs. Read once, then test for each known feature so a future feature is a
    // one-line add. BasicFederatedQuery (SERVICE-clause) and the sparq PROV-O lineage extension.
    let features = props.get(&sd("feature"));
    let has_feature = |iri: &str| {
        features
            .map(|fs| fs.iter().any(|t| iri_eq(t, iri)))
            .unwrap_or(false)
    };
    let federated_query = has_feature(SD_BASIC_FEDERATED_QUERY);
    // [OPUS-4.8] sq-yyy3: the sparq PROV-O data-lineage extension feature.
    let provenance_lineage = has_feature(PROV_LINEAGE_FEATURE_IRI);

    // A SPARQL endpoint evaluates a full FILTER grammar and a VALUES bind-join block; a
    // document with no supportedLanguage at all is treated as an endpoint with unknown
    // version (recall-safe: still full FILTER + VALUES, since it answered as a SPARQL service).
    let pushable_filters = match version {
        Some(SparqlVersion::Sparql11) | None => FilterClass::Full,
        // SPARQL 1.0 has FILTER too, so still Full; kept explicit for clarity.
        Some(SparqlVersion::Sparql10) => FilterClass::Full,
    };

    Ok(Some(Capability {
        interface: Interface::Endpoint,
        sparql_version: version,
        sparql_versions,
        pushable_filters,
        bind_join: BindJoin::Values,
        // SD does not enumerate per-feature push-ability beyond the feature IRIs above; the
        // recall-safe default is "do not claim a push the SD did not promise". Aggregates /
        // property-paths / ORDER+LIMIT are core SPARQL 1.1 a full endpoint evaluates, so a
        // SPARQL11 service is allowed to take them; anything weaker keeps them local.
        aggregates: version == Some(SparqlVersion::Sparql11),
        property_paths: version == Some(SparqlVersion::Sparql11),
        order_limit: version == Some(SparqlVersion::Sparql11),
        update,
        federated_query,
        provenance_lineage,
        result_formats,
    }))
}

fn iri_eq(t: &oxrdf::Term, iri: &str) -> bool {
    matches!(t, oxrdf::Term::NamedNode(n) if n.as_str() == iri)
}

// ─── The network seam (SSRF-guarded fetch) ───────────────────────────────────────────

/// One HTTP GET, behind a trait so discovery is testable without a network. Implemented by
/// the production [`HttpFetcher`] (ureq + SSRF egress filter) and the in-memory
/// [`MapFetcher`] (tests / a pre-fetched document cache).
///
/// `accept` is the desired `Accept` header (discovery asks for `application/n-triples` so the
/// served descriptor comes back in the exact syntax the parsers consume). The implementation
/// returns the body on a 2xx, or an `Err` describing the failure (4xx/5xx/transport/SSRF).
pub trait Fetcher {
    /// GET `url` with the given `Accept`, returning the response body or an error string.
    fn get(&self, url: &str, accept: &str) -> Result<String, String>;
}

/// Per-query limits and fallback used by opt-in pattern-level probing.
///
/// `max_probes` counts actual HTTP requests, including failed ASK/SELECT requests; cache hits
/// are free. `cardinality_cap` bounds the number of solution rows requested from an endpoint.
/// [GPT-5.6] sq-fx5id.
#[cfg(feature = "pattern_probe")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternProbeConfig {
    /// Maximum ASK/SELECT HTTP requests issued during one query. Zero disables probing.
    pub max_probes: usize,
    /// Maximum solution rows requested by a cardinality observation (clamped to at least one).
    pub cardinality_cap: usize,
    /// Uniform estimate retained when probing is unavailable or inconclusive.
    pub fallback_cardinality: f64,
}

#[cfg(feature = "pattern_probe")]
impl Default for PatternProbeConfig {
    fn default() -> Self {
        Self {
            max_probes: 64,
            cardinality_cap: 100,
            fallback_cardinality: 1_000.0,
        }
    }
}

/// Recall-safe result of probing one `(endpoint, triple-pattern)` pair.
/// [GPT-5.6] sq-fx5id.
#[cfg(feature = "pattern_probe")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternProbeOutcome {
    /// The endpoint returned a well-formed ASK `false`; this is the only prune signal.
    Empty,
    /// A successful capped SELECT observed this many rows.
    Cardinality {
        /// Rows present in the returned result sequence.
        observed: usize,
        /// Whether the configured cap was reached (the real cardinality may be higher).
        capped: bool,
    },
    /// Timeout/error/malformed response/inconsistency/budget exhaustion: keep the source.
    Unknown,
}

/// Deterministic counters for one [`PatternProbeSession`].
/// [GPT-5.6] sq-fx5id.
#[cfg(feature = "pattern_probe")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PatternProbeStats {
    /// Actual HTTP requests issued; never exceeds `PatternProbeConfig::max_probes`.
    pub requests_issued: usize,
    /// Repeated source-pattern lookups answered from the per-query cache.
    pub cache_hits: usize,
    /// Whether at least one needed request was skipped because the budget was exhausted.
    pub budget_exhausted: bool,
}

/// Bounded, per-query pattern probe cache over the existing SSRF-policy-controlled [`Fetcher`].
///
/// Construct one session per federated query and pass it to
/// `crate::planner::select_sources_with_pattern_probes`. Failures are cached as `Unknown`, so a
/// repeated pattern (or a bind-join with many bindings) never reissues the probe. [GPT-5.6]
/// sq-fx5id.
#[cfg(feature = "pattern_probe")]
pub struct PatternProbeSession<'a> {
    fetcher: &'a dyn Fetcher,
    config: PatternProbeConfig,
    cache: rustc_hash::FxHashMap<String, PatternProbeOutcome>,
    stats: PatternProbeStats,
}

#[cfg(feature = "pattern_probe")]
impl<'a> PatternProbeSession<'a> {
    /// Start an empty per-query session. Invalid/non-positive fallback estimates normalize to
    /// one, and a zero cardinality cap normalizes to one; a zero request budget remains valid.
    pub fn new(fetcher: &'a dyn Fetcher, mut config: PatternProbeConfig) -> Self {
        config.cardinality_cap = config.cardinality_cap.max(1);
        if !config.fallback_cardinality.is_finite() || config.fallback_cardinality <= 0.0 {
            config.fallback_cardinality = 1.0;
        }
        Self {
            fetcher,
            config,
            cache: rustc_hash::FxHashMap::default(),
            stats: PatternProbeStats::default(),
        }
    }

    /// The normalized configuration used by this session.
    pub fn config(&self) -> PatternProbeConfig {
        self.config
    }

    /// Current deterministic request/cache counters.
    pub fn stats(&self) -> PatternProbeStats {
        self.stats
    }

    /// Probe one source-pattern pair, returning a cached outcome on repeats.
    ///
    /// ASK is authoritative only when its exact result is `false`. After `true`, a capped
    /// SELECT supplies a cardinality observation. Any failure or an inconsistent empty SELECT
    /// after ASK `true` degrades to [`PatternProbeOutcome::Unknown`] and keeps the source.
    pub fn probe(
        &mut self,
        endpoint: &str,
        pattern: &sparq_fedplan::TriplePattern,
    ) -> PatternProbeOutcome {
        let clause = crate::planner::render_pattern_clause(pattern);
        let key = format!("{}\0{}", endpoint, clause);
        if let Some(outcome) = self.cache.get(&key).copied() {
            self.stats.cache_hits += 1;
            return outcome;
        }

        let ask = format!("ASK {{ {clause} }}");
        let outcome = match self.fetch_query(endpoint, &ask, "application/sparql-results+json") {
            Ok(body) => match parse_ask_boolean(&body) {
                Ok(false) => PatternProbeOutcome::Empty,
                Ok(true) => self.observe_cardinality(endpoint, &clause),
                Err(_) => PatternProbeOutcome::Unknown,
            },
            Err(()) => PatternProbeOutcome::Unknown,
        };
        self.cache.insert(key, outcome);
        outcome
    }

    fn observe_cardinality(&mut self, endpoint: &str, clause: &str) -> PatternProbeOutcome {
        let query = format!(
            "SELECT * WHERE {{ {clause} }} LIMIT {}",
            self.config.cardinality_cap
        );
        let Ok(body) = self.fetch_query(endpoint, &query, "application/sparql-results+json") else {
            return PatternProbeOutcome::Unknown;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
            return PatternProbeOutcome::Unknown;
        };
        let Some(bindings) = json
            .get("results")
            .and_then(|results| results.get("bindings"))
            .and_then(serde_json::Value::as_array)
        else {
            return PatternProbeOutcome::Unknown;
        };
        if bindings.is_empty() {
            // ASK true + zero SELECT rows is internally inconsistent. Fail open.
            return PatternProbeOutcome::Unknown;
        }
        PatternProbeOutcome::Cardinality {
            observed: bindings.len(),
            capped: bindings.len() >= self.config.cardinality_cap,
        }
    }

    fn fetch_query(&mut self, endpoint: &str, query: &str, accept: &str) -> Result<String, ()> {
        if self.stats.requests_issued >= self.config.max_probes {
            self.stats.budget_exhausted = true;
            return Err(());
        }
        self.stats.requests_issued += 1;
        self.fetcher
            .get(&probe_url(endpoint, query), accept)
            .map_err(|_| ())
    }
}

#[cfg(feature = "pattern_probe")]
fn probe_url(endpoint: &str, query: &str) -> String {
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    format!("{endpoint}{separator}query={}", pct_encode(query))
}

/// An in-memory fetcher mapping exact URLs → response bodies. Any URL not in the map yields a
/// `404`-style error (so a missing SD/VoID exercises the ASK-probe fallback path in tests).
#[derive(Debug, Default, Clone)]
pub struct MapFetcher {
    responses: rustc_hash::FxHashMap<String, String>,
}

impl MapFetcher {
    /// An empty map (every GET 404s) — models an endpoint that publishes no descriptors.
    pub fn new() -> MapFetcher {
        MapFetcher::default()
    }
    /// Registers `body` as the response for an exact `url`.
    pub fn with(mut self, url: impl Into<String>, body: impl Into<String>) -> MapFetcher {
        self.responses.insert(url.into(), body.into());
        self
    }
}

impl Fetcher for MapFetcher {
    fn get(&self, url: &str, _accept: &str) -> Result<String, String> {
        self.responses
            .get(url)
            .cloned()
            .ok_or_else(|| format!("MapFetcher: no response registered for {url} (simulated 404)"))
    }
}

/// Classifies a resolved [`IpAddr`](std::net::IpAddr) as a forbidden (private / internal /
/// non-global) destination for a discovery fetch — the SSRF default-deny set.
///
/// This **mirrors `sparq_engine::service::is_forbidden_ip`** (which is `pub(crate)` in the
/// engine, hence re-derived here rather than imported). Forbidden ranges: loopback
/// (`127/8`, `::1`); RFC1918 private (`10/8`, `172.16/12`, `192.168/16`); link-local
/// (`169.254/16`, incl. the `169.254.169.254` cloud-metadata IP, and IPv6 `fe80::/10`);
/// unique-local IPv6 (`fc00::/7`); unspecified (`0.0.0.0`, `::`); broadcast; CGNAT
/// (`100.64/10`). IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is unwrapped and re-classified as the
/// embedded IPv4, so a private v4 cannot ride in through a v6 host literal.
#[cfg(not(target_arch = "wasm32"))]
pub fn is_forbidden_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || matches!(v4.octets(), [100, b, ..] if (64..=127).contains(&b))
            // 100.64/10 CGNAT
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        }
    }
}

/// The production network fetcher: a blocking ureq GET behind the **same default-deny SSRF
/// egress filter the engine's SERVICE transport uses**. Native-only — neither ureq nor its
/// TLS stack ever enters the wasm bundle.
#[cfg(not(target_arch = "wasm32"))]
pub struct HttpFetcher {
    timeout: std::time::Duration,
    /// Hosts (bare authority, lower-cased) exempt from the SSRF filter — for a trusted
    /// internal federation endpoint that resolves privately.
    allow_private: std::sync::Arc<std::collections::HashSet<String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for HttpFetcher {
    fn default() -> Self {
        HttpFetcher::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl HttpFetcher {
    /// A fetcher with a finite default timeout (so an unreachable endpoint cannot hang
    /// discovery) and the strict default-deny SSRF policy (no private host allowed).
    pub fn new() -> HttpFetcher {
        HttpFetcher {
            timeout: std::time::Duration::from_secs(15),
            allow_private: std::sync::Arc::new(std::collections::HashSet::new()),
        }
    }

    /// Sets the per-request timeout.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> HttpFetcher {
        self.timeout = timeout;
        self
    }

    /// Exempts a trusted `host` (bare authority, e.g. `sparql.internal`) from the SSRF
    /// filter — its resolved addresses are dialled even when private. Use ONLY for a
    /// deliberately-trusted internal endpoint.
    pub fn allow_private_host(mut self, host: impl Into<String>) -> HttpFetcher {
        std::sync::Arc::make_mut(&mut self.allow_private).insert(host.into().to_ascii_lowercase());
        self
    }
}

/// ureq resolver enforcing the SSRF egress policy on the *resolved* addresses
/// (DNS-rebinding-safe): it resolves the host, drops every [`is_forbidden_ip`] address
/// (unless the bare host is on the allowlist), and returns only survivors — so ureq dials
/// only vetted IPs. Mirrors the engine's `EgressFilterResolver`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct EgressFilterResolver {
    allow_private: std::sync::Arc<std::collections::HashSet<String>>,
}

// [OPUS-4.8] sq-g2xs: ureq-3's `Resolver` takes a parsed `&http::Uri` (+ config + timeout) and
// returns an `ArrayVec<SocketAddr, 16>` rather than ureq-2's `&str` netloc → `Vec`. The SSRF
// policy is byte-for-byte the ureq-2 one; the shared `crate::ureq_egress` helpers carry the
// ureq-3 boilerplate so the three sparq-fedclient transports share one implementation. (The engine
// SERVICE resolver applies equivalent, unit-tested logic via its own inline copy — `sparq-engine`
// cannot depend on `sparq-fedclient` — so it does not share this module.)
#[cfg(not(target_arch = "wasm32"))]
impl ureq::unversioned::resolver::Resolver for EgressFilterResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let (host_port, host, port) = crate::ureq_egress::uri_host_port(uri).ok_or_else(|| {
            crate::ureq_egress::egress_refused(format!(
                "discovery egress refused: request URI {uri} has no host authority to vet"
            ))
        })?;
        // Port-scoped allowlist: a `host:port` entry re-opens a private host only on its exact
        // dialled port (shared host:port rule with the engine SERVICE guard; sq-vbnyc).
        let allowed = crate::ureq_egress::allowlist_permits(&self.allow_private, &host, port);
        crate::ureq_egress::filter_resolved(&host_port, allowed, is_forbidden_ip, || {
            format!(
                "discovery egress refused: {host_port} resolves only to private/internal \
                 addresses (default-deny SSRF policy; allow it with \
                 HttpFetcher::allow_private_host)"
            )
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Fetcher for HttpFetcher {
    fn get(&self, url: &str, accept: &str) -> Result<String, String> {
        // [OPUS-4.8] sq-g2xs: ureq-3 `Agent::with_parts` + the default-deny SSRF resolver — ureq
        // dials only the addresses the resolver returns (the vetted IP is the dialled IP).
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .user_agent(concat!("sparq-fedclient/", env!("CARGO_PKG_VERSION")))
            .build();
        let agent = ureq::Agent::with_parts(
            config,
            ureq::unversioned::transport::DefaultConnector::new(),
            EgressFilterResolver {
                allow_private: std::sync::Arc::clone(&self.allow_private),
            },
        );
        match agent.get(url).header("Accept", accept).call() {
            // ureq-3 caps `read_to_string` at 10 MiB by default; raise it to match the other
            // transports (a finite cap still bounds memory).
            Ok(mut r) => r
                .body_mut()
                .with_config()
                .limit(DISCOVERY_MAX_BODY_BYTES)
                .read_to_string()
                .map_err(|e| format!("discovery: reading response from {url}: {e}")),
            Err(ureq::Error::StatusCode(code)) => {
                Err(format!("discovery: {url} returned HTTP {code}"))
            }
            Err(e) => Err(format!("discovery: request to {url} failed: {e}")),
        }
    }
}

/// Max bytes read from a discovery (VoID/SD) response body. ureq-3's default `read_to_string`
/// cap is 10 MiB; raised to a generous-but-finite bound (memory stays bounded). [OPUS-4.8] sq-g2xs.
#[cfg(not(target_arch = "wasm32"))]
const DISCOVERY_MAX_BODY_BYTES: u64 = 256 * 1024 * 1024;

// ─── Orchestration: discover() ────────────────────────────────────────────────────────

/// The N-Triples `Accept` discovery asks for, so the served descriptor comes back in the
/// exact syntax the [`parse_service_description`] / [`SourceDescriptor::from_void_nt`]
/// parsers consume (rather than negotiating Turtle/RDF-XML and re-parsing).
const NT_ACCEPT: &str = "application/n-triples";

/// `ASK { ?s ?p ?o }` — the FedX-style reachability probe. A source that answers it (with the
/// SPARQL-Results-JSON `boolean`) is a live SPARQL endpoint, so the ASK-fallback capability
/// applies.
const ASK_PROBE: &str = "ASK { ?s ?p ?o }";

/// The outcome of discovering one source: its capability and (when published) its statistics.
#[derive(Debug, Clone)]
pub struct Discovery {
    /// What the source can do.
    pub capability: Capability,
    /// The source's statistics (VoID + `scs:`), `None` when it publishes no VoID document.
    pub descriptor: Option<SourceDescriptor>,
    /// How the capability was determined — useful for diagnostics / logging.
    pub provenance: Provenance,
}

/// How a [`Discovery`]'s capability was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Parsed from a served SPARQL Service Description.
    ServiceDescription,
    /// No SD published; an `ASK` probe confirmed a live SPARQL endpoint (conservative caps).
    AskProbe,
}

/// Derives the `/.well-known/void` URL for a SPARQL endpoint URL. The well-known URI is
/// resolved against the endpoint's **authority** (scheme + `host[:port]`), per RFC 8615 — e.g.
/// `https://host/sparql` and `https://host/db/sparql` both map to
/// `https://host/.well-known/void`. Returns `None` if `endpoint` has no parseable authority.
pub fn well_known_void_url(endpoint: &str) -> Option<String> {
    let scheme_end = endpoint.find("://")?;
    let after = scheme_end + 3;
    let rest = &endpoint[after..];
    let host_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if host_len == 0 {
        return None;
    }
    Some(format!(
        "{}/.well-known/void",
        &endpoint[..after + host_len]
    ))
}

/// Discovers a source's [`Capability`] + statistics from a SPARQL `endpoint` URL, using
/// `fetcher` for every network access (so the caller controls the SSRF policy and tests
/// inject canned documents).
///
/// Steps (design §4.1):
///   1. GET the **Service Description** (`endpoint` with no `query`) → [`Capability`] via
///      [`parse_service_description`]. If the SD is absent or empty, fall through to (3).
///   2. GET `<authority>/.well-known/void` → [`SourceDescriptor`] via the REUSED
///      [`SourceDescriptor::from_void_nt`] seam. A missing VoID leaves the descriptor `None`.
///   3. If no SD was parsed, **ASK-probe** the endpoint: a live SPARQL endpoint gets the
///      conservative [`Capability::ask_fallback`]; an endpoint that answers nothing is an
///      error.
///
/// The `SourceId` for the descriptor is the `endpoint` URL (the planner's stable key). This
/// function is the one-shot uncached primitive; a caller that discovers repeatedly should
/// cache the [`Discovery`] (design §4.1 — "`discover()` is cached so the hot path pays
/// nothing"), which the source-adapter phase wires.
pub fn discover(endpoint: &str, fetcher: &dyn Fetcher) -> Result<Discovery, String> {
    // --- (1) Service Description (GET the endpoint with no query).
    let capability = match fetcher.get(endpoint, NT_ACCEPT) {
        Ok(body) => parse_service_description(&body)?,
        // A transport/HTTP failure on the SD GET is non-fatal: many bare endpoints 400 a
        // no-query GET. Fall through to the ASK probe.
        Err(_) => None,
    };

    // --- (2) VoID statistics (REUSE from_void_nt). Best-effort: a missing/!parseable VoID
    // leaves the descriptor None (the planner uses its uniform fallback).
    let descriptor = well_known_void_url(endpoint).and_then(|void_url| {
        fetcher
            .get(&void_url, NT_ACCEPT)
            .ok()
            .and_then(|nt| SourceDescriptor::from_void_nt(SourceId::new(endpoint), &nt).ok())
    });

    // --- (3) Resolve the capability: SD if present, else an ASK-probe fallback.
    match capability {
        Some(cap) => Ok(Discovery {
            capability: cap,
            descriptor,
            provenance: Provenance::ServiceDescription,
        }),
        None => {
            // The ASK probe proves *reachability*, not contents: ANY well-formed boolean
            // answer (true OR false) means the endpoint evaluated the query and is a live
            // SPARQL service. Only a transport/HTTP/parse failure (the `?`) means unreachable.
            let _answered: bool = ask_probe(endpoint, fetcher).map_err(|e| {
                format!(
                    "discovery: {endpoint} published no Service Description and did not answer an \
                     ASK probe — not a reachable SPARQL endpoint ({e})"
                )
            })?;
            Ok(Discovery {
                capability: Capability::ask_fallback(),
                descriptor,
                provenance: Provenance::AskProbe,
            })
        }
    }
}

/// Runs the FedX-style `ASK { ?s ?p ?o }` reachability probe by POSTing the query — here
/// modelled as a GET with the query in the URL (`?query=...`), the simplest protocol form a
/// [`Fetcher`] can express. Returns the boolean answer, or an error if the endpoint is
/// unreachable or returns a non-ASK body.
fn ask_probe(endpoint: &str, fetcher: &dyn Fetcher) -> Result<bool, String> {
    let url = format!("{endpoint}?query={}", pct_encode(ASK_PROBE));
    let body = fetcher
        .get(&url, "application/sparql-results+json")
        .map_err(|e| format!("discovery: ASK probe to {endpoint} failed: {e}"))?;
    parse_ask_boolean(&body)
}

/// Extracts the `boolean` field from a SPARQL-Results-JSON ASK response. Deliberately a tiny
/// scan rather than a JSON dependency — the only shape we need is `"boolean": true|false`.
///
/// The match is STRICT: the value must be an exact lowercase JSON `true`/`false` literal whose
/// next character is a value boundary (end-of-input, ASCII whitespace, or a JSON structural
/// `,` / `}` / `]`). This rejects junk-suffixed tokens like `trueish` / `falsex` / `truex`
/// that a `starts_with` scan would silently read as a boolean (per the SPARQL 1.1 Query
/// Results JSON format, the value of `boolean` is a JSON boolean — lowercase, case-sensitive).
fn parse_ask_boolean(body: &str) -> Result<bool, String> {
    // Find the "boolean" key and read the following true/false token.
    let key = body
        .find("\"boolean\"")
        .ok_or_else(|| "discovery: ASK response had no \"boolean\" field".to_string())?;
    let after = &body[key + "\"boolean\"".len()..];
    let after = after.trim_start();
    let after = after
        .strip_prefix(':')
        .ok_or_else(|| "discovery: malformed ASK response (no ':' after \"boolean\")".to_string())?
        .trim_start();
    // A literal ends at a JSON value boundary: end-of-input, ASCII whitespace, or a structural
    // delimiter. `trueish` etc. fail this because the next char (`i`) is none of those.
    let bounded = |rest: &str| {
        rest.chars()
            .next()
            .map(|c| c.is_ascii_whitespace() || matches!(c, ',' | '}' | ']'))
            .unwrap_or(true)
    };
    if after.strip_prefix("true").is_some_and(bounded) {
        Ok(true)
    } else if after.strip_prefix("false").is_some_and(bounded) {
        Ok(false)
    } else {
        Err("discovery: ASK response \"boolean\" was not an exact JSON true/false".to_string())
    }
}

// ─── Tests (parser + orchestration; NO public network) ────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A served-shape Service Description (the exact N-Triples
    /// `sparq-server/src/descriptors.rs::sd_ntriples` writes for a read-only, federation-
    /// capable endpoint). The round-trip test parses THIS and asserts the capability matches.
    const SERVED_SD_NT: &str = r#"<http://host/sparql> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#endpoint> <http://host/sparql> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#SPARQL11Query> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#resultFormat> <http://www.w3.org/ns/formats/SPARQL_Results_JSON> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#resultFormat> <http://www.w3.org/ns/formats/SPARQL_Results_XML> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#resultFormat> <http://www.w3.org/ns/formats/Turtle> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#inputFormat> <http://www.w3.org/ns/formats/Turtle> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#feature> <http://www.w3.org/ns/sparql-service-description#BasicFederatedQuery> .
<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#defaultDataset> _:dataset .
_:dataset <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Dataset> .
_:dataset <http://www.w3.org/ns/sparql-service-description#defaultGraph> _:defaultGraph .
_:defaultGraph <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Graph> .
_:defaultGraph <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://rdfs.org/ns/void#Dataset> .
<http://host/sparql> <http://purl.org/dc/terms/source> <http://host/.well-known/void#dataset> .
"#;

    /// A served-shape VoID document (the `from_void_nt` consumer's input — one property + one
    /// class partition + a top-level triple count).
    const SERVED_VOID_NT: &str = r#"<http://host/.well-known/void#dataset> <http://rdfs.org/ns/void#triples> "300"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://host/.well-known/void#dataset> <http://rdfs.org/ns/void#propertyPartition> _:p1 .
_:p1 <http://rdfs.org/ns/void#property> <http://xmlns.com/foaf/0.1/knows> .
_:p1 <http://rdfs.org/ns/void#triples> "200"^^<http://www.w3.org/2001/XMLSchema#integer> .
_:p1 <http://rdfs.org/ns/void#distinctSubjects> "100"^^<http://www.w3.org/2001/XMLSchema#integer> .
<http://host/.well-known/void#dataset> <http://rdfs.org/ns/void#classPartition> _:c1 .
_:c1 <http://rdfs.org/ns/void#class> <http://xmlns.com/foaf/0.1/Person> .
_:c1 <http://rdfs.org/ns/void#entities> "100"^^<http://www.w3.org/2001/XMLSchema#integer> .
"#;

    // ---- The net-new Service-Description parser ------------------------------------

    #[test]
    fn parse_sd_extracts_full_capability() {
        let cap = parse_service_description(SERVED_SD_NT)
            .expect("well-formed SD must parse")
            .expect("document has an sd:Service");
        assert_eq!(cap.interface, Interface::Endpoint);
        assert_eq!(cap.sparql_version, Some(SparqlVersion::Sparql11));
        assert_eq!(cap.pushable_filters, FilterClass::Full);
        assert_eq!(cap.bind_join, BindJoin::Values);
        assert!(
            cap.federated_query,
            "feature sd:BasicFederatedQuery present"
        );
        assert!(!cap.update, "no SPARQL11Update advertised ⇒ read-only");
        // result formats are sorted + de-duplicated.
        assert!(cap.returns_sparql_results_json());
        assert_eq!(cap.result_formats.len(), 3);
        assert!(cap.result_formats.windows(2).all(|w| w[0] < w[1]), "sorted");
        // SPARQL 1.1 ⇒ aggregates / paths / order-limit pushable.
        assert!(cap.aggregates && cap.property_paths && cap.order_limit);
        // [OPUS-4.8] sq-yyy3: this SD does NOT advertise the PROV-O lineage feature, so the flag
        // is the recall-safe `false` (it is set ONLY by the exact lineage feature IRI).
        assert!(
            !cap.provenance_lineage,
            "no sd:feature <…/prov#lineage> ⇒ provenance_lineage must be false"
        );
    }

    // ---- [OPUS-4.8] sq-ym6kf: sd:supportedVersion (the SPARQL 1.2 SD version advertisement) ----

    /// A served-shape SD fragment carrying the EXACT `sd:supportedVersion <sparql:version-*>`
    /// triples `sparq-server/src/descriptors.rs` emits for a build that conformance-verifies
    /// SPARQL 1.0 / 1.1 / FULL 1.2 (sq-2msb / `CONFORMANCE_VERIFIED_VERSIONS`).
    const SERVED_SD_WITH_VERSIONS_NT: &str = concat!(
        "<http://host/sparql> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n",
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#SPARQL11Query> .\n",
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedVersion> <http://www.w3.org/ns/sparql#version-1.0> .\n",
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedVersion> <http://www.w3.org/ns/sparql#version-1.1> .\n",
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedVersion> <http://www.w3.org/ns/sparql#version-1.2> .\n",
    );

    #[test]
    fn sparql_language_version_round_trips_known_iris() {
        // Every known `sparql:version-*` IRI parses to its typed variant and renders back.
        for (iri, v) in [
            (
                "http://www.w3.org/ns/sparql#version-1.0",
                SparqlLanguageVersion::V10,
            ),
            (
                "http://www.w3.org/ns/sparql#version-1.1",
                SparqlLanguageVersion::V11,
            ),
            (
                "http://www.w3.org/ns/sparql#version-1.2-basic",
                SparqlLanguageVersion::Basic12,
            ),
            (
                "http://www.w3.org/ns/sparql#version-1.2",
                SparqlLanguageVersion::Full12,
            ),
        ] {
            assert_eq!(
                SparqlLanguageVersion::from_version_iri(iri),
                Some(v.clone())
            );
            assert_eq!(v.iri(), iri, "iri() must round-trip {iri}");
        }
    }

    #[test]
    fn sparql_language_version_carries_unknown_iri_verbatim() {
        // A FUTURE `sparql:version-*` IRI is carried in `Other` (not dropped) and round-trips.
        let v = SparqlLanguageVersion::from_version_iri("http://www.w3.org/ns/sparql#version-2.0")
            .expect("a sparql:version-* IRI must parse");
        assert_eq!(v, SparqlLanguageVersion::Other("version-2.0".to_string()));
        assert_eq!(v.iri(), "http://www.w3.org/ns/sparql#version-2.0");
        // A non-`sparql:` namespace or a non-version local name is NOT a version IRI.
        assert_eq!(
            SparqlLanguageVersion::from_version_iri(
                "http://www.w3.org/ns/sparql-service-description#SPARQL11Query"
            ),
            None,
            "an sd: language term is not a sparql:version-* IRI"
        );
        assert_eq!(
            SparqlLanguageVersion::from_version_iri("http://www.w3.org/ns/sparql#feature-x"),
            None,
            "a sparql: IRI that is not version-* is not a version"
        );
    }

    #[test]
    fn parse_sd_extracts_supported_versions() {
        // [OPUS-4.8] sq-ym6kf: the CLIENT consumes the server-side `sd:supportedVersion`
        // advertisement (sq-2msb): the version set is parsed, sorted, and a 1.2-aware client can
        // detect FULL 1.2 (triple-term / dir-lang) support without probing.
        let cap = parse_service_description(SERVED_SD_WITH_VERSIONS_NT)
            .unwrap()
            .unwrap();
        assert_eq!(
            cap.sparql_versions,
            vec![
                SparqlLanguageVersion::V10,
                SparqlLanguageVersion::V11,
                SparqlLanguageVersion::Full12,
            ],
            "supportedVersion IRIs parsed + sorted by variant order (V10 < V11 < Full12)"
        );
        assert!(
            cap.advertises_sparql_versions(),
            "a non-empty sd:supportedVersion set ⇒ version posture is KNOWN"
        );
        assert!(
            cap.advertises_full_sparql_1_2(),
            "version-1.2 advertised ⇒ full SPARQL 1.2 detectable without probing"
        );
        // The legacy `sd:supportedLanguage` parse is unaffected by the new predicate.
        assert_eq!(cap.sparql_version, Some(SparqlVersion::Sparql11));
    }

    #[test]
    fn parse_sd_absent_supported_version_is_unknown_not_unsupported() {
        // HONEST BOUNDARY (load-bearing): the base served fixture predates `sd:supportedVersion`
        // (an older endpoint). Its version posture must be UNKNOWN — an EMPTY set — NOT a claim of
        // "no 1.2". `advertises_full_sparql_1_2()` is `false`, but ONLY because no claim was made,
        // which `advertises_sparql_versions()` (also `false`) lets a caller distinguish.
        let cap = parse_service_description(SERVED_SD_NT).unwrap().unwrap();
        assert!(
            cap.sparql_versions.is_empty(),
            "no sd:supportedVersion ⇒ empty version set (UNKNOWN, not unsupported)"
        );
        assert!(
            !cap.advertises_sparql_versions(),
            "absence ⇒ version posture is unknown"
        );
        assert!(
            !cap.advertises_full_sparql_1_2(),
            "absence is NOT a positive '1.2' — false here means 'no claim', see advertises_sparql_versions()"
        );
    }

    #[test]
    fn parse_sd_basic_1_2_does_not_satisfy_full_1_2() {
        // A source advertising ONLY `version-1.2-basic` (the narrower RDF-1.2-BASIC profile) has a
        // KNOWN version posture, but `advertises_full_sparql_1_2()` is false — Basic12 ≠ Full12.
        let nt = format!(
            "{SERVED_SD_NT}<http://host/sparql> \
             <http://www.w3.org/ns/sparql-service-description#supportedVersion> \
             <http://www.w3.org/ns/sparql#version-1.2-basic> .\n"
        );
        let cap = parse_service_description(&nt).unwrap().unwrap();
        assert_eq!(cap.sparql_versions, vec![SparqlLanguageVersion::Basic12]);
        assert!(
            cap.advertises_sparql_versions(),
            "version-1.2-basic IS a claim ⇒ posture known"
        );
        assert!(
            !cap.advertises_full_sparql_1_2(),
            "version-1.2-basic alone does NOT prove full 1.2 (triple-term / dir-lang)"
        );
    }

    #[test]
    fn parse_sd_ignores_non_version_supported_version_objects() {
        // Robustness: a non-`sparql:version-*` object on `sd:supportedVersion` (a literal, or an
        // IRI outside the sparql: namespace) is ignored — it never invents a version, and a
        // genuine version IRI alongside it is still read.
        let nt = format!(
            "{SERVED_SD_NT}\
             <http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedVersion> \"1.1\" .\n\
             <http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedVersion> <http://example.org/not-a-version> .\n\
             <http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedVersion> <http://www.w3.org/ns/sparql#version-1.1> .\n"
        );
        let cap = parse_service_description(&nt).unwrap().unwrap();
        assert_eq!(
            cap.sparql_versions,
            vec![SparqlLanguageVersion::V11],
            "only the genuine sparql:version-* IRI is kept; junk objects ignored"
        );
    }

    #[test]
    fn ask_fallback_has_unknown_version_set() {
        // [OPUS-4.8] sq-ym6kf: an ASK-only endpoint published no SD, so it advertises no version —
        // the fallback capability's set is empty (UNKNOWN), never an over-claim of 1.2 support.
        let cap = Capability::ask_fallback();
        assert!(cap.sparql_versions.is_empty());
        assert!(!cap.advertises_sparql_versions());
        assert!(!cap.advertises_full_sparql_1_2());
    }

    #[test]
    fn parse_sd_with_provenance_feature_sets_flag() {
        // [OPUS-4.8] sq-yyy3: a node advertising the sparq PROV-O data-lineage feature
        // (`sd:feature <http://sparq.dev/ns/prov#lineage>`, exactly what
        // `descriptors::sd_ntriples` emits when `Capabilities::provenance`) is surfaced as
        // `Capability::provenance_lineage`. Adding it leaves BasicFederatedQuery untouched.
        let nt = format!(
            "{SERVED_SD_NT}<http://host/sparql> \
             <http://www.w3.org/ns/sparql-service-description#feature> \
             <http://sparq.dev/ns/prov#lineage> .\n"
        );
        let cap = parse_service_description(&nt).unwrap().unwrap();
        assert!(
            cap.provenance_lineage,
            "sd:feature <…/prov#lineage> advertised ⇒ provenance_lineage flag set"
        );
        // The other features are unaffected by the new one.
        assert!(cap.federated_query, "BasicFederatedQuery still detected");
    }

    #[test]
    fn parse_sd_with_update_sets_flag() {
        let nt = format!(
            "{SERVED_SD_NT}<http://host/sparql> \
             <http://www.w3.org/ns/sparql-service-description#supportedLanguage> \
             <http://www.w3.org/ns/sparql-service-description#SPARQL11Update> .\n"
        );
        let cap = parse_service_description(&nt).unwrap().unwrap();
        assert!(cap.update, "SPARQL11Update advertised ⇒ update flag set");
        // Adding the Update language must not downgrade the query version.
        assert_eq!(cap.sparql_version, Some(SparqlVersion::Sparql11));
    }

    #[test]
    fn parse_sd_without_service_returns_none() {
        // A VoID document (no sd:Service) must NOT parse as a Service Description.
        let cap = parse_service_description(SERVED_VOID_NT).expect("well-formed N-Triples");
        assert!(cap.is_none(), "no sd:Service ⇒ not an SD ⇒ None");
    }

    #[test]
    fn parse_sd_malformed_is_error() {
        let err = parse_service_description("this is not <ntriples at all");
        assert!(err.is_err(), "malformed N-Triples must surface an error");
    }

    #[test]
    fn parse_sd_sparql10_only() {
        let nt = "<http://h/s> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n<http://h/s> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#SPARQL10Query> .\n";
        let cap = parse_service_description(nt).unwrap().unwrap();
        assert_eq!(cap.sparql_version, Some(SparqlVersion::Sparql10));
        // SPARQL 1.0 ⇒ no aggregate/path/order-limit push (those are 1.1).
        assert!(!cap.aggregates && !cap.property_paths && !cap.order_limit);
        // FILTER still evaluates (1.0 has FILTER), and VALUES bind-join still applies.
        assert_eq!(cap.pushable_filters, FilterClass::Full);
    }

    // ---- The round-trip test (THE deliverable): served SD/VoID → Discovery -----------

    #[test]
    fn sd_round_trip_against_served_fixture() {
        // The endpoint serves the EXACT documents sparq-server writes, at the conventional
        // URLs: the SD at the endpoint URL (no query) and the VoID at /.well-known/void.
        let endpoint = "http://host/sparql";
        let fetcher = MapFetcher::new()
            .with(endpoint, SERVED_SD_NT)
            .with("http://host/.well-known/void", SERVED_VOID_NT);

        let d = discover(endpoint, &fetcher).expect("discovery against a served fixture");

        // Capability came from the SD.
        assert_eq!(d.provenance, Provenance::ServiceDescription);
        assert_eq!(d.capability.sparql_version, Some(SparqlVersion::Sparql11));
        assert!(d.capability.federated_query);
        assert!(d.capability.returns_sparql_results_json());

        // Statistics came from the REUSED from_void_nt seam — and match the server's writer.
        let desc = d.descriptor.expect("VoID was served ⇒ descriptor present");
        assert_eq!(desc.id(), &SourceId::new(endpoint));
        let p = desc
            .predicate("http://xmlns.com/foaf/0.1/knows")
            .expect("served property partition");
        assert_eq!(p.triples, 200);
        assert_eq!(p.distinct_subjects, 100);
        assert!(desc.has_class("http://xmlns.com/foaf/0.1/Person"));
    }

    // ---- ASK-probe fallback when nothing is published --------------------------------

    #[test]
    fn ask_fallback_when_no_descriptor_published() {
        // The endpoint serves no SD (no-query GET 404s in the map) and no VoID, but answers
        // the ASK probe affirmatively.
        let endpoint = "http://bare/sparql";
        let ask_url = format!("{endpoint}?query={}", pct_encode(ASK_PROBE));
        let fetcher = MapFetcher::new().with(ask_url, r#"{ "head": {}, "boolean": true }"#);

        let d = discover(endpoint, &fetcher).expect("ASK-reachable endpoint discovers");
        assert_eq!(d.provenance, Provenance::AskProbe);
        // The conservative fallback capability.
        assert_eq!(d.capability.interface, Interface::Endpoint);
        assert_eq!(d.capability.bind_join, BindJoin::Values);
        assert_eq!(d.capability.pushable_filters, FilterClass::Full);
        assert!(d.capability.returns_sparql_results_json());
        // No descriptor invented.
        assert!(d.descriptor.is_none());
    }

    #[test]
    fn unreachable_endpoint_is_an_error() {
        // No SD, no VoID, and the ASK probe is not registered either (so it errors) ⇒ the
        // endpoint is not a reachable SPARQL service.
        let endpoint = "http://dead/sparql";
        let fetcher = MapFetcher::new();
        let err = discover(endpoint, &fetcher);
        assert!(err.is_err(), "an endpoint that answers nothing must error");
    }

    #[test]
    fn ask_probe_negative_boolean_still_reachable() {
        // An ASK that returns false STILL proves the endpoint is a live SPARQL service (it
        // evaluated the query) — so discovery succeeds with the fallback capability.
        let endpoint = "http://empty/sparql";
        let ask_url = format!("{endpoint}?query={}", pct_encode(ASK_PROBE));
        let fetcher = MapFetcher::new().with(ask_url, r#"{ "boolean": false }"#);
        let d = discover(endpoint, &fetcher).expect("a false ASK still proves reachability");
        assert_eq!(d.provenance, Provenance::AskProbe);
    }

    // ---- ASK boolean parsing -------------------------------------------------------

    #[test]
    fn ask_boolean_parsing() {
        // Legitimate SPARQL-Results-JSON ASK shapes the function already handled — still parse.
        assert!(parse_ask_boolean(r#"{"boolean": true}"#).unwrap());
        assert!(!parse_ask_boolean(r#"{"boolean":false}"#).unwrap());
        assert!(parse_ask_boolean(r#"{ "head": {}, "boolean" : true }"#).unwrap());
        assert!(parse_ask_boolean(r#"{"results":{}}"#).is_err());
    }

    #[test]
    fn ask_boolean_strict_exact_token() {
        // ACCEPT: every JSON value boundary after the exact literal — `}`, `,`, `]`, whitespace,
        // and end-of-input — for both true and false.
        assert!(parse_ask_boolean(r#"{"boolean":true}"#).unwrap());
        assert!(parse_ask_boolean(r#"{"boolean":true,"x":1}"#).unwrap());
        assert!(parse_ask_boolean(r#"{"head":{},"boolean":true ,"more":0}"#).unwrap());
        assert!(parse_ask_boolean("\"boolean\": true").unwrap()); // EOF boundary
        assert!(parse_ask_boolean("\"boolean\":true\n").unwrap()); // newline boundary
        assert!(parse_ask_boolean(r#"{"a":["boolean":true]}"#).unwrap()); // `]` boundary
        assert!(!parse_ask_boolean(r#"{"boolean":false}"#).unwrap());
        assert!(!parse_ask_boolean(r#"{"boolean": false , "x":1}"#).unwrap());
        assert!(!parse_ask_boolean("\"boolean\":false").unwrap()); // EOF boundary

        // REJECT: junk-suffixed tokens the old `starts_with` scan silently accepted. THIS is the
        // sq-2gfe regression guard — `trueish` must NOT be read as `true`.
        assert!(parse_ask_boolean(r#"{"boolean": trueish}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": falsey}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": truex}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": falsex}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": true_}"#).is_err()); // `_` is not a boundary

        // REJECT: truncated / empty / malformed values (a prefix that is not the exact literal).
        assert!(parse_ask_boolean(r#"{"boolean": tr}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": fal}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean":}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": }"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean"}"#).is_err()); // no ':' after key
        assert!(parse_ask_boolean("").is_err()); // empty body
        assert!(parse_ask_boolean("   ").is_err()); // whitespace-only

        // REJECT: wrong case (JSON booleans are lowercase; the SPARQL spec inherits this).
        assert!(parse_ask_boolean(r#"{"boolean": True}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": FALSE}"#).is_err());

        // REJECT: wrong JSON type for `boolean` (string / number).
        assert!(parse_ask_boolean(r#"{"boolean": "true"}"#).is_err());
        assert!(parse_ask_boolean(r#"{"boolean": 1}"#).is_err());
    }

    // ---- well-known VoID URL derivation --------------------------------------------

    #[test]
    fn well_known_void_url_derivation() {
        assert_eq!(
            well_known_void_url("http://host/sparql").as_deref(),
            Some("http://host/.well-known/void")
        );
        assert_eq!(
            well_known_void_url("https://host:8080/db/sparql").as_deref(),
            Some("https://host:8080/.well-known/void")
        );
        // No authority ⇒ None.
        assert_eq!(well_known_void_url("not-a-url"), None);
    }

    // ---- SSRF classifier (mirrors the engine's is_forbidden_ip) ---------------------

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ssrf_forbids_private_and_metadata_ranges() {
        use std::net::IpAddr;
        let forbid = |s: &str| is_forbidden_ip(s.parse::<IpAddr>().unwrap());
        // Forbidden: loopback, RFC1918, link-local incl. cloud-metadata, CGNAT, ::1, ULA.
        assert!(forbid("127.0.0.1"));
        assert!(forbid("10.1.2.3"));
        assert!(forbid("172.16.5.5"));
        assert!(forbid("192.168.1.1"));
        assert!(forbid("169.254.169.254")); // cloud metadata
        assert!(forbid("100.64.0.1")); // CGNAT
        assert!(forbid("::1"));
        assert!(forbid("fc00::1")); // unique-local
        assert!(forbid("fe80::1")); // link-local v6
        assert!(forbid("::ffff:127.0.0.1")); // v4-mapped loopback can't smuggle through
                                             // Permitted: public addresses.
        assert!(!forbid("8.8.8.8"));
        assert!(!forbid("1.1.1.1"));
        assert!(!forbid("2606:4700:4700::1111"));
    }

    // ---- EgressFilterResolver::resolve (the ureq resolver that fronts every fetch) ----
    //
    // The classifier above proves WHICH addresses are forbidden; these prove the *resolver*
    // that ureq actually calls applies it correctly — including the host-authority parse
    // (`rsplit(':')`), the IPv6 bracket strip, and the `allow_private_host` allowlist bypass.
    // All cases use IP-LITERAL netlocs so `to_socket_addrs` is hermetic (no DNS lookup, no
    // network), so the tests are deterministic. [OPUS-4.8] sq-73om (follow-up to sq-nfxl).

    #[cfg(not(target_arch = "wasm32"))]
    fn resolver_denying() -> EgressFilterResolver {
        EgressFilterResolver {
            allow_private: std::sync::Arc::new(std::collections::HashSet::new()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn resolver_allowing(host: &str) -> EgressFilterResolver {
        let mut set = std::collections::HashSet::new();
        set.insert(host.to_ascii_lowercase());
        EgressFilterResolver {
            allow_private: std::sync::Arc::new(set),
        }
    }

    /// [OPUS-4.8] sq-g2xs: invoke the ureq-3 `Resolver` for a `host:port` netloc by building the
    /// `http://<netloc>/` URI the resolver parses, with a default `Config` + no-deadline timeout.
    #[cfg(not(target_arch = "wasm32"))]
    fn resolve_netloc(
        r: &EgressFilterResolver,
        netloc: &str,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        use ureq::unversioned::resolver::Resolver;
        let uri: ureq::http::Uri = format!("http://{netloc}/").parse().unwrap();
        let config = ureq::Agent::config_builder().build();
        let timeout = ureq::unversioned::transport::NextTimeout {
            after: ureq::unversioned::transport::time::Duration::NotHappening,
            reason: ureq::Timeout::Global,
        };
        r.resolve(&uri, &config, timeout)
    }

    /// `true` iff `e` is the egress-refusal `PermissionDenied` io error.
    #[cfg(not(target_arch = "wasm32"))]
    fn is_permission_denied(e: &ureq::Error) -> bool {
        matches!(e, ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied)
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_refuses_private_v4_literal() {
        // A bare host:port whose ONLY resolved address is private must be refused outright —
        // ureq never sees a dialable address.
        let err = resolve_netloc(&resolver_denying(), "10.0.0.5:443")
            .expect_err("a private-only host must be refused by the egress resolver");
        assert!(is_permission_denied(&err), "got {err:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_refuses_cloud_metadata_literal() {
        // The 169.254.169.254 cloud-metadata IP is the canonical SSRF target — refused.
        let err = resolve_netloc(&resolver_denying(), "169.254.169.254:80")
            .expect_err("cloud-metadata IP must be refused");
        assert!(is_permission_denied(&err), "got {err:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_strips_ipv6_brackets_and_refuses_loopback() {
        // The IPv6 loopback arrives bracketed (`[::1]:80`); the resolver must strip the
        // brackets to recover the host, classify ::1 as forbidden, and refuse.
        let err = resolve_netloc(&resolver_denying(), "[::1]:80")
            .expect_err("bracketed IPv6 loopback must be refused");
        assert!(is_permission_denied(&err), "got {err:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_permits_public_v4_literal() {
        // A public address survives the filter and is handed back to ureq to dial.
        let addrs = resolve_netloc(&resolver_denying(), "8.8.8.8:80")
            .expect("a public address must survive the egress filter");
        assert!(addrs.iter().any(|sa| sa.ip().to_string() == "8.8.8.8"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_permits_public_v6_literal() {
        // A public IPv6 literal (bracketed) is bracket-stripped, classified public, kept.
        let addrs = resolve_netloc(&resolver_denying(), "[2606:4700:4700::1111]:443")
            .expect("a public IPv6 address must survive");
        assert!(addrs
            .iter()
            .any(|sa| sa.ip().to_string() == "2606:4700:4700::1111"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_allowlist_reopens_private_v4_literal() {
        // The allowlist bypass (`HttpFetcher::allow_private_host`): the SAME private literal
        // refused by the default-deny resolver is now dialled because the bare host is on the
        // allowlist. This is the only path that lets a private federation endpoint through.
        let addrs = resolve_netloc(&resolver_allowing("10.0.0.5"), "10.0.0.5:443")
            .expect("an allowlisted private host must be permitted");
        assert!(addrs.iter().any(|sa| sa.ip().to_string() == "10.0.0.5"));
        // And it is exactly that host that opens it: a DIFFERENT allowlist entry does not.
        let err = resolve_netloc(&resolver_allowing("192.168.0.1"), "10.0.0.5:443")
            .expect_err("allowlisting a different host must not re-open 10.0.0.5");
        assert!(is_permission_denied(&err), "got {err:?}");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn resolve_allowlist_match_is_case_insensitive() {
        // `allow_private_host` lower-cases its key; the resolver lower-cases the parsed host,
        // so an IPv6 literal with upper-case hex digits still matches its allowlist entry.
        // (`fc00::AB` is unique-local ⇒ forbidden unless allowlisted.)
        let addrs = resolve_netloc(&resolver_allowing("fc00::ab"), "[fc00::AB]:80")
            .expect("case-insensitive allowlist match must permit the private host");
        assert!(addrs.iter().any(|sa| sa.ip().to_string() == "fc00::ab"));
    }
}
