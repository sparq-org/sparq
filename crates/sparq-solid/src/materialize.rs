//! The materializer pipeline: facts (loader) + N3 rules (rules/*.n3) → N3 reasoning
//! strata → the auth-view named graph `<urn:sparq:auth>` swapped into the dataset.
//!
//! Stratification (design doc §3.5): the engine's `log:notIncludes` never retracts, so
//! each negated predicate must be COMPLETE before its stratum runs. WAC negates only
//! input facts → 1 stratum. ACP runs accepts (A) → rejections (B) → grants (C), each
//! stratum's closure carried forward in memory as the next one's facts.
//!
//! sq-zgbso.4: evaluation runs on the **id-level compiled evaluator**
//! (`sparq_reason::n3::compiled`, epic sq-zgbso / issue #1582). The rule text is parsed
//! and lowered to the compiled IR ONCE per process ([`wac_rules`] / [`acp_rules`]), and
//! the facts enter as `[Id; 3]` straight from the source `Graph`'s terms
//! ([`crate::loader::assemble_input_ids`]) — the per-call
//! graph → N3 text → re-parse → string-term fixpoint round trip is gone. The text
//! engine (`sparq_reason::reason_n3`) stays reachable as the `#[cfg(test)]` differential
//! reference, and the tests below assert the two produce the SAME `<urn:sparq:auth>`
//! triple set on the WAC/ACP fixtures.

use crate::budget::{default_auth_budget, AuthBudget, DecisionWork};
use crate::loader::{assemble_input_ids, strip_reserved_graphs, System};
use crate::{AccessProvenance, VerifiedCredentials, AUTH_GRAPH, AUTH_NS};
use oxrdf::{NamedNode, Term};
use rustc_hash::FxHashSet;
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_reason::n3::compiled::{compile, compile_with_cycles, eval_charged, CompiledRuleSet};
use sparq_reason::{ClosureLimits, NegationCycles};
use std::sync::OnceLock;
// `std::time::Instant` is unusable on `wasm32-unknown-unknown` — `Instant::now()`
// panics there (no monotonic clock). The wall-clock plumbing for `stats.millis` is
// purely informational, so it is `cfg`-gated off and reported as `0.0` on wasm32
// rather than trapping at runtime (sq-7agop).
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

const COMMON_RULES: &str = include_str!("../rules/common.n3");
const WAC_RULES: &str = include_str!("../rules/wac.n3");
const ACP_A: &str = include_str!("../rules/acp-a.n3");
const ACP_B: &str = include_str!("../rules/acp-b.n3");
const ACP_C: &str = include_str!("../rules/acp-c.n3");

/// Which reasoning allowance an authorization materialization reached.
///
/// Carried by the refusal ([`auth_allowance_refusal`]) so a caller can act on the kind
/// rather than on wording. `RoundsOrFacts` is one variant because the reasoner reports a
/// closure that ran out of rounds and one that ran out of facts with the same cut, naming
/// both; the message that accompanies the kind gives the values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthAllowance {
    /// `max_total_join_rows`: the binding rows and firings of one stratum, over all its
    /// rounds.
    TotalJoinRows,
    /// `max_join_rows`: the binding rows and firings of one round.
    JoinRows,
    /// `max_rounds` or `max_facts`: the stratum did not reach a fixpoint inside them.
    RoundsOrFacts,
    /// `max_term_bytes`: a term the rules would derive.
    TermBytes,
    /// `max_document_bytes` or `max_input_bytes`: the bytes one control document, or all
    /// of them together, contribute to the reasoning INPUT ([`crate::AuthBudget`]).
    InputBytes,
    /// `max_input_facts`: the facts of the assembled reasoning input.
    InputFacts,
    /// `max_input_term_bytes`: a term of the reasoning INPUT, as opposed to
    /// [`AuthAllowance::TermBytes`], which is one the rules derive.
    InputTermBytes,
    /// `max_resource_iri_bytes` or `max_resource_path_depth`: the length or the container
    /// depth of one resource IRI the input's resource enumeration would walk
    /// ([`crate::AuthBudget`]).
    ResourceIri,
    /// `max_decision_join_rows` or `max_decision_facts`: the work of ALL the closures of
    /// one authorization materialization added up, which the per-closure [`ClosureLimits`]
    /// cannot bound.
    DecisionWork,
    /// An allowance this crate does not name individually (a newer reasoner cut).
    Unspecified,
}

impl AuthAllowance {
    /// The [`ClosureLimits`] or [`crate::AuthBudget`] field(s) this kind stands for, for a
    /// message.
    pub fn limit_name(self) -> &'static str {
        match self {
            AuthAllowance::TotalJoinRows => "max_total_join_rows",
            AuthAllowance::JoinRows => "max_join_rows",
            AuthAllowance::RoundsOrFacts => "max_rounds/max_facts",
            AuthAllowance::TermBytes => "max_term_bytes",
            AuthAllowance::InputBytes => "max_document_bytes/max_input_bytes",
            AuthAllowance::InputFacts => "max_input_facts",
            AuthAllowance::InputTermBytes => "max_input_term_bytes",
            AuthAllowance::ResourceIri => "max_resource_iri_bytes/max_resource_path_depth",
            AuthAllowance::DecisionWork => "max_decision_join_rows/max_decision_facts",
            AuthAllowance::Unspecified => "an unnamed limit",
        }
    }

    /// The tag this kind is carried as.
    fn tag(self) -> &'static str {
        match self {
            AuthAllowance::TotalJoinRows => "total-join-rows",
            AuthAllowance::JoinRows => "join-rows",
            AuthAllowance::RoundsOrFacts => "rounds-or-facts",
            AuthAllowance::TermBytes => "term-bytes",
            AuthAllowance::InputBytes => "input-bytes",
            AuthAllowance::InputFacts => "input-facts",
            AuthAllowance::InputTermBytes => "input-term-bytes",
            AuthAllowance::ResourceIri => "resource-iri",
            AuthAllowance::DecisionWork => "decision-work",
            AuthAllowance::Unspecified => "unspecified",
        }
    }

    /// The kind a tag stands for; [`AuthAllowance::Unspecified`] for one this build does
    /// not know.
    fn from_tag(tag: &str) -> AuthAllowance {
        for kind in [
            AuthAllowance::TotalJoinRows,
            AuthAllowance::JoinRows,
            AuthAllowance::RoundsOrFacts,
            AuthAllowance::TermBytes,
            AuthAllowance::InputBytes,
            AuthAllowance::InputFacts,
            AuthAllowance::InputTermBytes,
            AuthAllowance::ResourceIri,
            AuthAllowance::DecisionWork,
        ] {
            if kind.tag() == tag {
                return kind;
            }
        }
        AuthAllowance::Unspecified
    }
}

/// The text a materialization refused by its reasoning allowance puts in its error — the
/// one refusal an operator must read as "this pod's policies are too big for the allowance
/// in force", not as "these policies deny access".
///
/// A refusal of either kind leaves no grant behind (the view in force is kept or dropped,
/// never widened), so the distinction is operational rather than one of access: a deny is
/// the policies' decision, while this says the policies were never decided at all and the
/// allowance has to be raised ([`crate::PodStore::set_auth_limits`]) before they will be.
///
/// Read it with [`auth_allowance_refusal`] rather than by matching this text: every refusal
/// also carries a machine-readable kind tag directly after this wording, and that function
/// finds the pair ANYWHERE in the
/// message. This crate's materialization errors are `String`s with no error chain to walk,
/// and the write paths wrap a failed rebuild's error in their own sentence, so a refusal
/// must be recognizable from the middle of a longer message — not only as a prefix.
pub const AUTH_ALLOWANCE_REFUSAL: &str = "authorization reasoning allowance exceeded";

/// The opening of the kind tag every refusal carries; the kind's own tag and a `]` follow.
const AUTH_ALLOWANCE_TAG: &str = "[allowance=";

/// Which allowance `err` was refused by, or `None` when `err` is not a refusal by the
/// reasoning allowance.
///
/// `err` is an error from any `materialize_*` entry point, free function or
/// [`crate::PodStore`] method, INCLUDING one a write path wrapped in its own sentence (an
/// update whose rebuild was refused, a rollback that could not rebuild): the kind tag is
/// read wherever it appears, so wrapping does not hide it — but only where the refusal
/// WORDING immediately precedes it, which is the shape every refusal is written in, so a
/// message that merely quotes the tag literal (inside a pod's own IRI, say) is not read as
/// a refusal.
///
/// # Examples
///
/// ```
/// assert_eq!(sparq_solid::auth_allowance_refusal("some other error"), None);
/// ```
pub fn auth_allowance_refusal(err: &str) -> Option<AuthAllowance> {
    // Anchored to the shape `allowance_refusal` writes — the refusal wording, one space,
    // then the tag — rather than to the tag alone. A wrapping sentence keeps that pair
    // adjacent wherever it puts it, so a wrapped refusal is still found.
    for (at, _) in err.match_indices(AUTH_ALLOWANCE_TAG) {
        if !err[..at]
            .strip_suffix(' ')
            .is_some_and(|before| before.ends_with(AUTH_ALLOWANCE_REFUSAL))
        {
            continue;
        }
        let rest = &err[at + AUTH_ALLOWANCE_TAG.len()..];
        if let Some(end) = rest.find(']') {
            return Some(AuthAllowance::from_tag(&rest[..end]));
        }
    }
    None
}

/// Whether `err` is a refusal by the reasoning allowance rather than any other
/// materialization error — [`auth_allowance_refusal`] without the kind.
///
/// # Examples
///
/// ```
/// assert!(!sparq_solid::is_auth_allowance_refusal("some other materialization error"));
/// ```
pub fn is_auth_allowance_refusal(err: &str) -> bool {
    auth_allowance_refusal(err).is_some()
}

/// The reasoning allowance every authorization closure runs under unless the store is given
/// another one ([`crate::PodStore::set_auth_limits`]).
///
/// The reasoner's own [`ClosureLimits::default`] leaves `max_join_rows` unbounded, so one
/// round's premise joins may produce rows without bound — and the authorization rules join
/// patterns whose matches multiply (a policy's matchers against its resources, agents,
/// clients and issuers). The authorization path therefore sets its own finite allowance:
/// the limits below, which a pod far larger than anything this crate materializes today
/// stays well inside.
///
/// # How the values were chosen
///
/// Measured, not guessed. For the largest fixtures this crate has — `fixture::wac_fixture`
/// (~1.1k named graphs, the one `examples/bench.rs` materializes), the same fixture with
/// filler content (`fixture::wac_fixture_sized(50)`, which reasons over the same control
/// documents) and `fixture::acp_fixture` across all three strata — the SMALLEST allowance
/// each field could take while the view still materialized was found by bisection, and
/// each value here is about ten times the largest of those, rounded:
///
/// | field                 | largest minimum over those fixtures | the value here | margin |
/// |-----------------------|------------------------------------:|---------------:|-------:|
/// | `max_total_join_rows` |                             357,200 |      4,000,000 |  ~11×  |
/// | `max_join_rows`       |                              82,435 |      1,000,000 |  ~12×  |
/// | `max_facts`           |                              15,847 |        256,000 |  ~16×  |
/// | `max_rounds`          |                                  11 |            128 |  ~12×  |
/// | `max_term_bytes`      |                               1,197 |         64 KiB |  ~55×  |
///
/// (A fixture-and-box-specific measurement, so it is recorded here rather than in any
/// markdown.) The two that matter are the join-row allowances, which the reasoner's own
/// default leaves unbounded; `max_total_join_rows` is the one that caps the work a single
/// materialization can be made to do, because the per-round allowance is refilled every
/// round and so bounds a closure only as its own value times `max_rounds`. `max_facts`
/// bounds one closure's held facts, so under ACP each of the three strata is bounded by
/// it separately, as each is its own closure.
///
/// The margins are deliberate but no longer open-ended: an untrusted writer of a control
/// document chooses how far its policies multiply, so the ceiling is what they can spend.
/// A pod that genuinely needs more raises it ([`crate::PodStore::set_auth_limits`]).
pub fn default_auth_limits() -> ClosureLimits {
    let mut limits = ClosureLimits::default();
    limits.max_rounds = AUTH_MAX_ROUNDS;
    limits.max_facts = AUTH_MAX_FACTS;
    limits.max_term_bytes = AUTH_MAX_TERM_BYTES;
    limits.max_join_rows = AUTH_MAX_JOIN_ROWS;
    limits.max_total_join_rows = AUTH_MAX_TOTAL_JOIN_ROWS;
    limits
}

/// Rounds one authorization stratum may run. The rules are not recursive over a growing
/// term, so a stratum reaches its fixpoint in a handful of rounds on any pod; this is a
/// runaway guard, not a working bound.
const AUTH_MAX_ROUNDS: usize = 128;
/// Facts one authorization stratum may hold, asserted and derived together.
const AUTH_MAX_FACTS: usize = 256_000;
/// Bytes of one term the rules may derive. The rules derive compound terms only as minted
/// candidate/matcher-instance IRIs (a few hundred bytes of percent-encoded components), so
/// 1 MiB is already far above anything they build.
const AUTH_MAX_TERM_BYTES: usize = 64 << 10;
/// Binding rows and rule firings one round of one authorization stratum may produce.
const AUTH_MAX_JOIN_ROWS: usize = 1_000_000;
/// Binding rows and rule firings one authorization stratum may produce over ALL its rounds
/// — the bound that actually caps the work a materialization can be made to do, since the
/// per-round allowance is refilled every round. Both of these are unbounded in the
/// reasoner's defaults. See [`default_auth_limits`] for how they were measured.
const AUTH_MAX_TOTAL_JOIN_ROWS: usize = 4_000_000;

/// One authorization stratum, under `limits`: [`eval_with_limits`] with the allowance
/// refusal tagged ([`AUTH_ALLOWANCE_REFUSAL`]) so the caller can tell a pod too big for the
/// allowance from any other materialization failure. Every other error is passed through
/// unchanged.
///
/// The tag is added here, at the one seam every authorization closure in this crate goes
/// through, so no stratum can return an untagged allowance refusal.
///
/// `work` is the materialization's whole-decision ledger: what this closure charged is
/// added to it and the aggregate re-checked, because `limits` applies per CLOSURE and a
/// materialization runs several (three ACP strata, five ODRL ones) — see
/// [`crate::AuthBudget`].
pub(crate) fn auth_eval(
    dict: &mut Dict,
    facts: &[[sparq_core::dict::Id; 3]],
    rules: &CompiledRuleSet,
    limits: ClosureLimits,
    work: &mut DecisionWork,
) -> Result<Vec<[sparq_core::dict::Id; 3]>, String> {
    let (closure, charge) =
        eval_charged(dict, facts, rules, limits).map_err(tag_allowance_refusal)?;
    work.charge(charge)?;
    Ok(closure)
}

/// An authorization refusal of kind `kind` carrying `detail`: the same
/// [`AUTH_ALLOWANCE_REFUSAL`] wording and machine-readable kind tag
/// [`auth_allowance_refusal`] reads off a reasoning-allowance cut, so the input and
/// whole-decision budgets ([`crate::AuthBudget`]) join ONE refusal family rather than
/// adding a second one a caller has to learn.
pub(crate) fn allowance_refusal(kind: AuthAllowance, detail: &str) -> String {
    format!(
        "{} {}{}]: {}",
        AUTH_ALLOWANCE_REFUSAL,
        AUTH_ALLOWANCE_TAG,
        kind.tag(),
        detail
    )
}

/// `e` tagged as an allowance refusal ([`AUTH_ALLOWANCE_REFUSAL`] plus the kind
/// [`auth_allowance_refusal`] reads) when it is the reasoner's forward-closure cut,
/// unchanged otherwise.
pub(crate) fn tag_allowance_refusal(e: String) -> String {
    if !e.contains(CLOSURE_CUT) {
        return e;
    }
    // Which limit the cut names. `max_total_join_rows` contains `max_join_rows`, so it is
    // tested first.
    let kind = if e.contains("max_total_join_rows") {
        AuthAllowance::TotalJoinRows
    } else if e.contains("max_join_rows") {
        AuthAllowance::JoinRows
    } else if e.contains("max_term_bytes") || e.contains("would be a term of about") {
        AuthAllowance::TermBytes
    } else if e.contains("before reaching a fixpoint") {
        AuthAllowance::RoundsOrFacts
    } else {
        AuthAllowance::Unspecified
    };
    format!(
        "{} {}{}]: {}",
        AUTH_ALLOWANCE_REFUSAL,
        AUTH_ALLOWANCE_TAG,
        kind.tag(),
        e
    )
}

/// The reasoner's own text for a forward closure stopped by its [`ClosureLimits`] (every
/// cut message contains it, and each names the limit it reached).
const CLOSURE_CUT: &str = "forward closure limit exceeded";

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
                // acp-c.n3 concludes a variable predicate (`{ ?p ?pred ?r }`) next to
                // store-scoped negation, so it cannot be stratified yet: keep single-pass
                // semantics by explicit opt-in. Follow-up: rewrite it with one concrete
                // conclusion per mode and drop the opt-in.
                compile_with_cycles(ACP_C, NegationCycles::SinglePass)?,
            ])
        })
        .as_ref()
        .map_err(|e| format!("ACP rules: {}", e))
}

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const EXCEPT_MATCHER: &str = "https://sparq.dev/ns/auth#exceptMatcher";
/// solidx facts copied into the auth view for matchers referenced by conditional
/// grants (the session layer evaluates `exceptMatcher`s from these).
/// sq-3jtd.6 adds `acceptsIssuerP` — the noneOf exception check is three-dimensional.
/// `provUndecided` marks a provenance matcher instance whose resource has no recorded
/// creator/owner: the session layer reads it as the third truth value (an allow that needs
/// the exception decided does not apply, a deny that needs it does).
const MATCHER_FACTS: [&str; 4] = [
    "https://sparq.dev/ns/solidx#acceptsAgentP",
    "https://sparq.dev/ns/solidx#acceptsClientP",
    "https://sparq.dev/ns/solidx#acceptsIssuerP",
    "https://sparq.dev/ns/solidx#provUndecided",
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
/// assert!(graph.named().iter().any(|(name, _)| matches!(
///     name, Term::NamedNode(n) if n.as_str() == sparq_solid::AUTH_GRAPH)));
/// # Ok::<(), String>(())
/// ```
pub fn materialize_wac(graph: &mut Graph) -> Result<MaterializeStats, String> {
    materialize_wac_limited(graph, default_auth_limits(), default_auth_budget())
}

/// [`materialize_wac`] under `limits` and `budget` instead of [`default_auth_limits`] and
/// [`default_auth_budget`] — the seam [`crate::PodStore`] uses to apply the allowance and
/// budget it was configured with.
pub(crate) fn materialize_wac_limited(
    graph: &mut Graph,
    limits: ClosureLimits,
    budget: AuthBudget,
) -> Result<MaterializeStats, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    // Every fallible step runs BEFORE the first mutation of `graph`, so an `Err` leaves
    // the dataset — including any previously materialized `<urn:sparq:auth>` view —
    // byte-for-byte untouched, as the contract above promises. Assembly ignores the
    // reserved graph space itself, so it does not depend on the strip having run.
    crate::loader::require_writable(graph)?;
    let rules = wac_rules()?;
    let mut dict = Dict::new();
    // WAC has no creator/owner vocabulary; provenance is ignored (the loader skips it).
    let facts = assemble_input_ids(
        &mut dict,
        graph,
        System::Wac,
        &AccessProvenance::new(),
        &VerifiedCredentials::new(),
        budget,
    )?;
    let mut work = DecisionWork::new(budget);
    let closure = auth_eval(&mut dict, &facts, rules, limits, &mut work)?;
    prepare_auth_view(graph)?;
    let mut stats = install_auth_view(graph, &dict, &closure)?;
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
/// matchers (sq-3jtd.5).
///
/// The free-function form of [`crate::PodStore::materialize_acp_with`]. Identical to
/// [`materialize_acp`] (one three-stratum `reason_n3_stratified` run) except the loader also
/// synthesizes `<r> solidx:creator|owner <webid>` facts from `provenance` — the trusted
/// channel for "who created/owns `<r>`". These facts are **never** read from the resource
/// graphs (design doc §2.4): a writer cannot self-grant via a forged `solidx:creator`
/// triple in a document they control.
///
/// This free function's `materialize_acp(graph)` is exactly
/// `materialize_acp_with(graph, &AccessProvenance::new())` — with no provenance, no
/// `CreatorAgent`/`OwnerAgent` matcher ever grants (fail-closed). (The
/// `PodStore::materialize_acp` method instead reuses the store's stored context.)
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
/// `acp:vc` matchers (sq-ysv3u).
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
    materialize_acp_limited(
        graph,
        provenance,
        credentials,
        default_auth_limits(),
        default_auth_budget(),
    )
}

/// [`materialize_acp_with_credentials`] under `limits` and `budget` instead of
/// [`default_auth_limits`] and [`default_auth_budget`] — the seam [`crate::PodStore`] uses
/// to apply the allowance and budget it was configured with.
///
/// Every stratum runs under the same `limits`, and each gets them afresh (they bound one
/// closure, so a later stratum is not charged for an earlier one). `budget` is what bounds
/// the three strata TOGETHER: each closure's charge goes into one [`DecisionWork`] ledger,
/// which refuses once their aggregate is past
/// [`AuthBudget::max_decision_join_rows`] / [`AuthBudget::max_decision_facts`].
pub(crate) fn materialize_acp_limited(
    graph: &mut Graph,
    provenance: &AccessProvenance,
    credentials: &VerifiedCredentials,
    limits: ClosureLimits,
    budget: AuthBudget,
) -> Result<MaterializeStats, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let t0 = Instant::now();
    // Fallible work first, mutation last — see [`materialize_wac`].
    crate::loader::require_writable(graph)?;
    let rules = acp_rules()?;
    let mut dict = Dict::new();
    let facts = assemble_input_ids(
        &mut dict,
        graph,
        System::Acp,
        provenance,
        credentials,
        budget,
    )?;
    let mut work = DecisionWork::new(budget);
    // Chain the three strata entirely at the id level, in ONE dictionary: each stratum's
    // closure is the next one's fact set, so a negated predicate is complete before the
    // stratum that negates it runs (design doc §3.5) and nothing round-trips through text.
    let mut closure = facts;
    let mut strata_facts = Vec::with_capacity(rules.len());
    for stratum in rules {
        closure = auth_eval(&mut dict, &closure, stratum, limits, &mut work)?;
        strata_facts.push(closure.len());
    }

    prepare_auth_view(graph)?;
    let mut stats = install_auth_view(graph, &dict, &closure)?;
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
/// rather than trapping.
#[cfg(not(target_arch = "wasm32"))]
fn elapsed_millis(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1e3
}

#[cfg(target_arch = "wasm32")]
fn elapsed_millis() -> f64 {
    0.0
}

/// Remove other reserved graphs while retaining a durable auth slot for replacement.
fn prepare_auth_view(graph: &mut Graph) -> Result<(), String> {
    if !graph.is_directory_backed() {
        return strip_reserved_graphs(graph);
    }
    let reserved: Vec<_> = graph
        .named()
        .iter()
        .filter_map(|(name, _)| match name {
            Term::NamedNode(n)
                if n.as_str().starts_with("urn:sparq:") && n.as_str() != AUTH_GRAPH =>
            {
                Some(name.clone())
            }
            _ => None,
        })
        .collect();
    for name in reserved {
        graph.drop_named_durable(&name)?;
    }
    Ok(())
}

/// Whether a closure triple belongs in the auth view: `auth:*` predicates, `rdf:type`
/// with an `auth:` class, plus the accept-set facts of matchers referenced by
/// `auth:exceptMatcher` (collected in a first pass).
fn install_auth_view(
    graph: &mut Graph,
    dict: &Dict,
    closure: &[[sparq_core::dict::Id; 3]],
) -> Result<MaterializeStats, String> {
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
    crate::loader::install_named_view(graph, name, auth)?;
    Ok(stats)
}

/// sq-zgbso.4 — the RESULT-EQUIVALENCE oracle for the compiled flip.
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
    use sparq_reason::{reason_n3, reason_n3_stratified_with_cycles};
    use std::collections::BTreeSet;

    /// The WAC auth view as the TEXT engine derives it (the pre-sq-zgbso.4 pipeline).
    fn wac_text(graph: &mut Graph) -> Result<MaterializeStats, String> {
        strip_reserved_graphs(graph)?;
        let input = assemble_input(
            graph,
            System::Wac,
            &AccessProvenance::new(),
            &VerifiedCredentials::new(),
            default_auth_budget(),
        )?;
        let src = format!("{}\n{}\n{}", input, COMMON_RULES, WAC_RULES);
        let mut dict = Dict::new();
        let closure = reason_n3(&mut dict, &src)?;
        install_auth_view(graph, &dict, &closure)
    }

    /// The ACP auth view as the TEXT engine derives it (the pre-sq-zgbso.4 pipeline).
    fn acp_text(
        graph: &mut Graph,
        provenance: &AccessProvenance,
        credentials: &VerifiedCredentials,
    ) -> Result<MaterializeStats, String> {
        strip_reserved_graphs(graph)?;
        let input = assemble_input(
            graph,
            System::Acp,
            provenance,
            credentials,
            default_auth_budget(),
        )?;
        let accepts = format!("{}\n{}\n{}", input, COMMON_RULES, ACP_A);
        let mut dict = Dict::new();
        // acp-c.n3 keeps single-pass semantics by explicit opt-in, as in `acp_rules`.
        let closure = reason_n3_stratified_with_cycles(
            &mut dict,
            &[&accepts, ACP_B, ACP_C],
            NegationCycles::SinglePass,
        )?;
        install_auth_view(graph, &dict, &closure.facts)
    }

    /// The installed `<urn:sparq:auth>` view as a comparable set of `s p o` strings.
    fn auth_view(graph: &Graph) -> BTreeSet<String> {
        let name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
        let Some((_, g)) = graph.named().iter().find(|(n, _)| *n == name) else {
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
        for (name, _) in Graph::load_dataset(&src, "nquads").expect("fixture loads").named().iter() {
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
            strip_reserved_graphs(&mut graph).unwrap();
            let prov = AccessProvenance::new();

            let mut dict = Dict::new();
            let ids = assemble_input_ids(
                &mut dict,
                &graph,
                system,
                &prov,
                &VerifiedCredentials::new(),
                default_auth_budget(),
            )
            .expect("id facts");
            let direct: BTreeSet<String> = ids
                .iter()
                .map(|t| format!("{} {} {}", dict.term(t[0]), dict.term(t[1]), dict.term(t[2])))
                .collect();

            let text = assemble_input(
                &graph,
                system,
                &prov,
                &VerifiedCredentials::new(),
                default_auth_budget(),
            )
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
        Graph::load_dataset(&nq, "nquads")
            .expect("poison graph loads")
            .take_named()
            .expect("an in-memory dataset")
    }

    /// The documented contract is that an `Err` leaves the dataset's previous auth view
    /// untouched. That holds ONLY if every fallible step (rule compilation, fact
    /// assembly / principal validation) runs BEFORE `strip_reserved_graphs` deletes the
    /// old `<urn:sparq:auth>` — strip-first would fail OPEN into a dataset whose
    /// authorization view had silently vanished. A document naming a reserved principal is
    /// rejected at admission and fails nothing, so the error comes from trusted provenance.
    #[test]
    fn a_failed_rematerialization_leaves_the_previous_auth_view_in_place() {
        let mut graph = Graph::load_dataset(&crate::wac_fixture(), "nquads").expect("fixture loads");
        materialize_wac(&mut graph).expect("first WAC materialization succeeds");
        graph.extend_named(poison_graph(
            "https://pod.ex/evil.acl",
            "http://www.w3.org/ns/auth/acl#agent",
        ))
        .unwrap();
        materialize_wac(&mut graph).expect("a document naming a reserved agent is rejected, not fatal");
        assert!(
            !auth_view(&graph).iter().any(|t| t.contains("urn:sparq:evil")),
            "the rejected document grants nothing"
        );

        let mut graph = Graph::load_dataset(&crate::acp_fixture(), "nquads").expect("fixture loads");
        materialize_acp(&mut graph).expect("first ACP materialization succeeds");
        let before = auth_view(&graph);
        assert!(!before.is_empty(), "no auth view to preserve — vacuous test");
        let mut poisoned = AccessProvenance::new();
        poisoned.set_creator("https://pod.ex/docs/d0", "urn:sparq:evil");
        let err = materialize_acp_with_credentials(&mut graph, &poisoned, &VerifiedCredentials::new())
            .expect_err("a reserved creator IRI must fail");
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
