//! sq-3jtd.9 — a library-level **ACP conformance harness**.
//!
//! # What this is (and the decision behind it)
//!
//! The conformance surface targeted here is the **Solid Access Control Policy (ACP)
//! specification** — <https://solidproject.org/TR/acp> — exercised at the *library*
//! level: a scenario is an ACR-document corpus plus a table of expected
//! `(agent, client, mode, resource) → allow | deny` decisions, and the harness asserts
//! that this crate's ACP authorization engine
//! ([`crate::materialize_acp`] + [`crate::AuthIndex::accessible`]) reproduces every
//! expected decision (`research/sparq-solid-scope.md` §4).
//!
//! This is the **realistic, achievable** conformance signal for an authorization
//! *oracle*. Two routes were considered and deliberately NOT taken here:
//!
//! - **CTH-over-HTTP** (the Solid Conformance Test Harness / `solid/specification-tests`)
//!   drives a *server* and asserts on HTTP outputs (status codes, `WAC-Allow`). This
//!   crate has no HTTP surface — that route is out-of-scope (it belongs to the Solid
//!   server, conformance-tested *through* the server), per the scope record.
//! - **A differential oracle against a JS reference evaluator** (Community Solid Server /
//!   Inrupt ESS — same corpus, diff decisions) is the credible *second* oracle, but is
//!   research-open (JS-toolchain cost) and tracked as a follow-up — it is NOT built here.
//!
//! Instead, scenarios are declared as **data** (this module is a builder + runner), and
//! the expected-decision table is derived from the ACP spec's normative semantics:
//! matchers (`acp:agent` / `acp:client`, `acp:PublicAgent`, `acp:AuthenticatedAgent`),
//! policies (`acp:allow` / `acp:deny`, `acp:allOf` / `acp:anyOf` / `acp:noneOf`), and
//! access control with **cumulative** ancestor inheritance
//! (`acp:accessControl` / `acp:memberAccessControl`). The corpus that consumes this
//! harness lives in `tests/conformance_acp.rs`.
//!
//! # Relationship to `tests/acp.rs`
//!
//! `tests/acp.rs` asserts a hand-derived access *matrix* over the one big synthetic
//! `acp_fixture()` pod — bespoke, internally authored. This harness is structurally
//! different: it is **table-driven over independent, minimal scenarios**, each isolating
//! one ACP construct, with a uniform pass/fail report. The two are complementary: the
//! matrix exercises depth and interaction on a realistic pod; this harness exercises
//! spec-construct coverage with small, auditable, independently-failing cases.
//!
//! # Example
//!
//! ```
//! use sparq_solid::conformance::{AcrBuilder, AcpScenario, Decision, Expect};
//! use sparq_solid::Mode;
//!
//! let alice = "https://alice.example/card#me";
//! let bob = "https://bob.example/card#me";
//! let doc = "https://pod.example/notes/n1";
//!
//! // An ACR on the document grants Read to exactly { alice } via an anyOf agent matcher.
//! let mut acr = AcrBuilder::new();
//! acr.access_control(doc, |p| p.allow(Mode::Read).any_of_agent(alice));
//! acr.document(doc); // the protected resource is a (possibly empty) named graph
//!
//! let scenario = AcpScenario::new("agent-allow")
//!     .acr(acr)
//!     .expect(Expect::agent(alice).read(doc).is(Decision::Allow))
//!     .expect(Expect::agent(bob).read(doc).is(Decision::Deny))
//!     .expect(Expect::anonymous().read(doc).is(Decision::Deny));
//!
//! let report = scenario.run().expect("materializes");
//! assert!(report.passed(), "{report}"); // every expected decision reproduced
//! ```

use crate::{materialize_acp, AuthIndex, Mode, Session};
use sparq_core::Graph;
use std::fmt::{self, Write as _};

const ACP: &str = "http://www.w3.org/ns/solid/acp#";
const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const EX_PRED: &str = "https://ex.dev/ns#prop";

/// The ACP `acp:PublicAgent` IRI — the matcher that accepts every requestor, including
/// the anonymous one.
pub const PUBLIC_AGENT: &str = "http://www.w3.org/ns/solid/acp#PublicAgent";
/// The ACP `acp:AuthenticatedAgent` IRI — the matcher that accepts any *authenticated*
/// requestor (an agent is present) but not the anonymous one.
pub const AUTHENTICATED_AGENT: &str = "http://www.w3.org/ns/solid/acp#AuthenticatedAgent";

/// A single authorization decision: the access is granted (`Allow`) or refused
/// (`Deny`). This is the binary the harness compares against the engine's verdict
/// (a graph is in the session's accessible set ⇒ `Allow`, otherwise `Deny`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The access mode is granted on the resource for the requestor.
    Allow,
    /// The access mode is refused (no matching grant, or a deny overrides).
    Deny,
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Decision::Allow => "allow",
            Decision::Deny => "deny",
        })
    }
}

/// One expected decision in a scenario's table: a `(agent, client, mode, resource)`
/// request and the decision the ACP spec requires for it.
///
/// Build with the fluent helpers: [`Expect::agent`] / [`Expect::anonymous`] /
/// [`Expect::pair`] pick the requestor, then a mode helper
/// ([`ExpectBuilder::read`] / [`ExpectBuilder::write`] / [`ExpectBuilder::append`] /
/// [`ExpectBuilder::control`]) names the resource, then [`ExpectWithMode::is`] fixes the
/// expected [`Decision`].
#[derive(Debug, Clone)]
pub struct Expect {
    agent: Option<String>,
    client: Option<String>,
    mode: Mode,
    resource: String,
    decision: Decision,
}

impl Expect {
    /// Start an expectation for an authenticated `agent` (no client constraint).
    pub fn agent(agent: &str) -> ExpectBuilder {
        ExpectBuilder {
            agent: Some(agent.to_owned()),
            client: None,
        }
    }

    /// Start an expectation for the **anonymous** requestor (no agent, no client).
    pub fn anonymous() -> ExpectBuilder {
        ExpectBuilder {
            agent: None,
            client: None,
        }
    }

    /// Start an expectation for an `(agent, client)` pair — the native ACP
    /// (user, application) principal.
    pub fn pair(agent: &str, client: &str) -> ExpectBuilder {
        ExpectBuilder {
            agent: Some(agent.to_owned()),
            client: Some(client.to_owned()),
        }
    }

    /// The decision this expectation asserts the engine must reproduce.
    pub fn decision(&self) -> Decision {
        self.decision
    }

    /// The requestor's agent WebID (`None` = anonymous). sq-3jtd.8 — exposed so
    /// the sibling WAC harness ([`crate::wac_conformance`]) can build a [`crate::Session`]
    /// from a shared [`Expect`]. (Named `req_*` so it does not shadow the [`Expect::agent`]
    /// *constructor*.)
    pub fn req_agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    /// The requestor's client identifier (`None` = any client). sq-3jtd.8.
    pub fn req_client(&self) -> Option<&str> {
        self.client.as_deref()
    }

    /// The access mode this expectation is about. sq-3jtd.8.
    pub fn req_mode(&self) -> Mode {
        self.mode
    }

    /// The resource (named graph) this expectation is about. sq-3jtd.8.
    pub fn req_resource(&self) -> &str {
        &self.resource
    }
}

/// Half-built [`Expect`]: requestor chosen, resource + mode + decision still to come.
#[derive(Debug, Clone)]
pub struct ExpectBuilder {
    agent: Option<String>,
    client: Option<String>,
}

/// [`Expect`] with the requestor + mode + resource fixed, awaiting the [`Decision`].
#[derive(Debug, Clone)]
pub struct ExpectWithMode {
    agent: Option<String>,
    client: Option<String>,
    mode: Mode,
    resource: String,
}

impl ExpectBuilder {
    /// Expect a read decision on `resource`.
    pub fn read(self, resource: &str) -> ExpectWithMode {
        self.with_mode(Mode::Read, resource)
    }
    /// Expect a write decision on `resource`.
    pub fn write(self, resource: &str) -> ExpectWithMode {
        self.with_mode(Mode::Write, resource)
    }
    /// Expect an append decision on `resource`.
    pub fn append(self, resource: &str) -> ExpectWithMode {
        self.with_mode(Mode::Append, resource)
    }
    /// Expect a control decision on `resource` (governs the resource's ACR document).
    pub fn control(self, resource: &str) -> ExpectWithMode {
        self.with_mode(Mode::Control, resource)
    }
    fn with_mode(self, mode: Mode, resource: &str) -> ExpectWithMode {
        ExpectWithMode {
            agent: self.agent,
            client: self.client,
            mode,
            resource: resource.to_owned(),
        }
    }
}

impl ExpectWithMode {
    /// Fix the expected [`Decision`], completing the [`Expect`].
    pub fn is(self, decision: Decision) -> Expect {
        Expect {
            agent: self.agent,
            client: self.client,
            mode: self.mode,
            resource: self.resource,
            decision,
        }
    }
}

/// A builder for one ACP access-control resource (ACR) corpus, written as N-Quads (each
/// ACR into its own named graph `<{resource}.acr>`).
///
/// An ACR binds the protected `resource` to one or more **access controls** — either
/// [`AcrBuilder::access_control`] (`acp:accessControl`, applies to the resource itself)
/// or [`AcrBuilder::member_access_control`] (`acp:memberAccessControl`, applies to the
/// resource's members, i.e. descendants). Each carries one or more **policies**, each
/// declared by a `|policy|` closure over a [`PolicyBuilder`].
///
/// To make a resource a concrete protected graph (so an `Allow` is observable as a
/// non-empty accessible set), register it with [`AcrBuilder::document`].
///
/// **Containment is by IRI structure**, not by an `ldp:contains` triple: this crate's
/// ACP engine derives the ancestry of a resource from its Solid slash path (design doc /
/// `rules/common.n3`), so a `memberAccessControl` on `https://pod.example/c/` reaches
/// `https://pod.example/c/d` because the latter is structurally under the former. To
/// make a container a known ancestor it must be a resource — give it its own ACR (via
/// `member_access_control`) or register it with [`AcrBuilder::document`]; a structural
/// prefix of any present graph is also recognised automatically.
#[derive(Debug, Default, Clone)]
pub struct AcrBuilder {
    quads: String,
    controls: usize,
}

impl AcrBuilder {
    /// A new, empty ACR builder.
    pub fn new() -> AcrBuilder {
        AcrBuilder::default()
    }

    /// Add an `acp:accessControl` on `resource` — its policies apply to the resource
    /// itself. The `build` closure declares the (single) policy attached to it.
    pub fn access_control(
        &mut self,
        resource: &str,
        build: impl FnOnce(PolicyBuilder<'_>) -> PolicyBuilder<'_>,
    ) -> &mut AcrBuilder {
        self.add_control(resource, "accessControl", build)
    }

    /// Add an `acp:memberAccessControl` on `resource` — its policies apply to the
    /// resource's **members** (its IRI-structural descendants; containment is by slash
    /// path, see the type-level docs). ACP inheritance is cumulative: every ancestor's
    /// member access control applies.
    pub fn member_access_control(
        &mut self,
        resource: &str,
        build: impl FnOnce(PolicyBuilder<'_>) -> PolicyBuilder<'_>,
    ) -> &mut AcrBuilder {
        self.add_control(resource, "memberAccessControl", build)
    }

    fn add_control(
        &mut self,
        resource: &str,
        pred: &str,
        build: impl FnOnce(PolicyBuilder<'_>) -> PolicyBuilder<'_>,
    ) -> &mut AcrBuilder {
        let g = format!("{resource}.acr");
        // bind the ACR graph to its resource (ACP `acp:resource`)
        let _ = writeln!(self.quads, "<{g}> <{ACP}resource> <{resource}> <{g}> .");
        let ix = self.controls;
        self.controls += 1;
        let control = format!("{g}#ctl{ix}");
        let _ = writeln!(self.quads, "<{g}> <{ACP}{pred}> <{control}> <{g}> .");
        let pol = format!("{g}#pol{ix}");
        let _ = writeln!(self.quads, "<{control}> <{ACP}apply> <{pol}> <{g}> .");
        // `ix` namespaces this policy's matcher IRIs so two policies in the SAME ACR
        // graph never collide on a matcher node (a collision would silently merge their
        // matcher attributes — e.g. an allow policy's PublicAgent matcher fusing with a
        // deny policy's agent matcher).
        let pb = PolicyBuilder {
            quads: &mut self.quads,
            graph: g,
            control_ix: ix,
            matchers: 0,
            policy: pol,
        };
        build(pb);
        self
    }

    /// Register `resource` as a concrete protected document: emit a placeholder triple
    /// into `<{resource}>` so that a granted access is observable as a non-empty
    /// accessible set.
    pub fn document(&mut self, resource: &str) -> &mut AcrBuilder {
        let _ = writeln!(
            self.quads,
            "<{resource}#it> <{EX_PRED}> \"x\" <{resource}> ."
        );
        self
    }

    /// The accumulated N-Quads of this ACR corpus.
    pub fn into_nquads(self) -> String {
        self.quads
    }
}

/// A fluent builder for one ACP policy: its `acp:allow` / `acp:deny` modes and its
/// `acp:allOf` / `acp:anyOf` / `acp:noneOf` matchers.
///
/// ACP policy semantics (normative): a policy is **satisfied** for a request when every
/// `allOf` matcher matches AND at least one `anyOf` matcher matches (a policy with no
/// `anyOf` matchers requires only the `allOf` set) AND no `noneOf` matcher matches. A
/// satisfied policy contributes its `acp:allow` modes as grants and its `acp:deny`
/// modes as denials; **deny overrides** allow for the same `(principal, resource, mode)`.
#[derive(Debug)]
pub struct PolicyBuilder<'a> {
    quads: &'a mut String,
    graph: String,
    policy: String,
    /// Namespaces this policy's matcher IRIs within the shared ACR graph.
    control_ix: usize,
    matchers: usize,
}

impl PolicyBuilder<'_> {
    fn mode_iri(mode: Mode) -> &'static str {
        match mode {
            Mode::Read => "Read",
            Mode::Write => "Write",
            Mode::Append => "Append",
            Mode::Control => "Control",
        }
    }

    /// Add an `acp:allow <mode>` to this policy.
    pub fn allow(self, mode: Mode) -> Self {
        let m = Self::mode_iri(mode);
        let _ = writeln!(
            self.quads,
            "<{}> <{ACP}allow> <{ACL}{m}> <{}> .",
            self.policy, self.graph
        );
        self
    }

    /// Add an `acp:deny <mode>` to this policy (deny overrides allow at decision time).
    pub fn deny(self, mode: Mode) -> Self {
        let m = Self::mode_iri(mode);
        let _ = writeln!(
            self.quads,
            "<{}> <{ACP}deny> <{ACL}{m}> <{}> .",
            self.policy, self.graph
        );
        self
    }

    fn matcher(&mut self, combinator: &str) -> String {
        let m = format!(
            "{}#m{}-{combinator}{}",
            self.graph, self.control_ix, self.matchers
        );
        self.matchers += 1;
        let _ = writeln!(
            self.quads,
            "<{}> <{ACP}{combinator}> <{m}> <{}> .",
            self.policy, self.graph
        );
        m
    }

    /// Add an `acp:anyOf` matcher accepting the given `agent` (a WebID, or
    /// [`PUBLIC_AGENT`] / [`AUTHENTICATED_AGENT`]).
    pub fn any_of_agent(mut self, agent: &str) -> Self {
        let m = self.matcher("anyOf");
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}agent> <{agent}> <{}> .",
            self.graph
        );
        self
    }

    /// Add an `acp:allOf` matcher accepting the given `agent`.
    pub fn all_of_agent(mut self, agent: &str) -> Self {
        let m = self.matcher("allOf");
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}agent> <{agent}> <{}> .",
            self.graph
        );
        self
    }

    /// Add an `acp:allOf` matcher accepting the given `client` (application id) — the
    /// other half of the native ACP (user, application) pair.
    pub fn all_of_client(mut self, client: &str) -> Self {
        let m = self.matcher("allOf");
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}client> <{client}> <{}> .",
            self.graph
        );
        self
    }

    /// Add an `acp:anyOf` matcher accepting the given `client`.
    pub fn any_of_client(mut self, client: &str) -> Self {
        let m = self.matcher("anyOf");
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}client> <{client}> <{}> .",
            self.graph
        );
        self
    }

    /// Add an `acp:noneOf` matcher: the policy is satisfied only when this matcher does
    /// NOT match (the ACP "except" carve-out). Matches on `agent`.
    pub fn none_of_agent(mut self, agent: &str) -> Self {
        let m = self.matcher("noneOf");
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}agent> <{agent}> <{}> .",
            self.graph
        );
        self
    }

    /// Add a single `acp:allOf` matcher requiring BOTH `agent` AND `client` to match —
    /// the explicit (user, application) pair on one matcher.
    pub fn all_of_pair(mut self, agent: &str, client: &str) -> Self {
        let m = self.matcher("allOf");
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}agent> <{agent}> <{}> .",
            self.graph
        );
        let _ = writeln!(
            self.quads,
            "<{m}> <{ACP}client> <{client}> <{}> .",
            self.graph
        );
        self
    }
}

/// One conformance scenario: a named, self-contained ACR corpus plus the table of
/// expected `(agent, client, mode, resource) → decision` decisions the ACP spec
/// requires for it.
///
/// Build with [`AcpScenario::new`], attach the ACR corpus with [`AcpScenario::acr`]
/// (or raw N-Quads with [`AcpScenario::nquads`]), add expectations with
/// [`AcpScenario::expect`], then [`AcpScenario::run`] it through the real ACP engine.
#[derive(Debug, Clone)]
pub struct AcpScenario {
    name: String,
    nquads: String,
    expects: Vec<Expect>,
}

impl AcpScenario {
    /// A new, empty scenario with the given `name` (used in the failure report).
    pub fn new(name: &str) -> AcpScenario {
        AcpScenario {
            name: name.to_owned(),
            nquads: String::new(),
            expects: Vec::new(),
        }
    }

    /// Attach an [`AcrBuilder`]'s ACR corpus to this scenario (appended to any N-Quads
    /// already present).
    pub fn acr(mut self, acr: AcrBuilder) -> Self {
        self.nquads.push_str(&acr.into_nquads());
        self
    }

    /// Append raw N-Quads (e.g. extra document content) to the scenario corpus.
    pub fn nquads(mut self, nquads: &str) -> Self {
        self.nquads.push_str(nquads);
        self.nquads.push('\n');
        self
    }

    /// Add one expected decision to the scenario's table.
    pub fn expect(mut self, e: Expect) -> Self {
        self.expects.push(e);
        self
    }

    /// Add many expected decisions at once.
    pub fn expect_all(mut self, es: impl IntoIterator<Item = Expect>) -> Self {
        self.expects.extend(es);
        self
    }

    /// The scenario's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The scenario's accumulated N-Quads corpus (its `.acr` graphs + document
    /// placeholders). sq-t58w.7 — exposed read-only so a SECOND consumer (the
    /// differential oracle, `tests/differential_oracle.rs`) can feed the IDENTICAL RDF to
    /// an **independent** procedural ACP evaluator without re-emitting it. This is the raw
    /// input the engine's [`AcpScenario::run`] loads via `Graph::load_dataset`; a
    /// different-paradigm reference reader parses the same bytes by hand.
    pub fn nquads_str(&self) -> &str {
        &self.nquads
    }

    /// The scenario's expected `(agent, client, mode, resource) → decision` table.
    /// sq-t58w.7 — exposed read-only so the differential oracle can use the hand
    /// table as a THIRD decider alongside the engine and the reference evaluator.
    pub fn expects(&self) -> &[Expect] {
        &self.expects
    }

    /// Materialize the scenario's ACP corpus and check every expected decision against
    /// the engine's verdict, returning a [`ScenarioReport`].
    ///
    /// # Errors
    ///
    /// Returns `Err` only if the corpus fails to load or [`materialize_acp`] errors
    /// (e.g. a reserved-encoding collision). A *wrong decision* is NOT an error — it is
    /// a recorded mismatch in the returned report (so a whole corpus can be run and all
    /// mismatches reported at once).
    pub fn run(&self) -> Result<ScenarioReport, String> {
        let mut graph = Graph::load_dataset(&self.nquads, "nquads")
            .map_err(|e| format!("scenario {:?}: load failed: {e}", self.name))?;
        materialize_acp(&mut graph)
            .map_err(|e| format!("scenario {:?}: materialize_acp failed: {e}", self.name))?;
        let index = AuthIndex::from_graph(&graph);
        ScenarioReport::build(&self.name, &self.expects, |e| {
            let session = Session {
                agent: e.req_agent(),
                client: e.req_client(),
                // sq-3jtd.9: this harness exercises the agent/client matcher
                // dimensions; the issuer dimension (sq-3jtd.6, ACP `acp:issuer`) is left
                // unconstrained (`None` ⇒ AnyIssuer), so issuer-blind decision parity is
                // unaffected by the (agent, client, issuer) principal model.
                issuer: None,
                // sq-0q7n: no time-windowed grant in the ACP conformance
                // scenarios → the clock dimension is irrelevant here.
                now: None,
            };
            index
                .accessible(&session, e.req_mode())
                .iter()
                .any(|g| g.as_str() == e.req_resource())
        })
    }
}

/// The outcome of checking one expected decision: what was expected vs. what the engine
/// decided.
#[derive(Debug, Clone)]
pub struct DecisionResult {
    expect: Expect,
    actual: Decision,
}

impl DecisionResult {
    /// `true` iff the engine's decision matched the expectation.
    pub fn matched(&self) -> bool {
        self.actual == self.expect.decision
    }
}

/// The result of running one [`AcpScenario`]: the per-decision outcomes, with helpers to
/// assert overall parity.
#[derive(Debug, Clone)]
pub struct ScenarioReport {
    name: String,
    results: Vec<DecisionResult>,
}

impl ScenarioReport {
    /// Build a report by asking `granted` whether the engine grants each expectation's
    /// `(agent, client, mode, resource)` request, comparing it to the expected [`Decision`].
    ///
    /// sq-3jtd.8 — the shared comparison/reporting core for both the ACP harness
    /// ([`AcpScenario::run`]) and the WAC harness ([`crate::wac_conformance::WacScenario::run`]):
    /// only the *engine* differs (which auth view `granted` consults), not the
    /// decision-parity machinery. `granted` returns `true` when the resource is in the
    /// session's accessible set for the mode (⇒ [`Decision::Allow`]), `false` otherwise
    /// (⇒ [`Decision::Deny`]). It is `FnMut` so callers may consult a `&mut` engine handle
    /// (e.g. [`crate::PodStore`], whose `accessible` memoizes per session).
    ///
    /// # Errors
    ///
    /// Never — this is infallible (it records mismatches in the report rather than
    /// erroring). It returns `Result` only so callers can `?`-chain it after a fallible
    /// materialization step.
    pub fn build(
        name: &str,
        expects: &[Expect],
        mut granted: impl FnMut(&Expect) -> bool,
    ) -> Result<ScenarioReport, String> {
        let results = expects
            .iter()
            .map(|e| {
                let actual = if granted(e) {
                    Decision::Allow
                } else {
                    Decision::Deny
                };
                DecisionResult {
                    expect: e.clone(),
                    actual,
                }
            })
            .collect();
        Ok(ScenarioReport {
            name: name.to_owned(),
            results,
        })
    }

    /// `true` iff every expected decision was reproduced by the engine.
    pub fn passed(&self) -> bool {
        self.results.iter().all(DecisionResult::matched)
    }

    /// The decisions whose engine verdict did NOT match the expectation.
    pub fn mismatches(&self) -> impl Iterator<Item = &DecisionResult> {
        self.results.iter().filter(|r| !r.matched())
    }

    /// The number of expected decisions checked.
    pub fn checked(&self) -> usize {
        self.results.len()
    }

    /// The scenario name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ScenarioReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mismatches = self.results.iter().filter(|r| !r.matched()).count();
        writeln!(
            f,
            "scenario {:?}: {}/{} decisions matched ({mismatches} mismatch(es))",
            self.name,
            self.checked() - mismatches,
            self.checked(),
        )?;
        for r in self.results.iter().filter(|r| !r.matched()) {
            let who = match (&r.expect.agent, &r.expect.client) {
                (Some(a), Some(c)) => format!("(agent {a}, client {c})"),
                (Some(a), None) => format!("agent {a}"),
                (None, _) => "anonymous".to_owned(),
            };
            writeln!(
                f,
                "  MISMATCH: {who} {:?} {} => expected {}, engine decided {}",
                r.expect.mode, r.expect.resource, r.expect.decision, r.actual,
            )?;
        }
        Ok(())
    }
}

/// Run a whole corpus of scenarios, returning the per-scenario reports. A convenience
/// for `tests/conformance_acp.rs`: every scenario is run even if an earlier one mismatches,
/// so a single test surfaces all mismatches at once.
///
/// # Errors
///
/// Returns `Err` if any scenario fails to materialize (load / reasoner error). Decision
/// mismatches are not errors — they are in the returned reports.
pub fn run_corpus(scenarios: &[AcpScenario]) -> Result<Vec<ScenarioReport>, String> {
    scenarios.iter().map(AcpScenario::run).collect()
}
