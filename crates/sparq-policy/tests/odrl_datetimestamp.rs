//! #3902: a timezone-free `xsd:dateTimeStamp` right operand is ill-typed (XSD 1.1 §3.4.28
//! requires the timezone). Comparing its lexical would decide access on a literal with no
//! value, and dropping just its constraint would disable a prohibition, so the whole policy
//! is refused (fail-closed on both rule kinds).

use sparq_policy::parse_policy_str;

fn policy(rule: &str, stamp: &str) -> String {
    format!(
        r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:{rule} [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
      odrl:rightOperand "{stamp}"^^xsd:dateTimeStamp ] ] .
"#
    )
}

#[test]
fn a_timezone_free_datetimestamp_operand_refuses_the_policy() {
    for rule in ["permission", "prohibition"] {
        let err = parse_policy_str(&policy(rule, "2026-12-31T00:00:00"), "turtle").unwrap_err();
        assert!(err.contains("xsd:dateTimeStamp requires"), "{rule}: {err}");
    }
}

#[test]
fn a_zoned_datetimestamp_operand_still_parses() {
    for rule in ["permission", "prohibition"] {
        for stamp in ["2026-12-31T00:00:00Z", "2026-12-31T00:00:00-05:00"] {
            assert!(parse_policy_str(&policy(rule, stamp), "turtle").is_ok(), "{rule} {stamp}");
        }
    }
}

/// The refusal runs on every operand BEFORE deduplication: a same-lexical `xsd:string`
/// sibling must not absorb the ill-typed stamp, in a collection or as repeated objects.
#[test]
fn a_same_lexical_sibling_cannot_absorb_the_ill_typed_operand() {
    let list = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
      odrl:rightOperand ( "2026-12-31T00:00:00"^^xsd:string "2026-12-31T00:00:00"^^xsd:dateTimeStamp ) ] ] .
"#;
    assert!(parse_policy_str(list, "turtle").is_err(), "collection member");
    let repeated = r#"
@prefix odrl: <http://www.w3.org/ns/odrl/2/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:pol/p> a odrl:Set ;
  odrl:permission [ odrl:action odrl:read ; odrl:target <urn:asset/x> ] ;
  odrl:prohibition [ odrl:action odrl:read ; odrl:target <urn:asset/x> ;
    odrl:constraint [ odrl:leftOperand odrl:dateTime ; odrl:operator odrl:lteq ;
      odrl:rightOperand "2026-12-31T00:00:00"^^xsd:string , "2026-12-31T00:00:00"^^xsd:dateTimeStamp ] ] .
"#;
    assert!(parse_policy_str(repeated, "turtle").is_err(), "repeated objects");
}

/// XSD whitespace collapse applies before the timezone check: a padded zoned stamp is valid.
#[test]
fn a_whitespace_padded_zoned_datetimestamp_still_parses() {
    for rule in ["permission", "prohibition"] {
        assert!(parse_policy_str(&policy(rule, " 2026-12-31T00:00:00Z "), "turtle").is_ok(), "{rule}");
    }
}
