//! The materializer pipeline: facts (loader) + N3 rules (rules/*.n3) → N3 reasoning
//! strata → the auth-view named graph `<urn:sparq:auth>` swapped into the dataset.
//!
//! Stratification (design doc §3.5): the engine's `log:notIncludes` never retracts, so
//! each negated predicate must be COMPLETE before its stratum runs. WAC negates only
//! input facts → 1 stratum. ACP runs accepts (A) → rejections (B) → grants (C), each
//! stratum's closure carried forward in memory as the next one's facts.
//!
//! [SONNET-4.6] sq-zgbso.4: evaluation runs on the **id-level compiled evaluator**
//! (`sparq_reason::n3::compiled`, epic sq-zgbso / issue #1582). The rule text is parsed
//! and lowered to the compiled IR ONCE per process ([`wac_rules`] / [`acp_rules`]), and
//! the facts enter as `[Id; 3]` straight from the source `Graph`'s terms
//! ([`crate::loader::assemble_input_ids`]) — the per-call
//! graph → N3 text → re-parse → string-term fixpoint round trip is gone. The text
//! engine (`sparq_reason::reason_n3`) stays reachable as the `#[cfg(test)]` differential
//! reference, and the tests below assert the two produce the SAME `<urn:sparq:auth>`
//! triple set on the WAC/ACP fixtures.

use crate::loader::{assemble_input_ids, strip_reserved_graphs, System};
use crate::{AccessProvenance, VerifiedCredentials, AUTH_GRAPH, AUTH_NS};
use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_reason::n3::compiled::{compile, eval, CompiledRuleSet};
use std::sync::OnceLock;
// `std::time::Instant` is unusable on `wasm32-unknown-unknown` — `Instant::now()`
// panics there (no monotonic clock). The wall-clock plumbing for `stats.millis` is
// purely informational, so it is `cfg`-gated off and reported as `0.0` on wasm32
// rather than trapping at runtime (sq-7agop). [OPUS-4.8]
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

const COMMON_RULES: &str = include_str!("../rules/common.n3");
const WAC_RULES: &str = include_str!("../rules/wac.n3");
const ACP_A: &str = include_str!("../rules/acp-a.n3");
const ACP_B: &str = include_str!("../rules/acp-b.n3");
const ACP_C: &str = include_str!("../rules/acp-c.n3");

/// The WAC rule set (`common.n3` + `wac.n3`) compiled to the id-level IR, once per
/// process. Compilation is deterministic over a `const` input, so the result is cached
/// verbatim — including a failure, which is returned as an ordinary `Err` (a rule that
/// leaves the compiled subset must surface as a materialize error, never a panic).
fn wac_rules() -> Result<&'static CompiledRuleSet, String> {
    static RULES: OnceLock<Result<CompiledRuleSet, String>> = OnceLock::new();
    RULES
        .get_or_init(|| compile(&format!("{COMMON_RULES}\n{WAC_RULES}")))
        .as_ref()
        .map_err(|e| format!("WAC rules: {}", e))
}

/// The three ACP strata (`common.n3` + `acp-a.n3`, then `acp-b.n3`, then `acp-c.n3`)
/// compiled to the id-level IR, once per process. See [`wac_rules`].
fn acp_rules() -> Result<&'static [CompiledRuleSet; 3], String> {
    static RULES: OnceLock<Result<[CompiledRuleSet; 3], String>> = OnceLock::new();
    RULES
        .get_or_init(|| {
            Ok([
                compile(&format!("{COMMON_RULES}\n{ACP_A}"))?,
                compile(ACP_B)?,
                compile(ACP_C)?,
            ])
        })
        .as_ref()
        .map_err(|e| format!("ACP rules: {}", e))
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const EXCEPT_MATCHER: &str = "https://sparq.dev/ns/auth#exceptMatcher";
/// solidx facts copied into the auth view for matchers referenced by conditional
/// grants (the session layer evaluates `exceptMatcher`s from these). [OPUS-4.8]
/// sq-3jtd.6 adds `acceptsIssuerP` — the noneOf exception check is three-dimensional.
const MATCHER_FACTS: [&str; 3] = [
    "https://sparq.dev/ns/solidx#acceptsAgentP",
    "https://sparq.dev/ns/solidx#acceptsClientP",
    "https://sparq.dev/ns/solidx#acceptsIssuerP",
];

/// What a `materialize_*` run produced: auth-view size, per-stratum closure sizes,
/// and wall-clock time. Purely informational (logging / benchmarking) — the result
/// itself is the `<urn:sparq:auth>` graph installed into the dataset.
///
/// # Examples
///
/// ```no_run
/// let mut graph = sparq_core::Graph::load_dataset(&sparq_solid::wac_fixture(), "nquads")?;
/// let stats = sparq_solid::materialize_wac(&mut graph)?;
/// println!("{} auth triples, closure {:?}, {:.0} ms",
///          stats.auth_triples, stats.strata_facts, stats.millis);
/// # Ok::<(), String>(())
/// ```
#[derive(Debug, Default)]
pub struct MaterializeStats {
    /// Triples in the produced auth view.
    pub auth_triples: usize,
    /// Closure size after each reasoning stratum (1 entry for WAC, 3 for ACP) — the
    /// number of distinct ground triples the stratum's fixpoint had derived, input
    /// facts included. Informational only.
    pub strata_facts: Vec<usize>,
    /// Wall-clock total.
    pub millis: f64,
}

/// Materialize the WAC auth view (single stratum) and install it as `<urn:sparq:auth>`.
///
/// The free-function form of [`crate::PodStore::materialize_wac`], for callers
/// managing a [`Graph`] directly: assembles the reasoning input (`.acl` graphs + group
/// documents + structural facts — pod content and the reserved `urn:sparq:` graph space
/// are **never** fed to the reasoner), evaluates the compiled `rules/common.n3` +
/// `rules/wac.n3` rule set over those facts at the id level, then — once all of that has
/// succeeded — drops the reserved `urn:sparq:` graphs and swaps the filtered closure in
/// as the `<urn:sparq:auth>` named graph (replacing any previous view). The dataset is
/// mutated only after the last fallible step, so an `Err` changes nothing.
///
/// Note: a [`crate::PodStore`] does NOT observe direct calls on its `graph` field —
/// use the method form so the session index and cache are rebuilt.
///
/// # Errors
///
/// Returns `Err` if an agent / group-member / origin IRI collides with the reserved
/// principal encoding (starts with `urn:sparq:` or contains the literal `&client=`),
/// or if a rule file falls outside the compiled evaluator's N3 subset (a build-time
/// property of `rules/*.n3`, so in practice this cannot fire at runtime — but it is
/// reported as an error rather than a panic, and no view is installed). The dataset's
/// previous auth view (if any) is left untouched on error.
///
/// # Examples
///
/// ```
/// use oxrdf::Term;
///
/// let nquads = r#"
/// <https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/auth/acl#Authorization> <https://pod.ex/.acl> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#default> <https://pod.ex/> <https://pod.ex/.acl> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#agent> <https://alice.ex/card#me> <https://pod.ex/.acl> .
/// <https://pod.ex/.acl#owner> <http://www.w3.org/ns/auth/acl#mode> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acl> .
/// "#;
/// let mut graph = sparq_core::Graph::load_dataset(nquads, "nquads")?;
/// let stats = sparq_solid::materialize_wac(&mut graph)?;
/// assert!(stats.auth_triples > 0);
/// // the view is an ordinary named graph now
/// assert!(graph.named.iter().any(|(name, _)| matches!(
///     name, Term::NamedNode(n) if n.as_str() == sparq_solid::AUTH_GRAPH)));
/// # Ok::<(), String>(())
/// ```
pub fn materialize_wac(graph: &mut Graph) -> Result<MaterializeStats, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    // Every fallible step runs BEFORE the first mutation of `graph`, so an `Err` leaves
    // the dataset — including any previously materialized `<urn:sparq:auth>` view —
    // byte-for-byte untouched, as the contract above promises. Assembly ignores the
    // reserved graph space itself, so it does not depend on the strip having run.
    let rules = wac_rules()?;
    let mut dict = Dict::new();
    // WAC has no creator/owner vocabulary; provenance is ignored (the loader skips it).
    let facts = assemble_input_ids(&mut dict, graph, System::Wac, &AccessProvenance::new(), &VerifiedCredentials::new())?;
    let closure = eval(&mut dict, &facts, rules);
    strip_reserved_graphs(graph);
    let mut stats = install_auth_view(graph, &dict, &closure);
    stats.strata_facts = vec![closure.len()];
    stats.millis = elapsed_millis(
        #[cfg(not(target_arch = "wasm32"))]
        t0,
    );
    Ok(stats)
}

/// Materialize the ACP auth view (three strata) and install it as `<urn:sparq:auth>`.
///
/// The free-function form of [`crate::PodStore::materialize_acp`]. Same contract as
/// [`materialize_wac`], but the input graphs are the `.acr` ones and the rules run as
/// three chained strata (`rules/acp-a.n3` accepts → `rules/acp-b.n3` rejections →
/// `rules/acp-c.n3` grants), each stratum's closure carried forward in one dictionary
/// as the next one's facts — the engine's negation-as-failure never retracts, so each
/// negated predicate must be complete before its stratum runs (design doc §3.5).
///
/// # Errors
///
/// As [`materialize_wac`].
///
/// # Examples
///
/// ```
/// // One document; an ACR on the pod root grants alice Read on all member resources.
/// let nquads = r#"
/// <https://pod.ex/notes/n1#it> <https://ex.dev/ns#title> "hello" <https://pod.ex/notes/n1> .
/// <https://pod.ex/.acr> <http://www.w3.org/ns/solid/acp#memberAccessControl> <https://pod.ex/.acr#c> <https://pod.ex/.acr> .
/// <https://pod.ex/.acr#c> <http://www.w3.org/ns/solid/acp#apply> <https://pod.ex/.acr#pol> <https://pod.ex/.acr> .
/// <https://pod.ex/.acr#pol> <http://www.w3.org/ns/solid/acp#allow> <http://www.w3.org/ns/auth/acl#Read> <https://pod.ex/.acr> .
/// <https://pod.ex/.acr#pol> <http://www.w3.org/ns/solid/acp#allOf> <https://pod.ex/.acr#m> <https://pod.ex/.acr> .
/// <https://pod.ex/.acr#m> <http://www.w3.org/ns/solid/acp#agent> <https://alice.ex/card#me> <https://pod.ex/.acr> .
/// "#;
/// let mut graph = sparq_core::Graph::load_dataset(nquads, "nquads")?;
/// let stats = sparq_solid::materialize_acp(&mut graph)?;
/// assert_eq!(stats.strata_facts.len(), 3); // accepts → rejections → grants
///
/// let index = sparq_solid::AuthIndex::from_graph(&graph);
/// let alice = sparq_solid::Session { agent: Some("https://alice.ex/card#me"), client: None, issuer: None, now: None };
/// assert_eq!(index.accessible(&alice, sparq_solid::Mode::Read).len(), 2); // n1 + notes/
/// # Ok::<(), String>(())
/// ```
pub fn materialize_acp(graph: &mut Graph) -> Result<MaterializeStats, String> {
    materialize_acp_with(graph, &AccessProvenance::new())
}

/// Materialize the ACP auth view from the `.acr` graphs PLUS the TRUSTED per-resource
/// creator/owner facts in `provenance`, resolving `acp:CreatorAgent` / `acp:OwnerAgent`
/// matchers ([OPUS-4.8] sq-3jtd.5).
///
/// The free-function form of [`crate::PodStore::materialize_acp_with`]. Identical to
/// [`materialize_acp`] (one three-stratum `reason_n3_stratified` run) except the loader also
/// synthesizes `<r> solidx:creator|owner <webid>` facts from `provenance` — the trusted
/// channel for "who created/owns `<r>`". These facts are **never** read from the resource
/// graphs (design doc §2.4): a writer cannot self-grant via a forged `solidx:creator`
/// triple in a document they control.
///
/// `materialize_acp(graph)` is exactly `materialize_acp_with(graph, &AccessProvenance::new())`
/// — with no provenance, no `CreatorAgent`/`OwnerAgent` matcher ever grants (fail-closed).
///
/// # Errors
///
/// As [`materialize_acp`], and additionally if a creator/owner WebID collides with the
/// reserved principal encoding (starts with `urn:sparq:` or contains the literal
/// `&client=`).
pub fn materialize_acp_with(
    graph: &mut Graph,
    provenance: &AccessProvenance,
) -> Result<MaterializeStats, String> {
    materialize_acp_with_credentials(graph, provenance, &VerifiedCredentials::new())
}

/// Materialize the ACP auth view from the `.acr` graphs PLUS the TRUSTED creator/owner facts
/// in `provenance` AND the TRUSTED verified-credential holdings in `credentials`, resolving
/// `acp:vc` matchers ([SONNET-4.6] sq-ysv3u).
///
/// The free-function form of [`crate::PodStore::materialize_acp_with_credentials`], and the
/// credential twin of [`materialize_acp_with`]: the loader additionally synthesizes
/// `<webid> solidx:holdsVc <requirement>` facts from `credentials` — the trusted channel for
/// "which `acp:vc` requirements this agent has been VERIFIED to satisfy". Those facts are
/// **never** read from pod or `.acr` content (design doc §2.4), so an agent cannot self-grant
/// by writing a forged holding into a document they control.
///
/// `materialize_acp_with(graph, prov)` is exactly
/// `materialize_acp_with_credentials(graph, prov, &VerifiedCredentials::new())` — with no
/// credential supplied, no `acp:vc` matcher ever grants (fail-closed).
///
/// # Errors
///
/// As [`materialize_acp_with`], and additionally if a credential holder's WebID collides with
/// the reserved principal encoding.
pub fn materialize_acp_with_credentials(
    graph: &mut Graph,
    provenance: &AccessProvenance,
    credentials: &VerifiedCredentials,
) -> Result<MaterializeStats, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    // Fallible work first, mutation last — see [`materialize_wac`].
    let rules = acp_rules()?;
    let mut dict = Dict::new();
    let facts = assemble_input_ids(&mut dict, graph, System::Acp, provenance, credentials)?;
    // Chain the three strata entirely at the id level, in ONE dictionary: each stratum's
    // closure is the next one's fact set, so a negated predicate is complete before the
    // stratum that negates it runs (design doc §3.5) and nothing round-trips through text.
    let mut closure = facts;
    let mut strata_facts = Vec::with_capacity(rules.len());
    for stratum in rules {
        closure = eval(&mut dict, &closure, stratum);
        strata_facts.push(closure.len());
    }

    strip_reserved_graphs(graph);
    let mut stats = install_auth_view(graph, &dict, &closure);
    stats.strata_facts = strata_facts;
    stats.millis = elapsed_millis(
        #[cfg(not(target_arch = "wasm32"))]
        t0,
    );
    Ok(stats)
}

/// Wall-clock milliseconds since `t0`, or `0.0` on `wasm32-unknown-unknown` where
/// `std::time::Instant` is unavailable (sq-7agop). `stats.millis` is purely
/// informational (logging / benchmarking), so wasm32 simply reports no timing
/// rather than trapping. [OPUS-4.8]
#[cfg(not(target_arch = "wasm32"))]
fn elapsed_millis(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1e3
}

#[cfg(target_arch = "wasm32")]
fn elapsed_millis() -> f64 {
    0.0
}

/// Whether a closure triple belongs in the auth view: `auth:*` predicates, `rdf:type`
/// with an `auth:` class, plus the accept-set facts of matchers referenced by
/// `auth:exceptMatcher` (collected in a first pass).
fn install_auth_view(graph: &mut Graph, dict: &Dict, closure: &[[sparq_core::dict::Id; 3]]) -> MaterializeStats {
    // pass 1: matchers referenced by conditional grants
    let mut except_matchers: FxHashSet<sparq_core::dict::Id> = FxHashSet::default();
    for t in closure {
        if let Term::NamedNode(p) = dict.term(t[1]) {
            if p.as_str() == EXCEPT_MATCHER {
                except_matchers.insert(t[2]);
            }
        }
    }
    // pass 2: filter + re-intern into a fresh sub-graph dictionary
    let mut adict = Dict::new();
    let mut ids: Vec<[sparq_core::dict::Id; 3]> = Vec::new();
    for t in closure {
        let Term::NamedNode(p) = dict.term(t[1]) else { continue };
        let keep = p.as_str().starts_with(AUTH_NS)
            || (p.as_str() == RDF_TYPE
                && matches!(dict.term(t[2]), Term::NamedNode(o) if o.as_str().starts_with(AUTH_NS)))
            || (MATCHER_FACTS.contains(&p.as_str()) && except_matchers.contains(&t[0]));
        // exclude the rules' own mode-mapping constants (solidx:allowPred facts have a
        // solidx predicate and are filtered out by the conditions above already)
        if !keep {
            continue;
        }
        let s = dict.term(t[0]);
        let o = dict.term(t[2]);
        ids.push([adict.intern(&s), adict.intern(&Term::NamedNode(p)), adict.intern(&o)]);
    }
    let stats = MaterializeStats { auth_triples: ids.len(), ..Default::default() };
    let auth = Graph::from_parts(adict, ids);
    let name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
    if let Some(slot) = graph.named.iter_mut().find(|(n, _)| *n == name) {
        slot.1 = auth;
    } else {
        graph.named.push((name, auth));
    }
    stats
}

/// [SONNET-4.6] sq-zgbso.4 — the RESULT-EQUIVALENCE oracle for the compiled flip.
///
/// The text engine (`sparq_reason::reason_n3` over N3-serialized facts + rule text) is
/// kept alive here, and ONLY here, as the differential reference: these tests assert the
/// compiled id-level materializer installs the IDENTICAL `<urn:sparq:auth>` triple set.
/// The bead invariant is auth-view equivalence, so the comparison is on the installed
/// view — not on the raw closure, which legitimately differs (the text path's closure
/// carries string-parsed input facts the auth filter drops anyway).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::assemble_input;
    use sparq_reason::{reason_n3, reason_n3_stratified};
    use std::collections::BTreeSet;

    /// The WAC auth view as the TEXT engine derives it (the pre-sq-zgbso.4 pipeline).
    fn wac_text(graph: &mut Graph) -> Result<MaterializeStats, String> {
        strip_reserved_graphs(graph);
        let input = assemble_input(graph, System::Wac, &AccessProvenance::new(), &VerifiedCredentials::new())?;
        let src = format!("{}\n{}\n{}", input, COMMON_RULES, WAC_RULES);
        let mut dict = Dict::new();
        let closure = reason_n3(&mut dict, &src)?;
        Ok(install_auth_view(graph, &dict, &closure))
    }

    /// The ACP auth view as the TEXT engine derives it (the pre-sq-zgbso.4 pipeline).
    fn acp_text(
        graph: &mut Graph,
        provenance: &AccessProvenance,
        credentials: &VerifiedCredentials,
    ) -> Result<MaterializeStats, String> {
        strip_reserved_graphs(graph);
        let input = assemble_input(graph, System::Acp, provenance, credentials)?;
        let accepts = format!("{}\n{}\n{}", input, COMMON_RULES, ACP_A);
        let mut dict = Dict::new();
        let closure = reason_n3_stratified(&mut dict, &[&accepts, ACP_B, ACP_C])?;
        Ok(install_auth_view(graph, &dict, &closure.facts))
    }

    /// The installed `<urn:sparq:auth>` view as a comparable set of `s p o` strings.
    fn auth_view(graph: &Graph) -> BTreeSet<String> {
        let name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
        let Some((_, g)) = graph.named.iter().find(|(n, _)| *n == name) else {
            return BTreeSet::new();
        };
        crate::loader::graph_triples(g)
            .iter()
            .map(|t| format!("{} {} {}", t[0], t[1], t[2]))
            .collect()
    }

    fn assert_same(text: &BTreeSet<String>, compiled: &BTreeSet<String>, what: &str) {
        assert!(!text.is_empty(), "{}: the reference auth view is empty — vacuous test", what);
        let missing: Vec<_> = text.difference(compiled).take(5).collect();
        let extra: Vec<_> = compiled.difference(text).take(5).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "{}: compiled auth view diverges from the text engine\n  missing ({}): {:?}\n  EXTRA ({}): {:?}",
            what,
            text.difference(compiled).count(),
            missing,
            compiled.difference(text).count(),
            extra
        );
    }

    #[test]
    fn wac_auth_view_is_identical_to_the_text_engine() {
        let src = crate::wac_fixture();
        let mut a = Graph::load_dataset(&src, "nquads").expect("fixture loads");
        let mut b = Graph::load_dataset(&src, "nquads").expect("fixture loads");
        wac_text(&mut a).expect("text WAC");
        let stats = materialize_wac(&mut b).expect("compiled WAC");
        assert_same(&auth_view(&a), &auth_view(&b), "WAC");
        assert_eq!(stats.auth_triples, auth_view(&b).len());
        assert_eq!(stats.strata_facts.len(), 1, "WAC is a single stratum");
    }

    #[test]
    fn acp_auth_view_is_identical_to_the_text_engine() {
        let src = crate::acp_fixture();
        let mut a = Graph::load_dataset(&src, "nquads").expect("fixture loads");
        let mut b = Graph::load_dataset(&src, "nquads").expect("fixture loads");
        let prov = AccessProvenance::new();
        acp_text(&mut a, &prov, &VerifiedCredentials::new()).expect("text ACP");
        let stats = materialize_acp_with(&mut b, &prov).expect("compiled ACP");
        assert_same(&auth_view(&a), &auth_view(&b), "ACP");
        assert_eq!(stats.strata_facts.len(), 3, "ACP runs three strata");
    }

    /// The trusted creator/owner channel (`acp:CreatorAgent` / `acp:OwnerAgent`) enters
    /// the compiled path through `assemble_input_ids` — so it gets its own differential.
    #[test]
    fn acp_with_provenance_auth_view_is_identical_to_the_text_engine() {
        let src = crate::acp_fixture();
        let mut prov = AccessProvenance::new();
        for (name, _) in Graph::load_dataset(&src, "nquads").expect("fixture loads").named.iter() {
            if let Term::NamedNode(n) = name {
                if !n.as_str().ends_with(".acr") {
                    prov.set_creator(n.as_str(), "https://carol.ex/card#me");
                }
            }
        }
        let mut a = Graph::load_dataset(&src, "nquads").expect("fixture loads");
        let mut b = Graph::load_dataset(&src, "nquads").expect("fixture loads");
        acp_text(&mut a, &prov, &VerifiedCredentials::new()).expect("text ACP");
        materialize_acp_with(&mut b, &prov).expect("compiled ACP");
        assert_same(&auth_view(&a), &auth_view(&b), "ACP + provenance");
    }

    /// The id-level fact entry and the N3-text entry must carry the SAME facts — the
    /// property that makes the auth-view equivalence above structural rather than lucky.
    #[test]
    fn id_level_facts_match_the_text_round_trip() {
        for (system, src) in [
            (System::Wac, crate::wac_fixture()),
            (System::Acp, crate::acp_fixture()),
        ] {
            let mut graph = Graph::load_dataset(&src, "nquads").expect("fixture loads");
            strip_reserved_graphs(&mut graph);
            let prov = AccessProvenance::new();

            let mut dict = Dict::new();
            let ids = assemble_input_ids(&mut dict, &graph, system, &prov, &VerifiedCredentials::new())
                .expect("id facts");
            let direct: BTreeSet<String> = ids
                .iter()
                .map(|t| format!("{} {} {}", dict.term(t[0]), dict.term(t[1]), dict.term(t[2])))
                .collect();

            let text = assemble_input(&graph, system, &prov, &VerifiedCredentials::new())
                .expect("text facts");
            let mut tdict = Dict::new();
            let parsed = sparq_reason::n3::compiled::intern_facts(&mut tdict, &text)
                .expect("the text entry re-parses");
            let round_tripped: BTreeSet<String> = parsed
                .iter()
                .map(|t| format!("{} {} {}", tdict.term(t[0]), tdict.term(t[1]), tdict.term(t[2])))
                .collect();

            assert!(!direct.is_empty(), "no facts assembled — vacuous test");
            assert_eq!(direct, round_tripped, "fact entry diverges (system {:?})", system as u8);
        }
    }

    /// One `.acl`/`.acr` graph asserting a reserved-encoding agent IRI — rejected by
    /// `validate_principal_iri` during fact assembly, i.e. after rule compilation and
    /// before any auth view could be installed.
    fn poison_graph(control_doc: &str, agent_predicate: &str) -> Vec<(Term, Graph)> {
        let nq = format!(
            "<{}#a> <{}> <urn:sparq:evil> <{}> .\n",
            control_doc, agent_predicate, control_doc
        );
        Graph::load_dataset(&nq, "nquads").expect("poison graph loads").named
    }

    /// The documented contract is that an `Err` leaves the dataset's previous auth view
    /// untouched. That holds ONLY if every fallible step (rule compilation, fact
    /// assembly / principal validation) runs BEFORE `strip_reserved_graphs` deletes the
    /// old `<urn:sparq:auth>` — strip-first would fail OPEN into a dataset whose
    /// authorization view had silently vanished. Both materializers are checked.
    #[test]
    fn a_failed_rematerialization_leaves_the_previous_auth_view_in_place() {
        let mut graph = Graph::load_dataset(&crate::wac_fixture(), "nquads").expect("fixture loads");
        materialize_wac(&mut graph).expect("first WAC materialization succeeds");
        let before = auth_view(&graph);
        assert!(!before.is_empty(), "no auth view to preserve — vacuous test");
        graph.named.extend(poison_graph(
            "https://pod.ex/evil.acl",
            "http://www.w3.org/ns/auth/acl#agent",
        ));
        let err = materialize_wac(&mut graph).expect_err("a reserved agent IRI must fail");
        assert!(err.contains("urn:sparq:"), "unexpected WAC error: {}", err);
        assert_eq!(auth_view(&graph), before, "WAC: the previous auth view must survive the error");

        let mut graph = Graph::load_dataset(&crate::acp_fixture(), "nquads").expect("fixture loads");
        materialize_acp(&mut graph).expect("first ACP materialization succeeds");
        let before = auth_view(&graph);
        assert!(!before.is_empty(), "no auth view to preserve — vacuous test");
        graph.named.extend(poison_graph(
            "https://pod.ex/evil.acr",
            "http://www.w3.org/ns/solid/acp#agent",
        ));
        let err = materialize_acp(&mut graph).expect_err("a reserved agent IRI must fail");
        assert!(err.contains("urn:sparq:"), "unexpected ACP error: {}", err);
        assert_eq!(auth_view(&graph), before, "ACP: the previous auth view must survive the error");
    }

    /// A rule set outside the compiled subset must surface as a materialize `Err`, never
    /// a panic and never a silently empty auth view (fail-closed).
    #[test]
    fn rule_sets_compile_at_startup() {
        wac_rules().expect("WAC rules are in the compiled subset");
        let acp = acp_rules().expect("ACP rules are in the compiled subset");
        assert_eq!(acp.len(), 3);
        assert!(acp.iter().all(|s| s.n_rules() > 0), "every ACP stratum carries rules");
    }
}
