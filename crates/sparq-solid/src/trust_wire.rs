//! Trust-graph admission → `<urn:sparq:auth>` wiring (the `trust-graph` feature).
//!
//! This is the ONE edge `sparq-solid` gains when the default-OFF `trust-graph`
//! feature is enabled (design `research/solid-trust-graph-authz-design.md` §6.0; epic
//! `sq-pfae`, issue #940). With the feature OFF this module is not compiled at all, so
//! `sparq-solid` behaves **byte-identically** to WAC/ACP today — the strict-additivity
//! property (G6, §2.2).
//!
//! With the feature on, [`PodStore::admit_trust_credential_and_materialize`] runs the
//! `sparq-trust` admission gate over a presented credential, derives the `auth:*`
//! grants by merging the admitted (issuer-tagged) facts with the controller-authored
//! `.acr` ABAC rule via the shipped N3 reasoner, and installs those grants into
//! `<urn:sparq:auth>` **on top of** the unchanged WAC/ACP view — exactly the
//! ODRL-bridge grant-injection precedent. Everything downstream (the session-scoped
//! `∪ allow ∖ ∪ deny` enforcement, the query rewrite) is the existing shipped code.
//!
//! ## Two install paths — the `sq-xc4y` static / dynamic split (read first)
//!
//! Admission has a session-INDEPENDENT (STATIC) class and a per-request (DYNAMIC)
//! class. There are TWO install methods, and choosing the wrong one is a soundness bug:
//!
//! - [`PodStore::admit_trust_credential_static`] is the **materialise-time** path. It
//!   runs ONLY the static class (`sparq_trust::admit_static`) and installs each derived
//!   grant as a **conditional grant** (`auth:ConditionalGrant`) whose holder
//!   (`auth:agent`) and freshness (`auth:notAfter`) are re-checked **per request** by
//!   the shipped sq-0q7n `AuthIndex::cond_applies` path. This composes with the
//!   materialise-once epoch cache: the static decision is frozen, the DYNAMIC
//!   holder/freshness verdict is NOT — a stale or wrong-holder credential is denied at
//!   query time without a re-materialise. **Use this for the materialise-once view.**
//! - [`PodStore::admit_trust_credential_and_materialize`] (and `_with_rule`) is the
//!   **single-request snapshot** path. It runs the COMBINED gate against one live
//!   `Session` and installs an UNCONDITIONAL grant valid for *that* request only.
//!   Calling it once freezes a per-request decision into the view — so it MUST be
//!   re-run per request, or used only for an ephemeral, request-scoped view. **Do NOT
//!   use it to populate a long-lived materialise-once view.**
//!
//! ## Honest scope (load-bearing — RESEARCH, NOT a security guarantee)
//!
//! The gate verifies a CHECKED issuer signature, but it does **NOT** deliver privacy,
//! unlinkability, or anonymity: the credential is admitted in the clear and the holder
//! binding authenticates the WebID in the clear (the non-anonymous degraded path,
//! `sq-wvne` / `sq-xc4y`). The ZK estate it composes with is externally **UNAUDITED**
//! (`sq-qhy4`). Full scope: the `sparq-trust` crate docs + README.
//!
//! [OPUS-4.8] sq-pfae PoC. 🤖 SPARQ agent — trust-graph authorisation PoC.

use crate::authindex::{ANY_CLIENT, ANY_ISSUER};
use crate::{PodStore, AUTH_GRAPH, AUTH_NS};
use oxrdf::{Literal, NamedNode, Term, Triple};
use sparq_core::Graph;
use sparq_trust::admit::{
    admit, admit_static, AdmittedFact, PresentedCredential, Session as TrustSession,
};
use sparq_trust::policy::TrustRule;
use sparq_trust::wire::{derive_conditional_grants, derive_grants, ConditionalGrant};

/// What the trust-graph admission pass decided and installed.
#[derive(Debug, Clone, Default)]
pub struct TrustAdmissionOutcome {
    /// The issuer-tagged facts that passed the admission gate (possibly empty —
    /// default-deny). Each is marked `trust:admitted` in the design's terms.
    pub admitted: Vec<AdmittedFact>,
    /// The `auth:*` grant triples derived by merging the admitted facts with the
    /// `.acr` ABAC rule and installed into `<urn:sparq:auth>` (possibly empty).
    pub installed_grants: Vec<Triple>,
}

/// What the **static** (materialise-time) trust-graph admission pass decided and
/// installed (the `sq-xc4y` static/dynamic split — see
/// [`PodStore::admit_trust_credential_static`]).
#[derive(Debug, Clone, Default)]
pub struct TrustStaticOutcome {
    /// The conditional grants derived from the statically-admitted facts. Each carries
    /// the bound holder + the freshness deadline as DYNAMIC conditions re-checked per
    /// request (NOT a frozen unconditional allow).
    pub conditional_grants: Vec<ConditionalGrant>,
    /// The number of distinct `auth:ConditionalGrant` head-triple sets installed into
    /// `<urn:sparq:auth>` (one per derived conditional grant).
    pub installed_count: usize,
}

impl PodStore {
    /// Run the trust-graph admission gate over a presented `credential`, merge the
    /// admitted facts with the controller-authored `.acr` `abac_rule_n3` via the
    /// shipped N3 reasoner, and install the derived `auth:*` grants into
    /// `<urn:sparq:auth>` on top of the current (WAC/ACP) view, then reindex so the
    /// grant takes effect on the next [`PodStore::accessible`] / [`PodStore::query_as`].
    ///
    /// Call a static `materialize_*` FIRST if you also want the WAC/ACP grants present;
    /// this adds to whatever auth view currently exists. The grants are **appended** —
    /// a trust-graph grant can never drop a WAC/ACP grant (it only unions allows).
    ///
    /// `rules` is the parsed, **Control-gated** trust policy (see
    /// [`sparq_trust::parse_policy`]); `session` is the **per-request** requester
    /// context (agent + `now`); `target` is the requested resource (the `trust:scope`
    /// check). This is a **per-request** decision and MUST be re-run per request for a
    /// credential-gated resource (`sq-xc4y`) — it is NOT a materialise-once operation.
    ///
    /// # Honest scope
    ///
    /// RESEARCH prototype, NOT a security guarantee. No privacy / unlinkability /
    /// anonymity; the holder binding is clear-WebID (`sq-wvne`); the ZK estate is
    /// externally unaudited (`sq-qhy4`). See the module + `sparq-trust` docs.
    ///
    /// # Errors
    ///
    /// Returns `Err` only if the N3 merge of the admitted facts with `abac_rule_n3`
    /// fails to reason (a malformed rule). Admission itself never errors: a credential
    /// that fails any gate simply admits nothing (fail-closed, empty outcome).
    pub fn admit_trust_credential_and_materialize(
        &mut self,
        credential: &PresentedCredential,
        rules: &[TrustRule],
        session: &TrustSession,
        target: &NamedNode,
    ) -> Result<TrustAdmissionOutcome, String> {
        let admitted = admit(credential, rules, session, target);
        let mut grants = derive_grants(&admitted, abac_rule_n3_for(self))?;
        // A dropped view is not rebuilt by trust grants (see `PodStore::grant_view`).
        match self.grant_view() {
            Some(view) if !grants.is_empty() => {
                install_auth_grants(view, &grants);
                self.reindex_with(crate::ReindexScope::Full);
            }
            Some(_) => {}
            None => grants.clear(),
        }
        Ok(TrustAdmissionOutcome {
            admitted,
            installed_grants: grants,
        })
    }

    /// As [`PodStore::admit_trust_credential_and_materialize`] but with an explicit
    /// `abac_rule_n3` (the controller-authored `.acr` ABAC rule, e.g.
    /// `{ ?x schema:age ?y . ?y math:greaterThan 18 } => { ?x auth:read <r> }`),
    /// rather than reading it from the store. This is the v1 PoC entry point: the rule
    /// is the trusted root carried in the Control-gated `.acr` channel.
    ///
    /// **Snapshot path (`sq-xc4y`).** Like `admit_trust_credential_and_materialize`,
    /// this installs an UNCONDITIONAL grant valid for the supplied `session` only. To
    /// populate a long-lived materialise-once view, use
    /// [`PodStore::admit_trust_credential_static`] instead.
    pub fn admit_trust_credential_with_rule(
        &mut self,
        credential: &PresentedCredential,
        rules: &[TrustRule],
        session: &TrustSession,
        target: &NamedNode,
        abac_rule_n3: &str,
    ) -> Result<TrustAdmissionOutcome, String> {
        let admitted = admit(credential, rules, session, target);
        let mut grants = derive_grants(&admitted, abac_rule_n3)?;
        // A dropped view is not rebuilt by trust grants (see `PodStore::grant_view`).
        match self.grant_view() {
            Some(view) if !grants.is_empty() => {
                install_auth_grants(view, &grants);
                self.reindex_with(crate::ReindexScope::Full);
            }
            Some(_) => {}
            None => grants.clear(),
        }
        Ok(TrustAdmissionOutcome {
            admitted,
            installed_grants: grants,
        })
    }

    /// Run the **STATIC** trust-graph admission pass (the materialise-time half of the
    /// `sq-xc4y` split) over a presented `credential`, derive the `auth:*` grants by
    /// merging the statically-admitted facts with the controller-authored `.acr`
    /// `abac_rule_n3`, and install each as a **conditional grant**
    /// (`auth:ConditionalGrant`) into `<urn:sparq:auth>`.
    ///
    /// Unlike [`PodStore::admit_trust_credential_with_rule`], this takes **no
    /// `Session`**: it decides only the session-INDEPENDENT class (signature,
    /// statement-type scope, reserved-predicate guard, `trust:scope`) and DEFERS the
    /// per-request holder binding + freshness to the installed conditional grant. The
    /// grant's `auth:agent` is the credential subject (re-checked against
    /// `Session.agent` per request) and its `auth:notAfter` is `issued_at + freshWithin`
    /// rendered as an `xsd:dateTime` (re-checked against `Session.now` per request) — the
    /// shipped sq-0q7n path. So a request as the wrong holder, or after the credential
    /// lapses, is denied at **query time** without a re-materialise, and the dynamic
    /// decision is **never frozen** into the materialise-once view. This is the install
    /// path to use for a long-lived view.
    ///
    /// The grants are **appended** on top of the current (WAC/ACP) view — a trust-graph
    /// grant can never drop a WAC/ACP grant.
    ///
    /// # Honest scope
    ///
    /// RESEARCH prototype, NOT a security guarantee. No privacy / unlinkability /
    /// anonymity; the holder binding is clear-WebID (`sq-wvne`); the ZK estate is
    /// externally unaudited (`sq-qhy4`). See the module + `sparq-trust` docs.
    ///
    /// # Errors
    ///
    /// Returns `Err` only if the N3 merge of the statically-admitted facts with
    /// `abac_rule_n3` fails to reason (a malformed rule). Admission itself never errors:
    /// a credential that fails any static gate simply admits nothing (fail-closed).
    pub fn admit_trust_credential_static(
        &mut self,
        credential: &PresentedCredential,
        rules: &[TrustRule],
        target: &NamedNode,
        abac_rule_n3: &str,
    ) -> Result<TrustStaticOutcome, String> {
        let statics = admit_static(credential, rules, target);
        let conditional_grants = derive_conditional_grants(&statics, abac_rule_n3)?;
        let mut installed_count = 0usize;
        // A dropped view is not rebuilt by trust grants (see `PodStore::grant_view`).
        if let Some(view) = self.grant_view() {
            for cg in &conditional_grants {
                install_conditional_grant(view, target, cg);
                installed_count += 1;
            }
        }
        if installed_count > 0 {
            self.reindex_with(crate::ReindexScope::Full);
        }
        Ok(TrustStaticOutcome {
            conditional_grants,
            installed_count,
        })
    }
}

/// v1 PoC: there is no in-store ABAC-rule discovery yet (that is P4 — trust-document
/// storage). The default rule-less merge derives nothing; callers pass the `.acr`
/// rule explicitly via [`PodStore::admit_trust_credential_with_rule`]. Kept as a seam
/// so a future P4 can resolve the rule from the `.acr` graph.
fn abac_rule_n3_for(_store: &PodStore) -> &'static str {
    ""
}

/// Append `auth:*` grant triples into `<urn:sparq:auth>` (creating it if absent),
/// de-duplicated — the same install discipline the ODRL bridge uses, kept local to
/// the `trust-graph` feature so it composes with the unchanged materialiser without
/// touching the ODRL path.
fn install_auth_grants(graph: &mut Graph, grants: &[Triple]) {
    let g_name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
    let mut terms: Vec<[Term; 3]> = match graph.named.iter().find(|(n, _)| *n == g_name) {
        Some((_, sub)) => crate::loader::graph_triples(sub),
        None => Vec::new(),
    };
    for t in grants {
        let subj = match &t.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) => Term::NamedNode(n.clone()),
            oxrdf::NamedOrBlankNode::BlankNode(b) => Term::BlankNode(b.clone()),
        };
        let triple = [subj, Term::NamedNode(t.predicate.clone()), t.object.clone()];
        if !terms.contains(&triple) {
            terms.push(triple);
        }
    }
    write_back_auth_terms(graph, &g_name, terms);
}

/// Install one `auth:ConditionalGrant` for a derived trust-graph grant, with its DYNAMIC
/// conditions (holder + freshness) carried as data the shipped sq-0q7n
/// `AuthIndex::cond_applies` path re-checks **per request** — the materialise-time half
/// of the `sq-xc4y` split. The head is the SAME shape the ODRL bridge emits
/// (`append_conditional_grants`), so it is honoured by the unchanged session-scoped
/// evaluator:
///
/// - `auth:effect auth:Allow` — a conditional allow;
/// - `auth:agent <holder>` — the bound holder, re-checked against `Session.agent`
///   per request (the deferred DYNAMIC holder binding; a different requester never
///   matches);
/// - `auth:client auth:AnyClient` / `auth:issuer auth:AnyIssuer` — a trust-graph grant
///   carries no client/issuer constraint (it spans those dimensions' tops, stated
///   explicitly so the fail-closed head convention applies);
/// - `auth:mode <acl#…>` + `auth:graph <target>` — the access this grant confers;
/// - `auth:notAfter "<issued_at+freshWithin>"^^xsd:dateTime` — the freshness deadline,
///   re-checked against `Session.now` per request (the deferred DYNAMIC freshness check;
///   a lapsed credential denies *that request* without a re-materialise).
fn install_conditional_grant(graph: &mut Graph, target: &NamedNode, cg: &ConditionalGrant) {
    let Some(mode_iri) = mode_iri_for_auth_pred(cg.grant.predicate.as_str()) else {
        return; // not an auth:read|write|append|control grant → nothing to install
    };
    let g_name = Term::NamedNode(NamedNode::new_unchecked(AUTH_GRAPH));
    let mut terms: Vec<[Term; 3]> = match graph.named.iter().find(|(n, _)| *n == g_name) {
        Some((_, sub)) => crate::loader::graph_triples(sub),
        None => Vec::new(),
    };

    let not_after = unix_to_xsd_datetime(cg.not_after_unix_secs);
    // A deterministic grant IRI keyed on (holder, mode, graph, notAfter) so re-running
    // static admission for the same credential is idempotent (no duplicate heads) and a
    // different freshness deadline does not silently overwrite an earlier one.
    let grant_iri = format!(
        "urn:sparq:trust-cond?agent={}&mode={}&graph={}&notAfter={}",
        sparq_reason::n3::encode_for_uri(cg.holder.as_str()),
        sparq_reason::n3::encode_for_uri(mode_iri),
        sparq_reason::n3::encode_for_uri(target.as_str()),
        sparq_reason::n3::encode_for_uri(&not_after),
    );
    let g = Term::NamedNode(NamedNode::new_unchecked(&grant_iri));
    let xsd_datetime = NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#dateTime");
    let head: Vec<[Term; 3]> = vec![
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            )),
            Term::NamedNode(NamedNode::new_unchecked(format!(
                "{AUTH_NS}ConditionalGrant"
            ))),
        ],
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}effect"))),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}Allow"))),
        ],
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}agent"))),
            Term::NamedNode(cg.holder.clone()),
        ],
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}client"))),
            Term::NamedNode(NamedNode::new_unchecked(ANY_CLIENT)),
        ],
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}issuer"))),
            Term::NamedNode(NamedNode::new_unchecked(ANY_ISSUER)),
        ],
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}mode"))),
            Term::NamedNode(NamedNode::new_unchecked(mode_iri)),
        ],
        [
            g.clone(),
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}graph"))),
            Term::NamedNode(target.clone()),
        ],
        [
            g,
            Term::NamedNode(NamedNode::new_unchecked(format!("{AUTH_NS}notAfter"))),
            Term::Literal(Literal::new_typed_literal(not_after, xsd_datetime)),
        ],
    ];
    for t in head {
        if !terms.contains(&t) {
            terms.push(t);
        }
    }
    write_back_auth_terms(graph, &g_name, terms);
}

/// Re-intern the `<urn:sparq:auth>` sub-graph from a flat term list and write it back
/// (mirrors `install_auth_view`). Shared by the unconditional + conditional installers.
fn write_back_auth_terms(graph: &mut Graph, g_name: &Term, terms: Vec<[Term; 3]>) {
    let mut dict = sparq_core::dict::Dict::new();
    let ids: Vec<[sparq_core::dict::Id; 3]> = terms
        .iter()
        .map(|t| [dict.intern(&t[0]), dict.intern(&t[1]), dict.intern(&t[2])])
        .collect();
    let sub = Graph::from_parts(dict, ids);
    if let Some(slot) = graph.named.iter_mut().find(|(n, _)| n == g_name) {
        slot.1 = sub;
    } else {
        graph.named.push((g_name.clone(), sub));
    }
}

/// Map a derived `auth:read|write|append|control` predicate IRI to its `acl:` mode IRI
/// (the `auth:mode` object on a conditional grant). Returns `None` for any non-mode
/// `auth:` predicate (fail-closed: nothing installed).
fn mode_iri_for_auth_pred(pred: &str) -> Option<&'static str> {
    match pred.strip_prefix(AUTH_NS)? {
        "read" => Some("http://www.w3.org/ns/auth/acl#Read"),
        "write" => Some("http://www.w3.org/ns/auth/acl#Write"),
        "append" => Some("http://www.w3.org/ns/auth/acl#Append"),
        "control" => Some("http://www.w3.org/ns/auth/acl#Control"),
        _ => None,
    }
}

/// Epoch seconds (UTC) → an `xsd:dateTime` lexical `YYYY-MM-DDTHH:MM:SSZ`, dependency-
/// free (the workspace carries no `chrono`/`time`). The civil-date conversion is Howard
/// Hinnant's algorithm (the inverse of `days_from_civil`, the same one the sq-0q7n
/// `AuthIndex` `xsd:dateTime` parser uses), so a formatted lexical parses back to the
/// same instant the `cond_applies` window compares against. [OPUS-4.8] sq-xc4y.
fn unix_to_xsd_datetime(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let tod = epoch_secs.rem_euclid(86_400); // 0..86400, always non-negative
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, mi, s)
}

/// Days-from-epoch (1970-01-01 = 0) → `(year, month, day)` (Howard Hinnant).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}
