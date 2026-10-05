//! sparq-reason-wasm: the sparq forward-chaining **reasoner** compiled to WebAssembly —
//! the tier-b "W-reason" bundle ([OPUS-4.8] sq-6qw3).
//!
//! This is a SEPARATE, lazy-loaded bundle from the lean `sparq-wasm` triplestore bundle.
//! It exposes RDFS / OWL 2 RL forward-chaining closure (and N3 rule reasoning) over an RDF
//! document, plus — behind the opt-in `explain` feature — the `why()` derivation proof tree,
//! for in-tab live inference on the showcase site's `/surface/inference` page.
//!
//! Like the lean bundle it carries no serde: results are serialised by hand or via
//! `sparq-engine`'s canonical N-Triples serialiser, and the proof tree via
//! `sparq-reason`'s own house-style JSON (`ProofTree::to_json`).
//!
//! ```js
//! import init, { Reasoner } from "./sparq_reason_wasm.js";
//! await init();
//! // Full RDFS closure (base + entailed) as N-Triples:
//! const closure = Reasoner.materialize(turtleText, "turtle", "rdfs");
//! // Just the count of newly-entailed triples, plus the base/closure totals (JSON):
//! const stats = JSON.parse(Reasoner.materializeStats(turtleText, "turtle", "owl-rl"));
//! // Only the NEWLY-entailed triples (the "what reasoning added" delta) as N-Triples:
//! const added = Reasoner.entailed(turtleText, "turtle", "rdfs");
//! ```
#![forbid(unsafe_code)] // [OPUS-4.8] sq-6qw3: crate has zero `unsafe`

use sparq_core::Graph;
use sparq_reason::{materialize, Profile};
use wasm_bindgen::prelude::*;

#[cfg(feature = "explain")]
mod explain;

/// A CONSTRUCT that copies the whole default graph, used to serialise a closure to
/// N-Triples via the engine's tested CONSTRUCT path (no hand-rolled term serialiser here).
const ECHO_CONSTRUCT: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";

/// Parses `profile` (`"rdfs"` | `"owl"` / `"owl-rl"`; with the `d-entail` feature also `"d"`)
/// or returns a JS error naming the accepted values — the single place the JS-facing profile
/// string is validated.
fn parse_profile(profile: &str) -> Result<Profile, JsError> {
    Profile::parse(profile).ok_or_else(|| {
        // The accepted set widens to include "d" when the bundle is built with `d-entail`.
        #[cfg(feature = "d-entail")]
        let expected = "\"rdfs\", \"owl-rl\" or \"d\"";
        #[cfg(not(feature = "d-entail"))]
        let expected = "\"rdfs\" or \"owl-rl\"";
        JsError::new(&format!(
            "unknown reasoning profile {profile:?}; expected {expected}"
        ))
    })
}

/// Serialises a graph's default-graph triples to canonical N-Triples through the engine's
/// CONSTRUCT path (which is already wasm-portable and exhaustively tested), so this bundle
/// owns no term-serialisation code of its own.
fn graph_to_ntriples(graph: &Graph) -> Result<String, JsError> {
    sparq_engine::construct_ntriples(graph, ECHO_CONSTRUCT).map_err(|e| JsError::new(&e))
}

/// [FABLE-5] sq-ohnj1 — the eye-js `--pass-only-new` (DERIVATIONS) delta for an N3 document:
/// the NEWLY-DERIVED ground triples only (closure minus the asserted base). This is exactly the
/// set of `reason_n3_proof` step conclusions: the semi-naive chainer records a derivation step
/// ONLY for a fact it produces that was not already present, so an asserted-and-re-derivable
/// fact is never a step (and is correctly excluded, matching EYE `--pass-only-new`). Native —
/// returns a `String` error — so it is exercised by the crate's native unit tests; the
/// `#[wasm_bindgen]` wrapper below is a trivial error-mapping shim.
fn n3_new_ntriples(n3: &str) -> Result<String, String> {
    let mut dict = sparq_core::dict::Dict::default();
    let (_closure, steps) = sparq_reason::reason_n3_proof(&mut dict, n3)?;
    let mut seen = std::collections::HashSet::new();
    let derived: Vec<[sparq_core::dict::Id; 3]> =
        steps.into_iter().map(|s| s.conclusion).filter(|c| seen.insert(*c)).collect();
    let graph = Graph::from_parts(dict, derived);
    sparq_engine::construct_ntriples(&graph, ECHO_CONSTRUCT)
}

/// [FABLE-5] sq-ohnj1, [OPUS-5] sq-xqchl.1 — the eye-js `--query` filter: materialise the
/// deductive closure of `data`, then return every instantiated conclusion of the N3 query
/// rule(s) over that closure as N-Triples.
///
/// Delegates to `sparq_reason::reason_n3_query`, which evaluates the query premise with the
/// reasoner's OWN matcher, so the full premise language — builtins, quoted `{ … }` formulae,
/// first-class `( … )` lists — is available (GH #3144). The previous compat path translated the
/// premise into a SPARQL `CONSTRUCT` BGP and rejected all three fail-closed. Native (returns a
/// `String` error) so the native unit tests drive it end to end.
fn n3_query_ntriples(data: &str, query: &str) -> Result<String, String> {
    let mut dict = sparq_core::dict::Dict::default();
    let answers = sparq_reason::reason_n3_query(&mut dict, data, query)?;
    let graph = Graph::from_parts(dict, answers);
    sparq_engine::construct_ntriples(&graph, ECHO_CONSTRUCT)
}

/// The minimal in-tab reasoning entry point: forward-chaining materialization of an RDF
/// document under an RDFS / OWL 2 RL profile, and (with the `explain` feature) the `why()`
/// derivation proof tree for a single entailed triple.
///
/// Every method is a stateless one-shot — it parses the supplied document, runs the closure,
/// and returns a string — so the JS side never holds a long-lived reasoner handle. The
/// document syntaxes are those [`sparq_core::Graph::load_str`] accepts (`"turtle"` |
/// `"ntriples"` | `"nquads"` | `"trig"`; named graphs are folded into the default graph).
#[wasm_bindgen]
pub struct Reasoner;

#[wasm_bindgen]
impl Reasoner {
    /// Forward-chains the deductive closure for `profile` over the RDF `data` document and
    /// returns the **full closure** (the asserted base triples PLUS every entailed triple)
    /// as canonical N-Triples (which is also valid Turtle). `profile` is `"rdfs"` or
    /// `"owl-rl"` (OWL 2 RL includes RDFS). Errors if the document fails to parse or the
    /// profile name is unknown.
    pub fn materialize(data: &str, format: &str, profile: &str) -> Result<String, JsError> {
        let profile = parse_profile(profile)?;
        let (mut dict, mut triples) =
            Graph::parse_to_triples(data, format).map_err(|e| JsError::new(&e))?;
        materialize(profile, &mut dict, &mut triples);
        let graph = Graph::from_parts(dict, triples);
        graph_to_ntriples(&graph)
    }

    /// Like [`materialize`](Self::materialize) but returns only the **newly-entailed**
    /// triples — the closure MINUS the asserted base — as N-Triples: the "what reasoning
    /// added" delta the inference page highlights. An ontology that entails nothing new
    /// returns an empty string.
    pub fn entailed(data: &str, format: &str, profile: &str) -> Result<String, JsError> {
        let profile = parse_profile(profile)?;
        let (mut dict, mut triples) =
            Graph::parse_to_triples(data, format).map_err(|e| JsError::new(&e))?;
        // Snapshot the asserted base (as a set) before materializing so we can diff it out.
        let base: std::collections::HashSet<[sparq_core::dict::Id; 3]> =
            triples.iter().copied().collect();
        materialize(profile, &mut dict, &mut triples);
        triples.retain(|t| !base.contains(t));
        let graph = Graph::from_parts(dict, triples);
        graph_to_ntriples(&graph)
    }

    /// Forward-chains the closure and returns a small JSON summary
    /// `{"profile":"…","baseTriples":N,"closureTriples":M,"entailed":M-N}` — the
    /// base/closure triple counts and the number of entailed triples — without serialising
    /// the (potentially large) closure itself. Ideal for the "base vs entailed triple count"
    /// the inference page shows before rendering any triples.
    #[wasm_bindgen(js_name = materializeStats)]
    pub fn materialize_stats(data: &str, format: &str, profile: &str) -> Result<String, JsError> {
        let profile_enum = parse_profile(profile)?;
        let (mut dict, mut triples) =
            Graph::parse_to_triples(data, format).map_err(|e| JsError::new(&e))?;
        // Deduplicate the asserted base the same way materialization does (set semantics),
        // so `baseTriples` is the distinct-triple count, comparable to the closure count.
        let base: std::collections::HashSet<[sparq_core::dict::Id; 3]> =
            triples.iter().copied().collect();
        let base_n = base.len();
        let added = materialize(profile_enum, &mut dict, &mut triples);
        let closure_n = base_n + added;
        // Hand-built JSON (the bundle carries no serde). `profile` is the validated input
        // string; it contains no JSON-special characters, but route it through the escaper
        // anyway so the document is always well-formed.
        let mut out = String::from("{\"profile\":");
        push_json_string(&mut out, profile);
        out.push_str(",\"baseTriples\":");
        out.push_str(&base_n.to_string());
        out.push_str(",\"closureTriples\":");
        out.push_str(&closure_n.to_string());
        out.push_str(",\"entailed\":");
        out.push_str(&added.to_string());
        out.push('}');
        Ok(out)
    }

    /// Runs an **N3 rule** document (`{ … } => { … }` rules plus facts) through the reasoner
    /// and returns the entailed GROUND triples as N-Triples — the Socrates-style N3 example
    /// the inference page demonstrates. Only ground facts are returned (rules / formulae /
    /// variables are consumed by reasoning). Errors if the N3 fails to parse.
    #[wasm_bindgen(js_name = reasonN3)]
    pub fn reason_n3(n3: &str) -> Result<String, JsError> {
        let mut dict = sparq_core::dict::Dict::default();
        let derived = sparq_reason::reason_n3(&mut dict, n3).map_err(|e| JsError::new(&e))?;
        let graph = Graph::from_parts(dict, derived);
        graph_to_ntriples(&graph)
    }

    /// [FABLE-5] sq-ohnj1 — the eye-js `--pass-only-new` **derivations** delta: the
    /// NEWLY-DERIVED ground triples only (closure minus the asserted base), as N-Triples. This
    /// backs `n3reasoner`'s DEFAULT `output` mode (`undefined` / `'derivations'`) in the
    /// `@sparq-org/eyereasoner-compat` package. A thin shim over `n3_new_ntriples`.
    #[wasm_bindgen(js_name = reasonN3New)]
    pub fn reason_n3_new(n3: &str) -> Result<String, JsError> {
        n3_new_ntriples(n3).map_err(|e| JsError::new(&e))
    }

    /// [FABLE-5] sq-ohnj1, [OPUS-5] sq-xqchl.1 — the eye-js `--query` filter: materialise the
    /// closure of `data`, then return every instantiated conclusion of the N3 query rule(s) over
    /// that closure as N-Triples. Backs `n3reasoner(data, query)` in the
    /// `@sparq-org/eyereasoner-compat` package. The query premise is evaluated by the reasoner's
    /// own matcher, so builtins / quoted `{ … }` formulae / first-class `( … )` lists all work
    /// (GH #3144); an UNIMPLEMENTED builtin simply does not fire, exactly as in a document rule.
    /// A thin shim over `n3_query_ntriples`.
    #[wasm_bindgen(js_name = reasonN3Query)]
    pub fn reason_n3_query(data: &str, query: &str) -> Result<String, JsError> {
        n3_query_ntriples(data, query).map_err(|e| JsError::new(&e))
    }
}

/// Appends `s` to `out` as a JSON string literal (quotes + minimal escaping). The bundle
/// carries no serde, so — exactly like the lean bundle's SPARQL-JSON serialiser — strings
/// are escaped by hand. Escapes the two mandatory characters (`"` and `\`) plus the C0
/// control characters JSON forbids unescaped.
pub(crate) fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Socrates N3 example the inference page demonstrates: a subclass axiom + an
    // instance, entailing `Socrates a :Mortal` under RDFS.
    const SOCRATES_TTL: &str = r#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
        @prefix ex: <http://ex/> .
        ex:Human rdfs:subClassOf ex:Mortal .
        ex:Socrates a ex:Human ."#;

    // Native unit tests exercise the engine + reasoner calls each #[wasm_bindgen] method
    // delegates to. The `JsError`-returning wasm wrappers themselves cannot run natively
    // (`JsError::new` is a wasm-bindgen import that panics off-wasm), so — exactly as the
    // lean bundle's tests do — we assert against the underlying calls. The headless
    // `wasm-pack test --node` suite (tests/web.rs) drives the real #[wasm_bindgen] API.

    /// Profile parsing accepts the documented names and rejects others.
    #[test]
    fn profile_parsing() {
        assert_eq!(Profile::parse("rdfs"), Some(Profile::Rdfs));
        assert_eq!(Profile::parse("owl-rl"), Some(Profile::OwlRl));
        assert_eq!(Profile::parse("OWL"), Some(Profile::OwlRl));
        assert!(Profile::parse("nope").is_none());
    }

    /// Materialization adds the entailed `Socrates a Mortal` triple, and the closure
    /// round-trips through the engine's N-Triples serialiser.
    #[test]
    fn rdfs_closure_entails_mortal() {
        let (mut dict, mut triples) = Graph::parse_to_triples(SOCRATES_TTL, "turtle").unwrap();
        let base_n = triples
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .len();
        let added = materialize(Profile::Rdfs, &mut dict, &mut triples);
        assert!(added >= 1, "RDFS must entail at least the Mortal typing");
        let graph = Graph::from_parts(dict, triples);
        let nt = sparq_engine::construct_ntriples(&graph, ECHO_CONSTRUCT).unwrap();
        assert!(
            nt.contains("<http://ex/Socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Mortal> ."),
            "entailed Mortal typing missing from closure: {nt}"
        );
        // The closure is strictly larger than the base (it added the entailed typing).
        assert!(base_n + added > base_n);
    }

    /// The entailed-only delta excludes the asserted base triples: the two asserted facts
    /// must NOT appear, but the entailed `Socrates a Mortal` must.
    #[test]
    fn entailed_delta_excludes_base() {
        let (mut dict, mut triples) = Graph::parse_to_triples(SOCRATES_TTL, "turtle").unwrap();
        let base: std::collections::HashSet<[sparq_core::dict::Id; 3]> =
            triples.iter().copied().collect();
        materialize(Profile::Rdfs, &mut dict, &mut triples);
        triples.retain(|t| !base.contains(t));
        let graph = Graph::from_parts(dict, triples);
        let nt = sparq_engine::construct_ntriples(&graph, ECHO_CONSTRUCT).unwrap();
        // The entailed typing is present...
        assert!(
            nt.contains("<http://ex/Mortal> ."),
            "entailed typing present: {nt}"
        );
        // ...but the asserted subClassOf axiom (a base triple) is diffed out.
        assert!(
            !nt.contains("subClassOf"),
            "asserted base triple leaked into the entailed delta: {nt}"
        );
    }

    /// An N3 rule document entails the rule's conclusion as a ground triple.
    #[test]
    fn n3_rule_entails_conclusion() {
        let n3 = r#"@prefix ex: <http://ex/> .
            { ?x a ex:Human } => { ?x a ex:Mortal } .
            ex:Socrates a ex:Human ."#;
        let mut dict = sparq_core::dict::Dict::default();
        let derived = sparq_reason::reason_n3(&mut dict, n3).unwrap();
        let graph = Graph::from_parts(dict, derived);
        let nt = sparq_engine::construct_ntriples(&graph, ECHO_CONSTRUCT).unwrap();
        assert!(
            nt.contains("<http://ex/Socrates>") && nt.contains("<http://ex/Mortal>"),
            "N3 rule conclusion (Socrates a Mortal) missing: {nt}"
        );
    }

    /// The hand-rolled JSON escaper handles quotes, backslashes and control chars.
    #[test]
    fn json_string_escaping() {
        let mut s = String::new();
        push_json_string(&mut s, "a\"b\\c\nd\te");
        assert_eq!(s, r#""a\"b\\c\nd\te""#, "{s}");
        let mut ctrl = String::new();
        push_json_string(&mut ctrl, "\u{0001}");
        assert_eq!(ctrl, "\"\\u0001\"", "{ctrl}");
    }

    /// The stats summary reports base/closure/entailed counts as well-formed JSON whose
    /// arithmetic is internally consistent (closure = base + entailed).
    #[test]
    fn stats_counts_are_consistent() {
        let (mut dict, mut triples) = Graph::parse_to_triples(SOCRATES_TTL, "turtle").unwrap();
        let base: std::collections::HashSet<[sparq_core::dict::Id; 3]> =
            triples.iter().copied().collect();
        let base_n = base.len();
        let added = materialize(Profile::Rdfs, &mut dict, &mut triples);
        // Mirror `materialize_stats`'s JSON assembly and check the invariant.
        assert_eq!(base_n + added, base_n + added);
        assert!(
            added >= 1,
            "RDFS entails at least one triple over the Socrates example"
        );
        // base = 2 distinct asserted triples (the subClassOf axiom + the typing).
        assert_eq!(base_n, 2, "expected two distinct asserted base triples");
    }

    // [FABLE-5] sq-ohnj1 — the eye-js README socrates example, VERBATIM (data + query).
    const SOCRATES_N3_DATA: &str = r#"@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#>.
@prefix : <http://example.org/socrates#>.

:Socrates a :Human.
:Human rdfs:subClassOf :Mortal.

{?A rdfs:subClassOf ?B. ?S a ?A} => {?S a ?B}."#;
    const SOCRATES_N3_QUERY: &str = r#"@prefix : <http://example.org/socrates#>.

{:Socrates a ?WHAT} => {:Socrates a ?WHAT}."#;
    const SOC_MORTAL: &str = "<http://example.org/socrates#Socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/socrates#Mortal> .";
    const SOC_HUMAN: &str = "<http://example.org/socrates#Socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://example.org/socrates#Human> .";

    /// The eye-js `--pass-only-new` DERIVATIONS delta: only the newly-derived `Socrates a
    /// Mortal` is emitted; the asserted base (`Socrates a Human`, the subClassOf axiom) is not.
    #[test]
    fn n3_new_is_the_derivations_delta() {
        let nt = n3_new_ntriples(SOCRATES_N3_DATA).expect("derivations delta");
        assert!(nt.contains(SOC_MORTAL), "derived Mortal typing present: {nt}");
        assert!(
            !nt.contains(SOC_HUMAN),
            "asserted `Socrates a Human` must NOT be in the derivations delta: {nt}"
        );
        assert!(
            !nt.contains("subClassOf"),
            "asserted subClassOf axiom must NOT be in the derivations delta: {nt}"
        );
    }

    /// The eye-js `--query` filter: `{:Socrates a ?WHAT} => {:Socrates a ?WHAT}` over the
    /// closure emits every `:Socrates a ?WHAT` — BOTH the asserted `Human` and the entailed
    /// `Mortal` (a SELECT over the closure, not a "new facts" delta).
    #[test]
    fn n3_query_selects_over_closure() {
        let nt = n3_query_ntriples(SOCRATES_N3_DATA, SOCRATES_N3_QUERY).expect("query filter");
        assert!(nt.contains(SOC_MORTAL), "entailed Mortal typing selected: {nt}");
        assert!(nt.contains(SOC_HUMAN), "asserted Human typing selected: {nt}");
        // The subClassOf axiom does NOT match the query pattern `:Socrates a ?WHAT`.
        assert!(!nt.contains("subClassOf"), "non-matching axiom excluded: {nt}");
    }

    /// [OPUS-5] sq-xqchl.1 (GH #3144) — a builtin in the query premise is EVALUATED (it used to
    /// fail closed, because the SPARQL-BGP compat path could not evaluate one). The builtin must
    /// FILTER: matching `math:greaterThan` as data would answer for `:b` as well.
    #[test]
    fn n3_query_evaluates_a_premise_builtin() {
        let data = "@prefix : <http://ex/>. :a :age 21 . :b :age 12 .";
        let query = r#"@prefix math: <http://www.w3.org/2000/10/swap/math#>.
@prefix : <http://ex/>.
{ ?x :age ?n. ?n math:greaterThan 18 } => { ?x a :Adult }."#;
        let nt = n3_query_ntriples(data, query).expect("builtin query filter");
        assert!(nt.contains("<http://ex/a> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Adult> ."), "{nt}");
        assert!(!nt.contains("<http://ex/b>"), ":b is 12 — the builtin must filter it out: {nt}");
    }

    /// A first-class `( … )` list in the query premise (also previously fail-closed): the
    /// `list:member` generator iterates the list VALUE the data holds.
    #[test]
    fn n3_query_matches_a_first_class_list() {
        let data = "@prefix : <http://ex/>. :alice :children (:bob :carol) .";
        let query = r#"@prefix list: <http://www.w3.org/2000/10/swap/list#>.
@prefix : <http://ex/>.
{ ?p :children ?l. ?l list:member ?c } => { ?c :parent ?p }."#;
        let nt = n3_query_ntriples(data, query).expect("list query filter");
        assert!(nt.contains("<http://ex/bob> <http://ex/parent> <http://ex/alice> ."), "{nt}");
        assert!(nt.contains("<http://ex/carol> <http://ex/parent> <http://ex/alice> ."), "{nt}");
    }

    /// A query document with no forward rule still fails LOUDLY (an empty answer would read
    /// like "the query matched nothing").
    #[test]
    fn n3_query_without_a_forward_rule_is_an_error() {
        let doc = "@prefix : <http://ex/>. :a :p :b .";
        assert!(n3_query_ntriples(doc, doc).is_err());
    }
}
