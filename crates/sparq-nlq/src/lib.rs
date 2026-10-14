// sq-jxl0: single-source the crate overview from README.md so crates.io
// (package.readme) and the docs.rs front page render identical content. The README's
// quickstart fences are `no_run` doctests (they open a fixture file / the `live` API).
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)] // sq-emay: crate has zero `unsafe`

use std::cell::RefCell;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sparq_core::Graph;
use sparq_engine::query_with_budget;
use sparq_introspect::Introspection;

// Re-exported so callers configuring the loop need only this crate's namespace.
pub use sparq_engine::{QueryBudget, QueryResult};

#[cfg(feature = "live")]
pub mod live;

// sq-2m6zm.6 (PKG NL-tool PRODUCT flavor): the configurable, provider-agnostic
// OpenAI-compatible endpoint client behind the opt-in `nlq-endpoint` feature. Off by
// default so the lean loop carries no HTTP client; the productized counterpart of the
// agent-flavor win (sq-zbyo7) for external users who bring their own cheap-model endpoint.
#[cfg(feature = "nlq-endpoint")]
pub mod endpoint;

pub mod constrain;
pub mod eval;
// sq-j1wv: prompt/data-injection + budget hardening. NOT feature-gated — a
// security control that ships off is not a control, it adds no dependency, and every
// transform is a no-op on benign text (so no recorded fixture moves).
pub mod guard;
pub mod link;

/// Read-only versus mutating classification for parsed SPARQL algebra.
#[cfg(feature = "query-policy")]
pub mod policy;

// sq-2489d.1 (GenAI-KB Phase 1): the cross-cutting provenance-join primitive
// and the PKG-native citation renderer, behind the opt-in `citations` feature so the lean
// NL→SPARQL loop carries zero provenance machinery in the default build.
#[cfg(feature = "citations")]
pub mod cite;
#[cfg(feature = "citations")]
pub mod provenance;
// sq-2489d.2 (GenAI-KB Phase 2): answer-qualification (hedge + abstention) over
// the SAME provenance join — reads `pkg:assurance`/`pkg:confidence` into a weakest-link
// verb hedge + verbal band + a `min_confidence` abstention floor. Gated on `citations`
// because it consumes the Phase-1 provenance primitive (no second join machinery).
#[cfg(feature = "citations")]
pub mod qualify;

// ---------------------------------------------------------------------------
// The LLM boundary
// ---------------------------------------------------------------------------

/// The single seam between this crate and any language model. Synchronous and
/// stateless by design: every call carries the full prompt (grounding, examples,
/// question, and — on repair rounds — the failed query and parser error), so
/// recording and replaying a session is just a list of `(prompt, completion)` pairs.
pub trait Llm {
    fn complete(&self, prompt: &str) -> Result<String, String>;
}

/// Shared backends: keep a handle outside the loop (e.g. an `Rc<RecordingLlm<_>>`
/// whose clone goes into [`Nlq::new`] while the original collects the exchanges).
impl<L: Llm + ?Sized> Llm for std::rc::Rc<L> {
    fn complete(&self, prompt: &str) -> Result<String, String> {
        (**self).complete(prompt)
    }
}

/// One recorded LLM call. The fixture format of [`ReplayLlm`] / [`RecordingLlm`]:
/// a JSON array of these.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Exchange {
    pub prompt: String,
    pub completion: String,
}

/// Replays recorded prompt→completion pairs from a fixture — **the CI path**.
/// Lookup is by exact prompt match: the prompt is fully deterministic (the schema
/// summary is a pure function of the graph, examples and templates are fixed), so a
/// fixture recorded once keeps replaying until the prompt construction or the
/// dataset changes — at which point re-record (see `examples/record_olympics.rs`).
pub struct ReplayLlm {
    exchanges: Vec<Exchange>,
}

impl ReplayLlm {
    pub fn new(exchanges: Vec<Exchange>) -> Self {
        Self { exchanges }
    }

    /// Loads a fixture produced by [`RecordingLlm::to_json`] / `save`.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let exchanges: Vec<Exchange> =
            serde_json::from_str(json).map_err(|e| format!("invalid replay fixture: {e}"))?;
        Ok(Self { exchanges })
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path.as_ref()).map_err(|e| {
            format!(
                "cannot read replay fixture {}: {e}",
                path.as_ref().display()
            )
        })?;
        Self::from_json(&text)
    }

    pub fn len(&self) -> usize {
        self.exchanges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exchanges.is_empty()
    }
}

impl Llm for ReplayLlm {
    fn complete(&self, prompt: &str) -> Result<String, String> {
        if let Some(e) = self.exchanges.iter().find(|e| e.prompt == prompt) {
            return Ok(e.completion.clone());
        }
        // A miss means the prompt construction (or the dataset) drifted from the
        // recording. Surface the question lines of what IS recorded to make the
        // mismatch diagnosable without dumping kilobytes of schema summary.
        let recorded: Vec<&str> = self
            .exchanges
            .iter()
            .filter_map(|e| e.prompt.lines().rev().find(|l| l.starts_with("Question:")))
            .collect();
        Err(format!(
            "replay miss: no recorded completion for this prompt (its question line: {:?}); \
             {} recorded exchanges with question lines {:?} — re-record the fixture",
            prompt.lines().rev().find(|l| l.starts_with("Question:")),
            self.exchanges.len(),
            recorded
        ))
    }
}

/// Wraps any [`Llm`] and records every `(prompt, completion)` pair, for producing
/// [`ReplayLlm`] fixtures. Failed calls are not recorded (a fixture replays the
/// happy path; errors should be re-derived live).
pub struct RecordingLlm<L: Llm> {
    inner: L,
    recorded: RefCell<Vec<Exchange>>,
}

impl<L: Llm> RecordingLlm<L> {
    pub fn new(inner: L) -> Self {
        Self {
            inner,
            recorded: RefCell::new(Vec::new()),
        }
    }

    pub fn exchanges(&self) -> Vec<Exchange> {
        self.recorded.borrow().clone()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&*self.recorded.borrow()).expect("exchanges serialise to JSON")
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        std::fs::write(path.as_ref(), self.to_json())
            .map_err(|e| format!("cannot write fixture {}: {e}", path.as_ref().display()))
    }
}

impl<L: Llm> Llm for RecordingLlm<L> {
    fn complete(&self, prompt: &str) -> Result<String, String> {
        let completion = self.inner.complete(prompt)?;
        self.recorded.borrow_mut().push(Exchange {
            prompt: prompt.to_string(),
            completion: completion.clone(),
        });
        Ok(completion)
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// A few-shot example embedded in the grounding prompt.
#[derive(Debug, Clone)]
pub struct Example {
    pub question: String,
    pub sparql: String,
}

/// Knobs for the ask loop. `Default` is the configuration the committed fixtures
/// were recorded under — change a knob, re-record.
#[derive(Debug, Clone)]
pub struct NlqConfig {
    /// Character budget handed to [`Introspection::to_text_summary`] — the grounding
    /// deck. The introspect crate truncates greedily at line granularity, most
    /// important information first.
    pub summary_budget_chars: usize,
    /// Maximum REPAIR rounds after the initial generation (a parse or execution
    /// failure sends the error + failed query back to the LLM). The design doc caps
    /// this at 3; the default is the leanest loop that still self-corrects.
    pub max_repair_rounds: usize,
    /// Wall-clock cap on ONE query execution (turned into a [`QueryBudget`] deadline
    /// at execution time, so the config is reusable across `ask` calls). LLM-generated
    /// queries are untrusted input to the engine — the default is bounded (10 s);
    /// set `None` only to opt out explicitly. Native only (wasm has no `Instant`).
    pub exec_timeout: Option<std::time::Duration>,
    /// Cap on the rows of any materialised (intermediate or final) result — the
    /// other half of the budget. Default: 1,000,000. `None` opts out.
    pub max_rows: Option<usize>,
    /// Few-shot examples. The defaults are schema-agnostic (rdf:type / rdfs:label
    /// only) so one prompt template serves any dataset.
    pub examples: Vec<Example>,
    /// Whether to include the introspect schema summary in the prompt. `true`
    /// (default) is the grounded loop the committed fixtures were recorded under.
    /// Set `false` for the **ungrounded baseline** the design doc requires
    /// (`research/genai-design.md` §4, `sparq-nlq` row: exec-accuracy must beat
    /// the same LLM WITHOUT grounding — "the grounding must pay for itself").
    /// Construction still runs the introspection scan (it is cheap and the field
    /// is read at prompt time), so one [`Nlq`] can be reconfigured grounded vs
    /// ungrounded; the eval harness drives both off the same graph. sq-05rv
    pub ground: bool,
    /// Entity & relation linking ([`link`]): resolve the question's proper nouns to
    /// concrete IRIs in this store and surface them (with structurally-similar siblings
    /// from `sparq-sim`) in the grounding prompt — the design's index-grounded linking
    /// (`research/genai-nl-to-sparql.md` §2.6/§8.3). OFF by default because enabling it
    /// changes the prompt string (and therefore any recorded [`ReplayLlm`] fixture must
    /// be re-recorded); only takes effect when [`ground`](Self::ground) is also `true`.
    /// Construction builds the linker (one scan per present label predicate) so it can be
    /// toggled per `ask` without rebuilding the [`Nlq`]. sq-uw40
    pub link_entities: bool,
    /// How many structurally-similar siblings ([`sparq_sim::Sim::most_similar`]) to
    /// attach to each linked entity (0 disables the expansion, linking labels only).
    /// Only read when [`link_entities`](Self::link_entities) is `true`.
    pub link_expand_k: usize,
    /// Cap on linked entities (and, separately, relations) rendered into the prompt, so
    /// the linking section stays bounded. Only read when linking is on.
    pub max_links: usize,
    /// **Exact literal-value linking** ([`link::EntityLinker::with_values`], `sq-na0q`):
    /// resolve a question span that is verbatim the lexical form of a literal this store
    /// holds to that literal — **datatype and language tag intact** (`"1994"^^xsd:gYear`,
    /// `"France"@en`) — and surface it, with the predicates it objects, in the prompt.
    /// The schema card names classes and predicates but never the *values*, so without
    /// this a `FILTER`/value-bound query can only guess a lexical form, and a guessed form
    /// matches no triple. The same tier also binds any IRI written out in the question
    /// directly against the dictionary ([`sparq_core::Graph::id_of`]).
    ///
    /// OFF by default (like [`link_entities`](Self::link_entities)) because it adds a
    /// prompt block — re-record [`ReplayLlm`] fixtures before flipping it. It is an
    /// *extension of* the linking section, so it is only read when
    /// [`link_entities`](Self::link_entities) is also `true`; enabling it costs one
    /// object-sorted scan at construction and a bounded index of short literals.
    pub link_values: bool,
    /// N2 **dictionary-grounded constraint** ([`constrain`], `sq-9yjp`). When `true`,
    /// a query that parses is additionally checked against the **live dictionary**
    /// *before* execution: every predicate / `rdf:type` class IRI must be a term the
    /// store actually holds (via [`sparq_core::Graph::id_of`]). An ungrounded IRI
    /// matches no triple — a *valid-but-wrong* query — so it becomes a targeted repair
    /// signal ("predicate X not in the dictionary; did you mean Y?") with
    /// nearest-namespace candidates pulled from the dictionary, exactly as
    /// `research/genai-nl-to-sparql.md` §6 prescribes. Strict logit-level
    /// grammar-constrained *decoding* is not implementable against the Anthropic
    /// Messages API (no logit/grammar parameter); this is the design doc's API-backend
    /// fallback (§11).
    ///
    /// **Default `false`** — and deliberately so. The check is an *additive* constraint
    /// with a real false-positive: a query whose answer is legitimately empty (a class
    /// with no instances, e.g. `SELECT ?r WHERE { ?r a ex:Robot }` over a robot-free
    /// graph) uses a perfectly valid term that is simply *absent* from the dictionary,
    /// and must not be "repaired". So it stays opt-in, the same way [`ground`](Self::ground)
    /// is a knob: the exec-accuracy harness measures raw model accuracy with it off,
    /// and a caller who wants tighter grounding (accepting that empty-answer questions
    /// then need it off) sets it `true`. sq-9yjp
    pub check_dictionary: bool,
    /// **Answer-qualification knob** (GenAI-KB Phase 2, `sq-2489d.2`; `--features
    /// citations`). When `Some`, [`Nlq::ask_qualified`] folds the answer's supporting
    /// `pkg:assurance`/`pkg:confidence` provenance into an [`qualify::AnswerQualification`]
    /// (verb hedge + verbal band + a `min_confidence` abstention floor) alongside the
    /// `Answer`. `None` (default) leaves the loop a pure NL→SPARQL run — qualification is
    /// purely additive and read-only, exactly like `citations`. Gated on the `citations`
    /// feature because it reuses the Phase-1 provenance join (no second join machinery).
    /// Does NOT change the prompt string, so it is fixture-compatible — unlike
    /// [`link_entities`](Self::link_entities), flipping it needs no re-record.
    #[cfg(feature = "citations")]
    pub qualify: Option<qualify::QualifyConfig>,
    /// **Injection + budget hardening** ([`guard`], `sq-j1wv`). Caps the question
    /// length before any LLM call, bounds the untrusted text echoed into a repair
    /// prompt, and refuses to execute a generated query that federates (`SERVICE`).
    /// On by default and fixture-compatible: every transform is a no-op on benign
    /// text, so an unchanged prompt stays byte-identical. See
    /// `research/nlq-threat-model.md` for what this does and does **not** contain.
    pub guard: guard::GuardConfig,
}

impl Default for NlqConfig {
    fn default() -> Self {
        Self {
            summary_budget_chars: 4000,
            max_repair_rounds: 1,
            exec_timeout: Some(std::time::Duration::from_secs(10)),
            max_rows: Some(1_000_000),
            examples: vec![
                Example {
                    question: "How many entities of each type are there?".into(),
                    sparql: "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
                             SELECT ?type (COUNT(?s) AS ?count)\n\
                             WHERE { ?s rdf:type ?type }\n\
                             GROUP BY ?type\n\
                             ORDER BY DESC(?count)"
                        .into(),
                },
                Example {
                    question: "Show a few entities together with their labels.".into(),
                    sparql: "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>\n\
                             SELECT ?s ?label\n\
                             WHERE { ?s rdfs:label ?label }\n\
                             LIMIT 10"
                        .into(),
                },
            ],
            ground: true,
            // Linking off by default: it changes the prompt string (re-record fixtures
            // before flipping it on a recorded dataset). sq-uw40
            link_entities: false,
            link_expand_k: 3,
            max_links: 8,
            // Value linking off by default for the same reason as entity linking: it adds
            // a prompt block, so a recorded fixture must be re-recorded. sq-na0q
            link_values: false,
            check_dictionary: false,
            // Qualification off by default: additive + fixture-compatible, opt in when you
            // want hedged/abstaining answers over a provenance-bearing graph.
            #[cfg(feature = "citations")]
            qualify: None,
            // Hardening ON by default (sq-j1wv): benign input is untouched, so the
            // hardened posture costs nothing to keep.
            guard: guard::GuardConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Answer + transcript
// ---------------------------------------------------------------------------

/// What happened to one LLM completion inside the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// Parsed and executed; the query returned this many rows.
    Ok { rows: usize },
    /// The completion contained no extractable SPARQL.
    NoSparql,
    /// spargebra rejected the extracted query.
    ParseError(String),
    /// The engine rejected or aborted the (syntactically valid) query.
    ExecError(String),
    /// N2 dictionary constraint (`sq-9yjp`): the query parsed and executed, but used a
    /// predicate / class IRI absent from the live dictionary (so it matched no
    /// triples). Carries the ungrounded terms; the repair message lists them with
    /// nearest-namespace suggestions.
    UngroundedTerms(Vec<constrain::UnknownTerm>),
    /// Hardening (`sq-j1wv`): the query parsed but used a construct the loop refuses to
    /// execute — today, `SERVICE` federation. Carries what was refused; the repair
    /// message names each one. The query was **not** executed.
    Forbidden(Vec<guard::Forbidden>),
}

/// One generate→validate(→execute) round, kept verbatim for auditability — the
/// transcript IS the provenance of the answer.
#[derive(Debug, Clone)]
pub struct Turn {
    pub prompt: String,
    pub completion: String,
    /// The SPARQL extracted from the completion (`None` on [`TurnOutcome::NoSparql`]).
    pub sparql: Option<String>,
    pub outcome: TurnOutcome,
}

/// A successful run of the loop.
pub struct Answer {
    /// The query that produced `result` (post-repair, if any).
    pub sparql: String,
    /// Materialised solutions. ASK answers use the engine's unit-row encoding:
    /// zero variables, one empty row iff true.
    pub result: QueryResult,
    /// How many repair rounds were needed (0 = first completion was good).
    pub repairs: usize,
    /// Every round, including failed ones.
    pub transcript: Vec<Turn>,
}

// Manual impl: `QueryResult` (sparq-engine) does not derive `Debug`, and this crate
// does not modify core crates (workspace invariant) — summarise it instead.
impl std::fmt::Debug for Answer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Answer")
            .field("sparql", &self.sparql)
            .field("result_vars", &self.result.vars)
            .field("result_rows", &self.result.len())
            .field("repairs", &self.repairs)
            .field("transcript_turns", &self.transcript.len())
            .finish()
    }
}

// sq-2489d.1 (GenAI-KB Phase 1, Use 3): render citations for an answer from
// in-graph provenance. Gated behind `citations`; takes the `&Graph` the answer was
// produced over (the `Answer` is graph-lifetime-free by design, so the caller passes it
// back in — the same graph it called `ask` on).
#[cfg(feature = "citations")]
impl Answer {
    /// Resolve every answer-row binding to its in-graph provenance and render numbered
    /// citations (`[1]` → `prov:wasDerivedFrom` source + `dcterms:source` anchor +
    /// `pkg:confidence`/`pkg:assurance`). Citations are emitted from provenance, **never**
    /// generated, so each one resolves to a real in-graph source and un-sourced bindings
    /// are reported as "no source recorded", never guessed. See [`cite`] / [`provenance`].
    ///
    /// `graph` must be the graph this answer was produced over (the one passed to
    /// [`Nlq::new`]); the citation is the provenance of the binding in *that* graph.
    pub fn citations(&self, graph: &Graph) -> cite::CitedAnswer {
        cite::cite_result(graph, &self.result)
    }

    /// Qualify this answer from in-graph provenance (GenAI-KB Phase 2, `sq-2489d.2`):
    /// fold the supporting bindings' `pkg:assurance` / `pkg:confidence` weakest-link into a
    /// verb hedge + verbal confidence band, and, below `config.min_confidence`, abstain.
    /// Reflects asserted assurance — **not** a calibrated confidence (see
    /// [`qualify::AnswerQualification`]). `graph` must be the graph this answer was
    /// produced over (the one passed to [`Nlq::new`]).
    pub fn qualify(
        &self,
        graph: &Graph,
        config: &qualify::QualifyConfig,
    ) -> qualify::AnswerQualification {
        qualify::qualify_result(graph, &self.result, config)
    }
}

/// A failed run — carries the full transcript so the failure is diagnosable.
#[derive(Debug)]
pub struct NlqError {
    pub message: String,
    pub transcript: Vec<Turn>,
}

impl std::fmt::Display for NlqError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (after {} turn(s))",
            self.message,
            self.transcript.len()
        )
    }
}

impl std::error::Error for NlqError {}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

/// NL→SPARQL over one graph. Construction runs the introspection scan once and
/// renders the schema summary; `ask` is then grounding-free of further index work
/// until execution.
pub struct Nlq<'g> {
    graph: &'g Graph,
    schema_summary: String,
    /// The entity/relation linker — built (and the label index scanned) once at
    /// construction when [`NlqConfig::link_entities`] is set, then reused per `ask`.
    /// `None` when linking is off (the default), so the loop pays nothing for it.
    linker: Option<link::EntityLinker<'g>>,
    llm: Box<dyn Llm>,
    config: NlqConfig,
}

impl<'g> Nlq<'g> {
    pub fn new(graph: &'g Graph, llm: Box<dyn Llm>) -> Self {
        Self::with_config(graph, llm, NlqConfig::default())
    }

    pub fn with_config(graph: &'g Graph, llm: Box<dyn Llm>, config: NlqConfig) -> Self {
        // The summary is DATA-derived (it renders sampled values from the graph), so it
        // is untrusted text spliced into the prompt: neutralize fences and stray control
        // characters, keeping the deck's line structure. No-op on ordinary data.
        // sq-j1wv
        let schema_summary = guard::sanitize_block(
            &Introspection::build(graph).to_text_summary(config.summary_budget_chars),
        )
        .into_owned();
        let linker = config.link_entities.then(|| {
            let linker = link::EntityLinker::build(graph, config.link_expand_k, config.max_links);
            // The exact literal-value tier is a second, opt-in index over the same linker
            // (sq-na0q) — built here so `ask` pays only string lookups.
            if config.link_values {
                linker.with_values()
            } else {
                linker
            }
        });
        Self {
            graph,
            schema_summary,
            linker,
            llm,
            config,
        }
    }

    /// The grounding prompt for `question` — public (and deterministic) so that
    /// fixtures can be constructed and inspected outside the loop.
    ///
    /// When [`NlqConfig::ground`] is `false` the schema summary is omitted: this is
    /// the **ungrounded baseline** prompt (`research/genai-design.md` §4 — the same
    /// LLM with no schema deck). The two prompts are distinct strings, so a fixture
    /// recorded for one does not collide with the other under [`ReplayLlm`].
    ///
    /// The question is sanitized ([`guard::flatten_untrusted`]) before it is spliced in,
    /// so it cannot open a code fence or forge a prompt line; benign questions are
    /// unchanged, so a recorded fixture keeps replaying. sq-j1wv
    pub fn prompt_for(&self, question: &str) -> String {
        let question = guard::flatten_untrusted(question);
        let question = question.as_ref();
        let mut p = String::new();
        if self.config.ground {
            p.push_str(
                "You are a SPARQL query writer. Given the schema summary of an RDF dataset and a \
                 question, write ONE SPARQL 1.1 SELECT or ASK query that answers the question.\n\
                 \n\
                 Rules:\n\
                 - Use only classes and predicates that appear in the schema summary.\n\
                 - Declare every prefix you use with PREFIX lines (expand them from the summary's \
                 prefix glossary).\n\
                 - Do not use property paths or federation.\n\
                 - Output exactly one ```sparql code block and nothing else.\n\
                 \n",
            );
            p.push_str(&self.schema_summary);
            if !p.ends_with('\n') {
                p.push('\n');
            }
            p.push('\n');
            // Index-grounded entity/relation linking (sparq-sim), appended to the schema
            // deck when enabled. Adds nothing — and leaves the prompt byte-identical — if
            // the question linked nothing. sq-uw40
            if let Some(linker) = &self.linker {
                if let Some(section) = linker.link(question).to_prompt_section() {
                    p.push_str(&section);
                    p.push('\n');
                }
            }
        } else {
            // Ungrounded: no schema deck. The model must guess vocabulary — the
            // baseline that grounding has to beat.
            p.push_str(
                "You are a SPARQL query writer. Given a question about an RDF dataset, write ONE \
                 SPARQL 1.1 SELECT or ASK query that answers the question.\n\
                 \n\
                 Rules:\n\
                 - Declare every prefix you use with PREFIX lines.\n\
                 - Do not use property paths or federation.\n\
                 - Output exactly one ```sparql code block and nothing else.\n\
                 \n",
            );
        }
        for ex in &self.config.examples {
            p.push_str(&format!(
                "Question: {}\n```sparql\n{}\n```\n\n",
                ex.question, ex.sparql
            ));
        }
        p.push_str(&format!("Question: {question}"));
        p
    }

    /// The repair prompt: the failed query and the error, stacked onto the full
    /// grounding prompt (the trait is stateless — every call must carry everything).
    ///
    /// Both echoed strings are untrusted — the query is the model's own output, the
    /// error is derived from it — so each is fence-neutralized and capped at
    /// [`guard::GuardConfig::max_echo_chars`] (truncation is marked, never silent).
    /// The failed query keeps its newlines: it is meant to be read back as SPARQL.
    /// sq-j1wv
    pub fn repair_prompt_for(&self, question: &str, failed_sparql: &str, error: &str) -> String {
        let max_echo = self.config.guard.max_echo_chars;
        let failed_sparql = guard::neutralize_fences(failed_sparql);
        let failed_sparql = guard::truncate_untrusted(&failed_sparql, max_echo);
        let error = guard::sanitize_block(error);
        let error = guard::truncate_untrusted(&error, max_echo);
        format!(
            "{}\n\nYour previous query failed. Here is what you wrote:\n```sparql\n{}\n```\n\n\
             Error: {}\n\nFix the query. Output exactly one ```sparql code block and nothing else.",
            self.prompt_for(question),
            failed_sparql,
            error
        )
    }

    /// The per-execution [`QueryBudget`]: built fresh at execution time so the
    /// configured timeout is a per-query duration, not a fixed absolute deadline.
    fn execution_budget(&self) -> QueryBudget {
        let mut b = QueryBudget::unlimited();
        b.max_rows = self.config.max_rows;
        #[cfg(not(target_arch = "wasm32"))]
        {
            b.deadline = self
                .config
                .exec_timeout
                .map(|d| std::time::Instant::now() + d);
        }
        b
    }

    /// Runs the loop: ground → generate → validate → execute, with up to
    /// `max_repair_rounds` repair rounds on parse or execution failure.
    ///
    /// The question is checked against [`guard::GuardConfig`] **first**: an over-long
    /// question is rejected with an empty transcript, before a single token is spent.
    /// sq-j1wv
    pub fn ask(&self, question: &str) -> Result<Answer, NlqError> {
        let question = match guard::check_question(question, &self.config.guard) {
            Ok(q) => q,
            Err(e) => {
                return Err(NlqError {
                    message: format!("question rejected: {e}"),
                    transcript: Vec::new(),
                })
            }
        };
        let question = question.as_ref();
        let mut transcript: Vec<Turn> = Vec::new();
        let mut prompt = self.prompt_for(question);
        // Initial round + repair rounds.
        for round in 0..=self.config.max_repair_rounds {
            let completion = match self.llm.complete(&prompt) {
                Ok(c) => c,
                Err(e) => {
                    return Err(NlqError {
                        message: format!("LLM error: {e}"),
                        transcript,
                    })
                }
            };

            // VALIDATE: extract, then parse with spargebra (the parser error message
            // is the repair signal).
            let (sparql, failure): (Option<String>, Option<(String, TurnOutcome)>) =
                match extract_sparql(&completion) {
                    None => (
                        None,
                        Some((
                            "the completion contained no SPARQL code block".to_string(),
                            TurnOutcome::NoSparql,
                        )),
                    ),
                    Some(q) => match spargebra::SparqlParser::new().parse_query(&q) {
                        Err(e) => {
                            let msg = e.to_string();
                            (Some(q), Some((msg.clone(), TurnOutcome::ParseError(msg))))
                        }
                        Ok(parsed) => {
                            // Hardening (`sq-j1wv`): whatever the model was asked — or
                            // talked into — writing, refuse the constructs whose
                            // CONSEQUENCES the loop will not accept (outbound SERVICE
                            // federation) before the engine ever sees the query.
                            // Mutation needs no check: `parse_query` cannot yield one.
                            //
                            let forbidden =
                                guard::forbidden_constructs(&parsed, &self.config.guard);
                            // N2 dictionary constraint (`sq-9yjp`): a query whose
                            // predicate/class IRIs are not in the live dictionary
                            // cannot match — turn it into a targeted repair signal
                            // BEFORE spending an execution on a guaranteed-empty query.
                            let unknowns = if forbidden.is_empty() && self.config.check_dictionary {
                                constrain::unknown_terms(self.graph, &parsed)
                            } else {
                                Vec::new()
                            };
                            if !forbidden.is_empty() {
                                let msg = guard::forbidden_repair_message(&forbidden);
                                (Some(q), Some((msg, TurnOutcome::Forbidden(forbidden))))
                            } else if !unknowns.is_empty() {
                                let msg = constrain::dictionary_repair_message(&unknowns);
                                (Some(q), Some((msg, TurnOutcome::UngroundedTerms(unknowns))))
                            } else {
                                // EXECUTE under the budget. Engine-side failures
                                // (unsupported form, budget trip) are repairable too.
                                match query_with_budget(self.graph, &q, &self.execution_budget()) {
                                    Err(e) => {
                                        (Some(q), Some((e.clone(), TurnOutcome::ExecError(e))))
                                    }
                                    Ok(result) => {
                                        transcript.push(Turn {
                                            prompt,
                                            completion,
                                            sparql: Some(q.clone()),
                                            outcome: TurnOutcome::Ok { rows: result.len() },
                                        });
                                        return Ok(Answer {
                                            sparql: q,
                                            result,
                                            repairs: round,
                                            transcript,
                                        });
                                    }
                                }
                            }
                        }
                    },
                };

            let (error, outcome) = failure.expect("non-success path always has a failure");
            transcript.push(Turn {
                prompt: prompt.clone(),
                completion,
                sparql: sparql.clone(),
                outcome,
            });
            if round == self.config.max_repair_rounds {
                return Err(NlqError {
                    message: format!("no valid query after {round} repair round(s): {error}"),
                    transcript,
                });
            }
            prompt = self.repair_prompt_for(
                question,
                sparql.as_deref().unwrap_or("(no query produced)"),
                &error,
            );
        }
        unreachable!("loop returns from its last round")
    }

    /// [`ask`](Self::ask) THEN qualify the answer from in-graph provenance (GenAI-KB
    /// Phase 2, `sq-2489d.2`). Runs the loop unchanged, then folds the answer's supporting
    /// `pkg:assurance`/`pkg:confidence` into an [`qualify::AnswerQualification`] using
    /// [`NlqConfig::qualify`] (or [`qualify::QualifyConfig::default`] when that is `None`,
    /// so the call always yields a qualification). The hedge/abstention is **post-hoc and
    /// read-only** — it never alters the query, the execution, or the transcript, so it
    /// composes cleanly with the existing loop and the recorded fixtures.
    #[cfg(feature = "citations")]
    pub fn ask_qualified(&self, question: &str) -> Result<QualifiedAnswer, NlqError> {
        let answer = self.ask(question)?;
        let cfg = self.config.qualify.clone().unwrap_or_default();
        let qualification = answer.qualify(self.graph, &cfg);
        Ok(QualifiedAnswer {
            answer,
            qualification,
        })
    }
}

/// An [`Answer`] paired with its provenance-derived [`qualify::AnswerQualification`]
/// (GenAI-KB Phase 2, `sq-2489d.2`). Returned by [`Nlq::ask_qualified`]; the
/// `qualification` reflects asserted assurance — **not** a calibrated confidence.
#[cfg(feature = "citations")]
#[derive(Debug)]
pub struct QualifiedAnswer {
    /// The underlying loop answer — unchanged by qualification.
    pub answer: Answer,
    /// The hedge + band + abstention decision over the answer's supporting provenance.
    pub qualification: qualify::AnswerQualification,
}

/// Extracts the SPARQL query from a completion: the first ```sparql fenced block,
/// else the first anonymous ``` fenced block, else — if the text itself looks like a
/// bare query — the whole trimmed completion.
pub fn extract_sparql(completion: &str) -> Option<String> {
    if let Some(q) = extract_fenced(completion, "```sparql") {
        return Some(q);
    }
    if let Some(q) = extract_fenced(completion, "```") {
        return Some(q);
    }
    let t = completion.trim();
    let upper = t.to_ascii_uppercase();
    if upper.starts_with("PREFIX") || upper.starts_with("SELECT") || upper.starts_with("ASK") {
        return Some(t.to_string());
    }
    None
}

fn extract_fenced(text: &str, opener: &str) -> Option<String> {
    let start = text.find(opener)? + opener.len();
    // The fence opener runs to end-of-line (tolerates ```sparql\n and bare ```\n).
    let body_start = text[start..].find('\n').map(|i| start + i + 1)?;
    let body_end = text[body_start..].find("```").map(|i| body_start + i)?;
    let q = text[body_start..body_end].trim();
    (!q.is_empty()).then(|| q.to_string())
}

// ---------------------------------------------------------------------------
// Tests (offline: tiny in-memory graph, scripted/replayed LLMs)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Closure-backed test backend.
    struct FnLlm<F: Fn(&str) -> Result<String, String>>(F);
    impl<F: Fn(&str) -> Result<String, String>> Llm for FnLlm<F> {
        fn complete(&self, prompt: &str) -> Result<String, String> {
            (self.0)(prompt)
        }
    }

    fn tiny_graph() -> Graph {
        let ttl = r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:alice a ex:Person ; rdfs:label "Alice" ; ex:knows ex:bob .
            ex:bob a ex:Person ; rdfs:label "Bob" .
            ex:acme a ex:Company ; rdfs:label "Acme" .
        "#;
        Graph::load_str(ttl, "turtle").expect("tiny graph parses")
    }

    const COUNT_PEOPLE: &str = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>\n\
         PREFIX ex: <http://example.org/>\n\
         SELECT (COUNT(?p) AS ?n) WHERE { ?p rdf:type ex:Person }";

    #[test]
    fn extract_prefers_sparql_fence() {
        let c = "Here you go:\n```sparql\nSELECT * WHERE { ?s ?p ?o }\n```\nDone.";
        assert_eq!(extract_sparql(c).unwrap(), "SELECT * WHERE { ?s ?p ?o }");
    }

    #[test]
    fn extract_falls_back_to_anonymous_fence_and_bare_text() {
        let c = "```\nASK { ?s ?p ?o }\n```";
        assert_eq!(extract_sparql(c).unwrap(), "ASK { ?s ?p ?o }");
        let bare = "  SELECT ?s WHERE { ?s ?p ?o }  ";
        assert_eq!(
            extract_sparql(bare).unwrap(),
            "SELECT ?s WHERE { ?s ?p ?o }"
        );
        assert_eq!(extract_sparql("I cannot answer that."), None);
    }

    /// An EMPTY fenced block yields `None` (the `(!q.is_empty())` guard in `extract_fenced`),
    /// not an empty string the loop would then mis-report as `NoSparql`-via-parse. An
    /// UNCLOSED fence (opener, no closing ```` ``` ````) likewise yields `None` — the
    /// `body_end` search fails. Neither path is exercised by the happy-path tests.
    #[test]
    fn extract_returns_none_for_empty_or_unclosed_fence() {
        // Empty `sparql` fence body → None (not Some("")).
        assert_eq!(extract_sparql("```sparql\n```"), None);
        // Whitespace-only fence body trims to empty → None.
        assert_eq!(extract_sparql("```sparql\n   \n```"), None);
        // Opener present but no closing fence → None (no body_end).
        assert_eq!(
            extract_sparql("```sparql\nSELECT * WHERE { ?s ?p ?o }"),
            None
        );
        // Anonymous empty fence → None as well.
        assert_eq!(extract_sparql("```\n```"), None);
    }

    /// Bare-text extraction is case-insensitive on the leading keyword and accepts `ASK`,
    /// `SELECT`, and `PREFIX` starts — but rejects other leading words. Guards the
    /// `to_ascii_uppercase().starts_with(...)` branch.
    #[test]
    fn extract_bare_text_is_case_insensitive_and_keyword_gated() {
        assert_eq!(
            extract_sparql("ask { ?s ?p ?o }").unwrap(),
            "ask { ?s ?p ?o }"
        );
        assert_eq!(
            extract_sparql("prefix ex: <http://e/> select ?s {}").unwrap(),
            "prefix ex: <http://e/> select ?s {}"
        );
        // A non-SPARQL leading keyword is NOT picked up as a bare query.
        assert_eq!(extract_sparql("DESCRIBE is unsupported here"), None);
        assert_eq!(extract_sparql("construct your own"), None);
    }

    /// `RecordingLlm` records ONLY successful calls — the documented invariant ("Failed
    /// calls are not recorded; a fixture replays the happy path"). A failing inner call
    /// propagates the error and leaves the recording empty, so a saved fixture never carries
    /// a recorded failure.
    #[test]
    fn recording_llm_does_not_record_failed_calls() {
        let rec = RecordingLlm::new(FnLlm(|p: &str| {
            if p.contains("boom") {
                Err("inner failure".into())
            } else {
                Ok("ok-completion".into())
            }
        }));
        // A failing call propagates the error and records nothing.
        assert_eq!(rec.complete("boom"), Err("inner failure".into()));
        assert!(
            rec.exchanges().is_empty(),
            "failed call must not be recorded"
        );
        // A successful call IS recorded.
        assert_eq!(rec.complete("fine").unwrap(), "ok-completion");
        assert_eq!(rec.exchanges().len(), 1);
        assert_eq!(rec.exchanges()[0].prompt, "fine");
        assert_eq!(rec.exchanges()[0].completion, "ok-completion");
        // A second failure still does not grow the recording past the one success.
        assert!(rec.complete("boom again? boom").is_err());
        assert_eq!(rec.exchanges().len(), 1);
    }

    /// `ReplayLlm::from_json` over the empty fixture `[]` is `is_empty()`, `len()==0`, and
    /// every lookup misses with the diagnosable re-record error. Also: a malformed fixture
    /// surfaces an `invalid replay fixture` error rather than panicking.
    #[test]
    fn replay_llm_empty_and_malformed_fixtures() {
        let empty = ReplayLlm::from_json("[]").expect("empty array is a valid fixture");
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        let err = empty
            .complete("Question: anything")
            .expect_err("an empty fixture always misses");
        assert!(err.contains("replay miss"), "{err}");
        assert!(err.contains("0 recorded exchanges"), "{err}");

        // A malformed fixture is an error, never a panic. (`ReplayLlm` is not `Debug`, so
        // match on the result rather than `unwrap_err`.)
        match ReplayLlm::from_json("{not json") {
            Err(e) => assert!(
                e.contains("invalid replay fixture"),
                "malformed fixture must surface a descriptive error, got {e}"
            ),
            Ok(_) => panic!("malformed JSON must not parse as a fixture"),
        }
    }

    #[test]
    fn prompt_contains_grounding_examples_and_question() {
        let g = tiny_graph();
        let nlq = Nlq::new(&g, Box::new(FnLlm(|_| Err("unused".into()))));
        let p = nlq.prompt_for("How many people are there?");
        assert!(p.starts_with("You are a SPARQL query writer."));
        assert!(p.contains("# Schema summary"));
        assert!(p.contains("ex:Person") || p.contains("Person")); // grounded in THIS graph
        assert!(p.contains("Question: How many entities of each type are there?")); // few-shot
        assert!(p.ends_with("Question: How many people are there?"));
    }

    #[test]
    fn ask_happy_path() {
        let g = tiny_graph();
        let nlq = Nlq::new(
            &g,
            Box::new(FnLlm(|_| Ok(format!("```sparql\n{COUNT_PEOPLE}\n```")))),
        );
        let a = nlq
            .ask("How many people are there?")
            .expect("loop succeeds");
        assert_eq!(a.repairs, 0);
        assert_eq!(a.result.len(), 1);
        assert_eq!(a.transcript.len(), 1);
        assert_eq!(a.transcript[0].outcome, TurnOutcome::Ok { rows: 1 });
        let n = a.result.rows[0][0].as_ref().expect("bound count");
        assert!(n.to_string().contains('2'), "two people, got {n}");
    }

    #[test]
    fn ask_repairs_a_parse_error_once() {
        let g = tiny_graph();
        // First completion has a syntax error (unclosed brace); the repair prompt
        // must carry the failed query + parser error; second completion is fixed.
        let nlq =
            Nlq::new(
                &g,
                Box::new(FnLlm(|prompt| {
                    if prompt.contains("Your previous query failed") {
                        assert!(prompt.contains("SELECT (COUNT(?p) AS ?n) WHERE { ?p"));
                        Ok(format!("```sparql\n{COUNT_PEOPLE}\n```"))
                    } else {
                        Ok("```sparql\nSELECT (COUNT(?p) AS ?n) WHERE { ?p rdf:type ex:Person\n```"
                        .to_string())
                    }
                })),
            );
        let a = nlq
            .ask("How many people are there?")
            .expect("repair succeeds");
        assert_eq!(a.repairs, 1);
        assert_eq!(a.transcript.len(), 2);
        assert!(matches!(
            a.transcript[0].outcome,
            TurnOutcome::ParseError(_)
        ));
        assert_eq!(a.transcript[1].outcome, TurnOutcome::Ok { rows: 1 });
    }

    #[test]
    fn ask_gives_up_after_the_repair_budget() {
        let g = tiny_graph();
        let nlq = Nlq::new(&g, Box::new(FnLlm(|_| Ok("no query here, sorry".into()))));
        let err = nlq.ask("How many people are there?").unwrap_err();
        assert!(err
            .message
            .contains("no valid query after 1 repair round(s)"));
        assert_eq!(err.transcript.len(), 2); // initial + one repair, both NoSparql
        assert!(err
            .transcript
            .iter()
            .all(|t| t.outcome == TurnOutcome::NoSparql));
    }

    #[test]
    fn record_then_replay_round_trips() {
        let g = tiny_graph();
        let recorder = RecordingLlm::new(FnLlm(|_| Ok(format!("```sparql\n{COUNT_PEOPLE}\n```"))));
        // Drive the recorder directly with the loop's own (public) prompt builder.
        let probe = Nlq::new(&g, Box::new(FnLlm(|_| Err("unused".into()))));
        let prompt = probe.prompt_for("How many people are there?");
        recorder.complete(&prompt).unwrap();
        let json = recorder.to_json();

        // ...replay it through the full loop.
        let replay = ReplayLlm::from_json(&json).unwrap();
        assert_eq!(replay.len(), 1);
        let nlq = Nlq::new(&g, Box::new(replay));
        let a = nlq
            .ask("How many people are there?")
            .expect("replay run succeeds");
        assert_eq!(a.result.len(), 1);

        // A different question is a replay MISS with a diagnosable error.
        let replay = ReplayLlm::from_json(&json).unwrap();
        let nlq = Nlq::new(&g, Box::new(replay));
        let err = nlq.ask("Something never recorded?").unwrap_err();
        assert!(err.message.contains("replay miss"), "{}", err.message);
    }

    fn linkable_graph() -> Graph {
        let ttl = r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:tarantino a ex:Director ; rdfs:label "Quentin Tarantino" ; ex:directed ex:pulp .
            ex:nolan a ex:Director ; rdfs:label "Christopher Nolan" ; ex:directed ex:inception .
            ex:pulp a ex:Film ; rdfs:label "Pulp Fiction" ; ex:director ex:tarantino .
            ex:inception a ex:Film ; rdfs:label "Inception" ; ex:director ex:nolan .
        "#;
        Graph::load_str(ttl, "turtle").expect("graph parses")
    }

    #[test]
    fn linking_off_by_default_leaves_prompt_unchanged() {
        let g = linkable_graph();
        let nlq = Nlq::new(&g, Box::new(FnLlm(|_| Err("unused".into()))));
        let p = nlq.prompt_for("What did Quentin Tarantino direct?");
        assert!(!p.contains("# Linked from the question"));
    }

    #[test]
    fn linking_on_injects_linked_iris_and_siblings() {
        let g = linkable_graph();
        let cfg = NlqConfig {
            link_entities: true,
            link_expand_k: 3,
            ..NlqConfig::default()
        };
        let nlq = Nlq::with_config(&g, Box::new(FnLlm(|_| Err("unused".into()))), cfg);
        let p = nlq.prompt_for("Who is the director of films by Quentin Tarantino?");
        assert!(
            p.contains("# Linked from the question"),
            "linking section missing"
        );
        assert!(
            p.contains("http://example.org/tarantino"),
            "linked entity IRI missing"
        );
        // sparq-sim should surface the sibling Director (same structural shape).
        assert!(
            p.contains("http://example.org/nolan"),
            "structural sibling missing"
        );
        // The relation "director" should link from the word "director".
        assert!(
            p.contains("http://example.org/director"),
            "linked relation missing"
        );
    }

    /// `linkable_graph` plus the literal values a schema card can never carry.
    fn valued_graph() -> Graph {
        let ttl = r#"
            @prefix ex: <http://example.org/> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:pulp a ex:Film ; rdfs:label "Pulp Fiction" ; ex:year "1994"^^xsd:gYear .
            ex:inception a ex:Film ; rdfs:label "Inception" ; ex:year "2010"^^xsd:gYear .
        "#;
        Graph::load_str(ttl, "turtle").expect("graph parses")
    }

    #[test]
    fn value_linking_off_by_default_even_with_entity_linking_on() {
        let g = valued_graph();
        let cfg = NlqConfig {
            link_entities: true,
            ..NlqConfig::default()
        };
        let nlq = Nlq::with_config(&g, Box::new(FnLlm(|_| Err("unused".into()))), cfg);
        let p = nlq.prompt_for("Which films are from 1994?");
        assert!(!p.contains("# Values from the question"));
    }

    #[test]
    fn value_linking_on_injects_the_exact_literal_into_the_prompt() {
        // sq-na0q: without this the model can only guess a lexical form for 1994, and a
        // guessed form matches no triple — the reason value-bound questions fail even
        // with a perfect schema card.
        let g = valued_graph();
        let cfg = NlqConfig {
            link_entities: true,
            link_values: true,
            ..NlqConfig::default()
        };
        let nlq = Nlq::with_config(&g, Box::new(FnLlm(|_| Err("unused".into()))), cfg);
        let p = nlq.prompt_for("Which films are from 1994?");
        assert!(
            p.contains("# Values from the question found in THIS dataset"),
            "value block missing from the prompt"
        );
        assert!(
            p.contains("\"1994\"^^<http://www.w3.org/2001/XMLSchema#gYear>"),
            "the datatyped literal must reach the prompt verbatim, got:\n{p}"
        );
        assert!(
            p.contains("<http://example.org/year>"),
            "the predicate the value objects must reach the prompt"
        );
    }

    #[test]
    fn linking_does_not_break_the_ask_loop() {
        let g = linkable_graph();
        let cfg = NlqConfig {
            link_entities: true,
            ..NlqConfig::default()
        };
        // The model echoes a query that uses the linked IRI; the loop must run normally.
        let q = "PREFIX ex: <http://example.org/>\n\
                 SELECT ?film WHERE { ?film ex:director ex:tarantino }";
        let nlq = Nlq::with_config(
            &g,
            Box::new(FnLlm(move |prompt: &str| {
                assert!(prompt.contains("# Linked from the question"));
                Ok(format!("```sparql\n{q}\n```"))
            })),
            cfg,
        );
        let a = nlq
            .ask("What did Quentin Tarantino direct?")
            .expect("loop succeeds with linking on");
        assert_eq!(a.repairs, 0);
        assert_eq!(a.result.len(), 1); // ex:pulp
    }

    #[test]
    fn budget_trips_surface_as_exec_errors() {
        let g = tiny_graph();
        let cfg = NlqConfig {
            max_rows: Some(1),
            max_repair_rounds: 0,
            ..NlqConfig::default()
        };
        let nlq = Nlq::with_config(
            &g,
            Box::new(FnLlm(|_| {
                Ok("```sparql\nSELECT ?s ?p ?o WHERE { ?s ?p ?o }\n```".into())
            })),
            cfg,
        );
        let err = nlq.ask("Everything, please").unwrap_err();
        assert!(
            matches!(&err.transcript[0].outcome, TurnOutcome::ExecError(e) if e.contains("budget")),
            "expected a budget trip, got {:?}",
            err.transcript[0].outcome
        );
    }

    /// N2 (`sq-9yjp`): an ungrounded predicate is a repair signal, and the repair
    /// prompt carries the dictionary feedback + suggestion. The model fixes it and the
    /// loop succeeds — the dictionary constraint paid for itself.
    #[test]
    fn ungrounded_predicate_triggers_dictionary_repair() {
        let g = tiny_graph();
        // First completion uses `ex:know` (not in the dictionary); the repair prompt
        // must surface the dictionary message + the `ex:knows` suggestion; second
        // completion is grounded.
        let cfg = NlqConfig {
            check_dictionary: true,
            ..NlqConfig::default()
        };
        let nlq = Nlq::with_config(
            &g,
            Box::new(FnLlm(|prompt| {
                if prompt.contains("not in the dataset's dictionary") {
                    assert!(prompt.contains("http://example.org/knows")); // suggestion
                    Ok("```sparql\nPREFIX ex: <http://example.org/>\n\
                        SELECT ?s WHERE { ?s ex:knows ?o }\n```"
                        .to_string())
                } else {
                    Ok("```sparql\nPREFIX ex: <http://example.org/>\n\
                        SELECT ?s WHERE { ?s ex:know ?o }\n```"
                        .to_string())
                }
            })),
            cfg,
        );
        let a = nlq.ask("Who knows whom?").expect("repair succeeds");
        assert_eq!(a.repairs, 1);
        assert_eq!(a.transcript.len(), 2);
        assert!(
            matches!(&a.transcript[0].outcome, TurnOutcome::UngroundedTerms(u)
                if u.len() == 1 && u[0].iri == "http://example.org/know"),
            "got {:?}",
            a.transcript[0].outcome
        );
        assert_eq!(a.transcript[1].outcome, TurnOutcome::Ok { rows: 1 });
    }

    /// Disabling the constraint accepts the (valid-but-empty) query as-is — the
    /// pre-N2 behaviour stays available.
    #[test]
    fn check_dictionary_false_accepts_ungrounded_query() {
        let g = tiny_graph();
        let cfg = NlqConfig {
            check_dictionary: false,
            max_repair_rounds: 0,
            ..NlqConfig::default()
        };
        let nlq = Nlq::with_config(
            &g,
            Box::new(FnLlm(|_| {
                Ok("```sparql\nPREFIX ex: <http://example.org/>\n\
                    SELECT ?s WHERE { ?s ex:know ?o }\n```"
                    .to_string())
            })),
            cfg,
        );
        let a = nlq
            .ask("Who knows whom?")
            .expect("accepted without the dict check");
        assert_eq!(a.repairs, 0);
        assert_eq!(a.result.len(), 0); // ungrounded predicate → empty, but accepted
    }
}
