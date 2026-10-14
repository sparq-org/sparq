//! sq-3jtd.8 — a library-level **WAC conformance harness**.
//!
//! # What this is (and the decision behind it)
//!
//! The conformance surface targeted here is the **Solid Web Access Control (WAC)
//! specification** — <https://solidproject.org/TR/wac> — exercised at the *library*
//! level: a scenario is an `.acl`-document corpus plus a table of expected
//! `(agent, client, mode, resource) → allow | deny` decisions, and the harness asserts
//! that this crate's WAC authorization engine
//! ([`crate::materialize_wac`] + [`crate::AuthIndex::accessible`]) reproduces every
//! expected decision (`research/sparq-solid-scope.md` §4).
//!
//! This is the **realistic, achievable** conformance signal for an authorization
//! *oracle*. Two routes were considered and deliberately NOT taken here (identical
//! reasoning to the ACP harness in [`crate::conformance`] — kept in sync):
//!
//! - **CTH-over-HTTP** (the Solid Conformance Test Harness / `solid/specification-tests`)
//!   drives a *server* and asserts on HTTP outputs (status codes, the `WAC-Allow`
//!   header). This crate has no HTTP surface — that route is out-of-scope (it belongs
//!   to the Solid server, conformance-tested *through* the server), per the scope record.
//! - **A differential oracle against a JS reference evaluator** (Community Solid Server —
//!   same corpus, diff decisions) is the credible *second* oracle, but is research-open
//!   (JS-toolchain cost) and tracked as a follow-up — it is NOT built here.
//!
//! Instead, scenarios are declared as **data** (this module is a builder + runner), and
//! the expected-decision table is derived from the WAC spec's normative semantics:
//! ACL resolution (`acl:accessTo` on the resource's own ACL vs. `acl:default` on the
//! **nearest** ancestor with an ACL — nearest-wins, NOT cumulative), access subjects
//! (`acl:agent` / `acl:agentClass` `foaf:Agent` (public) / `acl:AuthenticatedAgent` /
//! `acl:agentGroup` via `vcard:hasMember` / `acl:origin` minting an (agent, client) pair),
//! and access modes (`acl:Read`/`Write`/`Append`/`Control`, where `Control` governs the
//! **ACL document** of the resource, not the resource). The corpus that consumes this
//! harness lives in `tests/conformance_wac.rs`.
//!
//! # Relationship to the ACP harness and to `tests/wac.rs`
//!
//! The decision/expectation/report vocabulary ([`Decision`](crate::conformance::Decision),
//! [`Expect`], [`ScenarioReport`]) is **shared** with the ACP harness
//! ([`crate::conformance`]) — both compare the same
//! `(agent, client, mode, resource) → allow | deny` binary against the same
//! [`AuthIndex::accessible`] verdict; only the *corpus shape* differs (`.acl`
//! authorizations here, `.acr` policies there). This module adds the WAC-specific corpus
//! builder ([`AclBuilder`]) and runner ([`WacScenario`]).
//!
//! `tests/wac.rs` asserts a hand-derived access *matrix* over the one big synthetic
//! `wac_fixture()` pod — bespoke, internally authored, depth-4. This harness is
//! structurally different: it is **table-driven over independent, minimal scenarios**,
//! each isolating one WAC construct, with a uniform pass/fail report. The two are
//! complementary: the matrix exercises depth and interaction on a realistic pod; this
//! harness exercises spec-construct coverage with small, auditable, independently-failing
//! cases (the natural input for the aspirational CSS differential oracle).
//!
//! # Example
//!
//! ```
//! use sparq_solid::wac_conformance::{AclBuilder, WacScenario};
//! use sparq_solid::conformance::{Decision, Expect};
//! use sparq_solid::Mode;
//!
//! let alice = "https://alice.example/card#me";
//! let bob = "https://bob.example/card#me";
//! let doc = "https://pod.example/notes/n1";
//!
//! // An ACL on the document grants Read to exactly { alice } via `acl:accessTo`.
//! let mut acl = AclBuilder::new();
//! acl.access_to(doc, |a| a.agent(alice).mode(Mode::Read));
//! acl.document(doc); // the protected resource is a (possibly empty) named graph
//!
//! let scenario = WacScenario::new("agent-allow")
//!     .acl(acl)
//!     .expect(Expect::agent(alice).read(doc).is(Decision::Allow))
//!     .expect(Expect::agent(bob).read(doc).is(Decision::Deny))
//!     .expect(Expect::anonymous().read(doc).is(Decision::Deny));
//!
//! let report = scenario.run().expect("materializes");
//! assert!(report.passed(), "{report}"); // every expected decision reproduced
//! ```

use crate::conformance::{Expect, ScenarioReport};
use crate::{materialize_wac, AuthIndex, Mode, PodStore, Session};
use sparq_core::Graph;
use std::fmt::Write as _;

const ACL: &str = "http://www.w3.org/ns/auth/acl#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const FOAF_AGENT: &str = "http://xmlns.com/foaf/0.1/Agent";
const VCARD_MEMBER: &str = "http://www.w3.org/2006/vcard/ns#hasMember";
const EX_PRED: &str = "https://ex.dev/ns#prop";

/// The WAC `acl:AuthenticatedAgent` class IRI — the matcher that accepts any
/// *authenticated* requestor (an agent is present) but not the anonymous one.
pub const AUTHENTICATED_AGENT: &str = "http://www.w3.org/ns/auth/acl#AuthenticatedAgent";
/// The `foaf:Agent` class IRI — WAC's "public" matcher, accepting every requestor
/// including the anonymous one.
pub const PUBLIC_AGENT: &str = FOAF_AGENT;

/// A builder for one WAC ACL-document corpus, written as N-Quads (each `acl:Authorization`
/// into the `<{resource}.acl>` graph named by Solid's `R` → `R.acl` convention, the same
/// linkage [`crate::materialize_wac`]'s loader resolves).
///
/// An `acl:Authorization` binds the protected `resource` either via
/// [`AclBuilder::access_to`] (`acl:accessTo`, applies to the resource named by the ACL —
/// the resource's own ACL) or [`AclBuilder::default_for`] (`acl:default`, applies to the
/// resource's **members** by *inheritance from the nearest ancestor* — WAC inheritance is
/// **nearest-wins**, NOT cumulative: a resource is governed by exactly the ACL of the
/// closest ancestor (or itself) that has one, and a closer ACL fully shadows the rest).
/// Each authorization carries access subjects and modes declared by an [`AuthBuilder`]
/// closure.
///
/// To make a resource a concrete protected graph (so an `Allow` is observable as a
/// non-empty accessible set, and so the resource exists as an inheritance anchor),
/// register it with [`AclBuilder::document`]. Group documents for `acl:agentGroup` are
/// emitted automatically by [`AuthBuilder::agent_group`].
///
/// **Containment is by IRI structure**, not by an `ldp:contains` triple: this crate's WAC
/// engine derives ancestry from the Solid slash path (`rules/common.n3`), so an
/// `acl:default` on `https://pod.example/c/` reaches `https://pod.example/c/d` because the
/// latter is structurally under the former — provided the container `…/c/` is a known
/// resource (give it its own ACL, or register it with [`AclBuilder::document`]; a
/// structural prefix of any present graph is also recognised automatically).
#[derive(Debug, Default, Clone)]
pub struct AclBuilder {
    quads: String,
    auths: usize,
}

impl AclBuilder {
    /// A new, empty ACL builder.
    pub fn new() -> AclBuilder {
        AclBuilder::default()
    }

    /// Add an `acl:Authorization` with `acl:accessTo <resource>` — it applies to the
    /// resource itself (the resource's own ACL). The `build` closure declares the
    /// authorization's access subjects and modes.
    pub fn access_to(
        &mut self,
        resource: &str,
        build: impl FnOnce(AuthBuilder<'_>) -> AuthBuilder<'_>,
    ) -> &mut AclBuilder {
        self.add_auth(resource, true, false, build)
    }

    /// Add an `acl:Authorization` with `acl:default <resource>` — it applies to the
    /// resource's **members** (its IRI-structural descendants) by nearest-ancestor
    /// inheritance, but NOT to the resource itself.
    pub fn default_for(
        &mut self,
        resource: &str,
        build: impl FnOnce(AuthBuilder<'_>) -> AuthBuilder<'_>,
    ) -> &mut AclBuilder {
        self.add_auth(resource, false, true, build)
    }

    /// Add an `acl:Authorization` with BOTH `acl:accessTo` and `acl:default <resource>` —
    /// it applies to the resource itself AND its members (the common "owns this container
    /// and everything under it" shape).
    pub fn access_to_and_default(
        &mut self,
        resource: &str,
        build: impl FnOnce(AuthBuilder<'_>) -> AuthBuilder<'_>,
    ) -> &mut AclBuilder {
        self.add_auth(resource, true, true, build)
    }

    fn add_auth(
        &mut self,
        resource: &str,
        access_to: bool,
        default: bool,
        build: impl FnOnce(AuthBuilder<'_>) -> AuthBuilder<'_>,
    ) -> &mut AclBuilder {
        let g = format!("{resource}.acl");
        let ix = self.auths;
        self.auths += 1;
        let auth = format!("{g}#auth{ix}");
        let _ = writeln!(
            self.quads,
            "<{auth}> <{RDF_TYPE}> <{ACL}Authorization> <{g}> ."
        );
        if access_to {
            let _ = writeln!(self.quads, "<{auth}> <{ACL}accessTo> <{resource}> <{g}> .");
        }
        if default {
            let _ = writeln!(self.quads, "<{auth}> <{ACL}default> <{resource}> <{g}> .");
        }
        let ab = AuthBuilder {
            quads: &mut self.quads,
            graph: g,
            auth,
        };
        build(ab);
        self
    }

    /// Register `resource` as a concrete protected document: emit a placeholder triple
    /// into `<{resource}>` so that a granted access is observable as a non-empty
    /// accessible set (and so the resource is a known inheritance anchor).
    pub fn document(&mut self, resource: &str) -> &mut AclBuilder {
        let _ = writeln!(
            self.quads,
            "<{resource}#it> <{EX_PRED}> \"x\" <{resource}> ."
        );
        self
    }

    /// The accumulated N-Quads of this ACL corpus.
    pub fn into_nquads(self) -> String {
        self.quads
    }
}

/// A fluent builder for one WAC `acl:Authorization`: its access subjects
/// (`acl:agent` / `acl:agentClass` / `acl:agentGroup` / `acl:origin`) and its `acl:mode`s.
///
/// WAC authorization semantics (normative): an authorization that *applies to* a resource
/// (by `acl:accessTo` on its own ACL, or by `acl:default` on the resource's nearest
/// ancestor ACL) grants its `acl:mode`s to every requestor matched by ANY of its access
/// subjects. There is no deny in core WAC — access is the union of all applicable grants;
/// the nearest-ACL resolution is the only "narrowing" (a closer ACL replaces, rather than
/// adds to, the ancestor's). An `acl:origin` makes the grant a (user, application) pair:
/// the agent matches only through that client/origin.
#[derive(Debug)]
pub struct AuthBuilder<'a> {
    quads: &'a mut String,
    graph: String,
    auth: String,
}

impl AuthBuilder<'_> {
    fn mode_iri(mode: Mode) -> &'static str {
        match mode {
            Mode::Read => "Read",
            Mode::Write => "Write",
            Mode::Append => "Append",
            Mode::Control => "Control",
        }
    }

    /// Add an `acl:mode <mode>` to this authorization.
    pub fn mode(self, mode: Mode) -> Self {
        let m = Self::mode_iri(mode);
        let _ = writeln!(
            self.quads,
            "<{}> <{ACL}mode> <{ACL}{m}> <{}> .",
            self.auth, self.graph
        );
        self
    }

    /// Add an `acl:agent <webid>` subject — the named WebID is granted.
    pub fn agent(self, webid: &str) -> Self {
        let _ = writeln!(
            self.quads,
            "<{}> <{ACL}agent> <{webid}> <{}> .",
            self.auth, self.graph
        );
        self
    }

    /// Add an `acl:agentClass foaf:Agent` subject — the **public** class, granting every
    /// requestor including the anonymous one. (Convenience for `agent_class(PUBLIC_AGENT)`.)
    pub fn public(self) -> Self {
        self.agent_class(PUBLIC_AGENT)
    }

    /// Add an `acl:agentClass acl:AuthenticatedAgent` subject — granting any *authenticated*
    /// requestor but not the anonymous one. (Convenience for
    /// `agent_class(AUTHENTICATED_AGENT)`.)
    pub fn authenticated(self) -> Self {
        self.agent_class(AUTHENTICATED_AGENT)
    }

    /// Add an `acl:agentClass <class>` subject (use [`PUBLIC_AGENT`] / [`AUTHENTICATED_AGENT`];
    /// any other class is inert — the WAC rules only recognise those two).
    pub fn agent_class(self, class: &str) -> Self {
        let _ = writeln!(
            self.quads,
            "<{}> <{ACL}agentClass> <{class}> <{}> .",
            self.auth, self.graph
        );
        self
    }

    /// Add an `acl:agentGroup <group>` subject AND emit the group document
    /// (`<group> vcard:hasMember <member>` for each member into the fragment-stripped
    /// group-document graph), granting every group member. The WAC loader resolves the
    /// group document by stripping the fragment from the group IRI, so `group` should be
    /// a fragment IRI like `https://pod.example/groups/team#g`.
    pub fn agent_group(self, group: &str, members: &[&str]) -> Self {
        let _ = writeln!(
            self.quads,
            "<{}> <{ACL}agentGroup> <{group}> <{}> .",
            self.auth, self.graph
        );
        // The group document is the group IRI without its fragment (loader convention).
        let doc = group.split('#').next().unwrap_or(group);
        for m in members {
            let _ = writeln!(self.quads, "<{group}> <{VCARD_MEMBER}> <{m}> <{doc}> .");
        }
        self
    }

    /// Add an `acl:origin <client>` to this authorization — the grant becomes a
    /// (user, application) pair: a matched agent is granted only when the session presents
    /// this client/origin. (An authorization with an `acl:origin` and no `acl:agent` grants
    /// nobody — the agent half is required.)
    pub fn origin(self, client: &str) -> Self {
        let _ = writeln!(
            self.quads,
            "<{}> <{ACL}origin> <{client}> <{}> .",
            self.auth, self.graph
        );
        self
    }
}

/// One WAC conformance scenario: a named, self-contained `.acl` corpus plus the table of
/// expected `(agent, client, mode, resource) → decision` decisions the WAC spec requires
/// for it.
///
/// Build with [`WacScenario::new`], attach the ACL corpus with [`WacScenario::acl`] (or
/// raw N-Quads with [`WacScenario::nquads`]), add expectations with [`WacScenario::expect`]
/// (the shared [`Expect`] builder from [`crate::conformance`]), then [`WacScenario::run`]
/// it through the real WAC engine.
#[derive(Debug, Clone)]
pub struct WacScenario {
    name: String,
    nquads: String,
    expects: Vec<Expect>,
}

impl WacScenario {
    /// A new, empty scenario with the given `name` (used in the failure report).
    pub fn new(name: &str) -> WacScenario {
        WacScenario {
            name: name.to_owned(),
            nquads: String::new(),
            expects: Vec::new(),
        }
    }

    /// Attach an [`AclBuilder`]'s ACL corpus to this scenario (appended to any N-Quads
    /// already present).
    pub fn acl(mut self, acl: AclBuilder) -> Self {
        self.nquads.push_str(&acl.into_nquads());
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

    /// The scenario's accumulated N-Quads corpus (its `.acl` graphs + document
    /// placeholders). sq-t58w.7 — exposed read-only so a SECOND consumer (the
    /// differential oracle, `tests/differential_oracle.rs`) can feed the IDENTICAL RDF to
    /// an **independent** procedural WAC evaluator without re-emitting it. This is the raw
    /// input the engine's [`WacScenario::run`] loads via `Graph::load_dataset`; a
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

    /// Materialize the scenario's WAC corpus and check every expected decision against the
    /// engine's verdict, returning a [`ScenarioReport`].
    ///
    /// # Errors
    ///
    /// Returns `Err` only if the corpus fails to load or [`materialize_wac`] errors (e.g. a
    /// reserved-encoding collision). A *wrong decision* is NOT an error — it is a recorded
    /// mismatch in the returned report (so a whole corpus can be run and all mismatches
    /// reported at once).
    pub fn run(&self) -> Result<ScenarioReport, String> {
        let mut graph = Graph::load_dataset(&self.nquads, "nquads")
            .map_err(|e| format!("scenario {:?}: load failed: {e}", self.name))?;
        materialize_wac(&mut graph)
            .map_err(|e| format!("scenario {:?}: materialize_wac failed: {e}", self.name))?;
        let index = AuthIndex::from_graph(&graph);
        ScenarioReport::build(&self.name, &self.expects, |e| {
            // WAC has no issuer dimension; the session's issuer stays None (an absent
            // `acl:issuer` notion — see authindex::Session). sq-3jtd.8
            let session = Session {
                agent: e.req_agent(),
                client: e.req_client(),
                issuer: None,
                // sq-0q7n: WAC has no time-window grant; the clock is unused.
                now: None,
            };
            index
                .accessible(&session, e.req_mode())
                .iter()
                .any(|g| g.as_str() == e.req_resource())
        })
    }

    /// Materialize the scenario through a full [`PodStore`] (the method form of the engine,
    /// with the per-session cache) and check decision parity. Equivalent verdict to
    /// [`WacScenario::run`]; this form additionally proves the public `PodStore` surface
    /// reproduces the same decisions (the path a PSS integration actually uses).
    ///
    /// # Errors
    ///
    /// As [`WacScenario::run`].
    pub fn run_via_podstore(&self) -> Result<ScenarioReport, String> {
        let graph = Graph::load_dataset(&self.nquads, "nquads")
            .map_err(|e| format!("scenario {:?}: load failed: {e}", self.name))?;
        let mut store = PodStore::new(graph);
        store
            .materialize_wac()
            .map_err(|e| format!("scenario {:?}: materialize_wac failed: {e}", self.name))?;
        ScenarioReport::build(&self.name, &self.expects, |e| {
            let session = Session {
                agent: e.req_agent(),
                client: e.req_client(),
                issuer: None,
                // sq-0q7n: WAC has no time-window grant; the clock is unused.
                now: None,
            };
            store
                .accessible(&session, e.req_mode())
                .iter()
                .any(|g| g.as_str() == e.req_resource())
        })
    }
}

/// Run a whole corpus of WAC scenarios, returning the per-scenario reports. A convenience
/// for `tests/conformance_wac.rs`: every scenario is run even if an earlier one mismatches,
/// so a single test surfaces all mismatches at once.
///
/// # Errors
///
/// Returns `Err` if any scenario fails to materialize (load / reasoner error). Decision
/// mismatches are not errors — they are in the returned reports.
pub fn run_corpus(scenarios: &[WacScenario]) -> Result<Vec<ScenarioReport>, String> {
    scenarios.iter().map(WacScenario::run).collect()
}
