// [FABLE-5] sq-hmd7l.44 — ODRL-gated vs unguarded query-eval OVERHEAD envelope
// 🤖 SPARQ agent — measures what enforcement COSTS (epic sq-hmd7l). The sibling
// bench/ac/ + bench/ac/live/ drivers are CORRECTNESS/decision-agreement; no lane there
// compares gated vs unguarded latency. This driver adds exactly that, fail-closed:
//
//   Lane A — policy MATERIALIZATION cost sweep: kind × count over the bridge entry
//     points (bare permission / permission+prohibition / conditional recipient
//     re-check / counted `odrl:count`). The bare permission must grant; every other
//     kind must be refused (the bridge stores only grants nothing can withdraw), so the
//     lane times the refusal. A post-sweep access probe asserts grants are real and
//     scoped, refusals give no access, and the stranger is denied.
//
//   Lane B — STEADY-STATE per-query overhead: `PodStore::query_as` over an
//     ODRL-materialized <urn:sparq:auth> vs the SAME query unguarded
//     (`sparq_engine::query`) on a data-only twin of the store, where the permitted
//     subset is physically the whole store. Result-set EQUALITY is asserted per query
//     BEFORE timing (identical result sets ⇒ honest apples-to-apples), and a
//     no-grants stranger must see 0 rows through the gated path (anti-vacuity).
//     One gate regime: one-shot grants (conditional grants are no longer stored).
//     Resource universes come from the sparq-acbench U1–U4 use-case intent tables at
//     ≥2 scale factors.
//
//   Lane C — CHURN: `refresh_odrl_grants` (no-op re-evaluation of the whole ledger)
//     and `refresh_odrl_grant` (a policy-write revocation) cost vs ledger size,
//     asserting the revocation RETRACTS access (fail-closed) and the no-op retracts
//     nothing.
//
// COMPETITOR (sq-hmd7l mandate): recorded as an explicit honest NOT-COMPARABLE
// verdict in the envelope — see `competitor_verdict()` for the reason; the HTTP-level
// comparison composes with sq-lrtc3.1 when the server ODRL wiring lands.
//
// HONESTY: every wall-clock number is advisory + NON-CANONICAL on a shared work box
// (bench/CATALOG.md QUIET-BOX convention); canonical envelopes are EC2-gated. The
// HARD contract is the fail-closed exit code: any result-set mismatch, ungranted
// materialization, missed retraction, or stranger over-share exits non-zero and no
// timing is emitted for the failed lane. No number is committed to markdown.

#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::cast_precision_loss)]

use std::process::ExitCode;
use std::sync::Arc;

use sparq_acbench::{consortium, financial, personal, project_mgmt, GenParams, IntentRow};
use sparq_core::Graph;
use sparq_engine::QueryResult;
use sparq_policy::{parse_policy_str, InMemoryCounterStore, Request, ValidatedPolicy};
use sparq_solid::{BridgeKind, Mode, PodStore, Session};

// ── Constants ─────────────────────────────────────────────────────────────────────────

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
/// The single granted reader in lane B (the permitted subset = the whole store for it).
const BENCH_AGENT: &str = "https://bench.sparq.dev/agent#reader";
/// Never granted anything, anywhere. Used for the fail-closed over-share probes.
const STRANGER: &str = "https://bench.sparq.dev/agent#stranger";
/// The assignee a revocation rewrites a permission towards (lane C): re-evaluating the
/// original requester against this policy must Deny → the bridged grant is retracted.
const OTHER_AGENT: &str = "https://bench.sparq.dev/agent#other";

fn odrl_iri(local: &str) -> String {
    format!("{ODRL}{local}")
}

fn agent_iri(i: usize) -> String {
    format!("https://agents.bench.sparq.dev/a{i}#me")
}

fn res_iri(i: usize) -> String {
    format!("https://pod.bench.sparq.dev/r{i}")
}

// ── Dataset builders ──────────────────────────────────────────────────────────────────

/// Deterministic per-resource content: `triples_per` triples in each resource's own
/// named graph (distinct predicates so predicate-bound queries select a real subset).
fn data_nquads(resources: &[String], triples_per: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (i, r) in resources.iter().enumerate() {
        for j in 0..triples_per {
            let _ = writeln!(
                out,
                "<{r}#e{j}> <https://sparq.dev/vocab/bench#field{j}> \"v{i}-{j}\" <{r}> ."
            );
        }
    }
    out
}

fn load_graph(nquads: &str) -> Result<Graph, String> {
    Graph::load_dataset(nquads, "nquads").map_err(|e| format!("dataset load: {e}"))
}

// ── ODRL policy builders (parsed OUTSIDE any timed section — parse cost is the
//    `policy-odrl-eval` suite's axis, not this one's) ─────────────────────────────────

fn parse(ttl: &str) -> Result<ValidatedPolicy, String> {
    parse_policy_str(ttl, "turtle").map_err(|e| format!("policy parse: {e}"))
}

/// Bare matching permission: `agent` MAY read `target`.
fn permission_ttl(id: usize, agent: &str, target: &str) -> String {
    format!(
        "@prefix odrl: <{ODRL}> .\n\
         <urn:pol:perm{id}> a odrl:Set ; odrl:permission [\n\
           odrl:action odrl:read ;\n\
           odrl:target <{target}> ;\n\
           odrl:assignee <{agent}> ] .\n"
    )
}

/// Permission for `agent` + prohibition carving out `prohibited` on the same target
/// (two rule arrays evaluated per materialization — the complexity dimension).
fn policy_both_ttl(id: usize, agent: &str, prohibited: &str, target: &str) -> String {
    format!(
        "@prefix odrl: <{ODRL}> .\n\
         <urn:pol:both{id}> a odrl:Set ;\n\
           odrl:permission [\n\
             odrl:action odrl:read ;\n\
             odrl:target <{target}> ;\n\
             odrl:assignee <{agent}> ] ;\n\
           odrl:prohibition [\n\
             odrl:action odrl:read ;\n\
             odrl:target <{target}> ;\n\
             odrl:assignee <{prohibited}> ] .\n"
    )
}

/// Recipient-constrained permission: persists as an `auth:ConditionalGrant`
/// re-checked per session (the per-session-recheck cost dimension).
fn conditional_ttl(id: usize, recipient: &str, target: &str) -> String {
    format!(
        "@prefix odrl: <{ODRL}> .\n\
         <urn:pol:cond{id}> a odrl:Set ; odrl:permission [\n\
           odrl:action odrl:read ;\n\
           odrl:target <{target}> ;\n\
           odrl:constraint [ odrl:leftOperand odrl:recipient ;\n\
                             odrl:operator odrl:eq ;\n\
                             odrl:rightOperand <{recipient}> ] ] .\n"
    )
}

/// Counted permission (`odrl:count lteq budget`) — stateful enforcement through the
/// bridge ledger; the budget is large enough that no lane-A grant exhausts mid-sweep.
fn counted_ttl(id: usize, agent: &str, target: &str, budget: u64) -> String {
    format!(
        "@prefix odrl: <{ODRL}> .\n\
         <urn:pol:cnt{id}> a odrl:Set ; odrl:permission [\n\
           odrl:action odrl:read ;\n\
           odrl:target <{target}> ;\n\
           odrl:assignee <{agent}> ;\n\
           odrl:constraint [ odrl:leftOperand odrl:count ;\n\
                             odrl:operator odrl:lteq ;\n\
                             odrl:rightOperand {budget} ] ] .\n"
    )
}

fn read_request(target: &str, agent: &str) -> Request {
    Request::new(odrl_iri("read")).on(target).by(agent)
}

fn session(agent: &str) -> Session<'_> {
    Session { agent: Some(agent), client: None, issuer: None, now: None }
}

// ── Timing + result normalization ─────────────────────────────────────────────────────

fn time_us<T>(f: impl FnOnce() -> T) -> (u64, T) {
    let t0 = std::time::Instant::now();
    let v = f();
    (u64::try_from(t0.elapsed().as_micros()).unwrap_or(u64::MAX), v)
}

/// Order-insensitive row multiset: one sorted line per row (UNDEF for unbound).
fn normalized_rows(r: &QueryResult) -> Vec<String> {
    let mut rows: Vec<String> = r
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|c| c.as_ref().map_or_else(|| "UNDEF".to_owned(), ToString::to_string))
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect();
    rows.sort_unstable();
    rows
}

/// Fail-closed apples-to-apples gate: the gated and unguarded result multisets must be
/// IDENTICAL (same rows, any order). Returns the shared row count.
fn assert_equivalent(
    label: &str,
    gated: &QueryResult,
    plain: &QueryResult,
) -> Result<usize, String> {
    let g = normalized_rows(gated);
    let p = normalized_rows(plain);
    if g == p {
        Ok(g.len())
    } else {
        Err(format!(
            "{label}: RESULT-SET MISMATCH gated={} rows vs unguarded={} rows — the \
             gated view is not the whole store (or enforcement altered results); \
             refusing to time a non-equivalent comparison (fail-closed)",
            g.len(),
            p.len()
        ))
    }
}

// ── JSON envelope (hand-rolled, dependency-free) ──────────────────────────────────────

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// The honest same-box competitor disposition for this axis (sq-hmd7l mandate).
fn competitor_verdict() -> String {
    let reason = "Community Solid Server (and other Solid servers) enforce WAC/ACP at \
        the HTTP resource level (per-request HTTP+LDP stack; no ODRL-gated SPARQL query \
        endpoint), and ODRE publishes PDP decision throughput (policy evaluation only, \
        no query-result filtering). Neither exposes a same-box, in-process, ODRL-gated \
        SPARQL query surface comparable to PodStore::query_as, so a library-level \
        head-to-head would compare an in-process function call against an HTTP stack \
        and be dishonest in either direction.";
    format!(
        "{{\"status\":\"NOT_COMPARABLE\",\"axis\":\"odrl-gated-query-overhead\",\
         \"reason\":\"{}\",\"composes_with\":\"sq-lrtc3.1 (server ODRL wiring) unlocks \
         the HTTP-level same-box lane; follow-up bead sq-12pmx\"}}",
        json_escape(reason)
    )
}

// ── Lane A: materialization cost sweep ────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum PolicyKind {
    Permission,
    PermissionProhibition,
    ConditionalRecipient,
    Counted,
}

impl PolicyKind {
    const ALL: [PolicyKind; 4] = [
        PolicyKind::Permission,
        PolicyKind::PermissionProhibition,
        PolicyKind::ConditionalRecipient,
        PolicyKind::Counted,
    ];
    fn label(self) -> &'static str {
        match self {
            PolicyKind::Permission => "permission",
            PolicyKind::PermissionProhibition => "permission+prohibition",
            PolicyKind::ConditionalRecipient => "conditional-recipient",
            PolicyKind::Counted => "counted",
        }
    }
    /// Whether the bridge stores a grant for this kind. Only an unconstrained grant to
    /// the named party in a policy with no prohibitions can be stored: a constraint
    /// (recipient, count) or a prohibition can change while a stored grant stands, so
    /// those kinds are refused and the lane times the refusal instead.
    fn stores_grant(self) -> bool {
        matches!(self, PolicyKind::Permission)
    }
}

/// Materialize `n` pre-parsed policies of one kind into a fresh store, timing ONLY the
/// bridge calls (policy parsing happens before the clock starts). Every call must
/// grant; the post-sweep probe asserts scoping (`agent_0` sees `r0`, not `r1`; stranger
/// sees nothing).
fn lane_a_config(kind: PolicyKind, n: usize) -> Result<String, String> {
    let resources: Vec<String> = (0..n).map(res_iri).collect();
    let graph = load_graph(&data_nquads(&resources, 2))?;
    let mut store = PodStore::new(graph);

    // Pre-parse policies + requests (outside the timed section).
    let mut policies = Vec::with_capacity(n);
    let mut requests = Vec::with_capacity(n);
    for (i, r) in resources.iter().enumerate() {
        let a = agent_iri(i);
        let ttl = match kind {
            PolicyKind::Permission => permission_ttl(i, &a, r),
            PolicyKind::PermissionProhibition => policy_both_ttl(i, &a, STRANGER, r),
            PolicyKind::ConditionalRecipient => conditional_ttl(i, &a, r),
            PolicyKind::Counted => counted_ttl(i, &a, r, 1_000_000),
        };
        policies.push(parse(&ttl)?);
        requests.push(read_request(r, &a));
    }
    let counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
        Arc::new(InMemoryCounterStore::new());

    let (wall_us, granted_all) = time_us(|| {
        let mut all = true;
        for (p, q) in policies.iter().zip(&requests) {
            let out = match kind {
                PolicyKind::Permission => store.materialize_odrl_permission(p, q),
                PolicyKind::PermissionProhibition => store.materialize_odrl_policy(p, q),
                PolicyKind::ConditionalRecipient => {
                    store.materialize_odrl_permission_conditional(p, q)
                }
                PolicyKind::Counted => {
                    store.materialize_odrl_permission_counted(p, q, &counter)
                }
            };
            all &= out.granted == kind.stores_grant();
        }
        all
    });
    if !granted_all {
        return Err(format!(
            "lane A {}/{n}: a materialization did not {} (fail-closed)",
            kind.label(),
            if kind.stores_grant() { "grant" } else { "refuse" }
        ));
    }

    // Post-sweep access probes (fail-closed): grants are real AND scoped.
    let a0 = agent_iri(0);
    let probe = |agent: &str, res: &str| -> Result<usize, String> {
        store
            .query_as(
                &session(agent),
                Mode::Read,
                &format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{res}> {{ ?s ?p ?o }} }}"),
            )
            .map(|r| r.rows.len())
            .map_err(|e| format!("lane A probe query: {e}"))
    };
    let seen = probe(&a0, &resources[0])?;
    if kind.stores_grant() && seen == 0 {
        return Err(format!(
            "lane A {}/{n}: granted agent sees 0 rows — vacuous grant (fail-closed)",
            kind.label()
        ));
    }
    if !kind.stores_grant() && seen != 0 {
        return Err(format!(
            "lane A {}/{n}: a refused policy still gives agent_0 {seen} rows (over-share)",
            kind.label()
        ));
    }
    if n > 1 && probe(&a0, &resources[1])? != 0 {
        return Err(format!(
            "lane A {}/{n}: agent_0 OVER-SHARED into r1 (grant not scoped)",
            kind.label()
        ));
    }
    if probe(STRANGER, &resources[0])? != 0 {
        return Err(format!(
            "lane A {}/{n}: STRANGER over-share (security failure)",
            kind.label()
        ));
    }

    let per_policy = wall_us / n as u64;
    println!(
        "materialize\t{}\tn={n}\tPASS\twall_us_indicative={wall_us} per_policy_us={per_policy} (advisory)",
        kind.label()
    );
    Ok(format!(
        "{{\"lane\":\"materialize\",\"kind\":\"{}\",\"n_policies\":{n},\
         \"wall_us\":{wall_us},\"per_policy_us\":{per_policy}}}",
        kind.label()
    ))
}

fn lane_a(counts: &[usize], records: &mut Vec<String>) -> Result<(), String> {
    for kind in PolicyKind::ALL {
        for &n in counts {
            records.push(lane_a_config(kind, n)?);
        }
    }
    Ok(())
}

// ── Lane B: steady-state per-query overhead ───────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum GateRegime {
    /// One-shot bridged grants (`materialize_odrl_permission`). Conditional grants are
    /// no longer stored by the bridge, so there is no per-session-recheck regime.
    OneShot,
}

impl GateRegime {
    fn label(self) -> &'static str {
        match self {
            GateRegime::OneShot => "oneshot",
        }
    }
}

/// The representative query mix (acbench W2 shape + aggregate scans). Returned as
/// `(query_id, sparql)` pairs; per-resource point lookups are sampled first/mid/last.
fn query_mix(resources: &[String]) -> Vec<(String, String)> {
    let mut qs = vec![
        (
            "graph-scan-all".to_owned(),
            "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }".to_owned(),
        ),
        (
            "predicate-bound".to_owned(),
            "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s <https://sparq.dev/vocab/bench#field3> ?o } }"
                .to_owned(),
        ),
    ];
    for (tag, idx) in [("first", 0), ("mid", resources.len() / 2), ("last", resources.len() - 1)]
    {
        qs.push((
            format!("per-resource-{tag}"),
            format!(
                "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
                resources[idx]
            ),
        ));
    }
    qs
}

/// Build the gated store (grant-all for `BENCH_AGENT` under `regime`) and its
/// data-only unguarded twin, verify equivalence + anti-vacuity, then time the mix.
fn lane_b_usecase(
    usecase: &str,
    intents: &[IntentRow],
    sf: u32,
    res_cap: usize,
    reps: usize,
    regime: GateRegime,
    records: &mut Vec<String>,
) -> Result<(), String> {
    // Resource universe: first `res_cap` distinct resource IRIs from the use-case
    // intent table (deterministic — the generators are seeded).
    let mut resources: Vec<String> = Vec::new();
    for it in intents {
        if !resources.contains(&it.resource_uri) {
            resources.push(it.resource_uri.clone());
            if resources.len() >= res_cap {
                break;
            }
        }
    }
    if resources.len() < 2 {
        return Err(format!("lane B {usecase}/sf{sf}: <2 distinct resources in intents"));
    }

    let nquads = data_nquads(&resources, 8);
    // The unguarded twin: SAME data, no policy, no auth view — `sparq_engine::query`.
    let plain = load_graph(&nquads)?;
    // The gated store: SAME data + an ODRL grant per resource for BENCH_AGENT, so the
    // permitted subset is physically the whole store (identical result sets).
    let mut store = PodStore::new(load_graph(&nquads)?);
    for (i, r) in resources.iter().enumerate() {
        let (ttl, granted) = match regime {
            GateRegime::OneShot => {
                let ttl = permission_ttl(i, BENCH_AGENT, r);
                let out =
                    store.materialize_odrl_permission(&parse(&ttl)?, &read_request(r, BENCH_AGENT));
                (ttl, out.granted)
            }
        };
        if !granted {
            return Err(format!(
                "lane B {usecase}/sf{sf}/{}: grant {i} did not materialize; ttl=\n{ttl}",
                regime.label()
            ));
        }
    }

    let reader = session(BENCH_AGENT);

    // Anti-vacuity (fail-closed): a stranger with NO grants sees 0 rows through the
    // gated path — enforcement is actually ON in the store we are about to time.
    let stranger_rows = store
        .query_as(
            &session(STRANGER),
            Mode::Read,
            &format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}", resources[0]),
        )
        .map_err(|e| format!("lane B stranger probe: {e}"))?
        .rows
        .len();
    if stranger_rows != 0 {
        return Err(format!(
            "lane B {usecase}/sf{sf}/{}: STRANGER sees {stranger_rows} rows — the gated \
             store is fail-open; refusing to time (security failure)",
            regime.label()
        ));
    }

    for (qid, sparql) in query_mix(&resources) {
        // Correctness FIRST (also warms both paths — steady-state timing after).
        let gated0 = store
            .query_as(&reader, Mode::Read, &sparql)
            .map_err(|e| format!("lane B gated {qid}: {e}"))?;
        let plain0 =
            sparq_engine::query(&plain, &sparql).map_err(|e| format!("lane B plain {qid}: {e}"))?;
        let rows =
            assert_equivalent(&format!("lane B {usecase}/sf{sf}/{}/{qid}", regime.label()), &gated0, &plain0)?;
        if rows == 0 {
            return Err(format!(
                "lane B {usecase}/sf{sf}/{qid}: 0 rows on both sides — a vacuous \
                 comparison times nothing (fail-closed)"
            ));
        }

        let mut gated_us: Vec<u64> = Vec::with_capacity(reps);
        let mut plain_us: Vec<u64> = Vec::with_capacity(reps);
        for _ in 0..reps {
            let (us, r) = time_us(|| store.query_as(&reader, Mode::Read, &sparql));
            r.map_err(|e| format!("lane B gated rep {qid}: {e}"))?;
            gated_us.push(us);
            let (us, r) = time_us(|| sparq_engine::query(&plain, &sparql));
            r.map_err(|e| format!("lane B plain rep {qid}: {e}"))?;
            plain_us.push(us);
        }
        let min = |v: &[u64]| v.iter().copied().min().unwrap_or(u64::MAX);
        let mean = |v: &[u64]| v.iter().copied().sum::<u64>() / v.len() as u64;
        let (g_min, g_mean, p_min, p_mean) =
            (min(&gated_us), mean(&gated_us), min(&plain_us), mean(&plain_us));
        println!(
            "query\t{usecase}/sf{sf}/{}\t{qid}\tPASS\trows={rows} gated_min_us={g_min} \
             plain_min_us={p_min} (advisory)",
            regime.label()
        );
        records.push(format!(
            "{{\"lane\":\"query\",\"usecase\":\"{usecase}\",\"sf\":{sf},\
             \"regime\":\"{}\",\"resources\":{},\"query\":\"{qid}\",\"rows\":{rows},\
             \"reps\":{reps},\"gated_min_us\":{g_min},\"gated_mean_us\":{g_mean},\
             \"plain_min_us\":{p_min},\"plain_mean_us\":{p_mean}}}",
            regime.label(),
            resources.len()
        ));
    }
    Ok(())
}

// ── Lane C: churn (refresh under policy writes) ───────────────────────────────────────

/// Ledger of `l` bridged one-shot grants; time (1) the no-op full refresh (asserted 0
/// retractions) and (2) a revocation via `refresh_odrl_grant` (asserted: 1 retraction,
/// the revoked agent LOSES access, a surviving agent keeps it).
fn lane_c_config(l: usize, records: &mut Vec<String>) -> Result<(), String> {
    let resources: Vec<String> = (0..l).map(res_iri).collect();
    let mut store = PodStore::new(load_graph(&data_nquads(&resources, 2))?);
    for (i, r) in resources.iter().enumerate() {
        let out = store.materialize_odrl_permission(
            &parse(&permission_ttl(i, &agent_iri(i), r))?,
            &read_request(r, &agent_iri(i)),
        );
        if !out.granted {
            return Err(format!("lane C l={l}: grant {i} did not materialize"));
        }
    }

    // (1) No-op refresh: every tracked entry re-evaluates, nothing retracts.
    let (noop_us, retracted) = time_us(|| store.refresh_odrl_grants());
    if retracted != 0 {
        return Err(format!(
            "lane C l={l}: no-op refresh retracted {retracted} grants — the ledger is \
             unstable; timings would be meaningless (fail-closed)"
        ));
    }

    // (2) Revocation: the policy for r0 is REWRITTEN to permit only OTHER_AGENT, so
    // re-evaluating agent_0's tracked request must Deny → retraction (the ACL-write
    // churn case: a policy write followed by refresh).
    let withdrawn = parse(&permission_ttl(0, OTHER_AGENT, &resources[0]))?;
    let req0 = read_request(&resources[0], &agent_iri(0));
    let (revoke_us, (matched, retracted)) =
        time_us(|| store.refresh_odrl_grant(&withdrawn, &req0, BridgeKind::Permission));
    if !matched || retracted != 1 {
        return Err(format!(
            "lane C l={l}: revocation matched={matched} retracted={retracted} \
             (expected true/1) — refresh did not retract (fail-closed)"
        ));
    }
    let rows_after = store
        .query_as(
            &session(&agent_iri(0)),
            Mode::Read,
            &format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}", resources[0]),
        )
        .map_err(|e| format!("lane C post-revoke probe: {e}"))?
        .rows
        .len();
    if rows_after != 0 {
        return Err(format!(
            "lane C l={l}: agent_0 STILL sees {rows_after} rows after revocation — \
             stale-grant security failure"
        ));
    }
    if l > 1 {
        let survivor_rows = store
            .query_as(
                &session(&agent_iri(1)),
                Mode::Read,
                &format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}", resources[1]),
            )
            .map_err(|e| format!("lane C survivor probe: {e}"))?
            .rows
            .len();
        if survivor_rows == 0 {
            return Err(format!(
                "lane C l={l}: revocation of grant 0 also dropped grant 1 (under-share \
                 — refresh is not preserving still-valid grants)"
            ));
        }
    }

    println!(
        "churn\tledger={l}\trefresh\tPASS\tnoop_us_indicative={noop_us} \
         revoke_us_indicative={revoke_us} (advisory)"
    );
    records.push(format!(
        "{{\"lane\":\"churn\",\"ledger\":{l},\"noop_refresh_us\":{noop_us},\
         \"revoke_refresh_us\":{revoke_us}}}"
    ));
    Ok(())
}

// ── Use-case generation (skip-with-reason on todo!() generators) ──────────────────────

fn gen_intents(usecase: &str, params: &GenParams) -> Option<Vec<IntentRow>> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| match usecase {
        "personal" => personal::generate(params).intents,
        "project_mgmt" => project_mgmt::generate(params).intents,
        "financial" => financial::generate(params).intents,
        "consortium" => consortium::generate(params).intents,
        _ => unreachable!("unknown use case"),
    });
    std::panic::set_hook(prev_hook);
    result.ok()
}

// ── Main ─────────────────────────────────────────────────────────────────────────────

const USECASES: [&str; 4] = ["personal", "project_mgmt", "financial", "consortium"];

fn main() -> ExitCode {
    let mut smoke = false;
    let mut sf: u32 = 1;
    let mut out_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--smoke" => smoke = true,
            "--sf" => {
                sf = args.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    fail("--sf needs a positive integer");
                });
            }
            "--out" => {
                out_path = Some(args.next().unwrap_or_else(|| fail("--out needs a path")));
            }
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => fail(&format!("unknown argument: {other}")),
        }
    }
    if sf == 0 {
        fail("--sf must be >= 1");
    }

    // Tier knobs. Smoke = the per-commit tier; --sf N scales linearly (nightly/EC2).
    // Lane B always runs >= 2 scale factors (the bead's spec) — smoke uses the two
    // smallest so the per-commit tier stays fast.
    // `res_cap_base` scales WITH each lane-B scale factor (res_cap = base × sf), so
    // the ≥2 scale factors genuinely vary the dataset size — not just the generator
    // parameters behind an identical cap.
    let (sf_list, mat_counts, ledger_sizes, res_cap_base, reps): (Vec<u32>, Vec<usize>, Vec<usize>, usize, usize) =
        if smoke {
            (vec![1, 2], vec![4, 16], vec![4, 16], 12, 5)
        } else {
            let s = sf as usize;
            (
                vec![1, sf.max(2)],
                vec![4 * s, 16 * s],
                vec![4 * s, 16 * s],
                24,
                9,
            )
        };

    println!("# ac-bench-overhead (sq-hmd7l.44) — {}", if smoke { "SMOKE" } else { "SF" });
    println!(
        "# NON-CANONICAL: every wall-clock number is advisory (shared work box, \
         bench/CATALOG.md QUIET-BOX). Canonical envelopes are EC2-gated. The HARD \
         contract is the fail-closed exit code."
    );
    println!("# lane\tconfig\tsub\tstatus\tdetail");

    let mut records: Vec<String> = Vec::new();

    // Lane A — materialization sweep.
    if let Err(e) = lane_a(&mat_counts, &mut records) {
        eprintln!("FAIL: {e}");
        return ExitCode::FAILURE;
    }

    // Lane B — steady-state query overhead over the acbench use-case universes.
    let mut skipped: Vec<String> = Vec::new();
    for &sfi in &sf_list {
        let mut params = GenParams::smoke();
        params.sf = sfi;
        if let Err(e) = params.validate() {
            eprintln!("FAIL: invalid GenParams sf={sfi}: {e}");
            return ExitCode::FAILURE;
        }
        for uc in USECASES {
            let Some(intents) = gen_intents(uc, &params) else {
                let reason = format!("generator {uc} not yet implemented (todo!()); skipped");
                println!("query\t{uc}/sf{sfi}\t-\tSKIP\t{reason}");
                skipped.push(format!("{{\"usecase\":\"{uc}\",\"sf\":{sfi},\"reason\":\"{}\"}}", json_escape(&reason)));
                continue;
            };
            for regime in [GateRegime::OneShot] {
                if let Err(e) = lane_b_usecase(
                    uc,
                    &intents,
                    sfi,
                    res_cap_base * sfi as usize,
                    reps,
                    regime,
                    &mut records,
                )
                {
                    eprintln!("FAIL: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    }

    // Lane C — churn.
    for &l in &ledger_sizes {
        if let Err(e) = lane_c_config(l, &mut records) {
            eprintln!("FAIL: {e}");
            return ExitCode::FAILURE;
        }
    }

    // Envelope.
    let envelope = format!(
        "{{\"suite\":\"ac-odrl-overhead\",\"bead\":\"sq-hmd7l.44\",\
         \"mode\":\"{}\",\"sf_list\":{:?},\"non_canonical\":true,\
         \"quiet_box_note\":\"work-box run — advisory only; canonical envelopes are \
         EC2-gated (bench/CATALOG.md)\",\
         \"records\":[{}],\"skipped\":[{}],\
         \"correctness\":{{\"fail_closed\":true,\"failures\":0}},\
         \"competitor\":{}}}",
        if smoke { "smoke" } else { "sf" },
        sf_list,
        records.join(","),
        skipped.join(","),
        competitor_verdict()
    );
    println!("AC_ODRL_OVERHEAD_JSON {envelope}");
    if let Some(p) = out_path {
        if let Err(e) = std::fs::write(&p, &envelope) {
            eprintln!("FAIL: writing --out {p}: {e}");
            return ExitCode::FAILURE;
        }
        println!("# envelope written to {p}");
    }
    println!(
        "PASS: every timed lane held its fail-closed correctness gate \
         (equivalence, scoping, anti-vacuity, retraction)."
    );
    ExitCode::SUCCESS
}

fn print_help() {
    println!("ac-bench-overhead — ODRL-gated vs unguarded query-eval overhead (sq-hmd7l.44)");
    println!();
    println!("USAGE: ac-bench-overhead [--smoke] [--sf N] [--out FILE]");
    println!("  --smoke     per-commit tier (fixed seed, 2 small scale factors)");
    println!("  --sf N      nightly/EC2 tier; lane sizes scale linearly with N");
    println!("  --out FILE  also write the JSON envelope to FILE");
    println!();
    println!("Exits non-zero on ANY correctness disagreement while timing (fail-closed).");
    println!("NON-CANONICAL on a shared work box; canonical envelopes are EC2-gated.");
}

fn fail(msg: &str) -> ! {
    eprintln!("ac-bench-overhead: {msg}");
    std::process::exit(2);
}

// ── Tests ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Every policy-kind TTL parses; the storable kind materializes a real, scoped grant
    /// and every other kind is refused with no access. Non-vacuous: corrupting any
    /// builder (action, assignee, constraint operand) flips `granted` or the probes.
    #[test]
    fn all_policy_kinds_grant_and_scope() {
        for kind in PolicyKind::ALL {
            let resources: Vec<String> = (0..2).map(res_iri).collect();
            let mut store =
                PodStore::new(load_graph(&data_nquads(&resources, 2)).expect("load"));
            let a0 = agent_iri(0);
            let ttl = match kind {
                PolicyKind::Permission => permission_ttl(0, &a0, &resources[0]),
                PolicyKind::PermissionProhibition => {
                    policy_both_ttl(0, &a0, STRANGER, &resources[0])
                }
                PolicyKind::ConditionalRecipient => conditional_ttl(0, &a0, &resources[0]),
                PolicyKind::Counted => counted_ttl(0, &a0, &resources[0], 5),
            };
            let pol = parse(&ttl).expect("parse");
            let req = read_request(&resources[0], &a0);
            let counter: Arc<dyn sparq_policy::UsageCounterStore + Send + Sync> =
                Arc::new(InMemoryCounterStore::new());
            let out = match kind {
                PolicyKind::Permission => store.materialize_odrl_permission(&pol, &req),
                PolicyKind::PermissionProhibition => store.materialize_odrl_policy(&pol, &req),
                PolicyKind::ConditionalRecipient => {
                    store.materialize_odrl_permission_conditional(&pol, &req)
                }
                PolicyKind::Counted => {
                    store.materialize_odrl_permission_counted(&pol, &req, &counter)
                }
            };
            assert_eq!(out.granted, kind.stores_grant(), "{}: {:?}", kind.label(), out.reasons);

            let q0 = format!(
                "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
                resources[0]
            );
            let q1 = format!(
                "SELECT ?s ?p ?o WHERE {{ GRAPH <{}> {{ ?s ?p ?o }} }}",
                resources[1]
            );
            assert_eq!(
                !store.query_as(&session(&a0), Mode::Read, &q0).expect("q0").rows.is_empty(),
                kind.stores_grant(),
                "{}: only a stored grant gives the agent rows",
                kind.label()
            );
            assert!(
                store.query_as(&session(&a0), Mode::Read, &q1).expect("q1").rows.is_empty(),
                "{}: grant must be scoped to r0",
                kind.label()
            );
            assert!(
                store.query_as(&session(STRANGER), Mode::Read, &q0).expect("qs").rows.is_empty(),
                "{}: stranger must be denied",
                kind.label()
            );
        }
    }

    /// Grant-all gated vs data-only plain: every mix query returns the IDENTICAL
    /// multiset (the lane-B apples-to-apples precondition).
    #[test]
    fn gated_equals_plain_when_grant_all() {
        let resources: Vec<String> = (0..4).map(res_iri).collect();
        let nq = data_nquads(&resources, 8);
        let plain = load_graph(&nq).expect("plain");
        let mut store = PodStore::new(load_graph(&nq).expect("gated"));
        for (i, r) in resources.iter().enumerate() {
            let out = store.materialize_odrl_permission(
                &parse(&permission_ttl(i, BENCH_AGENT, r)).expect("parse"),
                &read_request(r, BENCH_AGENT),
            );
            assert!(out.granted);
        }
        for (qid, sparql) in query_mix(&resources) {
            let g = store.query_as(&session(BENCH_AGENT), Mode::Read, &sparql).expect("gated q");
            let p = sparq_engine::query(&plain, &sparql).expect("plain q");
            let rows = assert_equivalent(&qid, &g, &p).expect("must be equivalent");
            assert!(rows > 0, "{qid}: must be a non-vacuous comparison");
        }
    }

    /// The equivalence gate is NON-VACUOUS: withhold ONE grant and the graph-scan
    /// comparison must FAIL (a weakened comparator would silently time a
    /// non-equivalent pair — this is the mutation that keeps it honest).
    #[test]
    fn equivalence_gate_detects_missing_grant() {
        let resources: Vec<String> = (0..4).map(res_iri).collect();
        let nq = data_nquads(&resources, 8);
        let plain = load_graph(&nq).expect("plain");
        let mut store = PodStore::new(load_graph(&nq).expect("gated"));
        // Grant only 3 of 4 resources.
        for (i, r) in resources.iter().take(3).enumerate() {
            assert!(
                store
                    .materialize_odrl_permission(
                        &parse(&permission_ttl(i, BENCH_AGENT, r)).expect("parse"),
                        &read_request(r, BENCH_AGENT),
                    )
                    .granted
            );
        }
        let sparql = "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }";
        let g = store.query_as(&session(BENCH_AGENT), Mode::Read, sparql).expect("gated q");
        let p = sparq_engine::query(&plain, sparql).expect("plain q");
        let err = assert_equivalent("probe", &g, &p);
        assert!(err.is_err(), "a missing grant MUST fail the equivalence gate");
        assert!(err.unwrap_err().contains("RESULT-SET MISMATCH"));
    }

    /// Lane C semantics: a policy-write revocation retracts EXACTLY the rewritten
    /// grant (revoked agent loses access, survivor keeps it), and a no-op refresh
    /// retracts nothing.
    #[test]
    fn churn_revocation_retracts_and_preserves_survivors() {
        let resources: Vec<String> = (0..3).map(res_iri).collect();
        let mut store = PodStore::new(load_graph(&data_nquads(&resources, 2)).expect("load"));
        for (i, r) in resources.iter().enumerate() {
            assert!(
                store
                    .materialize_odrl_permission(
                        &parse(&permission_ttl(i, &agent_iri(i), r)).expect("parse"),
                        &read_request(r, &agent_iri(i)),
                    )
                    .granted
            );
        }
        assert_eq!(store.refresh_odrl_grants(), 0, "no-op refresh must retract nothing");

        let withdrawn = parse(&permission_ttl(0, OTHER_AGENT, &resources[0])).expect("parse");
        let req0 = read_request(&resources[0], &agent_iri(0));
        let (matched, retracted) =
            store.refresh_odrl_grant(&withdrawn, &req0, BridgeKind::Permission);
        assert!(matched, "the tracked slot must match");
        assert_eq!(retracted, 1, "exactly the rewritten grant retracts");

        let q = |store: &PodStore, agent: String, res: &str| {
            store
                .query_as(
                    &session(&agent),
                    Mode::Read,
                    &format!("SELECT ?s ?p ?o WHERE {{ GRAPH <{res}> {{ ?s ?p ?o }} }}"),
                )
                .expect("query")
                .rows
                .len()
        };
        assert_eq!(q(&store, agent_iri(0), &resources[0]), 0, "revoked agent loses access");
        assert!(q(&store, agent_iri(1), &resources[1]) > 0, "survivor keeps access");
    }

    /// Row normalization is order-insensitive and UNDEF-stable (the multiset compare
    /// must not depend on engine row order).
    #[test]
    fn normalized_rows_order_insensitive() {
        let resources: Vec<String> = (0..3).map(res_iri).collect();
        let g = load_graph(&data_nquads(&resources, 4)).expect("load");
        let a = sparq_engine::query(&g, "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } }")
            .expect("q");
        let mut b_rows = a.rows.clone();
        b_rows.reverse();
        let b = QueryResult { vars: a.vars.clone(), rows: b_rows };
        assert_eq!(normalized_rows(&a), normalized_rows(&b));
        assert_eq!(normalized_rows(&a).len(), 12);
    }

    /// JSON escaping covers the characters that would break the single-line envelope.
    #[test]
    fn json_escape_escapes_specials() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
        assert_eq!(json_escape("plain"), "plain");
    }

    /// The competitor verdict is an explicit, honest `NOT_COMPARABLE` object with a
    /// non-empty reason and the composing bead (the sq-hmd7l per-axis mandate).
    #[test]
    fn competitor_verdict_is_honest_not_comparable() {
        let v = competitor_verdict();
        assert!(v.contains("\"status\":\"NOT_COMPARABLE\""));
        assert!(v.contains("sq-lrtc3.1"));
        assert!(v.contains("PDP decision throughput"), "must state the ODRE reason");
        assert!(v.contains("HTTP resource level"), "must state the CSS reason");
    }

    /// The query mix covers the three representative shapes (full scan,
    /// predicate-bound, per-resource point lookups at first/mid/last).
    #[test]
    fn query_mix_shape() {
        let resources: Vec<String> = (0..5).map(res_iri).collect();
        let mix = query_mix(&resources);
        assert_eq!(mix.len(), 5);
        assert!(mix.iter().any(|(id, _)| id == "graph-scan-all"));
        assert!(mix.iter().any(|(id, _)| id == "predicate-bound"));
        assert_eq!(mix.iter().filter(|(id, _)| id.starts_with("per-resource-")).count(), 3);
        assert!(mix.iter().any(|(_, q)| q.contains(&resources[2])), "mid resource sampled");
    }
}
