// Coverage-guided DECISION-CONTRACT differential target for the sparq-solid WAC
// per-resource decision surface. Bead sq-b9afe (de-risks gg0qq.5's "fail-closed on
// decision-engine error" acceptance clause). [FABLE-5]
//
// SURFACE: `PodStore::decide` / `decide_batch` (crates/sparq-solid/src/decide.rs) —
// the one question an LDP resource server asks per request. Unlike the parser targets
// in this harness, a clean parse is NOT the property under test here: the property is
// the decision layer's FAIL-CLOSED contract over corrupted/partial/hostile datasets.
//
// INVARIANT (the finding condition — each is asserted, so libFuzzer flags a violation
// as a crash and saves the reproducing input):
//   1. allow == true  ⇒  status == Resolved AND a governing ACL was discovered
//      (`governing_acl`/`scope` are Some) AND the decided mode is in `granted_modes`.
//   2. Any non-`Resolved` status (NoAcl / Unloaded / Transient) ⇒ allow == false and
//      granted_modes is empty — a partial/errored index NEVER allows.
//   3. If `materialize_wac` was never run (or returned Err — "the previous auth view
//      is left in place", which for a fresh store is NO view), NO decision allows,
//      and with no materialization attempted at all no decision is `Resolved`. This
//      also guards `PodStore::new`'s reserved-graph strip: a fuzz input that smuggles
//      a forged `<urn:sparq:auth>` grant into the dataset must not produce a verdict.
//   4. `decide_batch` agrees element-wise with per-call `decide` (same index, same
//      oracle — a divergence is a real decision-layer bug).
//   5. `decide` itself never panics — there is no panics-as-pass carve-out here; a
//      panic in the decision path is itself a finding (the resource server calls this
//      per request on whatever state the pod is in).
//
// INPUT SHAPE: byte 0 is a control byte (bit 0: run `materialize_wac`; bits 1–2: the
// decided `Mode`; bit 3: bind a request clock so time-windowed conditional grants are
// reachable). The remaining bytes are an N-Quads dataset. If the whole body does not
// parse, we SALVAGE the individually-parsable lines and build the store from that
// subset — this is exactly the bead's "truncated N-Triples / partial fixture" shape: a
// prefix-truncated or line-corrupted ACL fixture yields a dataset with SOME of its
// authorization quads missing, and the decision layer must degrade to deny, never
// widen. Principals and resources are then harvested from the input itself (acl:agent
// objects, named-graph IRIs and their `.acl`/`.acr`-stripped bases) so the fuzzer can
// actually reach allow == true on well-formed corpora (the invariant is not vacuous)
// while mutation walks the corrupted/partial space around it. Seed corpus:
// fuzz/seeds/wac_fail_closed/ (prefixed variants of the sparq-solid conformance
// fixture — materialized, unmaterialized, truncated, line-corrupted, forged-auth).
#![no_main]

use libfuzzer_sys::fuzz_target;
use oxrdf::Term;
use sparq_core::Graph;
use sparq_solid::{AclStatus, Mode, PodStore, Session};

/// Harvest the objects of `acl:agent` quads from the RAW input text (up to `cap`).
/// Text-side (not graph-side) on purpose: it also surfaces agents named on lines the
/// salvage pass DROPPED, so sessions are probed whose grants are exactly the corrupted
/// part of the fixture — the partial-index shape the invariant is about.
fn harvest_agents(text: &str, cap: usize) -> Vec<String> {
    const AGENT_PRED: &str = "<http://www.w3.org/ns/auth/acl#agent> <";
    let mut agents = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(AGENT_PRED) {
        rest = &rest[at + AGENT_PRED.len()..];
        if let Some(end) = rest.find('>') {
            let iri = &rest[..end];
            if !iri.is_empty() && !agents.iter().any(|a| a == iri) {
                agents.push(iri.to_owned());
                if agents.len() >= cap {
                    break;
                }
            }
        } else {
            break;
        }
    }
    agents
}

/// Candidate resources to decide on: every named-graph IRI, each control document's
/// governed base (`.acl`/`.acr` stripped) and a non-existent member under it (exercises
/// `acl:default` inheritance walks over partial chains), plus one line of raw input as
/// a deliberately-possibly-malformed IRI (the `Transient` path).
fn harvest_resources(graph: &Graph, text: &str, cap: usize) -> Vec<String> {
    let mut resources: Vec<String> = Vec::new();
    let push = |r: String, out: &mut Vec<String>| {
        if !out.contains(&r) {
            out.push(r);
        }
    };
    for (name, _) in graph.named() {
        if resources.len() >= cap {
            break;
        }
        let Term::NamedNode(n) = name else { continue };
        let iri = n.as_str();
        push(iri.to_owned(), &mut resources);
        for suffix in [".acl", ".acr"] {
            if let Some(base) = iri.strip_suffix(suffix) {
                push(base.to_owned(), &mut resources);
                push(format!("{base}/member"), &mut resources);
            }
        }
    }
    // A raw input line as the resource: reaches the malformed-IRI Transient deny.
    if let Some(line) = text.lines().next() {
        push(line.trim().trim_start_matches('<').trim_end_matches('>').to_owned(), &mut resources);
    }
    resources.truncate(cap);
    resources
}

/// Assert the fail-closed decision contract for one decision. `materialize_attempted` /
/// `materialize_ok` describe what the harness did to the store before deciding.
fn check_decision(
    d: &sparq_solid::WacDecision,
    mode: Mode,
    materialize_attempted: bool,
    materialize_ok: bool,
) {
    if d.allow {
        // Invariant 1: an allow is only ever an authoritative, fully-resolved verdict.
        assert_eq!(d.status, AclStatus::Resolved, "fail-open: allow with status {:?}", d.status);
        assert!(d.governing_acl.is_some(), "fail-open: allow without a discovered governing ACL");
        assert!(d.scope.is_some(), "fail-open: allow without an ACL scope");
        assert!(d.granted_modes.contains(&mode), "fail-open: allow but mode not in granted_modes");
    }
    if d.status != AclStatus::Resolved {
        // Invariant 2: a partial/errored index never allows (and grants nothing).
        assert!(!d.allow, "fail-open: allow on non-Resolved status {:?}", d.status);
        assert!(d.granted_modes.is_empty(), "non-Resolved decision leaked granted_modes");
    }
    // Invariant 3: no materialized view (never run, or run and Err on a fresh store)
    // ⇒ nothing allows; and if materialization was never even attempted, the reserved-
    // graph strip in `PodStore::new` must have removed any forged auth view, so no
    // decision can be authoritative at all.
    if !materialize_ok {
        assert!(!d.allow, "fail-open: allow on an un-materialized/errored auth view");
    }
    if !materialize_attempted {
        assert_ne!(
            d.status,
            AclStatus::Resolved,
            "forged/leaked auth view: Resolved verdict without any materialize call"
        );
    }
    // Structural: provenance fields travel together.
    assert_eq!(d.governing_acl.is_some(), d.scope.is_some(), "acl/scope provenance split");
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }
    let ctrl = data[0];
    let text = String::from_utf8_lossy(&data[1..]);

    // Build the dataset: whole-body N-Quads, else the per-line SALVAGE of a corrupted/
    // truncated fixture (the partial-index shape under test). A dataset with zero
    // parsable lines still exercises the empty-store deny paths below.
    let graph = match Graph::load_dataset(&text, "nquads") {
        Ok(g) => g,
        Err(_) => {
            let salvaged: String = text
                .lines()
                .filter(|l| Graph::load_dataset(l, "nquads").is_ok())
                .flat_map(|l| [l, "\n"])
                .collect();
            Graph::load_dataset(&salvaged, "nquads").unwrap_or_default()
        }
    };

    let agents = harvest_agents(&text, 3);
    let resources = harvest_resources(&graph, &text, 12);

    let mut store = PodStore::new(graph);
    let materialize_attempted = ctrl & 1 == 1;
    // materialize_wac Err (e.g. a reserved-encoding agent IRI in an ACL) must leave the
    // fresh store's NO-view state in place — decisions after it must all deny.
    let materialize_ok = materialize_attempted && store.materialize_wac().is_ok();

    let mode = match (ctrl >> 1) & 3 {
        0 => Mode::Read,
        1 => Mode::Write,
        2 => Mode::Append,
        _ => Mode::Control,
    };
    // A bound request clock makes time-windowed conditional grants reachable; without
    // one they must fail closed.
    let now = (ctrl & 8 != 0).then_some("2026-01-01T00:00:00Z");

    // Sessions: anonymous + every harvested agent (including agents whose grant lines
    // were corrupted away, and reserved-encoding principals the oracle must degrade on).
    let mut sessions: Vec<Session> = vec![Session { agent: None, client: None, issuer: None, now }];
    for a in &agents {
        sessions.push(Session { agent: Some(a), client: None, issuer: None, now });
    }

    for session in &sessions {
        let requests: Vec<(&str, Mode)> = resources.iter().map(|r| (r.as_str(), mode)).collect();
        // Invariant 5: neither call may panic — no catch_unwind, a panic IS the finding.
        let batch = store.decide_batch(session, &requests);
        assert_eq!(batch.len(), requests.len(), "decide_batch not parallel to its input");
        for (i, resource) in resources.iter().enumerate() {
            let d = store.decide(session, resource, mode);
            check_decision(&d, mode, materialize_attempted, materialize_ok);
            // Invariant 4: the batch path and the point path agree exactly.
            assert_eq!(batch[i], d, "decide_batch diverged from decide for {resource}");
        }
    }
});
