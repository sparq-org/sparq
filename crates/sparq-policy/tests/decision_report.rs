#![cfg(feature = "decision-report")]

use sparq_policy::{decide, parse_policy_str, report::DecisionReport, Request};

// [GPT-5.6] sq-mu4au: every input exercises a distinct aggregation branch. Removing
// any counter update, conflict recognition, or BTreeMap ordering makes this test fail.
#[test]
fn fixed_decision_batch_has_stable_complete_report() {
    // Decisions come only from `decide`: a grant, a plain no-match deny, and a deny by
    // an overriding prohibition.
    let policy = parse_policy_str(
        r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol/r> a odrl:Set ;
  odrl:permission <urn:rule/permission-read> ;
  odrl:prohibition <urn:rule/prohibition-write> .
<urn:rule/permission-read> odrl:action odrl:read ; odrl:target <urn:asset/x> .
<urn:rule/prohibition-write> odrl:action odrl:write ; odrl:target <urn:asset/x> ."#,
        "turtle",
    )
    .unwrap();
    let on_x = |a: &str| Request::new(format!("http://www.w3.org/ns/odrl/2/{a}")).on("urn:asset/x");
    let permitted = decide(&policy, &on_x("read"));
    let denied = decide(&policy, &Request::new("http://www.w3.org/ns/odrl/2/read"));
    let conflict = decide(&policy, &on_x("write"));
    assert!(permitted.allow && !denied.allow && !conflict.allow);
    let escaped_action = "urn:action/a\"\\\n";
    let inputs = [
        ("urn:action/read", &permitted),
        ("urn:action/read", &denied),
        (escaped_action, &conflict),
    ];

    let report = DecisionReport::summarize(inputs);
    assert_eq!(report.total, 3);
    assert_eq!(report.permitted, 1);
    assert_eq!(report.denied, 2);
    assert_eq!(report.conflicts, 1);
    assert_eq!(report.permitted + report.denied, report.total);
    assert_eq!(report.per_action.len(), 2);
    assert_eq!(
        report
            .per_action
            .iter()
            .map(|row| row.permitted)
            .sum::<usize>(),
        1
    );
    assert_eq!(
        report
            .per_action
            .iter()
            .map(|row| row.denied)
            .sum::<usize>(),
        2
    );
    assert_eq!(report.per_action[0].action, escaped_action);

    let expected = concat!(
        "{\"total\":3,\"permitted\":1,\"denied\":2,\"conflicts\":1,",
        "\"per_action\":[{\"action\":\"urn:action/a\\\"\\\\\\n\",\"permitted\":0,\"denied\":1},",
        "{\"action\":\"urn:action/read\",\"permitted\":1,\"denied\":1}]}"
    );
    assert_eq!(report.to_json(), expected);
    assert_eq!(report.to_json().as_bytes(), report.to_json().as_bytes());
}
