//! `target_memberships`: only `odrl:partOf` edges into a rule target are evidence.

use sparq_policy::parse_policy_str_with_memberships;

#[test]
fn only_edges_into_rule_targets_are_returned() {
    let ttl = r#"@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
<urn:pol> a odrl:Set ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:c:notes> ] .
<urn:a:n1> odrl:partOf <urn:c:notes> .
<urn:a:n2> odrl:partOf <urn:c:other> .
_:b odrl:partOf <urn:c:notes> .
<https://alice.ex/card#me> odrl:partOf <urn:c:team> .
"#;
    let (policy, edges) = parse_policy_str_with_memberships(ttl, "turtle").expect("parses");
    assert_eq!(policy.prohibitions.len(), 1);
    let edges: Vec<(&str, &str)> = edges.iter().map(|(m, c)| (m.as_str(), c.as_str())).collect();
    assert_eq!(edges, [("urn:a:n1", "urn:c:notes")]);
}
