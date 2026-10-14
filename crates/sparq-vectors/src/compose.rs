//! Read-path **URI-hiding compose** step + a closed **NL→NL A/B** harness
//! (FO-LLM-bridge **Phase 3**).
//!
//! sq-mztg8.1 (epic sq-mztg8; design `research/fo-llm-bridge.md` §2.3 / §3.3
//! "Hide the ontology + URIs" + §6 Phase 3). This module is the **compose** half of the
//! read-path facade: it takes the answer-row bindings of a query over the PKG and presents
//! them to the agent as a **label/grounded view instead of raw IRIs**, behind the
//! `sparq-terse`/#1074 **echo/confidence envelope** ("a convenience that shows its work,
//! never an oracle that hides it"). It is **opt-in** (the new `compose` cargo feature, which
//! implies `structure` for the [`crate::grounding`] / [`verbalize`](mod@crate::verbalize)
//! machinery, off by
//! default) and changes **nothing** in the default `sparq-vectors` build or the core engine.
//!
//! # The two views being A/B'd (the closed NL→NL loop)
//!
//! A read-path serves an answer-row binding (a backbone IRI the exact engine produced) to
//! the agent in one of two presentations — this is the K4 question (design §7) the closed
//! loop measures:
//!
//! 1. [`View::UriVisible`] — the raw IRI string, exactly as the NL→SPARQL incumbent path
//!    emits it. The agent sees `<https://sparq.dev/ns/pkg/kb#topic-merge-discipline>`.
//! 2. [`View::UriHidden`] — the IRI is **stripped to a human label** ([`crate::grounding`]
//!    `OutputType::Text → Modality::NlString`, the token-budgeted
//!    [`verbalize`](mod@crate::verbalize) rendering) plus an **echo** of how it resolved
//!    (label + confidence + method). The
//!    agent sees `Merge discipline` and never the IRI; the dict-id stays opaque.
//!
//! # The governing soundness envelope (design §3.3, from sparq-terse §6 / #1074)
//!
//! Every hidden binding carries a [`ResolutionEcho`] so a silent mis-resolution is
//! **visible**, never an oracle:
//!
//! - **Always echo the resolution** — resolved label + method + confidence, even though the
//!   IRI stays hidden ([`ResolutionEcho`]).
//! - **Lexical-first** — a real `rdfs:label`/`skos:prefLabel` is an exact, full-confidence
//!   resolution ([`ResolutionMethod::Lexical`]); a fall-back to the IRI local name is a
//!   **low-confidence** resolution ([`ResolutionMethod::LocalName`]) that says so.
//! - **Confidence-gate / fall-open to the IRI** — when a binding has **no usable label at
//!   all** (confidence below [`ComposeConfig::min_confidence`]), the hidden view *keeps the
//!   raw IRI* rather than inventing a label. Hiding never costs the agent the ability to
//!   identify the node; an un-labelled node is shown as-is, loudly.
//!
//! # What this is NOT (honest scope — the non-sycophantic stance)
//!
//! - **This module does not call a model and makes no accuracy claim.** It is the
//!   *deterministic mechanism* + a *fidelity* harness. The headline K4 question — *does
//!   hiding URIs help or hurt a real small model's answer accuracy?* — needs a real-model
//!   NL→NL fan-out, which is **unmeasured** in-tree (only the `sparq-nlq` record/replay
//!   fixtures exist; there is no API key on the work box). The [`AbReport`] this module
//!   produces measures **round-trip fidelity** (does the hidden view *preserve the answer's
//!   identity* — can every shown label be mapped back to exactly the IRI it hid?) and
//!   **presentation cost** (chars the agent reads), the two things measurable without a
//!   model. The accuracy verdict is registered as **UNMEASURED** in `bench/EXPERIMENTS.md`
//!   with the pre-registered null, per the design's falsifiable-not-assumed-better stance.
//! - **No ANN re-ranking / no approximate final answer.** The bindings come from the exact
//!   engine; this module only *projects* each one to a label. The lexical resolution is
//!   exact (a real label triple) or it abstains to the IRI — it never serves a cosine guess
//!   as the shown identity.

use crate::verbalize::{label_predicates, EntityTextConfig};
use oxrdf::Term;
use sparq_core::Graph;

/// Which presentation a read-path serves an answer-row binding in — the A/B axis the closed
/// NL→NL loop measures (design §6 Phase 3, resolving open question K4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The raw IRI string, exactly as the NL→SPARQL incumbent path emits it.
    UriVisible,
    /// The IRI stripped to a human label (+ a [`ResolutionEcho`]); the agent never sees the
    /// IRI.
    UriHidden,
}

/// How a binding's shown label was resolved — echoed so a mis-resolution is visible (the
/// envelope's "always show your work"). Ordered weakest-first so a fold toward the lowest
/// method is a `min`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResolutionMethod {
    /// No usable label and no local name — the hidden view kept the raw IRI (the
    /// fall-open). Confidence is 0.0.
    None,
    /// The IRI's local name (after the last `#`/`/`) — a weak, lexical-ish fallback when no
    /// real label triple exists. Low confidence.
    LocalName,
    /// A real `rdfs:label` / `skos:prefLabel` / `schema:name` / … literal — an exact,
    /// full-confidence resolution (lexical-first).
    Lexical,
}

impl ResolutionMethod {
    /// The confidence this method warrants: lexical = 1.0 (an exact label triple),
    /// local-name = 0.5 (a guess from the IRI shape), none = 0.0 (no label at all).
    pub fn confidence(self) -> f64 {
        match self {
            ResolutionMethod::Lexical => 1.0,
            ResolutionMethod::LocalName => 0.5,
            ResolutionMethod::None => 0.0,
        }
    }
}

/// The echo accompanying a hidden binding: how the shown label was resolved, so a silent
/// mis-resolution is visible. Carries the original IRI (for round-trip verification by the
/// harness — **not** shown to the agent) so the hidden view's fidelity is checkable.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolutionEcho {
    /// The label shown to the agent (the IRI when [`ResolutionMethod::None`]).
    pub label: String,
    /// How `label` was resolved.
    pub method: ResolutionMethod,
    /// The confidence of the resolution ([`ResolutionMethod::confidence`]).
    pub confidence: f64,
    /// The original IRI this binding hid — kept for the harness's round-trip check, **never**
    /// shown to the agent in the [`ComposedBinding::shown`] string.
    pub iri: String,
}

/// One composed answer-row binding: what the agent is shown, plus (for the hidden view) the
/// resolution echo. The [`shown`](ComposedBinding::shown) string is the *only* thing that
/// reaches the agent; [`echo`](ComposedBinding::echo) is the audit trail.
#[derive(Clone, Debug, PartialEq)]
pub struct ComposedBinding {
    /// The string the agent sees: the raw IRI ([`View::UriVisible`]) or the label
    /// ([`View::UriHidden`]).
    pub shown: String,
    /// The resolution echo — `Some` only for [`View::UriHidden`] (the visible view echoes
    /// nothing because the IRI is itself the identity).
    pub echo: Option<ResolutionEcho>,
}

/// Configuration for [`compose`]: the label budget and the confidence floor below which the
/// hidden view falls open to the raw IRI rather than inventing a label.
#[derive(Clone, Debug)]
pub struct ComposeConfig {
    /// The NL rendering template + char budget for a hidden label (reused from
    /// [`verbalize`](mod@crate::verbalize)). Default [`EntityTextConfig::default`].
    pub text: EntityTextConfig,
    /// The confidence floor: a hidden binding whose resolution confidence is **strictly
    /// below** this keeps the raw IRI (the fall-open). `0.5` (default) hides a real lexical
    /// label (1.0) but admits a local-name-only guess (0.5 is *at* the floor, so it IS
    /// shown — set `> 0.5` to fall open on the local-name path too). The default admits both
    /// real labels and local-name fallbacks; raise it to require a real label.
    pub min_confidence: f64,
}

impl Default for ComposeConfig {
    fn default() -> Self {
        ComposeConfig {
            text: EntityTextConfig::default(),
            min_confidence: 0.5,
        }
    }
}

/// Compose the answer-row `bindings` (the IRIs the exact engine produced over `graph`) into
/// the `view` presentation, under `cfg`.
///
/// Non-IRI bindings (literals, blank nodes) are shown as their own lexical form in **both**
/// views — only IRIs carry a hide/show distinction (a literal *is* its value; there is no
/// URI to hide). The order of `bindings` is preserved.
///
/// Read-only over `graph`; no model, no network. For [`View::UriHidden`] each binding is
/// resolved **lexical-first** (a real label triple → full confidence), falling back to the
/// IRI local name (low confidence), and finally **falling open to the raw IRI** when the
/// resolution confidence is below [`ComposeConfig::min_confidence`].
pub fn compose(
    graph: &Graph,
    bindings: &[Term],
    view: View,
    cfg: &ComposeConfig,
) -> Vec<ComposedBinding> {
    bindings
        .iter()
        .map(|b| compose_one(graph, b, view, cfg))
        .collect()
}

/// Compose one binding. See [`compose`].
fn compose_one(graph: &Graph, binding: &Term, view: View, cfg: &ComposeConfig) -> ComposedBinding {
    // Only IRIs carry a hide/show distinction. A literal IS its value; a blank node is
    // opaque either way. Show their lexical form in both views, with no echo.
    let Term::NamedNode(node) = binding else {
        return ComposedBinding {
            shown: lexical_of(binding),
            echo: None,
        };
    };
    let iri = node.as_str().to_string();

    match view {
        View::UriVisible => ComposedBinding {
            shown: iri,
            echo: None,
        },
        View::UriHidden => {
            let echo = resolve_label(graph, binding, &iri, cfg);
            ComposedBinding {
                shown: echo.label.clone(),
                echo: Some(echo),
            }
        }
    }
}

/// Resolve an IRI binding to a [`ResolutionEcho`] for the hidden view, lexical-first.
///
/// 1. **Lexical** — a token-budgeted [`verbalize`](mod@crate::verbalize) rendering grounded on a real label
///    triple → confidence 1.0.
/// 2. **Local name** — the IRI's local name when no label triple exists → confidence 0.5.
/// 3. **Fall-open** — when the chosen method's confidence is below `cfg.min_confidence`, keep
///    the raw IRI (method [`ResolutionMethod::None`], confidence 0.0): hiding never costs the
///    agent the node's identity.
fn resolve_label(graph: &Graph, binding: &Term, iri: &str, cfg: &ComposeConfig) -> ResolutionEcho {
    // Lexical-first: a real label literal grounds a full-confidence label. `verbalize`
    // returns `Some` only when a literal label/description group actually contributed (a
    // bare type word is not a label — see verbalize's doc contract), so a `Some` here is a
    // genuine lexical resolution.
    if let Some(label) = has_real_label(graph, binding, cfg) {
        let method = ResolutionMethod::Lexical;
        return ResolutionEcho {
            label,
            method,
            confidence: method.confidence(),
            iri: iri.to_string(),
        };
    }
    // Local-name fallback (a weak lexical-ish guess from the IRI shape).
    let local = local_name(iri);
    let method = ResolutionMethod::LocalName;
    if !local.is_empty() && method.confidence() >= cfg.min_confidence {
        return ResolutionEcho {
            label: local,
            method,
            confidence: method.confidence(),
            iri: iri.to_string(),
        };
    }
    // Fall open: no usable label at or above the floor → keep the raw IRI, loudly.
    ResolutionEcho {
        label: iri.to_string(),
        method: ResolutionMethod::None,
        confidence: ResolutionMethod::None.confidence(),
        iri: iri.to_string(),
    }
}

/// A real lexical label for `binding`, or `None` when it carries no label literal. Uses the
/// [`verbalize`](mod@crate::verbalize) machinery configured to render **labels only** (no type word, no
/// description prose) so a `Some` is a genuine `rdfs:label`/`skos:prefLabel`/… resolution,
/// not a bare type passage. Honours the budget in `cfg.text`.
fn has_real_label(graph: &Graph, binding: &Term, cfg: &ComposeConfig) -> Option<String> {
    let mut label_cfg = EntityTextConfig::labels_only(label_predicates(), cfg.text.batch.max(1));
    // Keep the consumer's budget + language preference for the shown label.
    label_cfg.max_chars = cfg.text.max_chars;
    label_cfg.languages = cfg.text.languages.clone();
    crate::verbalize::verbalize(graph, binding, &label_cfg).map(|s| s.trim().to_string())
}

/// The lexical form of a non-IRI term: a literal's value, a blank node's `_:label`.
fn lexical_of(term: &Term) -> String {
    match term {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        #[allow(unreachable_patterns)]
        _ => String::new(),
    }
}

/// The local name of an IRI (the part after the last `#` or `/`).
fn local_name(iri: &str) -> String {
    let local = iri.rsplit(['#', '/']).next().unwrap_or(iri);
    if local.is_empty() {
        String::new()
    } else {
        local.to_string()
    }
}

// ---- closed NL→NL A/B harness (fidelity + cost) ------------------------------------------

/// The fidelity + cost report for one answer table composed under both views — the
/// **measurable** half of the K4 closed loop (the accuracy half needs a real model; see the
/// module honesty contract).
///
/// **Round-trip fidelity** is the load-bearing soundness property: every label the hidden
/// view shows must map back to **exactly the IRI it hid** (no two distinct answer IRIs
/// collapse to the same shown label, and every shown label is re-identifiable). A hidden
/// view that loses identity would silently change the answer — that is the failure mode the
/// envelope exists to catch, and the harness counts the collisions when it happens, honestly.
#[derive(Clone, Debug, PartialEq)]
pub struct AbReport {
    /// How many IRI bindings were composed (the answer-table size, IRIs only).
    pub iri_bindings: usize,
    /// How many of those were hidden behind a real lexical label (the strongest resolution).
    pub lexical: usize,
    /// How many fell back to the IRI local name (low-confidence resolution).
    pub local_name: usize,
    /// How many **fell open** to the raw IRI (no usable label — the hidden view shows the
    /// IRI). These are the bindings hiding cannot help; a high count is the honest signal
    /// that this graph is under-labelled.
    pub fell_open: usize,
    /// **Round-trip-distinct**: among the hidden bindings, how many shown labels are unique
    /// (re-identifiable to exactly one IRI). When this equals `iri_bindings` the hidden view
    /// is **fidelity-preserving** — it loses no answer identity.
    pub distinct_labels: usize,
    /// How many distinct IRIs **collided** onto a shared shown label (lost identity). `0`
    /// means perfect fidelity. Any non-zero value is a hide that the agent could not undo —
    /// surfaced, never hidden.
    pub collisions: usize,
    /// Total characters the agent reads under [`View::UriVisible`] (raw IRIs).
    pub chars_visible: usize,
    /// Total characters the agent reads under [`View::UriHidden`] (labels, with fall-opens
    /// counted as the IRI they kept).
    pub chars_hidden: usize,
}

impl AbReport {
    /// Whether the hidden view **preserves answer identity** — no two distinct answer IRIs
    /// collapsed onto the same shown label. This is the soundness gate: a fidelity-preserving
    /// hide is *safe to A/B for accuracy*; a lossy one would change the answer regardless of
    /// the model and must not ship.
    pub fn fidelity_preserved(&self) -> bool {
        self.collisions == 0
    }

    /// The fraction of IRI bindings that hiding could actually help — those resolved to a
    /// real or local-name label rather than falling open to the IRI. `1.0` over a
    /// well-labelled graph; lower over a sparse one (the honest ceiling on any hiding win).
    pub fn coverage(&self) -> f64 {
        if self.iri_bindings == 0 {
            return 1.0;
        }
        (self.iri_bindings - self.fell_open) as f64 / self.iri_bindings as f64
    }

    /// The presentation-cost ratio `chars_hidden / chars_visible` — how much shorter (or
    /// longer) the labelled view is to read. `< 1.0` means hiding is more compact; this is a
    /// *cost* signal only, **not** an accuracy claim (a shorter answer is not a more correct
    /// one — that is what the unmeasured real-model A/B would decide).
    pub fn char_ratio(&self) -> f64 {
        if self.chars_visible == 0 {
            return 1.0;
        }
        self.chars_hidden as f64 / self.chars_visible as f64
    }

    /// A one-line summary for a benchmark log (printed at run time — never baked into
    /// committed markdown, per the repo's no-hard-coded-perf rule).
    pub fn summary(&self) -> String {
        format!(
            "compose A/B: {} IRI bindings | lexical={} local-name={} fell-open={} | \
             fidelity_preserved={} (collisions={}) coverage={:.3} char_ratio={:.3}",
            self.iri_bindings,
            self.lexical,
            self.local_name,
            self.fell_open,
            self.fidelity_preserved(),
            self.collisions,
            self.coverage(),
            self.char_ratio(),
        )
    }
}

/// Run the closed-loop A/B over one answer table: compose `bindings` under both views and
/// measure round-trip fidelity + presentation cost (the measurable half of K4). `graph` is
/// the graph the bindings were produced over; read-only.
///
/// This is the deterministic, model-free harness the bench fan-out drives once per arm. It
/// makes **no accuracy claim** — it measures whether the hidden view is *safe* (identity
/// preserving) and *how it reads* (chars). The accuracy verdict is the real-model A/B, which
/// is registered UNMEASURED (see the module + `bench/EXPERIMENTS.md`).
pub fn ab_report(graph: &Graph, bindings: &[Term], cfg: &ComposeConfig) -> AbReport {
    let visible = compose(graph, bindings, View::UriVisible, cfg);
    let hidden = compose(graph, bindings, View::UriHidden, cfg);

    let mut iri_bindings = 0usize;
    let mut lexical = 0usize;
    let mut local_name = 0usize;
    let mut fell_open = 0usize;
    let mut chars_visible = 0usize;
    let mut chars_hidden = 0usize;

    // label -> the set of distinct IRIs that mapped to it (for the collision/round-trip
    // check). Only IRI bindings are counted; a literal has no IRI to lose.
    let mut label_to_iris: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();

    for (v, h) in visible.iter().zip(hidden.iter()) {
        chars_visible += v.shown.chars().count();
        chars_hidden += h.shown.chars().count();
        let Some(echo) = &h.echo else {
            continue; // a non-IRI binding (no echo) — not part of the IRI fidelity check.
        };
        iri_bindings += 1;
        match echo.method {
            ResolutionMethod::Lexical => lexical += 1,
            ResolutionMethod::LocalName => local_name += 1,
            ResolutionMethod::None => fell_open += 1,
        }
        label_to_iris
            .entry(echo.label.clone())
            .or_default()
            .insert(echo.iri.clone());
    }

    // Round-trip fidelity: a label that maps to >1 distinct IRI is a collision (lost
    // identity). The distinct-label count is the number of unique shown labels.
    let distinct_labels = label_to_iris.len();
    let collisions: usize = label_to_iris
        .values()
        .filter(|iris| iris.len() > 1)
        .map(|iris| iris.len())
        .sum();

    AbReport {
        iri_bindings,
        lexical,
        local_name,
        fell_open,
        distinct_labels,
        collisions,
        chars_visible,
        chars_hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::close_for_vectorise;
    use sparq_reason::Profile;

    fn iri(s: &str) -> Term {
        Term::NamedNode(oxrdf::NamedNode::new_unchecked(s))
    }

    // A small PKG-shaped graph: two labelled concepts (one with punctuation in its label),
    // one un-labelled node (no label triple — exercises the local-name fallback), and a
    // literal answer cell.
    const TTL: &str = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix kb:   <https://sparq.dev/ns/pkg/kb#> .

kb:topic-merge-discipline a skos:Concept ; skos:prefLabel "Merge discipline" .
kb:topic-zk-discipline    a skos:Concept ; skos:prefLabel "ZK/MPC claim + circuit discipline" .
kb:bare-node              a skos:Concept .
"#;

    fn graph() -> Graph {
        close_for_vectorise(TTL, "turtle", Profile::Rdfs)
            .unwrap()
            .graph
    }

    #[test]
    fn visible_view_shows_raw_iris_with_no_echo() {
        let g = graph();
        let b = [iri("https://sparq.dev/ns/pkg/kb#topic-merge-discipline")];
        let out = compose(&g, &b, View::UriVisible, &ComposeConfig::default());
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].shown,
            "https://sparq.dev/ns/pkg/kb#topic-merge-discipline"
        );
        assert!(out[0].echo.is_none(), "the visible view echoes nothing");
    }

    #[test]
    fn hidden_view_strips_iri_to_a_real_label_with_full_confidence() {
        let g = graph();
        let b = [iri("https://sparq.dev/ns/pkg/kb#topic-merge-discipline")];
        let out = compose(&g, &b, View::UriHidden, &ComposeConfig::default());
        assert_eq!(
            out[0].shown, "Merge discipline",
            "IRI stripped to its label"
        );
        let echo = out[0].echo.as_ref().expect("hidden view echoes");
        assert_eq!(echo.method, ResolutionMethod::Lexical);
        assert_eq!(echo.confidence, 1.0);
        // The agent never sees the IRI in `shown`, but the echo keeps it for the audit.
        assert!(!out[0].shown.contains("http"), "no IRI leaks to the agent");
        assert_eq!(
            echo.iri,
            "https://sparq.dev/ns/pkg/kb#topic-merge-discipline"
        );
    }

    #[test]
    fn punctuation_label_round_trips_distinctly() {
        // The sq-26fdp punctuation label must hide cleanly and stay re-identifiable.
        let g = graph();
        let b = [iri("https://sparq.dev/ns/pkg/kb#topic-zk-discipline")];
        let out = compose(&g, &b, View::UriHidden, &ComposeConfig::default());
        assert_eq!(out[0].shown, "ZK/MPC claim + circuit discipline");
        assert_eq!(
            out[0].echo.as_ref().unwrap().method,
            ResolutionMethod::Lexical
        );
    }

    #[test]
    fn unlabelled_node_falls_back_to_local_name_then_floor_falls_open() {
        let g = graph();
        let b = [iri("https://sparq.dev/ns/pkg/kb#bare-node")];
        // Default floor (0.5) admits the local-name fallback (also 0.5).
        let out = compose(&g, &b, View::UriHidden, &ComposeConfig::default());
        assert_eq!(out[0].shown, "bare-node", "local-name fallback");
        assert_eq!(
            out[0].echo.as_ref().unwrap().method,
            ResolutionMethod::LocalName
        );
        assert_eq!(out[0].echo.as_ref().unwrap().confidence, 0.5);

        // Raise the floor above the local-name confidence → fall open to the raw IRI.
        let strict = ComposeConfig {
            min_confidence: 0.75,
            ..ComposeConfig::default()
        };
        let out2 = compose(&g, &b, View::UriHidden, &strict);
        assert_eq!(
            out2[0].shown, "https://sparq.dev/ns/pkg/kb#bare-node",
            "below the floor → fall open to the raw IRI, never invent a label"
        );
        assert_eq!(
            out2[0].echo.as_ref().unwrap().method,
            ResolutionMethod::None
        );
        assert_eq!(out2[0].echo.as_ref().unwrap().confidence, 0.0);
    }

    #[test]
    fn literal_binding_shows_its_value_in_both_views_no_echo() {
        let g = graph();
        let lit = Term::Literal(oxrdf::Literal::new_simple_literal("42 findings"));
        let v = compose(
            &g,
            std::slice::from_ref(&lit),
            View::UriVisible,
            &ComposeConfig::default(),
        );
        let h = compose(
            &g,
            std::slice::from_ref(&lit),
            View::UriHidden,
            &ComposeConfig::default(),
        );
        assert_eq!(v[0].shown, "42 findings");
        assert_eq!(h[0].shown, "42 findings");
        assert!(
            v[0].echo.is_none() && h[0].echo.is_none(),
            "a literal IS its value"
        );
    }

    #[test]
    fn ab_report_measures_fidelity_and_cost() {
        let g = graph();
        let b = [
            iri("https://sparq.dev/ns/pkg/kb#topic-merge-discipline"),
            iri("https://sparq.dev/ns/pkg/kb#topic-zk-discipline"),
            iri("https://sparq.dev/ns/pkg/kb#bare-node"),
        ];
        let report = ab_report(&g, &b, &ComposeConfig::default());
        assert_eq!(report.iri_bindings, 3);
        assert_eq!(report.lexical, 2, "two real labels");
        assert_eq!(report.local_name, 1, "the bare node → local name");
        assert_eq!(report.fell_open, 0, "default floor hides everything");
        // Fidelity: three distinct IRIs → three distinct shown labels, no collision.
        assert_eq!(report.distinct_labels, 3);
        assert_eq!(report.collisions, 0);
        assert!(report.fidelity_preserved(), "no answer identity lost");
        assert!((report.coverage() - 1.0).abs() < 1e-9);
        // Labels are shorter than the IRIs → hiding is more compact (a cost signal only).
        assert!(report.chars_hidden < report.chars_visible);
        assert!(report.char_ratio() < 1.0);
        // The summary renders without panicking.
        assert!(report.summary().contains("fidelity_preserved=true"));
    }

    #[test]
    fn ab_report_flags_a_collision_when_two_iris_share_a_label() {
        // Two distinct concepts with the SAME prefLabel: hiding collapses them to one shown
        // label — a LOST identity the harness must surface (not hide).
        let ttl = r#"
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix kb:   <https://sparq.dev/ns/pkg/kb#> .
kb:a a skos:Concept ; skos:prefLabel "Ambiguous" .
kb:b a skos:Concept ; skos:prefLabel "Ambiguous" .
"#;
        let g = close_for_vectorise(ttl, "turtle", Profile::Rdfs)
            .unwrap()
            .graph;
        let b = [
            iri("https://sparq.dev/ns/pkg/kb#a"),
            iri("https://sparq.dev/ns/pkg/kb#b"),
        ];
        let report = ab_report(&g, &b, &ComposeConfig::default());
        assert_eq!(report.iri_bindings, 2);
        assert_eq!(report.distinct_labels, 1, "both hid to the same label");
        assert_eq!(report.collisions, 2, "two IRIs collided onto one label");
        assert!(
            !report.fidelity_preserved(),
            "a label collision is a lost identity — must NOT be reported as fidelity-preserving"
        );
    }

    #[test]
    fn empty_answer_table_is_vacuously_fidelity_preserving() {
        let g = graph();
        let report = ab_report(&g, &[], &ComposeConfig::default());
        assert_eq!(report.iri_bindings, 0);
        assert!(report.fidelity_preserved());
        assert!((report.coverage() - 1.0).abs() < 1e-9);
        assert!((report.char_ratio() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn resolution_method_orders_weakest_first() {
        assert!(ResolutionMethod::None < ResolutionMethod::LocalName);
        assert!(ResolutionMethod::LocalName < ResolutionMethod::Lexical);
        assert_eq!(ResolutionMethod::Lexical.confidence(), 1.0);
        assert_eq!(ResolutionMethod::None.confidence(), 0.0);
    }
}
