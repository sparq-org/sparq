//! [SONNET-4.6] sq-x1ayt — **time-windowed conditional-grant** decision rows: the
//! `Session::now` dimension of the corpus.
//!
//! # Why this is a separate corpus
//!
//! ACP has no time vocabulary. The only producer of a live-clock validity window in
//! `sparq-solid` is the ODRL bridge: a faithfully-mappable `odrl:dateTime` constraint is
//! persisted onto an `auth:ConditionalGrant` as inclusive `auth:notBefore`/`auth:notAfter`
//! bounds, which the session layer re-checks against `Session::now` **per request**
//! rather than freezing at materialization time. That bridge is `sparq-solid`'s opt-in
//! `odrl-bridge` feature, so these rows live behind this crate's matching `odrl-bridge`
//! feature and the default corpus in the crate root is untouched.
//!
//! # What the rows pin
//!
//! Same [`Vector`]/[`Expected`] schema and the same [`run_vectors`] runner as the WAC/ACP
//! corpus, so a consumer asserts them exactly the same way — only the store construction
//! differs ([`build_window_store`] runs the ODRL bridge over a materialized ACP pod).
//!
//! The bridge no longer stores a time-windowed grant: a stored grant is never
//! re-checked, so only an unconstrained grant to the named party is stored, and per-request
//! ODRL decisions are tracked in #6743. These rows pin that intended behaviour change:
//!
//! - carol is **denied** at every instant (inside the window, at both bounds, before,
//!   after, and with no clock), since nothing was stored for her;
//! - bob and an anonymous session are denied inside the window;
//! - alice's unwindowed ACP grant ignores `now` entirely (inside the window, after it has
//!   closed, and with no clock).

use crate::{
    resolved_default, run_vectors, Expected, Mode, OracleReport, PodStore, Vector, ALICE, BOB,
    CAROL, RWC,
};
use sparq_core::Graph;

/// The embedded pod (N-Quads): one document under `win8/`, whose `.acr` carries alice's
/// UNWINDOWED grant. The windowed grant is layered on by [`build_window_store`].
pub const WINDOW_NQUADS: &str = include_str!("../fixtures/window.nq");

/// The embedded ODRL policy (Turtle) whose bridged grant carries the live-clock window.
pub const WINDOW_POLICY_TTL: &str = include_str!("../fixtures/window-policy.ttl");

/// The resource every window row decides on.
pub const WINDOW_RESOURCE: &str = "https://pod.ex/win8/d0.ttl";

/// The window's inclusive lower bound (`odrl:dateTime gteq` → `auth:notBefore`).
pub const WINDOW_NOT_BEFORE: &str = "2026-06-01T00:00:00Z";
/// The window's inclusive upper bound (`odrl:dateTime lteq` → `auth:notAfter`).
pub const WINDOW_NOT_AFTER: &str = "2026-12-31T00:00:00Z";

const R: Mode = Mode::Read;

/// An instant strictly inside `[WINDOW_NOT_BEFORE, WINDOW_NOT_AFTER]`.
const INSIDE: &str = "2026-06-17T09:00:00Z";
/// An instant strictly BEFORE the window opens.
const BEFORE: &str = "2026-05-01T00:00:00Z";
/// An instant strictly AFTER the window closes.
const AFTER: &str = "2027-01-01T00:00:00Z";

/// Decision rows over [`WINDOW_NQUADS`] + [`WINDOW_POLICY_TTL`]. Every row shares one
/// store, so the before/inside/after verdicts differ ONLY by the request clock.
pub static WINDOW_VECTORS: [Vector; 11] = [
    // carol INSIDE the window: the bridge stored nothing, so she is denied.
    v(
        "window-carol-inside",
        Some(CAROL),
        R,
        resolved_default(false, NONE),
    )
    .at(INSIDE),
    // …and at exactly the open and close instants.
    v(
        "window-carol-at-open-inclusive",
        Some(CAROL),
        R,
        resolved_default(false, NONE),
    )
    .at(WINDOW_NOT_BEFORE),
    v(
        "window-carol-at-close-inclusive",
        Some(CAROL),
        R,
        resolved_default(false, NONE),
    )
    .at(WINDOW_NOT_AFTER),
    // …and before it opens and after it closes.
    v(
        "window-carol-before-open",
        Some(CAROL),
        R,
        resolved_default(false, NONE),
    )
    .at(BEFORE),
    v(
        "window-carol-after-close",
        Some(CAROL),
        R,
        resolved_default(false, NONE),
    )
    .at(AFTER),
    // …and with no clock.
    v(
        "window-carol-no-clock-fail-closed",
        Some(CAROL),
        R,
        resolved_default(false, NONE),
    ),
    // the window is recipient-scoped: being inside it grants a non-recipient nothing.
    v(
        "window-bob-inside-not-recipient",
        Some(BOB),
        R,
        resolved_default(false, NONE),
    )
    .at(INSIDE),
    v(
        "window-anon-inside-denied",
        None,
        R,
        resolved_default(false, NONE),
    )
    .at(INSIDE),
    // alice's UNWINDOWED ACP grant ignores the clock: identical inside the window…
    v(
        "window-alice-unwindowed-with-clock",
        Some(ALICE),
        R,
        resolved_default(true, RWC),
    )
    .at(INSIDE),
    // …at the very instant carol's window has lapsed. Today the clock is read at exactly
    // one site (the conditional grant's head), which alice's plain ACP allow never
    // reaches, so this row is a REGRESSION guard rather than a live mutation witness: it
    // pins that expiry stays scoped to the grant carrying it, never global to the store…
    v(
        "window-alice-unwindowed-after-close",
        Some(ALICE),
        R,
        resolved_default(true, RWC),
    )
    .at(AFTER),
    // …and with no clock at all.
    v(
        "window-alice-unwindowed-no-clock",
        Some(ALICE),
        R,
        resolved_default(true, RWC),
    ),
];

const NONE: &[Mode] = &[];

/// Shorthand row constructor — every window row decides Read-or-Write on
/// [`WINDOW_RESOURCE`] as an agent-only session (the client/issuer dimensions are
/// exercised by the WAC/ACP corpus).
const fn v(
    name: &'static str,
    agent: Option<&'static str>,
    mode: Mode,
    expect: Expected,
) -> Vector {
    Vector {
        name,
        agent,
        client: None,
        issuer: None,
        now: None,
        resource: WINDOW_RESOURCE,
        mode,
        expect,
    }
}

/// Build the window corpus's store: load [`WINDOW_NQUADS`], materialize the ACP auth view
/// (alice's unwindowed grant), then run the ODRL bridge for carol's windowed permission,
/// which stores nothing.
///
/// Order matters — `materialize_acp` rebuilds `<urn:sparq:auth>` from scratch, so the
/// bridge runs after it, exactly as a server would sequence them.
///
/// # Errors
///
/// If the pod fails to load, if ACP materialization fails, or if the bridge stores a
/// windowed grant (a stored grant is never re-checked against the clock).
pub fn build_window_store() -> Result<PodStore, String> {
    let graph = Graph::load_dataset(WINDOW_NQUADS, "nquads")?;
    let mut store = PodStore::new(graph);
    store.materialize_acp()?;

    let policy = sparq_policy::parse_policy_str(WINDOW_POLICY_TTL, "turtle")?;
    let request = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
        .on(WINDOW_RESOURCE)
        .by(CAROL)
        .at(INSIDE);
    let outcome = store.materialize_odrl_permission_conditional(&policy, &request);
    if outcome.granted {
        return Err(format!("the bridge stored a windowed grant: {:?}", outcome));
    }
    Ok(store)
}

/// [`build_window_store`] + [`run_vectors`] over [`WINDOW_VECTORS`].
pub fn run_window_vectors() -> Result<OracleReport, String> {
    let store = build_window_store()?;
    Ok(run_vectors(&store, &WINDOW_VECTORS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AclStatus, READ};

    /// The row with this `name` — index-free, so growing or reordering
    /// [`WINDOW_VECTORS`] can never silently re-point a test at a different row.
    fn row(name: &str) -> Vector {
        *WINDOW_VECTORS
            .iter()
            .find(|row| row.name == name)
            .unwrap_or_else(|| panic!("no window row named {}", name))
    }

    #[test]
    fn window_vectors_all_pass() {
        let report = run_window_vectors().expect("window corpus builds");
        assert!(report.passed(), "{}", report);
        assert_eq!(report.total, WINDOW_VECTORS.len());
    }

    #[test]
    fn the_bridge_adds_nothing_to_the_acp_view() {
        // carol's windowed permission is allowed by the ODRL decision inside the window,
        // but the bridged store decides every row exactly as the ACP view alone does.
        let policy = sparq_policy::parse_policy_str(WINDOW_POLICY_TTL, "turtle").unwrap();
        let inside = sparq_policy::Request::new("http://www.w3.org/ns/odrl/2/read")
            .on(WINDOW_RESOURCE)
            .by(CAROL)
            .at(INSIDE);
        assert!(sparq_policy::evaluate(&policy, &inside).allow, "the decision allows carol");

        let graph = Graph::load_dataset(WINDOW_NQUADS, "nquads").expect("pod loads");
        let mut acp_only = PodStore::new(graph);
        acp_only.materialize_acp().expect("acp materializes");
        let bridged = build_window_store().expect("window store builds");
        for row in &WINDOW_VECTORS {
            let bare = acp_only.decide(&row.session(), row.resource, row.mode);
            let with_bridge = bridged.decide(&row.session(), row.resource, row.mode);
            assert_eq!(bare.allow, with_bridge.allow, "{}", row.name);
            assert_eq!(bare.status, AclStatus::Resolved, "the ACP view still governs");
        }
    }

    #[test]
    fn the_clock_is_the_only_difference_between_the_carol_rows() {
        // The before/inside/after rows must differ ONLY in `now` — otherwise they would
        // not isolate the live-clock re-check.
        let carol: Vec<&Vector> = WINDOW_VECTORS
            .iter()
            .filter(|row| row.agent == Some(CAROL))
            .collect();
        assert_eq!(carol.len(), 6);
        for row in &carol {
            assert_eq!(row.resource, WINDOW_RESOURCE);
            assert_eq!(row.mode, Mode::Read);
            assert_eq!(row.client, None);
            assert_eq!(row.issuer, None);
        }
        let clocks: Vec<Option<&str>> = carol.iter().map(|row| row.now).collect();
        assert_eq!(
            clocks,
            vec![
                Some(INSIDE),
                Some(WINDOW_NOT_BEFORE),
                Some(WINDOW_NOT_AFTER),
                Some(BEFORE),
                Some(AFTER),
                None,
            ]
        );
        for row in &carol {
            assert!(!row.expect.allow, "nothing is stored for carol: {}", row.name);
        }
    }

    #[test]
    fn the_unwindowed_alice_rows_span_the_lapsed_window_and_no_clock() {
        // The "a grant with no window ignores `now`" claim is only pinned if alice is
        // asserted AFTER carol's window has closed as well as with no clock at all —
        // with only the INSIDE + no-clock rows, a future change that let a window expire
        // grants other than the one carrying it would still pass the corpus.
        let alice: Vec<&Vector> = WINDOW_VECTORS
            .iter()
            .filter(|row| row.agent == Some(ALICE))
            .collect();
        let clocks: Vec<Option<&str>> = alice.iter().map(|row| row.now).collect();
        assert_eq!(clocks, vec![Some(INSIDE), Some(AFTER), None]);
        for row in &alice {
            assert!(row.expect.allow, "unwindowed grant: {} must allow", row.name);
            assert_eq!(row.expect.granted_modes, RWC);
        }
    }

    #[test]
    fn runner_flags_a_flipped_window_expectation() {
        // Mutation witness: claim carol is allowed AFTER the window closed -> red.
        let store = build_window_store().expect("window store builds");
        let mut wrong = row("window-carol-after-close");
        assert!(!wrong.expect.allow, "precondition: the after-close row expects deny");
        wrong.expect = resolved_default(true, READ);
        let report = run_vectors(&store, &[wrong]);
        assert!(!report.passed(), "a lapsed window must not decide allow");
        assert_eq!(report.failures[0].vector, wrong.name);
    }
}
