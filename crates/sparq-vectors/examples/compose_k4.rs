//! sq-3pb7f (FO-LLM-bridge Phase 3, epic sq-mztg8; design
//! `research/fo-llm-bridge.md` §4.3 kill-criterion **K4**): the **K4 question-battery
//! generator** for the read-path URI-hiding **accuracy** A/B.
//!
//! # What this is
//!
//! `compose_ab` (sq-mztg8.1) settled the *model-free* half of K4 — the URI-hidden view is
//! **sound** (0 collisions, full coverage) but **~2.4× larger to read** over the PKG. The
//! HEADLINE K4 question — *does presenting answer-row bindings as labels (`UriHidden`) instead
//! of raw IRIs (`UriVisible`) change a real model's ANSWER ACCURACY?* — needs a real model.
//!
//! This example does **not** call a model (there is no API key on the work box). It emits the
//! frozen **question battery** as line-delimited JSON: for each NL question whose correct
//! answer depends on grounding to the right PKG entity, it renders the answer-row bindings under
//! **both** views via the real [`sparq_vectors::compose`] module and pairs them with a
//! deterministically-computed gold answer. The two arms are then answered by tagged
//! `[ABMK arm=raw]` / `[ABMK arm=hidden]` Claude Code sub-agents (real-model telemetry, the
//! pkg-dogfood mechanism) and graded against the gold — that fan-out + grading lives in
//! `bench/compose/k4/` and the verdict in `bench/compose/RESULTS.md`.
//!
//! Determinism: the questions are derived from the checked-in PKG, the bindings come from the
//! exact scan API (same as `compose_ab`), and the gold answers are computed from the graph — no
//! hand-typed answer can drift from the data.
//!
//! ```sh
//! cargo run -q -p sparq-vectors --features compose --example compose_k4 \
//!   -- crates/sparq-kb/ingest/pkg-instances.ttl crates/sparq-kb/ingest/agents-findings.ttl \
//!   > bench/compose/k4/questions.jsonl
//! ```

use oxrdf::Term;
use sparq_core::dict::{self, TermParts};
use sparq_core::Graph;
use sparq_vectors::compose::{compose, ComposeConfig, View};
use std::collections::BTreeMap;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const SKOS_PREFLABEL: &str = "http://www.w3.org/2004/02/skos/core#prefLabel";
const DCT_TITLE: &str = "http://purl.org/dc/terms/title";
const PKG_ABOUT: &str = "https://sparq.dev/ns/pkg#about";
const DCT_SUBJECT: &str = "http://purl.org/dc/terms/subject";
const PKG_DEPENDSON: &str = "https://sparq.dev/ns/pkg#dependsOn";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<String> = if args.is_empty() {
        vec![
            "crates/sparq-kb/ingest/pkg-instances.ttl".to_string(),
            "crates/sparq-kb/ingest/agents-findings.ttl".to_string(),
        ]
    } else {
        args
    };
    let mut ttl = String::new();
    for p in &paths {
        match std::fs::read_to_string(p) {
            Ok(s) => {
                ttl.push_str(&s);
                ttl.push('\n');
            }
            Err(e) => eprintln!("skip {p}: {e}"),
        }
    }
    let graph = Graph::load_str(&ttl, "turtle").expect("PKG parse");
    let cfg = ComposeConfig::default();

    // Index helpers over the real graph.
    let idx = Index::build(&graph);

    let mut questions: Vec<Question> = Vec::new();

    // ---- Q-TYPE 1: "which topic is finding F about?" (Concept binding) ----------------------
    // Answer-binding = the Concept the finding is `pkg:about`. RAW shows `kb:topic-…`; HIDDEN
    // shows the Concept prefLabel. Gold = the Concept's prefLabel (the human topic name).
    for (find_iri, find_label) in idx.findings() {
        if let Some(topic_iri) = idx.object(&find_iri, PKG_ABOUT) {
            if let Some(topic_label) = idx.label(&topic_iri) {
                questions.push(Question::new(
                    &graph,
                    &cfg,
                    "about-topic",
                    &format!(
                        "A PKG finding states: \"{find_label}\". The query below returned the \
                         single topic this finding is ABOUT. In one short phrase, what is that \
                         topic?"
                    ),
                    &[Term::NamedNode(oxrdf::NamedNode::new_unchecked(&topic_iri))],
                    &topic_label,
                ));
            }
        }
    }

    // ---- Q-TYPE 2: disambiguate among candidate findings by intent --------------------------
    // Bindings = ALL findings of one topic; the agent must pick the ONE matching a paraphrased
    // intent. Gold = that finding's label. Tests label-vs-local-name selection (the local name
    // is semi-informative here, so this is the *fair* case, not a tautology).
    let probes: &[(&str, &str)] = &[
        (
            "the rule that a change reaches main only when the aggregate CI check is green and \
             all review threads are resolved",
            "find-merge-ci-summary-gate",
        ),
        (
            "the rule that a mutating sub-agent must run in its own isolated worktree and branch",
            "find-subagent-worktree-isolation",
        ),
        (
            "the rule that sub-agents must never run `git add -A` and never stage the beads \
             directory",
            "find-subagent-staging",
        ),
        (
            "the rule that any ZK or MPC privacy/soundness claim must stay hedged because of a \
             hard CI gate",
            "find-zk-privacy-claims-gate",
        ),
        (
            "the rule that you must never arm auto-merge on a stacked PR whose base is not main",
            "find-zk-arming-model",
        ),
        (
            "the rule that sub-agents proceed without waiting for the maintainer's greenlight",
            "find-subagent-proceed-no-greenlight",
        ),
    ];
    let all_findings: Vec<Term> = idx
        .findings()
        .iter()
        .map(|(iri, _)| Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri)))
        .collect();
    for (intent, gold_local) in probes {
        let gold_iri = idx
            .findings()
            .iter()
            .find(|(iri, _)| iri.ends_with(gold_local))
            .map(|(iri, _)| iri.clone())
            .expect("probe gold must exist");
        let gold_label = idx.label(&gold_iri).unwrap();
        questions.push(Question::new(
            &graph,
            &cfg,
            "disambiguate-finding",
            &format!(
                "The query below returned several PKG findings (the read-path candidate set). \
                 Exactly one of them is: {intent}. Reply with the full text of THAT finding \
                 (verbatim) and nothing else."
            ),
            &all_findings,
            &gold_label,
        ));
    }

    // ---- Q-TYPE 3: task-title lookup (opaque-IRI strong-hiding case) -------------------------
    // Binding = a single Task IRI (`kb:task-sq-XXXX` — local name is an opaque id). Gold = the
    // task title. RAW gives the agent an opaque id; HIDDEN gives the title directly. This is the
    // case where hiding *should* help most if it helps at all (a fair upper bound).
    for tid in idx.sample_tasks(8) {
        if let Some(title) = idx.object_lit(&tid, DCT_TITLE) {
            questions.push(Question::new(
                &graph,
                &cfg,
                "task-title",
                "The query below returned exactly one PKG task. In one sentence, what is that \
                 task about? (Summarise its subject; do not just echo an identifier.)",
                &[Term::NamedNode(oxrdf::NamedNode::new_unchecked(&tid))],
                &title,
            ));
        }
    }

    // ---- Q-TYPE 4: technique/surface identification -----------------------------------------
    // Binding = a Technique IRI (`kb:surface-…`, local name semi-informative). Gold = prefLabel.
    for (iri, label) in idx.techniques() {
        questions.push(Question::new(
            &graph,
            &cfg,
            "technique-name",
            "The query below returned one sparq capability surface. Name the surface in a short \
             phrase.",
            &[Term::NamedNode(oxrdf::NamedNode::new_unchecked(&iri))],
            &label,
        ));
    }

    // ---- Q-TYPE 5: punctuation/ambiguity stress (the known failure mode) --------------------
    // The ZK concept's label is punctuation-heavy ("ZK/MPC claim + circuit discipline"). Ask a
    // question whose gold is that exact topic, with the binding among the other two concepts.
    let concept_terms: Vec<Term> = idx
        .concepts()
        .iter()
        .map(|(iri, _)| Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri)))
        .collect();
    if let Some((zk_iri, zk_label)) = idx
        .concepts()
        .into_iter()
        .find(|(iri, _)| iri.ends_with("topic-zk-discipline"))
    {
        questions.push(Question::new(
            &graph,
            &cfg,
            "punct-concept",
            "The query below returned the PKG's topic catalog. Which ONE of these topics covers \
             the honesty + gate-count discipline specific to the ZK / MPC estate? Reply with its \
             name verbatim.",
            &concept_terms,
            &zk_label,
        ));
        let _ = zk_iri;
    }

    // ---- Q-TYPE 6: dependency-edge readout (two opaque task IRIs) ---------------------------
    // Binding = the tasks a given task depends on. Gold = the count + that those are tasks. Tests
    // whether opaque IRIs vs (long) titles change a simple structural readout.
    for tid in idx.tasks_with_deps(4) {
        let deps = idx.objects(&tid, PKG_DEPENDSON);
        if deps.is_empty() {
            continue;
        }
        let dep_terms: Vec<Term> = deps
            .iter()
            .map(|d| Term::NamedNode(oxrdf::NamedNode::new_unchecked(d)))
            .collect();
        let src_title = idx
            .object_lit(&tid, DCT_TITLE)
            .unwrap_or_else(|| tid.clone());
        questions.push(Question::new(
            &graph,
            &cfg,
            "dep-count",
            &format!(
                "The query below returned the tasks that the task \"{src_title}\" DEPENDS ON. \
                 How many tasks does it depend on? Answer with the number."
            ),
            &dep_terms,
            &deps.len().to_string(),
        ));
    }

    // Emit line-delimited JSON. Stable order already (graph scans are sorted).
    for (i, q) in questions.iter().enumerate() {
        println!("{}", q.to_json(i));
    }
    eprintln!("emitted {} K4 questions", questions.len());
}

/// One A/B question: the NL prompt, the two rendered arms, and the gold answer.
struct Question {
    kind: String,
    prompt: String,
    raw_block: String,
    hidden_block: String,
    gold: String,
    n_bindings: usize,
}

impl Question {
    fn new(graph: &Graph, cfg: &ComposeConfig, kind: &str, prompt: &str, bindings: &[Term], gold: &str) -> Self {
        let raw = compose(graph, bindings, View::UriVisible, cfg);
        let hidden = compose(graph, bindings, View::UriHidden, cfg);
        let raw_block = raw
            .iter()
            .enumerate()
            .map(|(i, b)| format!("  {}. {}", i + 1, b.shown))
            .collect::<Vec<_>>()
            .join("\n");
        let hidden_block = hidden
            .iter()
            .enumerate()
            .map(|(i, b)| format!("  {}. {}", i + 1, b.shown))
            .collect::<Vec<_>>()
            .join("\n");
        Question {
            kind: kind.to_string(),
            prompt: prompt.to_string(),
            raw_block,
            hidden_block,
            gold: gold.to_string(),
            n_bindings: bindings.len(),
        }
    }

    fn to_json(&self, id: usize) -> String {
        format!(
            "{{\"id\":{},\"kind\":{},\"n_bindings\":{},\"prompt\":{},\"raw\":{},\"hidden\":{},\"gold\":{}}}",
            id,
            json_str(&self.kind),
            self.n_bindings,
            json_str(&self.prompt),
            json_str(&self.raw_block),
            json_str(&self.hidden_block),
            json_str(&self.gold),
        )
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    sparq_core::json::escape_into(&mut out, s);
    out.push('"');
    out
}

/// A read-only index over the PKG graph built from the scan API.
struct Index<'g> {
    graph: &'g Graph,
}

impl<'g> Index<'g> {
    fn build(graph: &'g Graph) -> Self {
        Index { graph }
    }

    fn pid(&self, iri: &str) -> Option<dict::Id> {
        self.graph.id_of(&Term::NamedNode(oxrdf::NamedNode::new_unchecked(iri)))
    }

    /// All subjects of a given `rdf:type`.
    fn instances_of(&self, class_local: &str) -> Vec<String> {
        let Some(type_pid) = self.pid(RDF_TYPE) else { return vec![] };
        let scan = self.graph.store().scan(&[None, Some(type_pid), None]);
        let mut out = std::collections::BTreeSet::new();
        for row in scan.rows.iter() {
            let [s, _, o] = scan.to_spo(row);
            let (Some(c), Some(subj)) = (self.iri(o), self.iri(s)) else { continue };
            if c.ends_with(class_local) {
                out.insert(subj);
            }
        }
        out.into_iter().collect()
    }

    fn iri(&self, id: dict::Id) -> Option<String> {
        if dict::is_inline(id) {
            return None;
        }
        match self.graph.dict().term_parts(id) {
            TermParts::Iri { prefix, suffix } => Some(format!("{prefix}{suffix}")),
            _ => None,
        }
    }

    /// First IRI object of (subj, pred).
    fn object(&self, subj: &str, pred: &str) -> Option<String> {
        self.objects(subj, pred).into_iter().next()
    }

    fn objects(&self, subj: &str, pred: &str) -> Vec<String> {
        let (Some(sid), Some(pid)) = (self.pid(subj), self.pid(pred)) else { return vec![] };
        let scan = self.graph.store().scan(&[Some(sid), Some(pid), None]);
        let mut out = Vec::new();
        for row in scan.rows.iter() {
            let [_, _, o] = scan.to_spo(row);
            if let Some(iri) = self.iri(o) {
                out.push(iri);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// First literal object of (subj, pred).
    fn object_lit(&self, subj: &str, pred: &str) -> Option<String> {
        let (Some(sid), Some(pid)) = (self.pid(subj), self.pid(pred)) else { return None };
        let scan = self.graph.store().scan(&[Some(sid), Some(pid), None]);
        for row in scan.rows.iter() {
            let [_, _, o] = scan.to_spo(row);
            if let TermParts::Lit { value, .. } = self.graph.dict().term_parts(o) {
                return Some(value.to_string());
            }
        }
        None
    }

    /// The human label of an entity (rdfs:label / skos:prefLabel / dcterms:title).
    fn label(&self, subj: &str) -> Option<String> {
        for p in [RDFS_LABEL, SKOS_PREFLABEL, DCT_TITLE] {
            if let Some(l) = self.object_lit(subj, p) {
                return Some(l);
            }
        }
        None
    }

    fn findings(&self) -> Vec<(String, String)> {
        self.instances_of("Finding")
            .into_iter()
            .filter_map(|i| self.label(&i).map(|l| (i, l)))
            .collect()
    }

    fn concepts(&self) -> Vec<(String, String)> {
        self.instances_of("Concept")
            .into_iter()
            .filter_map(|i| self.label(&i).map(|l| (i, l)))
            .collect()
    }

    fn techniques(&self) -> Vec<(String, String)> {
        self.instances_of("Technique")
            .into_iter()
            .filter_map(|i| self.label(&i).map(|l| (i, l)))
            .collect()
    }

    /// A deterministic sample of `n` task IRIs spread across the sorted task list.
    fn sample_tasks(&self, n: usize) -> Vec<String> {
        let tasks = self.instances_of("Task");
        if tasks.is_empty() || n == 0 {
            return vec![];
        }
        let step = (tasks.len() / n).max(1);
        tasks.into_iter().step_by(step).take(n).collect()
    }

    /// `n` task IRIs that have at least one `pkg:dependsOn` edge (deterministic, sorted).
    fn tasks_with_deps(&self, n: usize) -> Vec<String> {
        let mut out = Vec::new();
        for t in self.instances_of("Task") {
            if !self.objects(&t, PKG_DEPENDSON).is_empty() {
                out.push(t);
                if out.len() >= n {
                    break;
                }
            }
        }
        out
    }
}

// Suppress unused-const warnings for predicates referenced only in some arms.
#[allow(dead_code)]
fn _refs() {
    let _ = (DCT_SUBJECT,);
    let _: BTreeMap<(), ()> = BTreeMap::new();
}
