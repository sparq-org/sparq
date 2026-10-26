//! Run the existing policy tests through both parsers, including their generated
//! inputs and the fetched conformance policies. `support/main_parse.rs` is the
//! unchanged parsing algorithm from origin/main at the revision named there.
//! Only set-operator right operands are normalized as sets; all other model fields,
//! rule order, compound structure and accepted/refused outcomes are compared.
extern crate self as sparq_policy;
extern crate sparq_policy as current;
pub use current::*;

#[path = "support/main_parse.rs"]
mod main_parse;

fn normalize(mut p: Policy) -> Policy {
    fn atom(c: &mut Constraint) {
        if matches!(
            c.operator,
            Operator::IsAnyOf | Operator::IsPartOf | Operator::IsNoneOf
        ) && matches!(c.right, Value::Str(_) | Value::SharedStr(_))
        {
            let parts: std::collections::BTreeSet<_> = c.right.as_str().split('|').collect();
            c.right = Value::Str(parts.into_iter().collect::<Vec<_>>().join("|"));
        }
    }
    fn compound(c: &mut LogicalConstraint) {
        for n in &mut c.operands {
            match n {
                ConstraintNode::Atomic(c) => atom(c),
                ConstraintNode::SharedAtomic(c) => atom(std::sync::Arc::make_mut(c)),
                ConstraintNode::Compound(c) => compound(c),
            }
        }
    }
    for r in p.permissions.iter_mut().chain(p.prohibitions.iter_mut()) {
        for c in &mut r.constraints {
            atom(c);
        }
        for c in &mut r.logical_constraints {
            compound(c);
        }
        for d in &mut r.duties {
            for c in &mut d.constraints {
                atom(c);
            }
        }
    }
    p
}

fn refusal_signature(err: &str) -> (Option<ConstraintBudget>, Option<&str>) {
    let tag = err
        .split_once("[budget=")
        .and_then(|(_, tail)| tail.split_once(']').map(|(tag, _)| tag));
    (current::constraint_budget_refusal(err), tag)
}

pub fn parse_policy(graph: &sparq_core::Graph) -> Result<ValidatedPolicy, String> {
    let actual = current::parse_policy(graph);
    let expected = main_parse::parse_policy(graph);
    match (&actual, &expected) {
        (Ok(a), Ok(b)) => assert_eq!(normalize(a.policy().clone()), normalize(b.policy().clone())),
        (Err(a), Err(b)) => assert_eq!(
            refusal_signature(a),
            refusal_signature(b),
            "refusal kinds differ: current={a}; main={b}"
        ),
        _ => panic!("parse parity differs: current={actual:?}; main={expected:?}"),
    }
    actual
}
pub fn parse_policy_str(rdf: &str, format: &str) -> Result<ValidatedPolicy, String> {
    // Load once, so randomly allocated blank-node labels are identical.
    let graph = sparq_core::Graph::load_str(rdf, format)?;
    let actual = current::parse_policy_str(rdf, format);
    let compared = parse_policy(&graph);
    assert_eq!(
        actual.is_ok(),
        compared.is_ok(),
        "string and graph wrappers disagree: {actual:?}; {compared:?}"
    );
    compared
}
pub fn parse_policy_str_with_memberships(
    rdf: &str,
    format: &str,
) -> Result<
    (
        ValidatedPolicy,
        std::collections::BTreeSet<(String, String)>,
    ),
    String,
> {
    let graph = sparq_core::Graph::load_str(rdf, format)?;
    let policy = parse_policy(&graph)?;
    let members = current::target_memberships(&policy, &graph)?;
    let old_members = main_parse::target_memberships(&policy, &graph)?;
    assert_eq!(members, old_members);
    let actual = current::parse_policy_str_with_memberships(rdf, format);
    assert!(actual.is_ok(), "membership wrapper refused {actual:?}");
    Ok((policy, members))
}

#[path = "decision_report.rs"]
mod decision_report;

#[path = "eval_metamorph.rs"]
mod eval_metamorph;

#[path = "odrl_compare.rs"]
mod odrl_compare;

#[path = "odrl_compare_compound.rs"]
mod odrl_compare_compound;

#[path = "odrl_compare_witness.rs"]
mod odrl_compare_witness;

#[path = "odrl_conflict_strategy.rs"]
mod odrl_conflict_strategy;

#[path = "odrl_constraint_budget.rs"]
mod odrl_constraint_budget;

#[path = "odrl_constraint_matching.rs"]
mod odrl_constraint_matching;

#[path = "odrl_count.rs"]
mod odrl_count;

#[path = "odrl_count_backend.rs"]
mod odrl_count_backend;

#[path = "odrl_count_file.rs"]
mod odrl_count_file;

#[path = "odrl_count_stateless.rs"]
mod odrl_count_stateless;

#[path = "odrl_datetimestamp.rs"]
mod odrl_datetimestamp;

#[path = "odrl_degraded_prohibition.rs"]
mod odrl_degraded_prohibition;

#[path = "odrl_eval.rs"]
mod odrl_eval;

#[path = "odrl_hierarchy.rs"]
mod odrl_hierarchy;

#[path = "odrl_right_operand_sets.rs"]
mod odrl_right_operand_sets;

#[path = "odrl_set_operators.rs"]
mod odrl_set_operators;

#[path = "odrl_test_suite.rs"]
mod odrl_test_suite;

#[path = "secprop_leftoperands.rs"]
mod secprop_leftoperands;

#[path = "target_memberships.rs"]
mod target_memberships;

#[test]
fn repeated_real_atoms_retain_main_results_with_default_limits() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> a o:Set .\n");
    for i in 0..64 {
        rdf.push_str(&format!(
            "<urn:p> o:permission <urn:r{i}> . <urn:r{i}> o:constraint <urn:lc> .\n"
        ));
    }
    for i in 0..1000 {
        rdf.push_str(&format!("<urn:lc> o:and <urn:a{i}> . <urn:a{i}> o:leftOperand o:purpose ; o:operator o:eq ; o:rightOperand <urn:v> .\n"));
    }
    let p = parse_policy_str(&rdf, "turtle").unwrap();
    assert_eq!(p.permissions.len(), 64);
    assert!(p
        .permissions
        .iter()
        .all(|r| r.logical_constraints[0].operands.len() == 1000));
}

#[test]
fn per_property_extraction_preserves_references_missing_fields_and_sets() {
    for left in ["o:leftOperand o:purpose;", ""] {
        for direct in ["o:rightOperand <urn:a>, <urn:b>;", ""] {
            for reference in ["o:rightOperandReference <urn:b>, <urn:c>;", ""] {
                for op in ["o:operator o:isAnyOf;", "o:operator o:eq;", ""] {
                    let rdf = format!("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission [ o:constraint <urn:c> ] . <urn:c> {left} {direct} {reference} {op} a o:Constraint .");
                    parse_policy_str(&rdf, "turtle").unwrap();
                }
            }
        }
    }
}

// Reuse the vendored and generated example fixtures through the same proxy.
#[allow(dead_code)]
#[path = "../examples/bench.rs"]
mod bench;
#[allow(dead_code)]
#[path = "../examples/policy_bench.rs"]
mod policy_bench;

#[test]
fn list_cells_and_several_compounds_on_one_rule_retain_main_results() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:lc1>, <urn:lc2> . <urn:lc1> o:and (");
    for i in 0..1000 {
        rdf.push_str(&format!(" <urn:a{i}>"));
    }
    rdf.push_str(") . <urn:lc2> o:and <urn:lc1> .");
    parse_policy_str(&rdf, "turtle").unwrap();
}

#[test]
fn shared_folded_set_atoms_retain_main_results() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:root> . <urn:atom> o:leftOperand o:purpose ; o:operator o:isAnyOf ; o:rightOperand (");
    for i in 0..2000 {
        rdf.push_str(&format!(" <urn:value{i}>"));
    }
    rdf.push_str(") .\n");
    for i in 0..500 {
        rdf.push_str(&format!(
            "<urn:root> o:and <urn:sub{i}> . <urn:sub{i}> o:and <urn:atom> .\n"
        ));
    }
    let graph = sparq_core::Graph::load_str(&rdf, "turtle").unwrap();
    let expected = main_parse::parse_policy(&graph).expect("main accepts this shared folded atom");
    let actual = current::parse_policy(&graph).expect("defaults cover shared folded atoms");
    assert_eq!(
        normalize(actual.into_inner()),
        normalize(expected.into_inner())
    );
    assert!(current::parse_policy_str(&rdf, "turtle").is_ok());
}

#[test]
fn repository_odrl_fixtures_match_main() {
    fn visit(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                visit(&path, found);
            } else if path.extension().is_some_and(|s| s == "ttl") {
                found.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates", "bench"] {
        visit(&root.join(dir), &mut files);
    }
    assert!(
        files.len() >= 114,
        "fixture discovery missed policies: {}",
        files.len()
    );
    for path in files {
        let rdf = std::fs::read_to_string(&path).unwrap();
        match sparq_core::Graph::load_str(&rdf, "turtle") {
            Ok(graph) => {
                let _ = parse_policy(&graph);
            }
            Err(_) => assert_eq!(
                current::parse_policy_str(&rdf, "turtle"),
                main_parse::parse_policy_str(&rdf, "turtle"),
                "{}",
                path.display()
            ),
        }
    }
}

/// Re-run in release mode with `--ignored --nocapture` on an otherwise idle host.
/// The scalar shape and timed RDF load match the original ceiling probe. The
/// folded shape checks sensitivity to atom size. Bisect the median parse time
/// inside a verified bracket, then multiply its upper rule count by expanded
/// operands per compound. This calibrates operand count, not elapsed work.
#[test]
#[ignore = "manual operand-count calibration against frozen main"]
fn calibrate_operand_ceiling() {
    fn document(rules: usize, width: usize, list: bool) -> String {
        let mut rdf =
            String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> a o:Set .\n");
        for i in 0..rules {
            rdf.push_str(&format!(
                "<urn:p> o:permission <urn:r{i}> . <urn:r{i}> o:constraint <urn:lc> .\n"
            ));
        }
        if list {
            rdf.push_str("<urn:a> o:leftOperand o:purpose; o:operator o:isAnyOf; o:rightOperand (");
            for i in 0..2000 {
                rdf.push_str(&format!(" <urn:value{i}>"));
            }
            rdf.push_str(") .\n");
            for i in 0..width {
                rdf.push_str(&format!(
                    "<urn:lc> o:and <urn:sub{i}> . <urn:sub{i}> o:and <urn:a> .\n"
                ));
            }
        } else {
            for i in 0..width {
                rdf.push_str(&format!(
                    "<urn:lc> o:and <urn:a{i}> . <urn:a{i}> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand <urn:v> .\n"
                ));
            }
        }
        rdf
    }
    fn median(rules: usize, width: usize, list: bool) -> std::time::Duration {
        let rdf = document(rules, width, list);
        let mut samples = Vec::new();
        for _ in 0..3 {
            let start = std::time::Instant::now();
            let policy = main_parse::parse_policy_str(&rdf, "turtle").unwrap();
            samples.push(start.elapsed());
            std::hint::black_box(&policy);
        }
        samples.sort();
        println!("folded={list} rules={rules} median={:?}", samples[1]);
        samples[1]
    }
    let envelope = std::time::Duration::from_secs(2);
    for (width, list, mut high) in [(1000, false, 4096), (500, true, 64)] {
        let mut low = 0;
        // Report an insufficient bracket instead of calling its endpoint a ceiling.
        if median(high, width, list) <= envelope {
            println!("folded={list} increase the upper rule bracket and rerun");
            continue;
        }
        while high - low > 1 {
            let rules = (low + high) / 2;
            if median(rules, width, list) <= envelope {
                low = rules;
            } else {
                high = rules;
            }
        }
        let operands = width * if list { 2 } else { 1 };
        println!(
            "folded={list} operand_bracket=[{}, {}] candidate_operand_ceiling={}",
            low * operands,
            high * operands,
            high * operands
        );
    }
}

#[test]
fn compound_operands_without_left_fields_keep_the_guard_reading() {
    for fields in [
        "o:operator o:eq, o:neq; o:rightOperand <urn:value>",
        r#"o:operator o:eq; o:rightOperand "invalid"^^<http://www.w3.org/2001/XMLSchema#dateTimeStamp>"#,
    ] {
        let rdf = format!("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:lc> . <urn:lc> o:and <urn:c> . <urn:c> {fields} .");
        parse_policy_str(&rdf, "turtle").unwrap();
    }
}

#[test]
fn padded_compound_retains_main_expansion_refusal() {
    let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:lc> .\n");
    for i in 0..2000 {
        rdf.push_str(&format!("<urn:lc> o:and <urn:a{i}> . <urn:a{i}> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand <urn:value> .\n"));
    }
    for i in 0..10 {
        rdf.push_str(&format!("<urn:p> o:permission <urn:pad{i}> .\n"));
    }
    let err = parse_policy_str(&rdf, "turtle").unwrap_err();
    assert_eq!(
        constraint_budget_refusal(&err),
        Some(ConstraintBudget::Expansion)
    );
}

#[test]
fn folded_cache_retains_typed_operand_identity() {
    for value in ["1", "true", "2000-01-01T00:00:00Z"] {
        for datatype in ["integer", "string", "boolean", "dateTime"] {
            let rdf = format!(
                r#"@prefix o: <http://www.w3.org/ns/odrl/2/> . @prefix x: <http://www.w3.org/2001/XMLSchema#> .
                <urn:p> o:permission <urn:r0>, <urn:r1> .
                <urn:r0> o:constraint <urn:c0> . <urn:r1> o:constraint <urn:c1> .
                <urn:c0> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand "{value}"^^x:{datatype} .
                <urn:c1> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand "{value}" ."#
            );
            parse_policy_str(&rdf, "turtle").unwrap();
        }
    }
}

#[test]
fn repeated_atomic_rights_retain_main_operand_allowance() {
    for list in [false, true] {
        let mut rdf = String::from("@prefix o: <http://www.w3.org/ns/odrl/2/> . <urn:p> o:permission <urn:r> . <urn:r> o:constraint <urn:root> .\n");
        if list {
            rdf.push_str("<urn:list> <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> <urn:value>; <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> .\n");
        }
        let right = if list { "<urn:list>" } else { "<urn:value>" };
        for i in 0..1000 {
            rdf.push_str(&format!("<urn:root> o:and <urn:c{i}> . <urn:c{i}> o:leftOperand o:purpose; o:operator o:eq; o:rightOperand {right} .\n"));
        }
        parse_policy_str(&rdf, "turtle").unwrap();
    }
}
