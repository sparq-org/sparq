//! `ac-odre-export` — the sparq-side half of the ODRE decision-agreement lane
//! (bead `sq-i6du2.8`, epic `sq-i6du2`, issue #1613).
//!
//! Emits `cases.json`: a deterministic corpus of ODRL `(policy, request)` cases drawn
//! from the two constraint-rich generators the design record nominates for this study —
//! U3 financial services and U4 research consortium
//! (`research/ac-query-benchmark.md` §1.3, §2.5, §4.2) — each carrying **sparq's own
//! decision**, produced by running the real `sparq_policy::parse_policy_str` +
//! `sparq_policy::evaluate` path.
//!
//! # What this binary does NOT do
//! - It does not run ODRE (that is `odre_adapter.py`).
//! - It does not decide agreement or classify divergences (that is `agreement.py`).
//! - It does not measure anything: this lane's contract is a decision ledger, not a
//!   timing number.
//!
//! # Determinism
//! Everything is a pure function of `(seed, sf, constraint_complexity, max_cases)`:
//! the generators are seeded (`SplitMix64` in `sparq-acbench`), case selection is a
//! stable round-robin over a `BTreeMap` keyed by construct signature, and the pinned
//! evaluation instant is a constant. No clock is read. Re-running with the same flags
//! produces a byte-identical `cases.json`.

use std::collections::{BTreeMap, BTreeSet};

use sparq_acbench::{
    AcModel, AccessMode, Audience, CompiledPolicy, Condition, ConstraintComplexity, Decision,
    Effect, ExpectedDecision, GenParams, IntentRow, Scope, compile_odrl, consortium, financial,
    oracle_odrl,
};
use sparq_policy::{Value, evaluate, matched_prohibition, parse_policy_str};

/// The pinned benchmark evaluation instant every temporal constraint is decided against.
///
/// This MUST stay identical to `sparq_acbench::oracle::BENCH_EVAL_INSTANT`, which is
/// `pub(crate)` and therefore not importable from outside the crate (a follow-up asks for
/// it to be exported so this duplication can go away). A drift here would silently
/// mis-date every temporal case, so `assert_eval_instant_matches_oracle` below pins the
/// constant against the oracle's OBSERVABLE behaviour at startup rather than trusting the
/// comment: it constructs an intent whose window straddles the instant and checks the
/// oracle admits it, plus one that closes before it and checks the oracle refuses.
const EVAL_INSTANT: &str = "2026-07-06T00:00:00Z";

/// The purpose the corpus grants for — mirrors `sparq_acbench::oracle::BENCH_GRANTED_PURPOSE`
/// (also `pub(crate)`), and pinned the same way by `assert_granted_purpose_matches_oracle`.
const GRANTED_PURPOSE: &str = "https://sparq.dev/vocab/purpose#reporting";

/// The count the request declares for an `odrl:count` constraint (a first use).
const DECLARED_COUNT: f64 = 1.0;

const ODRL_NS: &str = "http://www.w3.org/ns/odrl/2/";
const ACL_READ: &str = "https://sparq.dev/vocab/acl#Read";
const ACL_WRITE: &str = "https://sparq.dev/vocab/acl#Write";
const ACL_CONTROL: &str = "https://sparq.dev/vocab/acl#Control";

fn main() {
    let mut smoke = false;
    let mut sf: u32 = 1;
    let mut max_cases: usize = 24;
    let mut out = String::from("cases.json");

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--smoke" => smoke = true,
            "--sf" => {
                i += 1;
                sf = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("[odre-export] --sf needs a positive integer");
                    std::process::exit(2);
                });
            }
            "--max-cases" => {
                i += 1;
                max_cases = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("[odre-export] --max-cases needs a positive integer");
                    std::process::exit(2);
                });
            }
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("[odre-export] --out needs a path");
                    std::process::exit(2);
                });
            }
            other => {
                eprintln!("[odre-export] unknown argument {}", other);
                std::process::exit(2);
            }
        }
        i += 1;
    }

    // Anti-drift: the two pinned constants are duplicated from a `pub(crate)` oracle
    // surface, so verify them against the oracle's behaviour before exporting anything.
    assert_eval_instant_matches_oracle();
    assert_granted_purpose_matches_oracle();

    if smoke {
        sf = 1;
        max_cases = max_cases.min(24);
    }

    let params = corpus_params(sf);
    if let Err(e) = params.validate() {
        eprintln!("[odre-export] invalid GenParams: {}", e);
        std::process::exit(2);
    }

    let fin = financial::generate(&params);
    let con = consortium::generate(&params);

    let mut cases: Vec<Case> = Vec::new();
    cases.extend(build_cases(
        "financial",
        "u3",
        &fin.intents,
        &fin.expected_decisions,
        &BTreeMap::new(),
        max_cases,
    ));
    cases.extend(build_cases(
        "consortium",
        "u4",
        &con.intents,
        &con.expected_decisions,
        &con.group_memberships,
        max_cases,
    ));

    let json = render_json(&params, &cases);
    if let Err(e) = std::fs::write(&out, json) {
        eprintln!("[odre-export] cannot write {}: {}", out, e);
        std::process::exit(1);
    }

    let n_allow = cases.iter().filter(|c| c.sparq_allow).count();
    eprintln!(
        "[odre-export] wrote {} cases to {} ({} sparq-ALLOW, {} sparq-DENY)",
        cases.len(),
        out,
        n_allow,
        cases.len() - n_allow
    );
    if cases.is_empty() {
        eprintln!("[odre-export] ERROR: empty corpus — an empty ledger would make the");
        eprintln!("             agreement report vacuous. Refusing to claim a run.");
        std::process::exit(1);
    }
}

/// The corpus parameters for this lane.
///
/// `GenParams::smoke()` pins `constraint_complexity: None`, which would make an
/// ODRL-constraint study VACUOUS (no `odrl:constraint` would be emitted at all). This
/// lane therefore raises it to `Compound` — the level the design record §1.3 says U3/U4
/// exist to exercise ("constraint-rich by design"). Every other knob is left at the
/// smoke defaults so the corpus stays small and the run stays per-commit cheap.
fn corpus_params(sf: u32) -> GenParams {
    GenParams {
        sf,
        constraint_complexity: ConstraintComplexity::Compound,
        ..GenParams::smoke()
    }
}

/// One exported comparison case.
struct Case {
    id: String,
    use_case: &'static str,
    resource: String,
    party: String,
    /// The `odrl:PartyCollection` (and ODRL wildcard-collection) IRIs the requester is a
    /// member of, as membership EVIDENCE supplied to both systems. Neither evaluator
    /// invents membership; the harness derives these from the corpus's own group closure
    /// so the two sides see the same party facts (MAPPING.md §3.3).
    party_collections: Vec<String>,
    action: &'static str,
    client: Option<String>,
    policy_nt: String,
    constructs: BTreeSet<String>,
    intent_count: usize,
    sparq_allow: bool,
    sparq_matched_rules: Vec<String>,
    sparq_deny_kinds: BTreeSet<String>,
    sparq_reasons: Vec<String>,
    oracle_allow: bool,
    parse_error: Option<String>,
}

/// Build the comparison cases for one use case.
///
/// Selection is deterministic AND coverage-seeking: cases are bucketed by their ODRL
/// construct signature and drawn round-robin, so a `--max-cases` cap keeps every distinct
/// construct combination the generator produced rather than the first N of one shape.
fn build_cases(
    use_case: &'static str,
    prefix: &str,
    intents: &[IntentRow],
    expected: &[ExpectedDecision],
    group_memberships: &BTreeMap<String, BTreeSet<String>>,
    max_cases: usize,
) -> Vec<Case> {
    // Bucket the ODRL expected-decision rows by construct signature.
    let mut buckets: BTreeMap<String, Vec<&ExpectedDecision>> = BTreeMap::new();
    for ed in expected {
        if ed.model != AcModel::Odrl {
            continue;
        }
        let relevant = relevant_intents(intents, &ed.request.resource);
        if relevant.is_empty() {
            // No ODRL rule touches this resource: the case would be a trivial
            // empty-policy deny on both sides and teaches nothing. Skipped, not faked.
            continue;
        }
        let sig = constructs_of(&relevant).into_iter().collect::<Vec<_>>().join(",");
        buckets.entry(sig).or_default().push(ed);
    }

    // Round-robin draw across buckets (stable: BTreeMap iteration + input order).
    let mut picked: Vec<&ExpectedDecision> = Vec::new();
    let mut round = 0usize;
    loop {
        let mut drew = false;
        for rows in buckets.values() {
            if let Some(ed) = rows.get(round) {
                picked.push(ed);
                drew = true;
                if picked.len() >= max_cases {
                    break;
                }
            }
        }
        if !drew || picked.len() >= max_cases {
            break;
        }
        round += 1;
    }

    picked
        .into_iter()
        .enumerate()
        .map(|(n, ed)| {
            let relevant = relevant_intents(intents, &ed.request.resource);
            build_case(format!("{}-{:04}", prefix, n), use_case, ed, &relevant, group_memberships)
        })
        .collect()
}

/// The intents that bear on a resource: an exact `Scope::Resource` hit, or any
/// `Scope::Subtree` ancestor — the same reading `sparq_acbench::oracle` applies.
fn relevant_intents<'a>(intents: &'a [IntentRow], resource: &str) -> Vec<&'a IntentRow> {
    intents
        .iter()
        .filter(|it| match it.scope {
            Scope::Resource => resource == it.resource_uri,
            Scope::Subtree => resource.starts_with(&it.resource_uri),
        })
        .collect()
}

fn build_case(
    id: String,
    use_case: &'static str,
    ed: &ExpectedDecision,
    relevant: &[&IntentRow],
    group_memberships: &BTreeMap<String, BTreeSet<String>>,
) -> Case {
    let policy_iri = format!("urn:sparq:acbench:odre:{}", id);
    let policy_nt = compile_case_policy(&policy_iri, relevant);
    let action = primary_action(&ed.request.mode);

    // The request, as the ODRL evaluator sees it. Every context value supplied here is
    // a declared mapping decision, documented in MAPPING.md §3 — the corpus's requests
    // carry no purpose/count/device fields of their own (`sparq_acbench::Request` has
    // agent/client/resource/mode only), so the harness supplies the pinned corpus-wide
    // values the by-construction oracle itself assumes.
    let mut req = sparq_policy::Request::new(action)
        .on(ed.request.resource.clone())
        .by(ed.request.agent.clone())
        .at(EVAL_INSTANT)
        .for_purpose(Value::Iri(GRANTED_PURPOSE.to_owned()))
        .with(format!("{}count", ODRL_NS), Value::Num(DECLARED_COUNT));
    if let Some(client) = &ed.request.client {
        req = req.with(format!("{}systemDevice", ODRL_NS), Value::Iri(client.clone()));
    }
    // Party-collection / wildcard-assignee membership evidence. `sparq_policy` matches an
    // assignee by identity or by SUPPLIED membership edge — it never invents membership —
    // so the harness supplies exactly the edges the corpus's own group closure asserts
    // (mapped to this policy's PartyCollection IRIs), plus the ODRL wildcard collections.
    let mut party_collections: Vec<String> = vec![format!("{}All", ODRL_NS)];
    if !ed.request.agent.is_empty() {
        party_collections.push(format!("{}AllAuthenticated", ODRL_NS));
    }
    for (idx, it) in relevant.iter().enumerate() {
        if let Audience::Group(g) = &it.audience {
            if group_memberships.get(g).is_some_and(|members| members.contains(&ed.request.agent)) {
                party_collections.push(party_iri(&it.resource_uri, idx));
            }
        }
    }
    req = req.with_party_memberships(party_collections.iter()
        .map(|collection| (ed.request.agent.clone(), collection.clone())));

    let owned: Vec<IntentRow> = relevant.iter().map(|r| (*r).clone()).collect();
    let oracle_allow = oracle_odrl(&ed.request, &owned) == Decision::Allow;

    let (sparq_allow, matched_rules, reasons, deny_kinds, parse_error) =
        match parse_policy_str(&policy_nt, "ntriples") {
            Ok(policy) => {
                let d = evaluate(&policy, &req);
                let mut kinds = deny_reason_kinds(&d.unmet_constraints);
                if matched_prohibition(&policy, &req).is_some() {
                    kinds.insert("prohibition-override".to_owned());
                }
                (d.allow, d.matched_rules, d.unmet_constraints, kinds, None)
            }
            Err(e) => {
                // A REFUSED policy is a fail-closed deny in sparq, and the refusal reason
                // is carried into the report rather than being flattened into a plain deny.
                let mut kinds = BTreeSet::new();
                kinds.insert("policy-refused".to_owned());
                (false, Vec::new(), vec![e.clone()], kinds, Some(e))
            }
        };

    Case {
        id,
        use_case,
        resource: ed.request.resource.clone(),
        party: ed.request.agent.clone(),
        party_collections,
        action,
        client: ed.request.client.clone(),
        policy_nt,
        constructs: constructs_of(relevant),
        intent_count: relevant.len(),
        sparq_allow,
        sparq_matched_rules: matched_rules,
        sparq_deny_kinds: deny_kinds,
        sparq_reasons: reasons,
        oracle_allow,
        parse_error,
    }
}

/// The `odrl:PartyCollection` IRI `compile_case_policy` gives intent `idx`'s group.
fn party_iri(resource_uri: &str, idx: usize) -> String {
    format!("{}#i{}-odrl-party", resource_uri, idx)
}

/// Lower the relevant intents into ONE ODRL policy graph (N-Triples).
///
/// `sparq_acbench::compile_odrl` names every node after the intent's resource
/// (`<res#odrl-rule>`, `<res#odrl-cst-start>`, …), so two intents on the SAME resource —
/// which is the normal case for the high-fan-in U3 corpus — would collapse onto one rule
/// node and silently merge their assignees, actions and constraints. This function
/// re-fragments each intent's nodes with a per-intent `#i{idx}-` prefix and re-points
/// every intent's policy node at a single case policy IRI, so the case is exactly one
/// `odrl:Set` carrying N independent rules — the graph a real deployment would hold.
fn compile_case_policy(policy_iri: &str, relevant: &[&IntentRow]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let policy_token = format!("<{}>", policy_iri);
    let mut emitted_policy_type = false;

    for (idx, it) in relevant.iter().enumerate() {
        let CompiledPolicy { nquads, .. } = compile_odrl(it);
        let intent_policy_token = format!("<{}#odrl-policy>", it.resource_uri);
        for line in nquads {
            // Per-intent node renaming FIRST is wrong: `#odrl-policy` would become
            // `#i0-odrl-policy` and no longer match the policy token. Re-point the policy
            // node first, then re-fragment whatever `#odrl-` nodes remain (rule, party,
            // constraints).
            let line = line.replace(&intent_policy_token, &policy_token);
            let line = line.replace("#odrl-", &format!("#i{}-odrl-", idx));
            // `odrl:uid`/`rdf:type` are policy-level and identical for every intent;
            // emit them once so the graph has no duplicate triples.
            let is_policy_header = line
                .contains("22-rdf-syntax-ns#type> <http://www.w3.org/ns/odrl/2/Set>")
                || line.contains(&format!("{} <{}uid>", policy_token, ODRL_NS));
            if is_policy_header && emitted_policy_type {
                continue;
            }
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
        emitted_policy_type = true;
    }
    lines.join("\n") + "\n"
}

/// The ODRL constructs a case's intent set uses — the key both the coverage matrix and
/// the divergence classifier are computed over.
fn constructs_of(relevant: &[&IntentRow]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for it in relevant {
        match it.effect {
            Effect::Allow => set.insert("rule:permission".to_owned()),
            Effect::Deny => set.insert("rule:prohibition".to_owned()),
        };
        match it.scope {
            Scope::Resource => set.insert("scope:resource".to_owned()),
            Scope::Subtree => set.insert("scope:subtree".to_owned()),
        };
        // A rule carrying more than one `odrl:action` is called out explicitly: ODRE's
        // `_action` takes an arbitrary first row from an unordered result set, so a
        // multi-action rule is where its reported action (and hence its decision) can
        // vary between runs. The classifier keys a ledger entry on this construct.
        let n_modes =
            usize::from(it.mode.read) + usize::from(it.mode.write) + usize::from(it.mode.control);
        if n_modes > 1 {
            set.insert("rule:multi-action".to_owned());
        }
        match &it.audience {
            Audience::Public => set.insert("assignee:All".to_owned()),
            Audience::Authenticated => set.insert("assignee:AllAuthenticated".to_owned()),
            Audience::Owner | Audience::Agent(_) => set.insert("assignee:party".to_owned()),
            Audience::Group(_) => set.insert("assignee:PartyCollection".to_owned()),
            Audience::ClientRestricted { .. } => {
                set.insert("assignee:party".to_owned());
                set.insert("constraint:systemDevice".to_owned())
            }
            Audience::AllExcept(_) => set.insert("assignee:party-set".to_owned()),
        };
        collect_condition_constructs(&it.condition, &mut set);
    }
    set
}

fn collect_condition_constructs(c: &Condition, set: &mut BTreeSet<String>) {
    match c {
        Condition::None => {}
        Condition::Temporal { .. } => {
            set.insert("constraint:dateTime".to_owned());
        }
        Condition::Purpose(_) => {
            set.insert("constraint:purpose".to_owned());
        }
        Condition::Count(_) => {
            set.insert("constraint:count".to_owned());
        }
        Condition::And(a, b) => {
            set.insert("constraint:compound-and".to_owned());
            collect_condition_constructs(a, set);
            collect_condition_constructs(b, set);
        }
    }
}

/// Reduce `sparq_policy`'s human-readable deny explanations to a STRUCTURED kind set,
/// which is what the Python classifier keys on (never the prose).
///
/// The strings are produced by `sparq_policy::eval::rule_matches` / `evaluate`; the
/// substrings matched here are the stable, distinctive part of each. An explanation that
/// matches nothing yields `other`, which the classifier treats as unclassifiable rather
/// than guessing — so a future wording change surfaces as a loud FAIL, not a silent
/// mis-attribution.
fn deny_reason_kinds(reasons: &[String]) -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    for r in reasons {
        if r.starts_with("prohibition ") {
            kinds.insert("prohibition-override".to_owned());
        } else if r.contains(" action ") && r.contains("!= requested") {
            kinds.insert("action-mismatch".to_owned());
        } else if r.contains(" target ") && r.contains("!= requested") {
            kinds.insert("target-mismatch".to_owned());
        } else if r.contains(" assignee ") && r.contains("!= requester") {
            kinds.insert("assignee-mismatch".to_owned());
        } else if r.contains(" constraint ") || r.contains(" logical constraint ") {
            kinds.insert("constraint-unsat".to_owned());
        } else if r.contains("undischarged duty") {
            kinds.insert("duty-undischarged".to_owned());
        } else if r.contains("no permission matches") {
            kinds.insert("no-permission".to_owned());
        } else {
            kinds.insert("other".to_owned());
        }
    }
    kinds
}

fn primary_action(mode: &AccessMode) -> &'static str {
    if mode.read {
        ACL_READ
    } else if mode.write {
        ACL_WRITE
    } else {
        ACL_CONTROL
    }
}

// ── Pinned-constant anti-drift checks ────────────────────────────────────────────────

/// Prove [`EVAL_INSTANT`] is the instant the acbench oracle actually evaluates at.
///
/// Constructs two single-intent tables whose temporal windows straddle / precede the
/// constant and asserts the oracle admits the first and refuses the second. If the
/// oracle's own pinned instant ever moves, this aborts the export instead of silently
/// dating every temporal case wrongly.
fn assert_eval_instant_matches_oracle() {
    let resource = "urn:sparq:acbench:odre:probe";
    let agent = "urn:sparq:acbench:odre:probe-agent";
    let request = sparq_acbench::Request {
        agent: agent.to_owned(),
        client: None,
        resource: resource.to_owned(),
        mode: AccessMode::read_only(),
    };
    let intent = |start: &str, end: &str| IntentRow {
        audience: Audience::Agent(agent.to_owned()),
        scope: Scope::Resource,
        mode: AccessMode::read_only(),
        condition: Condition::Temporal { start: start.to_owned(), end: end.to_owned() },
        effect: Effect::Allow,
        resource_uri: resource.to_owned(),
    };
    let open = [intent("2026-07-05T00:00:00Z", "2026-07-07T00:00:00Z")];
    let closed = [intent("2026-07-01T00:00:00Z", "2026-07-06T00:00:00Z")];
    assert_eq!(
        oracle_odrl(&request, &open),
        Decision::Allow,
        "EVAL_INSTANT {} is not inside a window the acbench oracle admits — the pinned \
         instant has drifted from sparq_acbench::oracle::BENCH_EVAL_INSTANT",
        EVAL_INSTANT
    );
    assert_eq!(
        oracle_odrl(&request, &closed),
        Decision::Deny,
        "a window closing at {} is still admitted by the acbench oracle — the pinned \
         instant has drifted from sparq_acbench::oracle::BENCH_EVAL_INSTANT",
        EVAL_INSTANT
    );
}

/// Prove [`GRANTED_PURPOSE`] is the purpose the acbench oracle grants for.
fn assert_granted_purpose_matches_oracle() {
    let resource = "urn:sparq:acbench:odre:probe";
    let agent = "urn:sparq:acbench:odre:probe-agent";
    let request = sparq_acbench::Request {
        agent: agent.to_owned(),
        client: None,
        resource: resource.to_owned(),
        mode: AccessMode::read_only(),
    };
    let intent = |purpose: &str| IntentRow {
        audience: Audience::Agent(agent.to_owned()),
        scope: Scope::Resource,
        mode: AccessMode::read_only(),
        condition: Condition::Purpose(purpose.to_owned()),
        effect: Effect::Allow,
        resource_uri: resource.to_owned(),
    };
    let granted = [intent(GRANTED_PURPOSE)];
    let other = [intent("urn:sparq:acbench:odre:some-other-purpose")];
    assert_eq!(
        oracle_odrl(&request, &granted),
        Decision::Allow,
        "GRANTED_PURPOSE {} is not the purpose the acbench oracle grants for — it has \
         drifted from sparq_acbench::oracle::BENCH_GRANTED_PURPOSE",
        GRANTED_PURPOSE
    );
    assert_eq!(
        oracle_odrl(&request, &other),
        Decision::Deny,
        "the acbench oracle admits an unrelated purpose — BENCH_GRANTED_PURPOSE is no \
         longer a discriminating constant"
    );
}

// ── JSON rendering (hand-rolled: this driver takes no serde dependency) ──────────────

fn render_json(params: &GenParams, cases: &[Case]) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"schema\": \"sparq.acbench.odre.cases/1\",\n");
    s.push_str("  \"producer\": \"bench/ac/odre/exporter (ac-odre-export)\",\n");
    s.push_str("  \"sparq_decision_path\": \"sparq_policy::parse_policy_str + sparq_policy::evaluate (the ODRL evaluator the sparq-solid odrl-bridge materializes from)\",\n");
    s.push_str(&format!("  \"eval_instant\": {},\n", jstr(EVAL_INSTANT)));
    s.push_str(&format!("  \"granted_purpose\": {},\n", jstr(GRANTED_PURPOSE)));
    s.push_str(&format!("  \"declared_count\": {},\n", DECLARED_COUNT));
    s.push_str("  \"params\": {\n");
    s.push_str(&format!("    \"seed\": {},\n", params.seed));
    s.push_str(&format!("    \"sf\": {},\n", params.sf));
    s.push_str(&format!("    \"constraint_complexity\": {},\n", jstr("Compound")));
    s.push_str(&format!("    \"policies_per_resource\": {}\n", params.policies_per_resource));
    s.push_str("  },\n");
    s.push_str("  \"cases\": [\n");
    for (i, c) in cases.iter().enumerate() {
        s.push_str("    {\n");
        s.push_str(&format!("      \"id\": {},\n", jstr(&c.id)));
        s.push_str(&format!("      \"use_case\": {},\n", jstr(c.use_case)));
        s.push_str("      \"request\": {\n");
        s.push_str(&format!("        \"party\": {},\n", jstr(&c.party)));
        s.push_str(&format!("        \"party_collections\": {},\n", jarr(c.party_collections.iter())));
        s.push_str(&format!("        \"action\": {},\n", jstr(c.action)));
        s.push_str(&format!("        \"target\": {},\n", jstr(&c.resource)));
        s.push_str(&format!("        \"purpose\": {},\n", jstr(GRANTED_PURPOSE)));
        s.push_str(&format!("        \"instant\": {},\n", jstr(EVAL_INSTANT)));
        s.push_str(&format!("        \"count\": {},\n", DECLARED_COUNT));
        s.push_str(&format!(
            "        \"device\": {}\n",
            c.client.as_deref().map_or_else(|| "null".to_owned(), jstr)
        ));
        s.push_str("      },\n");
        s.push_str(&format!("      \"policy_ntriples\": {},\n", jstr(&c.policy_nt)));
        s.push_str(&format!("      \"intent_count\": {},\n", c.intent_count));
        s.push_str(&format!("      \"constructs\": {},\n", jarr(c.constructs.iter())));
        s.push_str("      \"sparq\": {\n");
        s.push_str(&format!("        \"decision\": {},\n", jstr(if c.sparq_allow { "allow" } else { "deny" })));
        s.push_str(&format!("        \"matched_rules\": {},\n", jarr(c.sparq_matched_rules.iter())));
        s.push_str(&format!("        \"deny_reason_kinds\": {},\n", jarr(c.sparq_deny_kinds.iter())));
        s.push_str(&format!("        \"reasons\": {},\n", jarr(c.sparq_reasons.iter())));
        s.push_str(&format!(
            "        \"parse_error\": {}\n",
            c.parse_error.as_deref().map_or_else(|| "null".to_owned(), jstr)
        ));
        s.push_str("      },\n");
        s.push_str("      \"acbench_oracle\": {\n");
        s.push_str(&format!(
            "        \"decision\": {},\n",
            jstr(if c.oracle_allow { "allow" } else { "deny" })
        ));
        s.push_str("        \"role\": \"context only — the by-construction oracle does NOT arbitrate the sparq-vs-ODRE diff\"\n");
        s.push_str("      }\n");
        s.push_str(if i + 1 == cases.len() { "    }\n" } else { "    },\n" });
    }
    s.push_str("  ]\n}\n");
    s
}

fn jstr<S: AsRef<str>>(s: S) -> String {
    let mut out = String::from("\"");
    for ch in s.as_ref().chars() {
        match ch {
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
    out
}

fn jarr<'a, I: Iterator<Item = &'a String>>(items: I) -> String {
    let body: Vec<String> = items.map(jstr).collect();
    format!("[{}]", body.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consortium_export_supplies_collection_evidence_to_both_evaluators() {
        let ds = consortium::generate(&GenParams::smoke());
        let mut ed = ExpectedDecision {
            request: sparq_acbench::Request {
                agent: "https://consortium.example/agents/inst0-m0".to_owned(),
                client: None,
                resource: "https://consortium.example/inst0/papers/p0".to_owned(),
                mode: AccessMode::read_only(),
            },
            model: AcModel::Odrl,
            decision: Decision::Allow,
        };
        let relevant = relevant_intents(&ds.intents, &ed.request.resource);
        let missing = build_case("missing".into(), "consortium", &ed, &relevant, &BTreeMap::new());
        assert!(!missing.sparq_allow);
        let member = build_case("member".into(), "consortium", &ed, &relevant, &ds.group_memberships);
        assert!(member.sparq_allow);
        assert!(member.party_collections.contains(&party_iri(&ed.request.resource, 1)));
        ed.request.agent = "https://consortium.example/agents/inst0-m999".to_owned();
        let outsider = build_case("outsider".into(), "consortium", &ed, &relevant, &ds.group_memberships);
        assert!(!outsider.sparq_allow);
        assert!(!outsider.party_collections.contains(&party_iri(&ed.request.resource, 1)));
    }
}
