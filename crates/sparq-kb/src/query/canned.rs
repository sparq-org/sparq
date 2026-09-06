//! Canned SPARQL queries over the ingested PKG — the **introspect → ground → ask**
//! library used by the `pkg-query` helper binary (sq-2m6zm.3) and by the
//! `query_canned` test.
//!
//! [OPUS-4.8] sq-2m6zm.3 (epic sq-2m6zm); design record
//! `research/dogfooding-sparq-knowledge-graph.md` §4.1 / §6 (PR #1063). 🤖 SPARQ agent
//! — dogfooding sparq as a project knowledge graph. Written while Fable unavailable;
//! flag for re-review when Fable returns.
//!
//! Each query is one of the **§4.1-class, verified-expressible** SPARQL frontier
//! queries (plain SELECT / `FILTER NOT EXISTS` / `GROUP BY` over the PKG terms). The
//! §4.2/§4.3 N3 rules are explicitly Phase-2/3 and are NOT used here.
//!
//! **Honesty.** These return exactly the binding rows the data contains — an
//! `answer`-sized result, not the source document. Whether that actually cuts an
//! agent's tokens is **measured by sq-2m6zm.4**, not claimed here.

/// A named, documented canned query. Queries whose `param` is `Some` accept one
/// argument substituted for the literal `{ARG}` token in [`sparql`](CannedQuery::sparql).
pub struct CannedQuery {
    /// The CLI / lookup name (`--query <name>`).
    pub name: &'static str,
    /// One-line description of what the query answers.
    pub about: &'static str,
    /// The SPARQL text. If [`param`](CannedQuery::param) is `Some`, it contains the
    /// `{ARG}` placeholder, replaced by the caller-supplied argument before execution.
    pub sparql: &'static str,
    /// `Some((label, default))` when the query is parameterised: the human label of the
    /// argument and the default substituted when none is given.
    pub param: Option<(&'static str, &'static str)>,
}

impl CannedQuery {
    /// The query text with `{ARG}` substituted: `arg` when given, else the registered
    /// default; for a non-parameterised query the text is returned unchanged.
    pub fn render(&self, arg: Option<&str>) -> String {
        match self.param {
            Some((_, default)) => self.sparql.replace("{ARG}", arg.unwrap_or(default)),
            None => self.sparql.to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// (a) INTROSPECT — the "schema" query: what classes + properties can I ask about?
// ---------------------------------------------------------------------------

/// INTROSPECT — list the PKG classes actually instantiated in the graph, each with a
/// count, so the agent learns what it can ask about without scanning the data.
pub const SCHEMA_CLASSES: CannedQuery = CannedQuery {
    name: "schema-classes",
    about: "INTROSPECT: the pkg: classes present in the graph + an instance count each",
    sparql: r#"
PREFIX pkg: <https://sparq.dev/ns/pkg#>
SELECT ?class (COUNT(?s) AS ?instances) WHERE {
  ?s a ?class .
  FILTER(STRSTARTS(STR(?class), STR(pkg:)))
} GROUP BY ?class ORDER BY DESC(?instances)"#,
    param: None,
};

/// INTROSPECT — list the predicates actually used in the graph, each with a use count,
/// so the agent learns the queryable property vocabulary (pkg: terms + the reused
/// PROV-O / SKOS / DC / CiTO predicates).
pub const SCHEMA_PROPERTIES: CannedQuery = CannedQuery {
    name: "schema-properties",
    about: "INTROSPECT: the predicates used in the graph + a use count each",
    sparql: r#"
SELECT ?p (COUNT(*) AS ?uses) WHERE { ?s ?p ?o }
GROUP BY ?p ORDER BY DESC(?uses)"#,
    param: None,
};

/// INTROSPECT — the **read-path facade** card: for each `pkg:` term, the fluent
/// `schema.org` term it bridges onto via the asserted `pkg.ttl` bridge axioms
/// (`rdfs:subClassOf` / `rdfs:subPropertyOf` / `skos:closeMatch`).
///
/// [`SCHEMA_CLASSES`] deliberately filters to the `pkg:` namespace, so on its own the
/// schema card shows the agent *none* of the facade vocabulary — even though
/// schema.org-as-top is the PKG's ratified read-path facade (bead `sq-mztg8.5`; the
/// facade-selection verdict is `sq-mztg8.4`, `research/fo-llm-bridge.md` §6 Phase 5).
/// That matters because the FO-KM Metric-1 measurement (`bench/fo-km/RESULTS.md`) found
/// the agent grounds *fluent* `schema:` terms markedly more reliably than rare academic
/// FO terms — so the facade is exactly the vocabulary worth surfacing at introspect time.
/// This query closes that gap without touching [`SCHEMA_CLASSES`], whose `pkg:`-only
/// filter is load-bearing (the FO-KM design record §7 notes the class card gets *noisier*
/// under closure, a small regression for agent grounding).
///
/// It reads the **asserted** bridge axioms, so it answers on the plain
/// [`load_pkg`](super::load_pkg) path with no `--close` step required; under OWL-RL
/// closure the bridged `schema:` types are additionally entailed on the instances
/// themselves and can be queried directly.
///
/// **Facade identity is a decision, not a constant.** The `schema:` namespace filter
/// below encodes the ratified schema.org-as-top choice; re-ratifying a different facade
/// (e.g. gist) would change this FILTER and the `pkg.ttl` bridge axioms together. As of
/// `sq-mztg8.4` no gist arm has been measured, so schema.org-as-top stands. [OPUS-5]
pub const FACADE_TERMS: CannedQuery = CannedQuery {
    name: "facade-terms",
    about: "INTROSPECT: the schema.org facade term each pkg: term bridges onto (the ratified read-path facade)",
    sparql: r#"
PREFIX pkg:    <https://sparq.dev/ns/pkg#>
PREFIX rdfs:   <http://www.w3.org/2000/01/rdf-schema#>
PREFIX skos:   <http://www.w3.org/2004/02/skos/core#>
PREFIX schema: <http://schema.org/>
SELECT ?pkgTerm ?bridge ?facadeTerm WHERE {
  ?pkgTerm ?bridge ?facadeTerm .
  FILTER(?bridge IN (rdfs:subClassOf, rdfs:subPropertyOf, skos:closeMatch))
  FILTER(STRSTARTS(STR(?pkgTerm), STR(pkg:)))
  FILTER(STRSTARTS(STR(?facadeTerm), STR(schema:)))
} ORDER BY ?pkgTerm ?facadeTerm"#,
    param: None,
};

// ---------------------------------------------------------------------------
// (b) GROUND — NL question → SPARQL over the introspected terms (worked templates)
// ---------------------------------------------------------------------------

/// GROUND — "what does the repo say about TOPIC?" The Findings about a topic, each with
/// its label + source section anchor + confidence, ordered by confidence. `{ARG}` is the
/// topic IRI (e.g. `https://sparq.dev/ns/pkg/kb#topic-zk-discipline`).
pub const FINDINGS_ABOUT: CannedQuery = CannedQuery {
    name: "findings-about",
    about: "GROUND: Findings about a topic IRI (label + source section + confidence)",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
PREFIX rdfs:    <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?section ?conf WHERE {
  ?f a pkg:Finding ;
     pkg:about <{ARG}> ;
     rdfs:label ?label ;
     dcterms:source ?section ;
     pkg:confidence ?conf .
} ORDER BY DESC(?conf)"#,
    param: Some((
        "topic IRI",
        "https://sparq.dev/ns/pkg/kb#topic-zk-discipline",
    )),
};

/// GROUND — "where was this decided / what is its provenance + confidence?" For every
/// Finding: its label, the source it was derived from, its assurance basis, and its
/// numeric confidence. The provenance + confidence are the citation half of the
/// guardrail, queryable.
pub const FINDING_PROVENANCE: CannedQuery = CannedQuery {
    name: "finding-provenance",
    about: "GROUND: provenance (source + assurance) + confidence of every Finding",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX prov:    <http://www.w3.org/ns/prov#>
PREFIX dcterms: <http://purl.org/dc/terms/>
PREFIX rdfs:    <http://www.w3.org/2000/01/rdf-schema#>
SELECT ?label ?source ?section ?assurance ?conf WHERE {
  ?f a pkg:Finding ;
     rdfs:label ?label ;
     prov:wasDerivedFrom ?source ;
     pkg:assurance ?assurance ;
     pkg:confidence ?conf .
  OPTIONAL { ?f dcterms:source ?section }
} ORDER BY DESC(?conf)"#,
    param: None,
};

/// GROUND — the **DQV-modelled** quality axis (sq-2489d.3): for every reified
/// `dqv:QualityMeasurement`, the subject it was `dqv:computedOn`, the `dqv:Metric` it
/// `dqv:isMeasurementOf`, and its `dqv:value`. This answers the same epistemic-weight
/// question as the [`FINDING_PROVENANCE`] `pkg:confidence` column, but via the standard
/// DQV model — so the confidence axis is queryable as named, distinct metrics, not one
/// overloaded scalar. Over a graph that records only the `pkg:confidence` shorthand (no
/// reified measurement) this returns zero rows — the shorthand still answers via
/// [`FINDING_PROVENANCE`]; the two are complementary, never contradictory.
pub const FINDING_QUALITY_DQV: CannedQuery = CannedQuery {
    name: "finding-quality-dqv",
    about: "GROUND: the DQV quality measurements (subject + metric + value) — the modelled confidence axis",
    sparql: r#"
PREFIX dqv: <http://www.w3.org/ns/dqv#>
SELECT ?subject ?metric ?value WHERE {
  ?m a dqv:QualityMeasurement ;
     dqv:computedOn ?subject ;
     dqv:isMeasurementOf ?metric ;
     dqv:value ?value .
} ORDER BY DESC(?value)"#,
    param: None,
};

/// GROUND — "which sources are still UNEXPLORED, so follow-up can be targeted?" Sources
/// whose `pkg:exploredStatus` is anything other than `pkg:Explored`, ordered by
/// follow-up priority. (Over the current Phase-1 ingest every source is `Explored`, so
/// this returns zero rows — the honest "none outstanding" answer, computed by the engine
/// rather than guessed.)
pub const UNEXPLORED_SOURCES: CannedQuery = CannedQuery {
    name: "unexplored-sources",
    about: "GROUND: sources NOT yet pkg:Explored (the targeted-follow-up list)",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT ?source ?title ?status ?prio WHERE {
  ?source a pkg:Source ;
          dcterms:title ?title ;
          pkg:exploredStatus ?status .
  FILTER(?status != pkg:Explored)
  OPTIONAL { ?source pkg:followUpPriority ?prio }
} ORDER BY ?prio"#,
    param: None,
};

/// GROUND — "what is the status / provenance of a source, by its explored-status?" The
/// full source catalog with its explored-status, follow-up priority and confidence —
/// the read-model of "what have I already looked at".
pub const SOURCE_STATUS: CannedQuery = CannedQuery {
    name: "source-status",
    about: "GROUND: the source catalog with explored-status + follow-up priority + confidence",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT ?title ?status ?prio ?conf WHERE {
  ?source a pkg:Source ;
          dcterms:title ?title ;
          pkg:exploredStatus ?status .
  OPTIONAL { ?source pkg:followUpPriority ?prio }
  OPTIONAL { ?source pkg:confidence ?conf }
} ORDER BY ?prio DESC(?conf)"#,
    param: None,
};

/// GROUND — "what does task `{ARG}` depend on, and what is the status of each
/// dependency?" The dependency edges of a task (`pkg:dependsOn`), each dependency's
/// title + status — answers "is X unblocked / what is blocking it". `{ARG}` is the bd id
/// (e.g. `sq-0po6`). bd's dotted child ids are projected with `_` (e.g. `sq-toze.1` →
/// `sq-toze_1`); pass the `dcterms:identifier` exactly as it appears in the data.
pub const TASK_DEPENDS_ON: CannedQuery = CannedQuery {
    name: "task-depends-on",
    about: "GROUND: what a task (bd id) depends on, with each dependency's status",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT ?depId ?depTitle ?depStatus WHERE {
  ?t a pkg:Task ; dcterms:identifier "{ARG}" ; pkg:dependsOn ?dep .
  ?dep dcterms:identifier ?depId ; pkg:status ?depStatus .
  OPTIONAL { ?dep dcterms:title ?depTitle }
} ORDER BY ?depId"#,
    param: Some(("bd task id", "sq-0po6")),
};

/// GROUND — "what is blocked-by task `{ARG}` (the downstream impact of finishing it)?"
/// The tasks that wait on a given task — the inverse direction. Written over
/// `pkg:dependsOn` (the §2.2 predicate; under OWL-RL closure this is equivalent to a
/// `?downstream pkg:blockedBy {ARG}` query). `{ARG}` is the bd id.
pub const TASK_BLOCKS: CannedQuery = CannedQuery {
    name: "task-blocks",
    about: "GROUND: the tasks that depend on (are blocked by) a given task (bd id)",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT ?downstreamId ?downstreamStatus ?downstreamTitle WHERE {
  ?blocker a pkg:Task ; dcterms:identifier "{ARG}" .
  ?downstream pkg:dependsOn ?blocker ;
              dcterms:identifier ?downstreamId ;
              pkg:status ?downstreamStatus .
  OPTIONAL { ?downstream dcterms:title ?downstreamTitle }
} ORDER BY ?downstreamId"#,
    param: Some(("bd task id", "sq-8thu")),
};

/// GROUND — "what are the high-follow-up-priority sources to read next?" The source
/// catalog ordered by `pkg:followUpPriority` ascending (lower = sooner) — the
/// targeted-reading queue, independent of explored-status.
pub const HIGH_FOLLOWUP_PRIORITY: CannedQuery = CannedQuery {
    name: "high-followup-priority",
    about: "GROUND: sources ordered by follow-up priority (lower = read sooner)",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT ?title ?prio ?status WHERE {
  ?source a pkg:Source ;
          dcterms:title ?title ;
          pkg:followUpPriority ?prio ;
          pkg:exploredStatus ?status .
} ORDER BY ?prio"#,
    param: None,
};

/// GROUND — the §4.1 ready-frontier (dependency half, verified-expressible TODAY): a
/// COUNT of open/in-progress tasks with no OPEN dependency, not human-gated, not an
/// umbrella. Proves the bd → Task projection is queryable as a read-model.
pub const READY_FRONTIER: CannedQuery = CannedQuery {
    name: "ready-frontier",
    about: "GROUND: §4.1 ready-frontier count (open, no open dep, not gated, not umbrella)",
    sparql: r#"
PREFIX pkg:     <https://sparq.dev/ns/pkg#>
PREFIX dcterms: <http://purl.org/dc/terms/>
SELECT (COUNT(DISTINCT ?t) AS ?ready) WHERE {
  ?t a pkg:Task ; pkg:status ?s ; pkg:priority ?prio .
  FILTER(?s IN (pkg:Open, pkg:InProgress))
  FILTER NOT EXISTS { ?t pkg:dependsOn ?b . ?b pkg:status ?bs . FILTER(?bs != pkg:Closed) }
  FILTER NOT EXISTS { ?t pkg:label ?l . FILTER(STRSTARTS(STR(?l), "needs:")) }
  FILTER NOT EXISTS { ?child dcterms:isPartOf ?t }
}"#,
    param: None,
};

/// Every canned query, in INTROSPECT → GROUND order — the registry the binary lists and
/// looks up by name.
pub const ALL: &[&CannedQuery] = &[
    &SCHEMA_CLASSES,
    &SCHEMA_PROPERTIES,
    &FACADE_TERMS,
    &FINDINGS_ABOUT,
    &FINDING_PROVENANCE,
    &FINDING_QUALITY_DQV,
    &UNEXPLORED_SOURCES,
    &SOURCE_STATUS,
    &TASK_DEPENDS_ON,
    &TASK_BLOCKS,
    &HIGH_FOLLOWUP_PRIORITY,
    &READY_FRONTIER,
];

/// Look up a canned query by its `--query <name>` name.
pub fn by_name(name: &str) -> Option<&'static CannedQuery> {
    ALL.iter().copied().find(|q| q.name == name)
}
