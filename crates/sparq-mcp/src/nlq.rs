//! [OPUS-4.8] sq-jxjgr: the server-side natural-language tools — behind the opt-in
//! `nlq` feature, OFF by default.
//!
//! Two tools share this module and the same backend resolution:
//! - [`ask`] — the full loop: NL→SPARQL→validate→**execute**, returning the executed query
//!   plus the real result rows;
//! - [`nl_query`] — **translate only** ([SONNET-4.6] sq-sj1f9): the same grounding, and
//!   the same pre-execution checks — the question guard, a `spargebra` parse, and the
//!   forbidden-construct refusal — stopping before execution so the caller reviews the
//!   query and decides whether to run it (via the `query` tool). Cheaper and reviewable;
//!   the honest cost is that a returned query is only *syntactically* validated.
//!
//! Neither tool applies sparq-nlq's **dictionary-grounding** constraint
//! ([`sparq_nlq::constrain`], `NlqConfig::check_dictionary`): it is opt-in and OFF in the
//! default config both tools build, because it false-positives on a legitimately-empty
//! answer. So an ungrounded predicate/class IRI is accepted by BOTH — `ask` executes it to
//! zero rows, `nl_query` hands it back — and that parity is pinned by
//! `both_tools_accept_an_ungrounded_predicate`. [OPUS-5]
//!
//! This is the second of the two complementary grounding tools chosen on the 2026-06-23
//! design call (`shapes` is the no-LLM structured one): instead of handing the client a
//! shape to ground its OWN model, the `ask` tool runs the whole NL→SPARQL loop
//! **server-side** via [`sparq_nlq`] — introspect → generate → validate → execute — and
//! returns the executed SPARQL plus the result rows (and in-graph citations, when the
//! underlying graph carries provenance).
//!
//! ## Honest framing (the load-bearing contract)
//!
//! - **It embeds a configurable LLM call.** Cost, latency, and answer quality depend
//!   entirely on the model/endpoint the operator configures — this crate ships **no
//!   default model** and never phones home. The structured `shapes` / `introspect` tools
//!   are the no-LLM default; this trades a model call for the convenience of not writing
//!   SPARQL.
//! - **It is NOT a token-saving claim.** The project measured representation/token tricks
//!   as duds; this tool is an *ergonomics/grounding aid* pending measurement, nothing more.
//! - **It degrades cleanly.** With the `nlq` feature ON but **no** backend configured, the
//!   tool is not advertised and a direct call returns a clear *"not configured"* error —
//!   **never** a fabricated answer, never a panic.
//! - **The answer is grounded in real result rows.** What comes back is the model's
//!   *executed* query and the rows that query actually produced — not a free-form prose
//!   paragraph the model could hallucinate. The loop validates (spargebra) and executes
//!   (the engine, under a [`QueryBudget`]) before any answer is returned.
//!
//! ## Backends
//!
//! The `nlq` feature enables BOTH of sparq-nlq's configurable backends; [`ask`] picks
//! whichever the environment configures, preferring the OpenAI-compatible endpoint:
//! - `SPARQ_NLQ_ENDPOINT_URL` + `SPARQ_NLQ_ENDPOINT_MODEL` (+ optional
//!   `SPARQ_NLQ_ENDPOINT_KEY`) → a provider-agnostic OpenAI-compatible chat endpoint
//!   ([`sparq_nlq::endpoint::EndpointLlm`]);
//! - else `ANTHROPIC_API_KEY` → the Anthropic Messages API
//!   ([`sparq_nlq::live::AnthropicLlm`]).
//!
//! No test makes a live network call: the real loop logic is exercised in [`run_ask`]
//! with a scripted in-process [`sparq_nlq::Llm`] stub, and the "not configured" degrade
//! path is tested directly.

use serde_json::{json, Value};

use sparq_core::Graph;
use sparq_engine::QueryBudget;
use sparq_nlq::{Answer, Llm, Nlq, NlqConfig};

/// Names the backend the environment configures, if any. Mirrors the resolution order in
/// `resolve_backend` without constructing a client (so it is cheap enough to call from
/// `tools/list`). Returns `false` when neither backend is configured — the signal that the
/// NL tools (`ask`, `nl_query`) must NOT be advertised and a direct call must fail closed.
pub fn backend_configured() -> bool {
    endpoint_configured() || anthropic_configured()
}

/// Whether the OpenAI-compatible endpoint backend is configured (URL + model both set).
fn endpoint_configured() -> bool {
    !env_empty(sparq_nlq::endpoint::ENV_BASE_URL) && !env_empty(sparq_nlq::endpoint::ENV_MODEL)
}

/// Whether the Anthropic backend is configured (`ANTHROPIC_API_KEY` set non-empty).
fn anthropic_configured() -> bool {
    !env_empty("ANTHROPIC_API_KEY")
}

/// `true` when an environment variable is unset or empty.
fn env_empty(var: &str) -> bool {
    std::env::var(var).map(|v| v.is_empty()).unwrap_or(true)
}

/// Construct the configured LLM backend, or an actionable "not configured" error. Endpoint
/// is preferred over Anthropic when both are set (the productized, provider-agnostic path).
fn resolve_backend() -> Result<Box<dyn Llm>, String> {
    if endpoint_configured() {
        let llm = sparq_nlq::endpoint::EndpointLlm::from_env()?;
        return Ok(Box::new(llm));
    }
    if anthropic_configured() {
        let llm = sparq_nlq::live::AnthropicLlm::from_env()?;
        return Ok(Box::new(llm));
    }
    Err(NOT_CONFIGURED.to_string())
}

/// The exact "not configured" message — degrade cleanly, never fabricate. Lists the
/// environment variables that select a backend so the operator can fix it.
const NOT_CONFIGURED: &str = "the natural-language tools (`ask`, `nl_query`) are not \
    configured: no LLM backend is set. \
    Configure one of:\n  - an OpenAI-compatible endpoint: SPARQ_NLQ_ENDPOINT_URL + \
    SPARQ_NLQ_ENDPOINT_MODEL (+ optional SPARQ_NLQ_ENDPOINT_KEY); or\n  - the Anthropic \
    Messages API: ANTHROPIC_API_KEY.\nThe server ships no default model. Until one is \
    configured, use the no-LLM `shapes` / `introspect` tools and write the query yourself.";

/// Run the `ask` tool: resolve the configured backend, run the NL→SPARQL loop, and render
/// the answer. Fails closed (the "not configured" error) when no backend is set — never a
/// fabricated answer, never a panic. The real loop logic lives in [`run_ask`] so it is
/// testable with a scripted backend (no network).
pub fn ask(graph: &Graph, question: &str, budget: &QueryBudget) -> Result<String, String> {
    let llm = resolve_backend()?;
    run_ask(graph, question, budget, llm)
}

/// Run the `nl_query` tool: resolve the configured backend and TRANSLATE `question` into a
/// validated SPARQL query — **without executing it**. Fails closed (the same "not
/// configured" error as [`ask`]) when no backend is set. The real translation lives in
/// [`run_nl_query`] so it is testable with a scripted backend (no network).
/// [SONNET-4.6] sq-sj1f9
pub fn nl_query(graph: &Graph, question: &str) -> Result<String, String> {
    let llm = resolve_backend()?;
    run_nl_query(graph, question, llm)
}

/// The backend-agnostic core of [`nl_query`]: ground → generate → **validate** → repair,
/// stopping short of execution.
///
/// This is deliberately the `ask` loop MINUS the execute step, so the two tools ground on
/// the same [`sparq_nlq::Nlq`] prompt and apply the same pre-execution checks — under the
/// same default [`NlqConfig`], which leaves the opt-in dictionary constraint
/// (`check_dictionary`) off for both, so neither repairs an ungrounded IRI:
/// - the question is length-checked by [`sparq_nlq::guard::check_question`] before a token
///   is spent, exactly as the executing loop does;
/// - the completion must yield a SPARQL block that `spargebra` parses — a parse error is
///   the repair signal, not a returned query;
/// - a query that parses is still refused if it uses a construct the loop will not run
///   (`SERVICE` federation). Translation must not become an exfiltration bypass: a caller
///   who feeds the returned query back through the `query` tool would otherwise get the
///   outbound request that `ask` exists to refuse.
///
/// What it does NOT do is execute, so — unlike [`run_ask`] — a query that parses but
/// fails at runtime (or matches nothing) is returned as-is. That is the honest trade the
/// tool advertises: cheaper and reviewable, but only *syntactically* validated.
pub fn run_nl_query(graph: &Graph, question: &str, llm: Box<dyn Llm>) -> Result<String, String> {
    let config = NlqConfig::default();
    // `Nlq` owns its `Llm` and does not lend it out, so build the grounding and repair
    // prompts through a prompt-only instance (neither `prompt_for` nor `repair_prompt_for`
    // calls the model) and drive the caller's model directly. Constructing it runs the one
    // introspection scan that produces the schema deck — the same deck `ask` grounds on.
    let prompts = Nlq::with_config(graph, Box::new(NeverCalledLlm), config.clone());

    let question = sparq_nlq::guard::check_question(question, &config.guard)
        .map_err(|e| format!("question rejected: {}", e))?;
    let question = question.as_ref();

    let mut prompt = prompts.prompt_for(question);
    for round in 0..=config.max_repair_rounds {
        let completion = llm
            .complete(&prompt)
            .map_err(|e| format!("could not translate the question: LLM error: {}", e))?;

        let (sparql, error): (Option<String>, String) = match sparq_nlq::extract_sparql(&completion)
        {
            None => (None, "the completion contained no SPARQL code block".to_string()),
            Some(q) => match spargebra::SparqlParser::new().parse_query(&q) {
                Err(e) => (Some(q), e.to_string()),
                Ok(parsed) => {
                    let forbidden = sparq_nlq::guard::forbidden_constructs(&parsed, &config.guard);
                    if forbidden.is_empty() {
                        return Ok(render_translation(question, &q, round));
                    }
                    let msg = sparq_nlq::guard::forbidden_repair_message(&forbidden);
                    (Some(q), msg)
                }
            },
        };

        if round == config.max_repair_rounds {
            return Err(format!(
                "could not translate the question: no valid query after {} repair round(s): {}",
                round, error
            ));
        }
        prompt = prompts.repair_prompt_for(
            question,
            sparql.as_deref().unwrap_or("(no query produced)"),
            &error,
        );
    }
    unreachable!("the loop returns from its last round")
}

/// A backend that is never asked to complete anything: it exists only so
/// [`run_nl_query`] can build an [`Nlq`] for its prompt templates while driving the
/// caller's real model itself. Reaching `complete` would be a bug in this module, so it
/// returns an error rather than panicking.
struct NeverCalledLlm;

impl Llm for NeverCalledLlm {
    fn complete(&self, _prompt: &str) -> Result<String, String> {
        Err("internal error: the nl_query prompt builder must never call a model".to_string())
    }
}

/// Render a translation as JSON: the question, the validated query, the repair count, and
/// an explicit `executed: false` so no caller can mistake it for an answer.
fn render_translation(question: &str, sparql: &str, repairs: usize) -> String {
    let result = json!({
        "question": question,
        "sparql": sparql,
        "repairs": repairs,
        "executed": false,
        "note": "Translation only: this query parsed and passed the loop's construct \
                 checks, but was NOT executed — no result rows are implied, and it may \
                 still fail or match nothing at runtime. Review it, then run it with the \
                 `query` tool (or use `ask` to translate and execute in one step).",
    });
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
}

/// The backend-agnostic core: run the sparq-nlq loop over `graph` with the supplied `llm`
/// and render the answer. Split out from [`ask`] so the loop is exercised in tests with an
/// in-process scripted backend (no network), and so the backend-resolution and loop
/// concerns stay separate.
pub fn run_ask(
    graph: &Graph,
    question: &str,
    budget: &QueryBudget,
    llm: Box<dyn Llm>,
) -> Result<String, String> {
    let config = config_from_budget(budget);
    let nlq = Nlq::with_config(graph, llm, config);
    match nlq.ask(question) {
        Ok(answer) => Ok(render_answer(graph, question, &answer)),
        // An NL-loop failure (the model produced no valid query, or the LLM call failed)
        // is a TOOL error string, not a panic — the caller wraps it as `isError: true`.
        Err(e) => Err(format!("could not answer the question: {}", e)),
    }
}

/// Map the MCP [`QueryBudget`] onto a [`NlqConfig`] so the `ask` tool inherits the
/// server's row cap and per-query deadline.
fn config_from_budget(budget: &QueryBudget) -> NlqConfig {
    let mut config = NlqConfig {
        max_rows: budget.max_rows,
        ..NlqConfig::default()
    };
    // Translate the budget's absolute deadline into a per-query duration (the loop builds
    // a fresh deadline at execution time). Use the remaining time; fall back to the
    // NlqConfig default if the deadline is already in the past (the loop will then trip
    // its own budget promptly).
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(deadline) = budget.deadline {
        let now = std::time::Instant::now();
        if deadline > now {
            config.exec_timeout = Some(deadline - now);
        }
    }
    config
}

/// Render the answer as a structured JSON object: the executed SPARQL, the result rows
/// (faithfully, never paraphrased into prose the model could distort), the repair count,
/// and — with `nlq/citations` (the `sparq-nlq/citations` feature) — in-graph citations.
///
/// `_graph` is the graph the answer was produced over; it is only consulted when the
/// `citations` feature is on (to resolve in-graph provenance).
fn render_answer(_graph: &Graph, question: &str, answer: &Answer) -> String {
    let result = json!({
        "question": question,
        "sparql": answer.sparql,
        "repairs": answer.repairs,
        "result": render_result(answer),
        "note": "Answer is grounded in the executed query's real result rows, not a \
                 generated paragraph. The `sparql` field is the exact query that ran.",
    });
    let result = with_citations(_graph, answer, result);
    serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
}

/// Render the [`sparq_nlq::QueryResult`] as `{ vars, rows }`: each row is a map from
/// variable name to the bound term's N-Triples form (`null` when unbound). ASK results
/// (zero vars, one empty row iff true) render as `{ "boolean": true|false }`.
fn render_result(answer: &Answer) -> Value {
    let r = &answer.result;
    if r.vars.is_empty() {
        // ASK encoding: one empty row iff true.
        return json!({ "boolean": !r.rows.is_empty() });
    }
    let var_names: Vec<String> = r.vars.iter().map(|v| v.as_str().to_string()).collect();
    let rows: Vec<Value> = r
        .rows
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for (i, cell) in row.iter().enumerate() {
                let key = var_names.get(i).cloned().unwrap_or_else(|| i.to_string());
                let val = match cell {
                    Some(term) => Value::String(term.to_string()),
                    None => Value::Null,
                };
                obj.insert(key, val);
            }
            Value::Object(obj)
        })
        .collect();
    json!({ "vars": var_names, "rows": rows, "row_count": rows.len() })
}

/// Attach in-graph citations to the rendered answer when the `sparq-nlq/citations` feature
/// is enabled; a no-op (returns `result` unchanged) otherwise.
#[cfg(feature = "citations")]
fn with_citations(graph: &Graph, answer: &Answer, mut result: Value) -> Value {
    let cited = answer.citations(graph);
    // Render the human-facing footnote block; the renderer emits "no source recorded" for
    // un-sourced bindings (never a guessed source), so this stays honest.
    if let Some(obj) = result.as_object_mut() {
        obj.insert("citations".to_string(), json!(cited.footnotes()));
    }
    result
}

/// Citations are off when `sparq-nlq/citations` is not enabled — return the answer as-is.
#[cfg(not(feature = "citations"))]
fn with_citations(_graph: &Graph, _answer: &Answer, result: Value) -> Value {
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_engine::QueryBudget;

    /// Serializes the tests that clear the backend-selecting environment variables.
    /// `std::env` is process-global, so two such tests running on different threads would
    /// otherwise interleave a remove with a restore and see the wrong backend state.
    /// [SONNET-4.6] sq-sj1f9
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The environment variables that select an LLM backend, cleared by the fail-closed
    /// tests so they are hermetic regardless of the host environment.
    const BACKEND_VARS: [&str; 4] = [
        sparq_nlq::endpoint::ENV_BASE_URL,
        sparq_nlq::endpoint::ENV_MODEL,
        sparq_nlq::endpoint::ENV_API_KEY,
        "ANTHROPIC_API_KEY",
    ];

    /// Run `f` with every backend variable cleared, restoring them (and releasing the
    /// lock) before the caller asserts — so a failed assertion never leaks env state.
    fn with_no_backend<T>(f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<(&str, Option<String>)> = BACKEND_VARS
            .iter()
            .map(|v| (*v, std::env::var(v).ok()))
            .collect();
        for v in BACKEND_VARS {
            std::env::remove_var(v);
        }
        let out = f();
        for (v, val) in saved {
            match val {
                Some(val) => std::env::set_var(v, val),
                None => std::env::remove_var(v),
            }
        }
        out
    }

    /// Closure-backed test backend — the real `Llm` trait, no network.
    struct FnLlm<F: Fn(&str) -> Result<String, String>>(F);
    impl<F: Fn(&str) -> Result<String, String>> Llm for FnLlm<F> {
        fn complete(&self, prompt: &str) -> Result<String, String> {
            (self.0)(prompt)
        }
    }

    const TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
ex:alice rdf:type ex:Person ; ex:name "Alice" .
ex:bob   rdf:type ex:Person ; ex:name "Bob" .
"#;

    fn graph() -> Graph {
        Graph::load_str(TTL, "turtle").expect("load turtle")
    }

    const COUNT_QUERY: &str =
        "PREFIX ex: <http://ex/> SELECT (COUNT(?s) AS ?n) WHERE { ?s a ex:Person }";

    #[test]
    fn run_ask_runs_the_real_loop_and_renders_the_executed_query() {
        // A scripted backend that returns a valid query — the REAL loop (validate +
        // execute via the engine) runs over the real graph; no mock bypasses it.
        let llm = Box::new(FnLlm(|_| Ok(format!("```sparql\n{}\n```", COUNT_QUERY))));
        let out = run_ask(&graph(), "how many people?", &QueryBudget::unlimited(), llm)
            .expect("the loop should answer");
        let v: Value = serde_json::from_str(&out).expect("ask output is JSON");
        // The executed query is reported verbatim.
        assert_eq!(v["sparql"], COUNT_QUERY);
        // The answer is grounded in the REAL result row (2 people).
        let rows = v["result"]["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        let count_cell = rows[0].as_object().unwrap().values().next().unwrap();
        assert!(
            count_cell.as_str().unwrap().contains('2'),
            "the count must be the real engine result (2): {count_cell}"
        );
    }

    #[test]
    fn run_ask_surfaces_a_failed_loop_as_a_tool_error_not_a_panic() {
        // A backend that never produces SPARQL — the loop exhausts its repairs and errors.
        let llm = Box::new(FnLlm(|_| Ok("sorry, I cannot help".to_string())));
        let err = run_ask(&graph(), "anything", &QueryBudget::unlimited(), llm)
            .expect_err("a no-SPARQL loop must error, not panic");
        assert!(err.contains("could not answer"), "{err}");
    }

    #[test]
    fn ask_with_no_backend_configured_fails_closed() {
        // `with_no_backend` clears every backend-selecting env var (so the test is
        // hermetic regardless of the host environment) and restores them BEFORE the
        // assertions below, so a failed assert does not leak env state.
        let result =
            with_no_backend(|| ask(&graph(), "how many people?", &QueryBudget::unlimited()));

        let err = result.expect_err("no backend ⇒ a clean 'not configured' error");
        assert!(err.contains("not configured"), "{err}");
        assert!(
            err.contains("SPARQ_NLQ_ENDPOINT_URL") && err.contains("ANTHROPIC_API_KEY"),
            "the error must name both backends so the operator can fix it: {err}"
        );
        // Crucially: it is an ERROR, NOT a fabricated answer.
        assert!(
            !err.contains("\"sparql\""),
            "an unconfigured ask must NEVER return a (fabricated) answer: {err}"
        );
    }

    // ---------------------------------------------------------------------
    // `nl_query` — translate only. [SONNET-4.6] sq-sj1f9
    // ---------------------------------------------------------------------

    #[test]
    fn run_nl_query_returns_the_validated_query_and_does_not_execute_it() {
        let llm = Box::new(FnLlm(|_| Ok(format!("```sparql\n{}\n```", COUNT_QUERY))));
        let out = run_nl_query(&graph(), "how many people?", llm).expect("translation succeeds");
        let v: Value = serde_json::from_str(&out).expect("nl_query output is JSON");
        assert_eq!(v["sparql"], COUNT_QUERY);
        assert_eq!(v["question"], "how many people?");
        assert_eq!(v["repairs"], 0);
        // The load-bearing difference from `ask`: nothing ran, so nothing that looks like
        // an answer may appear.
        assert_eq!(v["executed"], false);
        assert!(
            v.get("result").is_none(),
            "a translation must not carry result rows: {out}"
        );
    }

    #[test]
    fn run_nl_query_never_executes_the_generated_query() {
        // A query that PARSES but does not survive execution. Asserting BOTH halves is
        // what makes this test pin the tools apart: `ask` (which executes) fails on it,
        // `nl_query` (which does not) returns it verbatim.
        const PARSES_BUT_FAILS: &str =
            "SELECT ?s WHERE { ?s ?p ?o . FILTER(<http://ex/nope>(?o)) }";
        let completion = || format!("```sparql\n{}\n```", PARSES_BUT_FAILS);

        let ask_err = run_ask(
            &graph(),
            "anything",
            &QueryBudget::unlimited(),
            Box::new(FnLlm(move |_| Ok(completion()))),
        )
        .expect_err("the executing loop must reject a query it cannot run");
        assert!(ask_err.contains("could not answer"), "{ask_err}");

        let out = run_nl_query(&graph(), "anything", Box::new(FnLlm(move |_| Ok(completion()))))
            .expect("translation validates syntax only, so it succeeds where `ask` failed");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["sparql"], PARSES_BUT_FAILS);
    }

    #[test]
    fn run_nl_query_refuses_a_federating_query_rather_than_returning_it() {
        // Translation must not become an exfiltration bypass: a SERVICE query the
        // executing loop refuses must not be handed back for the caller to run instead.
        let llm = Box::new(FnLlm(|_| {
            Ok("```sparql\nSELECT ?s WHERE { SERVICE <http://evil.example/sparql> \
                { ?s ?p ?o } }\n```"
                .to_string())
        }));
        let err = run_nl_query(&graph(), "exfiltrate", llm)
            .expect_err("a SERVICE query must be refused, not returned");
        assert!(
            err.contains("federation") || err.contains("SERVICE"),
            "the refusal must name the forbidden construct: {err}"
        );
        // It is an ERROR, not a translation the caller could lift the query out of.
        assert!(
            !err.contains("\"executed\""),
            "the refused query must not come back as a rendered translation: {err}"
        );
    }

    #[test]
    fn both_tools_accept_an_ungrounded_predicate() {
        // The dictionary constraint (`NlqConfig::check_dictionary`) is opt-in and OFF in
        // the default config BOTH tools build, so a hallucinated predicate is accepted by
        // `ask` (executed, zero rows) exactly as it is by `nl_query` (handed back). This
        // pins that parity: it goes red the moment either tool grows a dictionary check
        // the other lacks.
        const UNGROUNDED: &str = "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:nosuch ?o }";
        let completion = || format!("```sparql\n{}\n```", UNGROUNDED);

        // Non-vacuity anchor: the predicate really IS absent from the dictionary, so the
        // assertions below are about an ungrounded term, not a grounded one.
        let g = graph();
        let parsed = spargebra::SparqlParser::new()
            .parse_query(UNGROUNDED)
            .expect("the ungrounded query parses");
        let unknowns = sparq_nlq::constrain::unknown_terms(&g, &parsed);
        assert!(
            unknowns.iter().any(|u| u.iri == "http://ex/nosuch"),
            "the fixture predicate must be absent from the dictionary: {unknowns:?}"
        );

        // `ask` executes it and returns zero rows — it does NOT repair or reject it.
        let out = run_ask(
            &g,
            "anything",
            &QueryBudget::unlimited(),
            Box::new(FnLlm(move |_| Ok(completion()))),
        )
        .expect("an ungrounded predicate is executed, not repaired");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["sparql"], UNGROUNDED);
        assert_eq!(v["result"]["row_count"], 0);

        // `nl_query` returns the same query — the same acceptance, minus the execution.
        let out = run_nl_query(&g, "anything", Box::new(FnLlm(move |_| Ok(completion()))))
            .expect("an ungrounded predicate is translated, not repaired");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["sparql"], UNGROUNDED);
        assert_eq!(v["repairs"], 0);
    }

    #[test]
    fn run_nl_query_surfaces_an_untranslatable_question_as_a_tool_error() {
        let llm = Box::new(FnLlm(|_| Ok("sorry, I cannot help".to_string())));
        let err = run_nl_query(&graph(), "anything", llm)
            .expect_err("a no-SPARQL completion must error, not panic");
        assert!(err.contains("could not translate"), "{err}");
    }

    #[test]
    fn nl_query_with_no_backend_configured_fails_closed() {
        let result = with_no_backend(|| nl_query(&graph(), "how many people?"));

        let err = result.expect_err("no backend ⇒ a clean 'not configured' error");
        assert!(err.contains("not configured"), "{err}");
        // Crucially: an ERROR, never a fabricated query.
        assert!(
            !err.contains("SELECT"),
            "an unconfigured nl_query must NEVER return a (fabricated) query: {err}"
        );
    }

    #[test]
    fn ask_result_renders_a_real_count() {
        // End-to-end render shape over the real engine result.
        let llm = Box::new(FnLlm(|_| Ok(format!("```sparql\n{}\n```", COUNT_QUERY))));
        let out = run_ask(&graph(), "count", &QueryBudget::unlimited(), llm).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["question"], "count");
        assert_eq!(v["repairs"], 0);
        assert!(v["result"]["vars"].is_array());
    }
}
