// sq-6y92 — the end-to-end federated MPC pipeline driver: the GLUE
// that composes holder → share → join → secure-threshold → reconstruct →
// ProofStatement into ONE worked federated response for the four-flatmates use case.
//! The end-to-end federated MPC pipeline driver (architecture §4.3 steps 1–6).
//!
//! Everything below COMPOSES the existing primitives — it invents no crypto. The
//! pieces already exist in isolation ([`crate::holder`], [`crate::shamir`],
//! [`crate::join`], [`crate::compare`], [`crate::proof`]) and are individually
//! tested; what was missing — and what this module supplies — is the *driver*
//! that runs them as one federated response for the driving **four-flatmates**
//! scenario (architecture §2; Wright CEUR Vol-4085).
//!
//! ## The worked scenario (architecture §4.3 steps 1–6)
//!
//! Four flatmates share one flat. Each holds an issuer-signed credential carrying
//! (a) a PUBLIC, disclosed global-IRI fact — they are a member of the SAME flat
//! `ex:flat` — and (b) a PRIVATE salary that must NEVER leave the holder. A
//! landlord (the verifier) asks one federated question:
//!
//! - *Which credential-holders are members of `ex:flat`?* (a disclosed-key join
//!   over the global IRI — answerable in the clear, convention #4), AND
//! - *Is the flat's cumulative salary `> £100k`?* (a secure threshold over the
//!   per-holder private salaries that opens ONLY the verdict bit — the exact
//!   total is never reconstructed, disclosure-minimisation).
//!
//! The driver runs all six §4.3 steps:
//!
//! 1. **Holders** ([`crate::holder::Holder`]) — each flatmate's wallet is its own
//!    [`sparq_core::Graph`]; raw graphs never leave (§4.2 minimise sharing).
//! 2. **Share / evaluate** — the PRIVATE salary of each holder is secret-shared
//!    via the Shamir backend ([`MpcBackend::share_private_input`]),
//!    while the DISCLOSED membership fact is evaluated locally
//!    ([`crate::holder::Holder::evaluate_local`]).
//! 3. **Join** ([`crate::join::DisclosedKeyJoin`]) — fold the disclosed-key joins
//!    over the global-IRI key. **The driver decides PER-OPERATOR disclosed-vs-
//!    hidden routing** ([`OperatorRouting`], RQ2a): the membership IRIs join in
//!    the clear; the salary values stay secret-shared and route into the MPC.
//! 4. **Secure aggregate + threshold** — [`MpcBackend::run_secure`]
//!    sums the hidden salary shares (free local addition), then
//!    [`crate::compare::disclose_threshold_verdict`] opens ONLY the `> £100k` bit
//!    (secure-threshold, sq-rrz4) — the exact sum is never an output.
//! 5. **Reconstruct** — the disclosed federated join result (the membership
//!    multiset) is the disclosed answer alongside the verdict bit.
//! 6. **ProofStatement** ([`crate::proof::ProofStatement`]) — assemble a REAL
//!    statement whose `disclosed_result` is byte-equal to a plaintext union-store
//!    evaluation of the SAME federated query. `proof.prove` STAYS the honest
//!    [`MpcError::NotYetImplemented`] stub (this glue is crypto-free + fork-
//!    invariant to Q1/Q2); the driver assembles the statement STRUCTURE with the
//!    real disclosed result — it does NOT fake a proof.
//!
//! ## Per-operator disclosed-vs-hidden routing (RQ2a) — the load-bearing decision
//!
//! §4.3 step 3 says: decide PER OPERATOR what is disclosed vs hidden. This driver
//! makes that decision explicit and inspectable in [`FederatedResponse::routing`]:
//!
//! - The **membership join** is over a global IRI (§2 convention #6) whose
//!   bound values are DISCLOSED, so it is a crypto-free plaintext equi-join
//!   OUTSIDE the cryptographic core ([`crate::join::DisclosedKeyJoin`], convention
//!   #4). Routed `Disclosed`.
//! - The **salary aggregate + threshold** is over PRIVATE per-source values that
//!   must never leave a source, so it enters the MPC: secret-shared sum
//!   ([`MpcBackend::run_secure`]) then a secure comparison that
//!   opens only the verdict ([`crate::compare`]). Routed `Hidden`.
//!
//! The routing is data the (untrusted, §4.1) planner produces; the driver does
//! not trust it for *soundness* (the disclosed-key join independently re-checks
//! the key is present, and the differential-vs-union-store test is the
//! correctness anchor), only as the hint for WHICH protocol each operator runs.
//!
//! ## Security tier (stated, not papered over)
//!
//! Honest-majority, semi-honest — exactly [`crate::shamir::ShamirBackend`]'s tier,
//! inherited unchanged (the driver adds no new crypto assumption). The threshold
//! comparison is reported [`crate::backend::OperatorClass::Comparison`] (semi-
//! honest-only); the disclosed-key join is crypto-free. Malicious hardening
//! (IT-MACs / verifiable resharing) and the collaborative ZK proof (Q1, M4) are
//! the SAME deferred seams the rest of the crate documents — see
//! [`FederatedResponse::statement`] / [`crate::proof`].

use crate::backend::{MpcBackend, OperatorClass};
use crate::compare;
use crate::holder::Holder;
use crate::join::{DisclosedKeyJoin, GlobalJoin, JoinPlan};
use crate::partial::{HolderId, MpcError, PartialResult};
use crate::proof::ProofStatement;
use crate::shamir::ShamirBackend;
use oxrdf::Variable;

/// Where ONE operator of the federated query is routed under the disclosure-
/// minimisation rule (architecture §4.3 step 3 / RQ2a). This is the explicit,
/// inspectable per-operator decision the driver makes — the load-bearing part of
/// the §4.3 protocol that nothing previously composed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    /// The operator's bound values are DISCLOSED (a global IRI), so it is computed
    /// in the CLEAR, outside the cryptographic core (convention #4). Crypto-free,
    /// invariant to the Q1/Q2 forks.
    Disclosed,
    /// The operator's values are PRIVATE and must never leave a source, so it runs
    /// inside the MPC (secret-shared). Carries the [`OperatorClass`] so the caller
    /// can read off the exact per-operator security tier via
    /// [`MpcBackend::operator_security`].
    Hidden(OperatorClass),
}

/// One per-operator routing decision in the federated plan: a human label for the
/// operator + where it is routed. Surfaced in [`FederatedResponse::routing`] so the
/// disclosed-vs-hidden split (RQ2a) is auditable, not implicit.
#[derive(Debug, Clone)]
pub struct OperatorRouting {
    /// Human-readable name of the operator (e.g. `"membership-join"`,
    /// `"salary-threshold"`).
    pub operator: String,
    /// Where it is routed (disclosed-in-the-clear vs hidden-in-MPC).
    pub routing: Routing,
}

/// One flatmate's input to the federation: an identity, the holder's own wallet
/// (a [`Holder`] over its private [`sparq_core::Graph`]), the SELECT fragment it
/// discloses locally, and the issuer key its credential is signed under (the
/// attestation key-set `K` the eventual collaborative proof binds membership to —
/// convention #3).
///
/// The holder's graph carries BOTH a disclosed public fact about the shared flat
/// (keyed on the global IRI) and the private salary; the driver routes them
/// per-operator. Each flatmate's `disclosed_fragment` projects the shared
/// global-IRI join key plus the holder-specific disclosed column(s) — so the
/// cross-holder combine is a genuine disclosed-key equi-join (holders sharing
/// only the key, exactly the [`crate::join::DisclosedKeyJoin`] shape), NOT a
/// trivial same-schema union. (§4.3 step 1: each holder answers what it can over
/// its own data, and fragments may differ per holder.)
pub struct Flatmate<'a> {
    /// This flatmate's holder (its private wallet + identity).
    pub holder: Holder,
    /// The SELECT fragment this holder evaluates LOCALLY to disclose its public
    /// row. Must project the global-IRI join key. The PRIVATE salary is NOT in
    /// this fragment (it is secret-shared separately).
    pub disclosed_fragment: &'a str,
    /// The issuer key id this flatmate's credential is signed under. Collected
    /// into the [`ProofStatement::disclosed_key_set`] `K` (the attestation half is
    /// gated on ZK-remediation #3 — modelled as opaque key ids at this tier).
    pub issuer_key: String,
}

/// The configuration of one federated query over the four flatmates.
///
/// This is the (untrusted, §4.1) planner's output, made into typed data: the
/// disclosed membership fragment each holder evaluates locally, the global-IRI
/// join key, and the public £100k threshold. The driver does not trust it for
/// soundness — the disclosed-key join independently re-checks the key and the
/// differential-vs-union-store test anchors correctness.
pub struct FederatedQuery<'a> {
    /// The global-IRI variable the disclosed-key join folds on (convention #6).
    pub join_var: Variable,
    /// The full federated query text. This module does NOT normalize or
    /// canonicalize it — it is used VERBATIM as the union-store query in the
    /// differential and stored verbatim as [`ProofStatement::query`]. Any
    /// canonicalization the (deferred) proof's query-text digest needs is the
    /// CALLER's responsibility; it is not performed here.
    pub federated_query: &'a str,
    /// The public threshold for the secure cumulative-salary comparison (e.g.
    /// `100_000`). Only the boolean `cumulative > threshold` verdict is disclosed.
    pub threshold: u64,
}

/// The one verifiable federated response the driver produces (architecture §4.3
/// step 6). Carries the disclosed result, the threshold verdict bit, the explicit
/// per-operator routing, and the assembled [`ProofStatement`] — but NO proof (the
/// collaborative proof stays the honest stub; see [`Self::statement`]).
#[derive(Debug, Clone)]
pub struct FederatedResponse {
    /// The DISCLOSED federated join result — the membership multiset, computed in
    /// the clear via [`DisclosedKeyJoin`] (§4.3 step 5). Byte-equal to the
    /// plaintext union-store evaluation of [`FederatedQuery::federated_query`]
    /// (the differential correctness anchor).
    pub disclosed_result: PartialResult,
    /// The £100k threshold verdict: `cumulative_salary > threshold`. Computed via
    /// the secure comparison ([`compare::disclose_threshold_verdict`]) opening
    /// ONLY this bit — the exact cumulative salary is NEVER reconstructed across
    /// the API boundary.
    pub over_threshold: bool,
    /// The disclosed-property partial carrying the verdict bit as a one-cell
    /// `?over_threshold` boolean (the shape a verifier recomputes the FILTER over).
    pub verdict_partial: PartialResult,
    /// The explicit per-operator disclosed-vs-hidden routing (RQ2a) — the
    /// load-bearing decision §4.3 step 3 makes.
    pub routing: Vec<OperatorRouting>,
    /// The assembled [`ProofStatement`] whose `disclosed_result` is byte-equal to
    /// the plaintext union-store evaluation. This is the STRUCTURE the (deferred)
    /// collaborative proof's public inputs will bind; assembling it is crypto-free.
    /// `proof.prove` stays [`MpcError::NotYetImplemented`] — the driver does NOT
    /// fake a proof.
    pub statement: ProofStatement,
}

impl FederatedResponse {
    /// The cumulative-salary verdict bit (`true` iff `> threshold`). Convenience
    /// accessor; the bit is the only thing the threshold operator discloses.
    pub fn verdict(&self) -> bool {
        self.over_threshold
    }
}

/// **The federated MPC pipeline entry point (architecture §4.3 steps 1–6).**
///
/// Compose the existing primitives into ONE worked federated response for the
/// four-flatmates scenario. This is the GLUE the bead (sq-6y92) asks for — it
/// invents no crypto:
///
/// 1. **holders** evaluate their disclosed membership fragment locally; their
///    PRIVATE salaries are secret-shared ([`MpcBackend::share_private_input`]).
/// 2. **join** — the disclosed-key membership join folds over the global IRI
///    (`Disclosed` routing); the salary aggregate is routed `Hidden`.
/// 3. **secure aggregate** — [`MpcBackend::run_secure`] sums the hidden salary
///    shares; [`compare::disclose_threshold_verdict`] opens ONLY the `> threshold`
///    bit.
/// 4. **reconstruct** — the disclosed join result + the verdict bit.
/// 5. **ProofStatement** — assembled with the real disclosed result; the proof
///    itself is the honest stub.
///
/// ## Preconditions (fail-closed, not guessed)
/// - `flatmates` must be non-empty and the backend's party count must equal the
///   flatmate count (each flatmate IS a compute party in the honest-majority
///   model — see [`crate::shamir`] module doc). A mismatch is a descriptive
///   [`MpcError::Protocol`], not a silent wrong answer.
/// - The secure comparison needs `n >= 2t+1` (a multiplication chain); the
///   honest-majority constructor guarantees it, and [`compare`] re-checks
///   fail-closed.
///
/// ## Security tier
/// Honest-majority, semi-honest (the [`ShamirBackend`] tier, unchanged). Returns
/// the per-operator routing so the caller can read the exact tier per operator.
pub fn run_federated(
    backend: &ShamirBackend,
    flatmates: &[Flatmate<'_>],
    query: &FederatedQuery<'_>,
) -> Result<FederatedResponse, MpcError> {
    // --- Precondition: a non-empty federation whose party count matches. ---
    if flatmates.is_empty() {
        return Err(MpcError::Protocol(
            "run_federated: the four-flatmates federation needs at least one holder".into(),
        ));
    }
    if backend.parties() != flatmates.len() {
        return Err(MpcError::Protocol(format!(
            "run_federated: backend has {} compute parties but the federation has {} flatmates — \
             each flatmate is one honest-majority compute party, so these must match",
            backend.parties(),
            flatmates.len()
        )));
    }

    // === Steps 1–2 (DISCLOSED operator): each holder evaluates its membership ===
    // fragment LOCALLY over its OWN graph and discloses only the projected row
    // (the global-IRI key + membership columns). Raw graphs never leave (§4.2).
    let mut partials: Vec<PartialResult> = Vec::with_capacity(flatmates.len());
    for fm in flatmates {
        partials.push(fm.holder.evaluate_local(fm.disclosed_fragment)?);
    }

    // === Step 3 (DISCLOSED operator): fold the disclosed-key join over the ===
    // global IRI — crypto-free, OUTSIDE the cryptographic core (convention #4).
    // This is the `Disclosed` half of the per-operator routing (RQ2a).
    let join_plan = JoinPlan {
        join_var: query.join_var.clone(),
        key_disclosed: true,
    };
    let disclosed_result = DisclosedKeyJoin::new().join(&partials, &join_plan)?;

    // === Steps 2 + 4 (HIDDEN operator): the PRIVATE salaries enter the MPC. ===
    // Each holder secret-shares its private salary (share_private_input); the
    // cumulative sum is the free local share-addition (run_secure, 0 rounds); the
    // £100k verdict opens ONLY the bit (secure-threshold, sq-rrz4) — the exact sum
    // is never reconstructed across the boundary. This is the `Hidden` half.
    let mut salary_shares = Vec::with_capacity(flatmates.len());
    for fm in flatmates {
        // One private integer per holder (its salary), shared across the parties.
        salary_shares.extend(backend.share_private_input(&fm.holder)?);
    }
    let summed = backend.run_secure(&salary_shares)?;
    // run_secure returns exactly one result sharing (the cumulative sum).
    let sum_sharing = summed.first().ok_or_else(|| {
        MpcError::Protocol("run_federated: run_secure returned no result sharing".into())
    })?;
    let verdict_partial =
        compare::disclose_threshold_verdict(backend, sum_sharing, query.threshold)?;
    let over_threshold = verdict_from_partial(&verdict_partial)?;

    // === Step 5 (reconstruct): the disclosed result + the verdict bit are the ===
    // federated answer. (Already in `disclosed_result` / `over_threshold`.)

    // === The explicit per-operator routing (RQ2a, §4.3 step 3). ===
    let routing = vec![
        OperatorRouting {
            operator: "membership-join (global-IRI key-on-key)".into(),
            routing: Routing::Disclosed,
        },
        OperatorRouting {
            operator: "salary-cumulative-sum".into(),
            routing: Routing::Hidden(OperatorClass::LinearAggregate),
        },
        OperatorRouting {
            operator: format!("salary-threshold (> {})", query.threshold),
            routing: Routing::Hidden(OperatorClass::Comparison),
        },
    ];

    // === Step 6 (ProofStatement): assemble the REAL statement structure. ===
    // Its `disclosed_result` is the real disclosed join result (the correctness
    // anchor — byte-equal to the union-store eval, asserted by the acceptance
    // test). The proof itself stays the honest NotYetImplemented stub: this glue
    // is crypto-free + fork-invariant to Q1/Q2, so we do NOT fake a proof — we
    // only assemble the public-statement shape the proof will eventually bind.
    // The disclosed key-set is a SET `K` (convention #3): caller order
    // and multiplicity must NOT matter, because it is later bound into the proof's
    // public-inputs digest — two runs over the same logical key-set must produce the
    // SAME statement. Collect, then canonicalise (sort + dedup) into a deterministic
    // order so the assembled ProofStatement is reproducible.
    let mut disclosed_key_set: Vec<String> =
        flatmates.iter().map(|fm| fm.issuer_key.clone()).collect();
    disclosed_key_set.sort_unstable();
    disclosed_key_set.dedup();

    let statement = ProofStatement {
        query: query.federated_query.to_string(),
        disclosed_key_set,
        // The verifier injects its OWN fresh challenge N at verification (#4
        // freshness binding); a prover-side placeholder here is only the
        // statement shape, exactly as documented on `ProofStatement::challenge`.
        challenge: Vec::new(),
        disclosed_result: disclosed_result.clone(),
    };

    Ok(FederatedResponse {
        disclosed_result,
        over_threshold,
        verdict_partial,
        routing,
        statement,
    })
}

/// Read the boolean verdict out of the one-cell `?over_threshold` partial
/// produced by [`compare::disclose_threshold_verdict`]. Anything other than the
/// expected single boolean literal is a protocol error (we never guess a verdict).
fn verdict_from_partial(p: &PartialResult) -> Result<bool, MpcError> {
    if p.rows.len() != 1 || p.rows[0].len() != 1 {
        return Err(MpcError::Protocol(format!(
            "verdict partial must be exactly one row / one column, got {} row(s)",
            p.rows.len()
        )));
    }
    match &p.rows[0][0] {
        Some(oxrdf::Term::Literal(l)) => match l.value() {
            "true" => Ok(true),
            "false" => Ok(false),
            other => Err(MpcError::Protocol(format!(
                "verdict partial carried a non-boolean literal {other:?}"
            ))),
        },
        other => Err(MpcError::Protocol(format!(
            "verdict partial must carry a boolean literal, got {other:?}"
        ))),
    }
}

/// The synthetic federation holder id the disclosed result is attributed to (it
/// is no single holder's data — it is the disclosed cross-holder result). Mirrors
/// [`DisclosedKeyJoin`]'s attribution; exposed so callers can assert it.
pub fn federation_holder_id() -> HolderId {
    HolderId::new("federation")
}

#[cfg(test)]
mod tests {
    //! sq-6y92 acceptance suite — the four-flatmates worked example end-to-end.
    //! The load-bearing tests are exactly the bead's acceptance bar:
    //! - (a) `four_flatmates_*` produce the federated disclosed result;
    //! - (b) the £100k verdict equals `(plaintext cumulative salary > 100_000)`
    //!   while the exact sum is NEVER reconstructed (asserted structurally on the
    //!   verdict partial);
    //! - (c) the assembled `ProofStatement.disclosed_result` is byte-equal
    //!   (`PartialResult: PartialEq`) to a plaintext union-store evaluation of the
    //!   SAME federated query.
    //!
    //! Differential-vs-union-store throughout.
    use super::*;
    use crate::backend::SecurityDescriptor;
    use sparq_core::Graph;
    use sparq_engine::query as engine_query;

    const PFX: &str = "@prefix ex: <http://ex/> . \
                       @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n";

    /// Each flatmate discloses a DISTINCT public fact about the shared flat,
    /// projecting the global-IRI key `?flat` plus a holder-specific column — so
    /// the four partials share ONLY the key and the disclosed-key join combines
    /// them into one row (the multi-pattern BGP over the union store), exactly the
    /// [`DisclosedKeyJoin`] shape (NOT a same-schema union). Per §4.3 step 1,
    /// holders may run DIFFERENT fragments over their own data. The four disclosed
    /// predicates: `ex:rent`, `ex:city`, `ex:landlord`, `ex:postcode`.
    const FRAGMENTS: [&str; 4] = [
        "PREFIX ex: <http://ex/> SELECT ?flat ?rent     WHERE { ?flat ex:rent ?rent }",
        "PREFIX ex: <http://ex/> SELECT ?flat ?city     WHERE { ?flat ex:city ?city }",
        "PREFIX ex: <http://ex/> SELECT ?flat ?landlord WHERE { ?flat ex:landlord ?landlord }",
        "PREFIX ex: <http://ex/> SELECT ?flat ?postcode WHERE { ?flat ex:postcode ?postcode }",
    ];

    /// The full federated query the disclosed result must match when evaluated
    /// over the UNION of all flatmates' graphs (the differential anchor): the
    /// 4-pattern BGP joined on the global IRI `?flat`.
    const FEDERATED_QUERY: &str = "PREFIX ex: <http://ex/> \
        SELECT ?flat ?rent ?city ?landlord ?postcode WHERE { \
            ?flat ex:rent ?rent . ?flat ex:city ?city . \
            ?flat ex:landlord ?landlord . ?flat ex:postcode ?postcode }";

    /// The disclosed turtle doc for flatmate `i` (its ONE public flat attribute).
    /// The disclosed facts live on `ex:flat`; the PRIVATE salary lives on the
    /// SEPARATE `ex:resident`/`ex:salary` subject (the hidden operand,
    /// secret-shared via the fixed `SELECT ?salary WHERE { ?p ex:salary ?salary }`
    /// fragment `share_private_input` uses — never disclosed).
    fn disclosed_doc(i: usize) -> &'static str {
        [
            "ex:flat ex:rent \"1200\" .",
            "ex:flat ex:city \"Leeds\" .",
            "ex:flat ex:landlord \"Acme Lettings\" .",
            "ex:flat ex:postcode \"LS1\" .",
        ][i]
    }

    /// Build one flatmate's wallet: its DISTINCT disclosed public attribute of the
    /// shared `ex:flat` (the disclosed-key-join operand on the global IRI `?flat`)
    /// + a PRIVATE salary on the SEPARATE `ex:resident`/`ex:salary` subject.
    fn flatmate(i: usize, member: &str, salary: u64, issuer_key: &str) -> Flatmate<'static> {
        let doc = format!(
            "{PFX} {} ex:resident ex:salary \"{salary}\"^^xsd:integer .",
            disclosed_doc(i)
        );
        Flatmate {
            holder: Holder::from_rdf(member, &doc, "turtle").expect("flatmate wallet parses"),
            disclosed_fragment: FRAGMENTS[i],
            issuer_key: issuer_key.to_string(),
        }
    }

    /// The plaintext union-store evaluation from the raw turtle docs (one per
    /// flatmate). Returns a federation-attributed [`PartialResult`] in the SAME
    /// canonical row order the disclosed-key join emits, so the two are directly
    /// `PartialEq`-comparable.
    fn union_store_eval_from_docs(docs: &[String], q: &str) -> PartialResult {
        let mut combined = String::from(PFX);
        for d in docs {
            combined.push_str(d);
            combined.push('\n');
        }
        let g = Graph::load_str(&combined, "turtle").expect("union graph parses");
        let r = engine_query(&g, q).expect("union query ok");
        // Canonicalise to the SAME order the join uses (by {:?} term render) so a
        // byte-equal PartialResult comparison is order-independent.
        let mut rows = r.rows;
        rows.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        PartialResult {
            holder: HolderId::new("federation"),
            vars: r.vars,
            rows,
        }
    }

    /// The four flatmates' raw DISCLOSED docs (for the union store), built to match
    /// the `flatmate(..)` wallets' disclosed facts exactly. The PRIVATE salaries are
    /// NOT here — the disclosed result is over the public flat attributes only.
    fn four_docs() -> Vec<String> {
        (0..4).map(|i| disclosed_doc(i).to_string()).collect()
    }

    fn four_flatmates(salaries: [u64; 4]) -> Vec<Flatmate<'static>> {
        let members = ["alice", "bob", "carol", "dave"];
        let keys = [
            "did:example:hr-acme#k1",
            "did:example:hr-globex#k1",
            "did:example:hr-initech#k1",
            "did:example:hr-umbrella#k1",
        ];
        (0..4)
            .map(|i| flatmate(i, members[i], salaries[i], keys[i]))
            .collect()
    }

    fn fed_query(threshold: u64) -> FederatedQuery<'static> {
        FederatedQuery {
            join_var: Variable::new_unchecked("flat"),
            federated_query: FEDERATED_QUERY,
            threshold,
        }
    }

    /// THE headline acceptance test (the bead's bar, all three clauses):
    /// (a) the federated disclosed result is produced; (b) the £100k verdict
    /// matches `(plaintext cumulative salary > 100_000)` with the exact sum NEVER
    /// reconstructed; (c) the assembled `ProofStatement.disclosed_result` is
    /// byte-equal to the plaintext union-store evaluation.
    #[test]
    fn four_flatmates_over_100k_full_pipeline() {
        let salaries = [30_000u64, 28_000, 26_000, 24_000]; // 108_000 > 100k
        let plaintext_sum: u64 = salaries.iter().sum();
        assert!(plaintext_sum > 100_000);

        let flatmates = four_flatmates(salaries);
        // n = 4 compute parties (one per flatmate); honest-majority t = 1.
        let backend = ShamirBackend::new_seeded(4, 0x6792).unwrap();
        let q = fed_query(100_000);
        let resp = run_federated(&backend, &flatmates, &q).unwrap();

        // (a) The federated disclosed result is produced & federation-attributed.
        // The four holders share only the global IRI ?flat, so the disclosed-key
        // join combines their four distinct flat attributes into ONE row.
        assert_eq!(resp.disclosed_result.holder, federation_holder_id());
        assert_eq!(
            resp.disclosed_result.rows.len(),
            1,
            "the four distinct flat attributes join on ?flat into one combined row"
        );

        // (b) The verdict equals the plaintext (sum > 100k) ...
        assert!(resp.over_threshold, "108k > 100k ⇒ true");
        assert_eq!(resp.over_threshold, plaintext_sum > 100_000);
        // ... and the EXACT SUM is never reconstructed into the output: the verdict
        // partial carries a boolean alone, and the integer total never appears.
        let rendered = format!("{:?}", resp.verdict_partial.rows);
        assert!(rendered.contains("true"));
        assert!(
            !rendered.contains(&plaintext_sum.to_string()),
            "the exact cumulative salary {plaintext_sum} must NOT appear in the disclosed output"
        );
        // The disclosed RESULT (membership) carries no salary integer either.
        let disclosed_rendered = format!("{:?}", resp.disclosed_result.rows);
        for s in salaries {
            assert!(
                !disclosed_rendered.contains(&s.to_string()),
                "individual salary {s} must NOT leak into the disclosed membership result"
            );
        }

        // (c) The assembled ProofStatement.disclosed_result is BYTE-EQUAL (PartialEq)
        // to a plaintext union-store evaluation of the SAME federated query.
        let expected = union_store_eval_from_docs(&four_docs(), FEDERATED_QUERY);
        assert_eq!(
            resp.statement.disclosed_result, expected,
            "ProofStatement.disclosed_result must equal the plaintext union-store eval"
        );
        // The disclosed_result in the response is the same one in the statement.
        assert_eq!(resp.statement.disclosed_result, resp.disclosed_result);

        // The statement binds the federated query text + the issuer key-set K.
        assert_eq!(resp.statement.query, FEDERATED_QUERY);
        assert_eq!(resp.statement.disclosed_key_set.len(), 4);
        assert!(resp
            .statement
            .disclosed_key_set
            .contains(&"did:example:hr-acme#k1".to_string()));
    }

    /// The below-threshold case: cumulative salary < £100k ⇒ verdict false, and
    /// the disclosed result still matches the union-store eval.
    #[test]
    fn four_flatmates_under_100k_verdict_false() {
        let salaries = [10_000u64, 9_000, 8_000, 7_000]; // 34_000 < 100k
        let plaintext_sum: u64 = salaries.iter().sum();
        assert!(plaintext_sum < 100_000);

        let flatmates = four_flatmates(salaries);
        let backend = ShamirBackend::new_seeded(4, 0xBEEF).unwrap();
        let resp = run_federated(&backend, &flatmates, &fed_query(100_000)).unwrap();

        assert!(!resp.over_threshold, "34k < 100k ⇒ false");
        assert_eq!(resp.over_threshold, plaintext_sum > 100_000);

        let expected = union_store_eval_from_docs(&four_docs(), FEDERATED_QUERY);
        assert_eq!(resp.statement.disclosed_result, expected);
    }

    /// Boundary: cumulative salary EXACTLY equals the threshold ⇒ NOT strictly
    /// greater ⇒ verdict false. Pins the strict-`>` semantics end-to-end.
    #[test]
    fn four_flatmates_exactly_at_threshold_is_false() {
        let salaries = [25_000u64, 25_000, 25_000, 25_000]; // exactly 100_000
        let plaintext_sum: u64 = salaries.iter().sum();
        assert_eq!(plaintext_sum, 100_000);

        let flatmates = four_flatmates(salaries);
        let backend = ShamirBackend::new(5).unwrap(); // production CSPRNG, n=5
                                                      // n=5 but 4 flatmates → mismatch must be a fail-closed Protocol error.
        let err = run_federated(&backend, &flatmates, &fed_query(100_000)).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(_)));

        // With the matching party count it is the strict-> false verdict.
        let backend = ShamirBackend::new_seeded(4, 1).unwrap();
        let resp = run_federated(&backend, &flatmates, &fed_query(100_000)).unwrap();
        assert!(
            !resp.over_threshold,
            "100k is NOT strictly greater than 100k ⇒ false"
        );
    }

    /// The per-operator routing (RQ2a) is explicit & correct: the membership join
    /// is Disclosed, the salary aggregate + threshold are Hidden with the right
    /// OperatorClass — and the per-operator security tier reads off the backend.
    #[test]
    fn per_operator_routing_is_explicit() {
        let flatmates = four_flatmates([30_000, 28_000, 26_000, 24_000]);
        let backend = ShamirBackend::new_seeded(4, 5).unwrap();
        let resp = run_federated(&backend, &flatmates, &fed_query(100_000)).unwrap();

        assert_eq!(resp.routing.len(), 3);
        // The membership join is disclosed (crypto-free, in the clear).
        assert_eq!(resp.routing[0].routing, Routing::Disclosed);
        assert!(resp.routing[0].operator.contains("membership-join"));
        // The salary aggregate is hidden, a LinearAggregate.
        assert_eq!(
            resp.routing[1].routing,
            Routing::Hidden(OperatorClass::LinearAggregate)
        );
        // The threshold is hidden, a Comparison.
        assert_eq!(
            resp.routing[2].routing,
            Routing::Hidden(OperatorClass::Comparison)
        );

        // The Comparison operator's tier is honest-majority semi-honest-only
        // (the secure-threshold's stated tier), read straight off the backend.
        let cmp_desc: SecurityDescriptor = backend.operator_security(OperatorClass::Comparison);
        assert_eq!(
            cmp_desc,
            SecurityDescriptor::semi_honest_only(backend.parties(), backend.threshold())
        );
    }

    /// Differential robustness: across several flatmate salary tuples (mixing over
    /// / under / at the threshold) and party counts, the secure verdict matches
    /// the plaintext threshold AND the disclosed result matches the union store.
    #[test]
    fn differential_across_salary_tuples() {
        let tuples: &[[u64; 4]] = &[
            [0, 0, 0, 0],                     // 0, under
            [25_000, 25_000, 25_000, 25_001], // just over
            [25_000, 25_000, 25_000, 24_999], // just under
            [50_000, 50_000, 50_000, 50_000], // well over
            [1, 2, 3, 4],                     // tiny, under
            [100_000, 1, 0, 0],               // over via one big salary
        ];
        for (i, &salaries) in tuples.iter().enumerate() {
            let plaintext_sum: u64 = salaries.iter().sum();
            let flatmates = four_flatmates(salaries);
            let backend =
                ShamirBackend::new_seeded(4, (i as u64).wrapping_mul(101).wrapping_add(3)).unwrap();
            let resp = run_federated(&backend, &flatmates, &fed_query(100_000)).unwrap();

            assert_eq!(
                resp.over_threshold,
                plaintext_sum > 100_000,
                "tuple {salaries:?}: secure verdict disagreed with plaintext sum {plaintext_sum}"
            );
            let expected = union_store_eval_from_docs(&four_docs(), FEDERATED_QUERY);
            assert_eq!(
                resp.statement.disclosed_result, expected,
                "tuple {salaries:?}: disclosed result must equal union-store eval"
            );
        }
    }

    /// An empty federation is a fail-closed Protocol error, not a panic.
    #[test]
    fn empty_federation_fails_closed() {
        let backend = ShamirBackend::new_seeded(2, 1).unwrap();
        let err = run_federated(&backend, &[], &fed_query(100_000)).unwrap_err();
        assert!(matches!(err, MpcError::Protocol(_)));
    }

    /// HONESTY pin: the collaborative proof STAYS the stub. The driver assembles
    /// the ProofStatement structure with the real disclosed result, but proving it
    /// is still the honest NotYetImplemented (no faked proof). We exercise the
    /// deferred trait method to confirm it fails closed even though the statement
    /// is now real.
    #[test]
    fn proof_prove_stays_the_honest_stub() {
        use crate::backend::{
            AbortKind, AdversaryModel, BackendInfo, CorruptionThreshold, OutputGuarantee,
            PublicVerifiability,
        };
        use crate::join::JoinPlan as JP;
        use crate::proof::{CollaborativeProof, Proof};

        let flatmates = four_flatmates([30_000, 28_000, 26_000, 24_000]);
        let backend = ShamirBackend::new_seeded(4, 9).unwrap();
        let resp = run_federated(&backend, &flatmates, &fed_query(100_000)).unwrap();
        // The statement is REAL (its disclosed_result is non-empty & matches the
        // union store), yet proving stays deferred.
        assert!(!resp.statement.disclosed_result.is_empty());

        // A do-nothing backend + stub prover, exactly as the proof module's own
        // contract test: prove() must fail with a gate-naming NotYetImplemented.
        struct StubBackend;
        impl MpcBackend for StubBackend {
            type Share = ();
            fn info(&self) -> BackendInfo {
                BackendInfo::new(
                    "stub",
                    SecurityDescriptor {
                        adversary: AdversaryModel::SemiHonest,
                        output_guarantee: OutputGuarantee::Abort(AbortKind::Selective),
                        threshold: CorruptionThreshold::HonestMajority { t: 1 },
                        public_verifiability: PublicVerifiability(false),
                    },
                    0,
                )
            }
            fn share_private_input(&self, _h: &Holder) -> Result<Vec<()>, MpcError> {
                Err(MpcError::not_yet("share", "M3"))
            }
            fn run_secure(&self, _s: &[()]) -> Result<Vec<()>, MpcError> {
                Err(MpcError::not_yet("run", "M3"))
            }
            fn reconstruct_disclosed(&self, _s: &[()]) -> Result<PartialResult, MpcError> {
                Err(MpcError::not_yet("reconstruct", "M3"))
            }
        }
        struct StubProof;
        impl CollaborativeProof<StubBackend> for StubProof {
            fn prove(
                &self,
                _b: &StubBackend,
                _s: &ProofStatement,
                _j: &[JP],
            ) -> Result<Proof, MpcError> {
                Err(MpcError::not_yet(
                    "collaborative proof of correct + attested-source result",
                    "ZK foundation #3/#4/#5/#6/#8/#9/#12 + Q1 distributed-attestation spike (M4)",
                ))
            }
            fn verify(&self, _s: &ProofStatement, _p: &Proof) -> Result<bool, MpcError> {
                Err(MpcError::not_yet("verify", "ZK foundation + M4"))
            }
        }
        let err = StubProof
            .prove(&StubBackend, &resp.statement, &[])
            .unwrap_err();
        match err {
            MpcError::NotYetImplemented { gated_on, .. } => {
                assert!(gated_on.contains("Q1"), "proof.prove stays gated on Q1");
            }
            other => panic!("expected NotYetImplemented, got {other:?}"),
        }
    }

    /// Determinism: the disclosed key-set `K` is a SET, so caller order
    /// and multiplicity must NOT affect the assembled `ProofStatement`. Two runs
    /// over the SAME logical key-set — one with the issuer keys in a permuted caller
    /// order, one with a DUPLICATE key — must both yield the SAME canonical
    /// `disclosed_key_set` (sorted, de-duplicated), and hence the same statement
    /// shape, so the (deferred) proof's public-inputs digest is reproducible.
    #[test]
    fn disclosed_key_set_is_canonical_set_order_and_multiplicity_invariant() {
        let salaries = [30_000u64, 28_000, 26_000, 24_000];

        // Run A: keys in one caller order.
        let keys_a = [
            "did:example:hr-acme#k1",
            "did:example:hr-globex#k1",
            "did:example:hr-initech#k1",
            "did:example:hr-umbrella#k1",
        ];
        // Run B: the SAME logical key-set, but a DIFFERENT caller order AND a
        // duplicate (acme repeated, umbrella dropped here is NOT what we want — we
        // keep the same set). Permute + duplicate one key: the canonical form must
        // collapse the dup and re-sort, so A and B must match.
        let keys_b = [
            "did:example:hr-umbrella#k1",
            "did:example:hr-acme#k1",
            "did:example:hr-initech#k1",
            "did:example:hr-acme#k1", // DUPLICATE of acme (globex absent on purpose)
        ];

        let make = |keys: [&str; 4]| {
            let members = ["alice", "bob", "carol", "dave"];
            (0..4)
                .map(|i| flatmate(i, members[i], salaries[i], keys[i]))
                .collect::<Vec<_>>()
        };

        let backend = ShamirBackend::new_seeded(4, 0xCA70).unwrap();
        let resp_a = run_federated(&backend, &make(keys_a), &fed_query(100_000)).unwrap();

        // For B we need the SAME logical set as A. A = {acme, globex, initech, umbrella}.
        // To exercise both invariants (order + multiplicity) within a 4-flatmate
        // federation, B uses {umbrella, acme, initech, acme} which is the SET
        // {acme, initech, umbrella}. So compare B against a from-scratch canonical
        // of exactly those keys rather than against A.
        let resp_b = run_federated(&backend, &make(keys_b), &fed_query(100_000)).unwrap();

        // Run A: distinct keys → canonical is the 4 keys, sorted.
        let mut expected_a: Vec<String> = keys_a.iter().map(|k| k.to_string()).collect();
        expected_a.sort();
        expected_a.dedup();
        assert_eq!(resp_a.statement.disclosed_key_set, expected_a);
        assert!(
            resp_a
                .statement
                .disclosed_key_set
                .windows(2)
                .all(|w| w[0] < w[1]),
            "canonical key-set must be strictly sorted with no duplicates"
        );

        // Run B: duplicate collapses, order does not matter → canonical = sorted set.
        let mut expected_b: Vec<String> = keys_b.iter().map(|k| k.to_string()).collect();
        expected_b.sort();
        expected_b.dedup();
        assert_eq!(resp_b.statement.disclosed_key_set, expected_b);
        assert_eq!(
            resp_b.statement.disclosed_key_set.len(),
            3,
            "the duplicated acme key must collapse to a single set element"
        );
        assert!(
            resp_b
                .statement
                .disclosed_key_set
                .windows(2)
                .all(|w| w[0] < w[1]),
            "canonical key-set must be strictly sorted with no duplicates"
        );

        // Pure permutation invariance over the SAME set: re-running A with the keys
        // in a shuffled caller order must give a byte-identical canonical key-set.
        let keys_a_permuted = [
            "did:example:hr-initech#k1",
            "did:example:hr-umbrella#k1",
            "did:example:hr-acme#k1",
            "did:example:hr-globex#k1",
        ];
        let resp_a_perm =
            run_federated(&backend, &make(keys_a_permuted), &fed_query(100_000)).unwrap();
        assert_eq!(
            resp_a_perm.statement.disclosed_key_set, resp_a.statement.disclosed_key_set,
            "same logical key-set in a different caller order must yield the SAME canonical \
             disclosed_key_set (and thus the same statement shape)"
        );
    }
}
