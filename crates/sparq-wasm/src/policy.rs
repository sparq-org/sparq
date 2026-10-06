//! [FABLE-5] sq-586sh (PSS consolidation ask A, issue #890): the opt-in ODRL
//! usage-control **probe** binding — `sparq-policy` compiled to wasm32.
//!
//! PROBE SCOPE, deliberately minimal: this module exists to (a) prove
//! `sparq-policy` compiles and runs on `wasm32-unknown-unknown`, and (b) give the
//! bundle-size measurement a *reachable* parse→evaluate→conflict path so the
//! measured `--features policy` delta reflects real linked code, not a stripped
//! no-op dependency. The **full** JS API surface (per-dimension audit statuses,
//! per-duty status, purpose/party/asset taxonomies, request time) is DRAFTED in
//! the sq-586sh maintainer report and awaits a public-contract decision — these
//! two exports are EXPERIMENTAL and may be renamed/reshaped by that decision.
//! The published `@sparq-org/sparq` bundle (js `build:wasm`) does NOT enable this
//! feature, so nothing here is a published contract.
//!
//! Behind the non-default `policy` Cargo feature: the lean default bundle links
//! zero ODRL/policy code (`sparq-policy` never enters the build graph), so the
//! `wasm_bundle_bytes` baseline is byte-identical. `sparq-policy` itself is
//! std-only; its stateful `count-enforcement` feature (a `Mutex`/lockfile
//! usage-counter store — meaningless in a single-threaded browser tab with no
//! POSIX filesystem) is deliberately NOT forwarded: the stateless evaluator
//! treats `odrl:count` as the numeric constraint the caller contextualises.
//!
//! The evaluator is **fail-closed** end-to-end (matching the native crate): a
//! malformed policy document is an `Err`, an empty/unmatched policy DENIES, and
//! an inadmissible (permission/prohibition-conflicted under `odrl:perm`) policy
//! is reported as such rather than silently evaluated.

use wasm_bindgen::prelude::*;

use sparq_policy::{
    conflict_admissibility, detect_conflicts, evaluate, parse_policy_str, Overlap, Request,
};

/// Appends `s` to `out` as a JSON string literal (quotes + minimal escaping).
/// The bundle carries no serde — strings are escaped by hand exactly like the
/// SPARQL-JSON and SHACL-report serialisers in this crate (the two mandatory
/// escapes plus the C0 control characters JSON forbids unescaped).
fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
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
}

/// Appends a JSON array of strings.
fn push_json_string_array(out: &mut String, items: &[String]) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_string(out, item);
    }
    out.push(']');
}

/// Parses an ODRL policy document and evaluates one access request against it,
/// returning the decision as a small JSON string:
///
/// ```json
/// {"allow":false,"matchedRules":["…rule id…"],"unmetConstraints":["…reason…"]}
/// ```
///
/// `format` is the RDF syntax of `policy_rdf` (`"turtle"`, `"ntriples"`, …, as
/// [`sparq_policy::parse_policy_str`] accepts). `action` is the requested action
/// IRI (e.g. `http://www.w3.org/ns/odrl/2/read`); `target`/`party` optionally pin
/// the requested asset and requesting party. Fail-closed: a malformed document is
/// `Err(String)`; a policy with no matching permission (including the empty
/// policy) is `allow: false`.
pub(crate) fn evaluate_policy_json(
    policy_rdf: &str,
    format: &str,
    action: &str,
    target: Option<&str>,
    party: Option<&str>,
) -> Result<String, String> {
    let policy = parse_policy_str(policy_rdf, format)?;
    let mut request = Request::new(action);
    if let Some(t) = target {
        request = request.on(t);
    }
    if let Some(p) = party {
        request = request.by(p);
    }
    let decision = evaluate(&policy, &request);
    let mut out = String::from("{\"allow\":");
    out.push_str(if decision.allow { "true" } else { "false" });
    out.push_str(",\"matchedRules\":");
    push_json_string_array(&mut out, &decision.matched_rules);
    out.push_str(",\"unmetConstraints\":");
    push_json_string_array(&mut out, &decision.unmet_constraints);
    out.push('}');
    Ok(out)
}

/// Parses an ODRL policy document and runs the request-free static analysis:
/// permission/prohibition conflict detection + the `odrl:conflict`-strategy
/// admissibility verdict, as a small JSON string:
///
/// ```json
/// {"admissible":false,
///  "reason":"…why not…",
///  "conflicts":[{"permissionId":"…","prohibitionId":"…","overlap":"certain",
///                "action":"http://www.w3.org/ns/odrl/2/read","target":null}]}
/// ```
///
/// `overlap` is `"certain"` (the prohibition provably carves out the whole
/// permission) or `"possible"` (not proven total — never silently dropped).
/// `reason` is `null` when the policy is admissible. Fail-closed: a malformed
/// document is `Err(String)`.
pub(crate) fn policy_conflicts_json(policy_rdf: &str, format: &str) -> Result<String, String> {
    let policy = parse_policy_str(policy_rdf, format)?;
    let conflicts = detect_conflicts(&policy);
    let admissible = conflict_admissibility(&policy);
    let mut out = String::from("{\"admissible\":");
    out.push_str(if admissible.is_ok() { "true" } else { "false" });
    out.push_str(",\"reason\":");
    match &admissible {
        Ok(()) => out.push_str("null"),
        Err(reason) => push_json_string(&mut out, reason),
    }
    out.push_str(",\"conflicts\":[");
    for (i, c) in conflicts.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"permissionId\":");
        push_json_string(&mut out, &c.permission_id);
        out.push_str(",\"prohibitionId\":");
        push_json_string(&mut out, &c.prohibition_id);
        out.push_str(",\"overlap\":");
        push_json_string(
            &mut out,
            match c.overlap {
                Overlap::Certain => "certain",
                Overlap::Possible => "possible",
            },
        );
        out.push_str(",\"action\":");
        match &c.action {
            Some(a) => push_json_string(&mut out, a),
            None => out.push_str("null"),
        }
        out.push_str(",\"target\":");
        match &c.target {
            Some(t) => push_json_string(&mut out, t),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push_str("]}");
    Ok(out)
}

/// EXPERIMENTAL (sq-586sh probe — see the module docs): evaluates one ODRL
/// access request against a policy document; returns the decision JSON
/// documented on `evaluate_policy_json`. A malformed document throws.
#[wasm_bindgen(js_name = policyEvaluate)]
pub fn policy_evaluate(
    policy_rdf: &str,
    format: &str,
    action: &str,
    target: Option<String>,
    party: Option<String>,
) -> Result<String, JsError> {
    evaluate_policy_json(policy_rdf, format, action, target.as_deref(), party.as_deref())
        .map_err(|e| JsError::new(&e))
}

/// EXPERIMENTAL (sq-586sh probe — see the module docs): request-free conflict +
/// admissibility analysis of a policy document; returns the JSON documented on
/// `policy_conflicts_json`. A malformed document throws.
#[wasm_bindgen(js_name = policyConflicts)]
pub fn policy_conflicts(policy_rdf: &str, format: &str) -> Result<String, JsError> {
    policy_conflicts_json(policy_rdf, format).map_err(|e| JsError::new(&e))
}

#[cfg(test)]
mod tests {
    // These run on the NATIVE target (like the canon-module tests: the
    // `#[wasm_bindgen]` exports' `JsError` mapping cannot run off-wasm, so the
    // tests assert against the plain `_json` functions the exports delegate to;
    // the exports are one `.map_err` away).

    use super::{evaluate_policy_json, policy_conflicts_json};

    const ALLOW_READ: &str = r#"
        @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/allow> a odrl:Set ;
          odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
    "#;

    const CONFLICTED: &str = r#"
        @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
        <urn:pol/c> a odrl:Set ;
          odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
          odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
    "#;

    #[test]
    fn permission_grants_allow() {
        let json = evaluate_policy_json(
            ALLOW_READ,
            "turtle",
            "http://www.w3.org/ns/odrl/2/read",
            Some("urn:asset/x"),
            None,
        )
        .unwrap();
        assert!(json.starts_with("{\"allow\":true,"), "got: {json}");
        assert!(json.contains("\"matchedRules\":["), "got: {json}");
    }

    #[test]
    fn prohibition_overrides_to_deny_with_matched_rule() {
        let json = evaluate_policy_json(
            CONFLICTED,
            "turtle",
            "http://www.w3.org/ns/odrl/2/read",
            Some("urn:asset/x"),
            None,
        )
        .unwrap();
        // Deny-overrides: the prohibition wins over the matching permission.
        assert!(json.starts_with("{\"allow\":false,"), "got: {json}");
    }

    #[test]
    fn unmatched_target_denies_fail_closed() {
        let json = evaluate_policy_json(
            ALLOW_READ,
            "turtle",
            "http://www.w3.org/ns/odrl/2/read",
            Some("urn:asset/OTHER"),
            None,
        )
        .unwrap();
        assert!(json.starts_with("{\"allow\":false,"), "got: {json}");
    }

    #[test]
    fn malformed_policy_is_err_not_allow() {
        assert!(evaluate_policy_json("@prefix broken", "turtle", "a", None, None).is_err());
        assert!(policy_conflicts_json("@prefix broken", "turtle").is_err());
    }

    #[test]
    fn conflicted_policy_reports_certain_conflict() {
        // No declared odrl:conflict strategy => deny-overrides is well-defined, so
        // the policy stays ADMISSIBLE — but the certain conflict is still reported.
        let json = policy_conflicts_json(CONFLICTED, "turtle").unwrap();
        assert!(json.starts_with("{\"admissible\":true,"), "got: {json}");
        assert!(json.contains("\"overlap\":\"certain\""), "got: {json}");
        assert!(
            json.contains("\"action\":\"http://www.w3.org/ns/odrl/2/read\""),
            "got: {json}"
        );
        assert!(json.contains("\"target\":\"urn:asset/x\""), "got: {json}");
    }

    #[test]
    fn perm_strategy_conflict_is_inadmissible_with_reason() {
        // Under odrl:conflict odrl:perm ("permission wins") a certain
        // permission/prohibition conflict makes the policy INADMISSIBLE.
        let policy = r#"
            @prefix odrl: <http://www.w3.org/ns/odrl/2/> .
            <urn:pol/c> a odrl:Set ; odrl:conflict odrl:perm ;
              odrl:permission  [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
              odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] .
        "#;
        let json = policy_conflicts_json(policy, "turtle").unwrap();
        assert!(json.starts_with("{\"admissible\":false,\"reason\":\""), "got: {json}");
        assert!(json.contains("\"overlap\":\"certain\""), "got: {json}");
    }

    #[test]
    fn clean_policy_is_admissible_with_no_conflicts() {
        let json = policy_conflicts_json(ALLOW_READ, "turtle").unwrap();
        assert_eq!(
            json,
            "{\"admissible\":true,\"reason\":null,\"conflicts\":[]}"
        );
    }

    #[test]
    fn json_string_escaping_is_wellformed() {
        // A policy whose rule id round-trips through the hand-rolled escaper is
        // covered by the tests above; exercise the escaper's special-character
        // arms directly so a broken escape shows up as a direct unit failure.
        let mut out = String::new();
        super::push_json_string(&mut out, "a\"b\\c\nd\te\u{1}");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\te\\u0001\"");
    }
}
