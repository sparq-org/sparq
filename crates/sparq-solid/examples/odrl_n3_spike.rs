//! [OPUS-4.8] sq-zgbso.1 — MEASURED SPIKE: ODRL permission evaluation as N3 inference
//! rules vs the hand-written Rust path. NOT production wiring; no public API; no
//! default-build change (this example is gated behind the default-OFF `odrl-bridge`
//! feature). Design record: `research/odrl-n3-compiled-rules.md` (epic sq-zgbso, #1582).
//!
//! Run:  cargo run -p sparq-solid --example odrl_n3_spike --features odrl-bridge --release
//!
//! What it does (the four things the design record §6.1 asks the spike to measure):
//!   1. DECISION DIFFERENTIAL — for ONE representative ODRL permission-evaluation case
//!      (a read permission gated by an `odrl:dateTime lteq` window, plus a deny-overrides
//!      prohibition), across allow + fail-closed-deny + deny-overrides inputs, the
//!      `auth:*` triple SET derived by the N3 rules (`rules/odrl-spike.n3` via
//!      `sparq_reason::reason_n3`) MUST equal the SET materialised by the real Rust path
//!      (`sparq_solid::odrl_bridge::materialize_policy` over `sparq_policy::evaluate`).
//!      The invariant is decision equality: ANY divergence exits non-zero (fail loudly).
//!   2. TIMING RATIO — N3 evaluation vs Rust evaluation on the same case.
//!   3. PARSE-FRACTION PROFILE — of the EXISTING WAC/ACP materialize pipeline (the
//!      headroom the design's build-time-compilation option would target).
//!   4. BUILD-SIZE — the byte cost of carrying the N3 rules at the parse-at-runtime
//!      baseline, with the honest note on what build-time compilation would/would not
//!      remove for sparq-solid.
//!
//! ALL wall-clock figures are best-of-N on THIS work box and are explicitly
//! NON-CANONICAL (EC2 execution-env protocol): they belong in the spike report / bead
//! comment, never in docs, tests, or a perf baseline. The DIFFERENTIAL is the load-bearing
//! deterministic result; the timings only inform the GO/NO-GO judgement.

use oxrdf::Term;
use sparq_core::dict::Dict;
use sparq_core::Graph;
use sparq_policy::{evaluate, parse_policy_str, Request, ValidatedPolicy};
use sparq_reason::n3::parser;
use sparq_reason::reason_n3;
use sparq_solid::odrl_bridge::materialize_policy;
use sparq_solid::{acp_fixture, wac_fixture, PodStore, AUTH_NS};
use std::time::Instant;

/// The spike rules under test, embedded exactly as WAC/ACP embed theirs (`include_str!`).
const RULES: &str = include_str!("../rules/odrl-spike.n3");
/// The existing WAC/ACP rule text, for the parse-fraction profile (§6.1 item 3).
const COMMON: &str = include_str!("../rules/common.n3");
const WAC: &str = include_str!("../rules/wac.n3");
const ACP_A: &str = include_str!("../rules/acp-a.n3");
const ACP_B: &str = include_str!("../rules/acp-b.n3");
const ACP_C: &str = include_str!("../rules/acp-c.n3");

const ODRL: &str = "http://www.w3.org/ns/odrl/2/";
const ALICE: &str = "urn:alice";
const BOB: &str = "urn:bob";
const TARGET: &str = "urn:t/1";

/// A sorted set of `auth:*` triples, as `(subject, predicate, object)` IRI strings — the
/// canonical form both paths reduce to for the differential.
type AuthSet = Vec<(String, String, String)>;

/// One representative scenario: the SAME policy text feeds both paths; the request is
/// built two ways (a Rust `Request`, an N3 request fact) from the same fields.
struct Scenario {
    name: &'static str,
    policy_ttl: &'static str,
    action_local: &'static str,
    target: &'static str,
    party: &'static str,
    /// canonical UTC `...Z` request instant, or `None` for "no time supplied".
    at: Option<&'static str>,
    /// The expected decision, asserted independently so a BOTH-WRONG agreement can't pass.
    expect: &'static [(&'static str, &'static str, &'static str)],
}

// ── the RUST path (real: parse -> evaluate -> materialize_policy into a Graph) ─────────

fn rust_auth_set(s: &Scenario) -> AuthSet {
    let policy: ValidatedPolicy = parse_policy_str(s.policy_ttl, "turtle").expect("policy parses");
    let mut req = Request::new(format!("{}{}", ODRL, s.action_local))
        .on(s.target)
        .by(s.party);
    if let Some(t) = s.at {
        req = req.at(t);
    }
    // Exercise the REAL bridge: materialize both sides into a fresh dataset's auth view.
    let mut graph = Graph::new();
    let outcome = materialize_policy(&mut graph, &policy, &req);
    let mut set: AuthSet = Vec::new();
    if let Some(t) = outcome.grant_triple {
        set.push(t);
    }
    if let Some(t) = outcome.deny_triple {
        set.push(t);
    }
    set.sort();
    set
}

// ── the N3 path (real: assemble N3 -> reason_n3 -> filter the auth view) ───────────────

/// Assemble the N3 reasoning source for a scenario: prefixes + the SAME policy text +
/// the request as an `odrl:Request` fact + the spike rules.
fn n3_source(s: &Scenario) -> String {
    let at = match s.at {
        Some(t) => format!(" ; spike:atTime \"{}\"^^xsd:dateTime", t),
        None => String::new(),
    };
    let request = format!(
        "<urn:req> a odrl:Request ; odrl:action odrl:{} ; odrl:target <{}> ; odrl:assignee <{}>{} .",
        s.action_local, s.target, s.party, at
    );
    format!(
        "@prefix odrl: <{}> .\n@prefix spike: <https://sparq.dev/ns/odrl-spike#> .\n\
         @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n{}\n{}\n{}",
        ODRL, s.policy_ttl, request, RULES
    )
}

fn n3_auth_set(s: &Scenario) -> AuthSet {
    let mut dict = Dict::new();
    let closure = reason_n3(&mut dict, &n3_source(s)).expect("N3 reasons");
    let mut set: AuthSet = Vec::new();
    for t in &closure {
        let p = dict.term(t[1]);
        let Term::NamedNode(pred) = &p else { continue };
        if !pred.as_str().starts_with(AUTH_NS) {
            continue;
        }
        let (Term::NamedNode(subj), Term::NamedNode(obj)) = (dict.term(t[0]), dict.term(t[2])) else {
            continue;
        };
        set.push((
            subj.as_str().to_owned(),
            pred.as_str().to_owned(),
            obj.as_str().to_owned(),
        ));
    }
    set.sort();
    set
}

fn expected_set(s: &Scenario) -> AuthSet {
    let mut set: AuthSet = s
        .expect
        .iter()
        .map(|(a, m, t)| {
            (
                (*a).to_owned(),
                format!("{}{}", AUTH_NS, m),
                (*t).to_owned(),
            )
        })
        .collect();
    set.sort();
    set
}

fn fmt_set(set: &AuthSet) -> String {
    if set.is_empty() {
        return "{}".to_owned();
    }
    let items: Vec<String> = set
        .iter()
        .map(|(s, p, o)| format!("{} {} {}", s, p.rsplit('#').next().unwrap_or(p), o))
        .collect();
    format!("{{ {} }}", items.join(" | "))
}

// ── timing helper (best-of-N; inner loop for sub-microsecond ops) ─────────────────────

fn best_ns(iters_outer: u32, iters_inner: u32, mut f: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..iters_outer {
        let t = Instant::now();
        for _ in 0..iters_inner {
            f();
        }
        let per = t.elapsed().as_nanos() as f64 / iters_inner as f64;
        best = best.min(per);
    }
    best
}

fn main() {
    let scenarios: &[Scenario] = &[
        Scenario {
            name: "A1  alice read, within dateTime window        -> ALLOW",
            policy_ttl: POLICY_A,
            action_local: "read",
            target: TARGET,
            party: ALICE,
            at: Some("2026-07-05T00:00:00Z"),
            expect: &[(ALICE, "read", TARGET)],
        },
        Scenario {
            name: "A2  alice read, AFTER window                  -> DENY (constraint)",
            policy_ttl: POLICY_A,
            action_local: "read",
            target: TARGET,
            party: ALICE,
            at: Some("2027-01-01T00:00:00Z"),
            expect: &[],
        },
        Scenario {
            name: "A3  alice read, no time supplied             -> DENY (unprovable)",
            policy_ttl: POLICY_A,
            action_local: "read",
            target: TARGET,
            party: ALICE,
            at: None,
            expect: &[],
        },
        Scenario {
            name: "A4  bob read, within window                  -> DENY (assignee)",
            policy_ttl: POLICY_A,
            action_local: "read",
            target: TARGET,
            party: BOB,
            at: Some("2026-07-05T00:00:00Z"),
            expect: &[],
        },
        Scenario {
            name: "A5  alice write, within window               -> DENY (action)",
            policy_ttl: POLICY_A,
            action_local: "write",
            target: TARGET,
            party: ALICE,
            at: Some("2026-07-05T00:00:00Z"),
            expect: &[],
        },
        Scenario {
            name: "B1  alice read, unconstrained permission     -> ALLOW",
            policy_ttl: POLICY_B,
            action_local: "read",
            target: TARGET,
            party: ALICE,
            at: None,
            expect: &[(ALICE, "read", TARGET)],
        },
        Scenario {
            name: "C1  alice read, permission + prohibition     -> DENY-OVERRIDES",
            policy_ttl: POLICY_C,
            action_local: "read",
            target: TARGET,
            party: ALICE,
            at: None,
            expect: &[(ALICE, "denyRead", TARGET)],
        },
    ];

    println!("== sq-zgbso.1 ODRL-as-N3 spike ==  (work-box measurements are NON-CANONICAL)\n");
    println!("--- 1. DECISION DIFFERENTIAL (real Rust path vs real N3 path) ---");
    let mut divergences = 0u32;
    for s in scenarios {
        let rust = rust_auth_set(s);
        let n3 = n3_auth_set(s);
        let expect = expected_set(s);
        let agree = rust == n3;
        let correct = rust == expect && n3 == expect;
        let tag = if agree && correct {
            "OK"
        } else if agree {
            "AGREE-BUT-WRONG"
        } else {
            "DIVERGENCE"
        };
        println!("  [{}] {}", tag, s.name);
        if !(agree && correct) {
            divergences += 1;
            println!("        rust   = {}", fmt_set(&rust));
            println!("        n3     = {}", fmt_set(&n3));
            println!("        expect = {}", fmt_set(&expect));
        }
    }
    println!(
        "  => {} scenario(s), {} divergence(s)\n",
        scenarios.len(),
        divergences
    );

    // 2. TIMING RATIO on the representative allow case (A1).
    println!("--- 2. TIMING: N3 evaluation vs Rust evaluation (best-of-N, work box) ---");
    let a1 = &scenarios[0];
    let policy = parse_policy_str(a1.policy_ttl, "turtle").unwrap();
    let req = Request::new(format!("{}read", ODRL))
        .on(a1.target)
        .by(a1.party)
        .at("2026-07-05T00:00:00Z");
    let src = n3_source(a1);
    let t_eval = best_ns(20, 20_000, || {
        let _ = evaluate(&policy, &req);
    });
    let t_rust_text = best_ns(20, 2_000, || {
        let p = parse_policy_str(a1.policy_ttl, "turtle").unwrap();
        let _ = evaluate(&p, &req);
    });
    let t_n3 = best_ns(20, 200, || {
        let mut d = Dict::new();
        let _ = reason_n3(&mut d, &src).unwrap();
    });
    let t_n3_parse = best_ns(20, 400, || {
        let _ = parser::parse(&src).unwrap();
    });
    println!("  Rust evaluate (policy pre-parsed)      {:>12.0} ns/op", t_eval);
    println!("  Rust parse_policy_str + evaluate       {:>12.0} ns/op", t_rust_text);
    println!("  N3   reason_n3 (parse + fixpoint)      {:>12.0} ns/op", t_n3);
    println!("  N3   parser::parse only                {:>12.0} ns/op", t_n3_parse);
    println!(
        "  ratios: N3/Rust(pre-parsed) = {:.0}x ; N3/Rust(from-text) = {:.1}x ; N3-parse share = {:.0}%\n",
        t_n3 / t_eval,
        t_n3 / t_rust_text,
        100.0 * t_n3_parse / t_n3
    );

    // 3. PARSE-FRACTION PROFILE of the EXISTING WAC/ACP materialize pipeline.
    println!("--- 3. PARSE-FRACTION PROFILE of the existing WAC/ACP materialize ---");
    let wac_rules = format!("{}\n{}", COMMON, WAC);
    let acp_rules = format!("{}\n{}\n{}\n{}", COMMON, ACP_A, ACP_B, ACP_C);

    let wac_graph = Graph::load_dataset(&wac_fixture(), "nquads").unwrap();
    let n_wac = wac_graph.named.len();
    let t_wac_full = best_ns(5, 1, || {
        // Fresh PodStore per iteration so each measures a cold full materialize (the real
        // `materialize_wac` path), not a re-run over an already-populated auth view.
        let mut store = PodStore::new(Graph::load_dataset(&wac_fixture(), "nquads").unwrap());
        let _ = store.materialize_wac().unwrap();
    }) / 1e6;

    let acp_graph = Graph::load_dataset(&acp_fixture(), "nquads").unwrap();
    let n_acp = acp_graph.named.len();
    let t_acp_full = best_ns(5, 1, || {
        let mut store = PodStore::new(Graph::load_dataset(&acp_fixture(), "nquads").unwrap());
        let _ = store.materialize_acp().unwrap();
    }) / 1e6;

    let t_wac_ruleparse = best_ns(20, 100, || {
        let _ = parser::parse(&wac_rules).unwrap();
    }) / 1e6;
    let t_acp_ruleparse = best_ns(20, 100, || {
        let _ = parser::parse(&acp_rules).unwrap();
    }) / 1e6;
    // rules-only closure (parse + trivial empty fixpoint) — the fixed floor.
    let t_wac_rulesonly = best_ns(20, 100, || {
        let mut d = Dict::new();
        let _ = reason_n3(&mut d, &wac_rules).unwrap();
    }) / 1e6;

    println!("  WAC fixture: {} named graphs; ACP fixture: {} named graphs", n_wac, n_acp);
    println!("  WAC materialize (full pipeline)        {:>10.3} ms", t_wac_full);
    println!("  ACP materialize (full pipeline, 3x)    {:>10.3} ms", t_acp_full);
    println!("  parse WAC rule text only               {:>10.3} ms", t_wac_ruleparse);
    println!("  parse ACP rule text only               {:>10.3} ms", t_acp_ruleparse);
    println!("  reason_n3 WAC rules-only (no facts)    {:>10.3} ms", t_wac_rulesonly);
    println!(
        "  => rule-parse share of materialize: WAC ~{:.2}% , ACP ~{:.2}%",
        100.0 * t_wac_ruleparse / t_wac_full,
        100.0 * t_acp_ruleparse / t_acp_full
    );
    println!(
        "     (item-3 'parse the rules once' is small; the design's candidate win is the\n\
         \x20     per-call FACT round-trip (items 1-2), which the public API cannot isolate\n\
         \x20     from the fixpoint here — isolating it needs the in-crate instrument of sq-zgbso.3/.4.)\n"
    );

    // 4. BUILD-SIZE of carrying the N3 rules (parse-at-runtime baseline).
    println!("--- 4. BUILD-SIZE delta of carrying the N3 rules (parse-at-runtime baseline) ---");
    println!("  odrl-spike.n3 embedded rule text       {:>6} bytes", RULES.len());
    println!("  (for scale) WAC rule text              {:>6} bytes", wac_rules.len());
    println!("  (for scale) ACP rule text              {:>6} bytes", acp_rules.len());
    println!(
        "  NOTE: the N3 parser is ALREADY linked into sparq-solid (WAC/ACP call reason_n3),\n\
         \x20     so embedding ODRL rules adds only the rule TEXT bytes to the sparq-solid\n\
         \x20     artifact, NOT a parser. The design ledger's 'parser leaves the runtime path'\n\
         \x20     saving (item 4) therefore does NOT apply to sparq-solid; the compiled-IR vs\n\
         \x20     rule-text size ledger is measured for real at sq-zgbso.4's build.rs.\n"
    );

    if divergences > 0 {
        eprintln!(
            "SPIKE FAILED: {} decision divergence(s) — the N3 path is NOT equivalent to the Rust path.",
            divergences
        );
        std::process::exit(1);
    }
    println!("SPIKE OK: N3 and Rust agree on every scenario decision; timings above are non-canonical.");
}

// ── the ONE representative policy, in three variants (same target/assignee) ────────────

const POLICY_A: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/a> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
                      odrl:rightOperand "2026-12-31T00:00:00Z"^^xsd:dateTime ] ] ."#;

const POLICY_B: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/b> a odrl:Set ; odrl:permission [
    odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;

const POLICY_C: &str = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/c> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:t/1> ; odrl:assignee <urn:alice> ] ."#;
