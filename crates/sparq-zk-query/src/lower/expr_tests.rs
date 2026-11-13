use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::expr::*;
use super::expr_semantics::{self as mirror, Error};
use oxrdf::Term;
use spargebra::{
    Query,
    algebra::{Expression, GraphPattern},
};

#[path = "../../../sparq-zk-compose/src/toolchain.rs"]
mod toolchain;

fn parsed(query: &str) -> Expression {
    fn find(p: &GraphPattern) -> Option<Expression> {
        match p {
            GraphPattern::Extend { expression, .. } => Some(expression.clone()),
            GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner } => find(inner),
            _ => None,
        }
    }
    let q = spargebra::SparqlParser::new().parse_query(query).unwrap();
    let Query::Select { pattern, .. } = q else {
        panic!("SELECT fixture")
    };
    find(&pattern).expect("projection expression")
}
fn slots() -> BTreeMap<String, (u32, LaneType)> {
    BTreeMap::from([
        ("missing".into(), (0, LaneType::Dynamic)),
        ("n".into(), (1, LaneType::Dynamic)),
        ("x".into(), (2, LaneType::Dynamic)),
        ("y".into(), (3, LaneType::Dynamic)),
    ])
}
fn expression(text: &str) -> Expr {
    from_spargebra(
        &parsed(&format!(
            "PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({text} AS ?v) {{}}"
        )),
        &slots(),
    )
    .unwrap()
}
fn call(op: ExprOp, args: Vec<Expr>) -> Expr {
    Expr::Call { op, args }
}

#[test]
fn lowering_types_parameters_and_named_refusals() {
    let a = lower(&expression("1 + 2.0")).unwrap();
    assert_eq!(a.nodes[a.root].ty, LaneType::Decimal);
    assert_eq!(a.constants.len(), 2);
    assert_eq!(a.constants[0].value, mirror::typed("1", "integer"));
    let b = lower(&expression("8 + 9.0")).unwrap();
    assert_eq!(a.nodes, b.nodes);
    for (text, ty) in [
        ("1 / 2", LaneType::Decimal),
        ("1 + \"2\"^^xsd:float", LaneType::Float),
        ("\"1\"^^xsd:float + 2e0", LaneType::Double),
        ("?x + 2", LaneType::Dynamic),
        ("IF(true,1,2.0)", LaneType::Dynamic),
        ("COALESCE(?missing,1)", LaneType::Dynamic),
    ] {
        let d = lower(&expression(text)).unwrap();
        assert_eq!(d.nodes[d.root].ty, ty, "{text}");
    }
    let repeated = Expr::Variable {
        slot: 7,
        ty: LaneType::Integer,
    };
    let d = lower(&call(ExprOp::Add, vec![repeated.clone(), repeated])).unwrap();
    assert_eq!(
        d.nodes
            .iter()
            .filter(|n| matches!(n.gadget, Gadget::Variable(7)))
            .count(),
        1
    );
    for (i, node) in d.nodes.iter().enumerate() {
        assert!(node.inputs.iter().all(|&input| input < i));
    }
    for name in ["REGEX", "NOW", "STRLEN", "SHA256", "custom:fn"] {
        let err = lower(&call(ExprOp::Unsupported(name.into()), vec![])).unwrap_err();
        assert_eq!(err.function, name);
    }
    for op in [ExprOp::And, ExprOp::If, ExprOp::UnaryPlus, ExprOp::In] {
        let err = lower(&call(op.clone(), vec![])).unwrap_err();
        assert_eq!(err.function, op.name());
    }
    let d = lower(&expression("sameTerm(+1,1)")).unwrap();
    assert!(d.nodes.iter().any(|n| n.gadget == Gadget::ToTerm));
    let d = lower(&expression("?x && true")).unwrap();
    assert!(d.nodes.iter().any(|n| n.gadget == Gadget::Ebv));
    assert!(
        from_spargebra(&parsed("SELECT (STRLEN(\"a\") AS ?v) {}"), &slots())
            .unwrap_err()
            .function
            .contains("STRLEN")
    );
    assert_eq!(
        from_spargebra(&parsed("SELECT (EXISTS { ?s ?p ?o } AS ?v) {}"), &slots())
            .unwrap_err()
            .function,
        "EXISTS"
    );
}

fn engine(query: &str, data: &str) -> Vec<Vec<Option<Term>>> {
    let graph = sparq_core::Graph::load_str(data, "n-triples").unwrap();
    sparq_engine::query(&graph, query).unwrap().rows
}
fn compare_engine(text: &str, exception: Option<&str>) {
    let query =
        format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({text} AS ?v) {{}}");
    let ir = from_spargebra(&parsed(&query), &slots()).unwrap();
    lower(&ir).unwrap();
    let expected = mirror::eval(&ir, &BTreeMap::new());
    let expected = match expected {
        Ok(t) => Some(mirror::normalized(&t)),
        Err(Error::Sparql) => None,
        Err(Error::Capacity) => return,
    };
    let actual = engine(&query, "");
    assert_eq!(actual.len(), 1);
    let actual = actual[0][0].as_ref().map(mirror::normalized);
    if let Some(reason) = exception {
        assert_ne!(actual, expected, "stale difference: {reason}");
    } else {
        assert_eq!(actual, expected, "{text}");
    }
}
#[test]
fn generated_expressions_against_engine() {
    for a in -5..=5 {
        for b in -3..=3 {
            for op in ["+", "-", "*", "/", "=", "!=", "<", "<=", ">", ">="] {
                compare_engine(&format!("({a}) {op} ({b})"), None);
            }
        }
    }
    for a in [
        "true",
        "false",
        "?missing",
        "<urn:x>",
        "\"bad\"^^xsd:integer",
    ] {
        for b in ["true", "false", "?missing"] {
            for op in ["&&", "||"] {
                compare_engine(&format!("({a}) {op} ({b})"), None);
            }
        }
    }
    for text in [
        "!true",
        "!?missing",
        "IF(true,7,1/0)",
        "IF(false,1/0,7)",
        "IF(?missing,1,2)",
        "COALESCE(1/0,?missing,7)",
        "COALESCE(?missing)",
        "BOUND(?missing)",
        "2 IN ()",
        "2 NOT IN ()",
        "?missing IN ()",
        "?missing NOT IN ()",
        "2 IN (1/0,2)",
        "2 IN (3,1/0)",
        "2 NOT IN (1/0,2)",
        "2 NOT IN (3,1/0)",
        "sameTerm(1,1.0)",
        "sameTerm(+\"+005\"^^xsd:byte,\"+005\"^^xsd:byte)",
        "0.1+0.2=0.3",
        "\"NaN\"^^xsd:double=\"NaN\"^^xsd:double",
        "1e0/0e0",
        "0e0/0e0",
        "16777217=\"16777216\"^^xsd:float",
        "\"a\"<\"b\"",
        "\"a\"@en=\"a\"@fr",
        "\"x\"^^<urn:dt>=\"y\"^^<urn:dt>",
    ] {
        compare_engine(text, None);
    }
}

#[test]
fn builtin_edge_entries_in_scope() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../sparq-engine/tests/fixtures/builtin_edges.json"
    ))
    .unwrap();
    let mut included = Vec::new();
    for case in corpus["cases"].as_array().unwrap() {
        let query = case["query"].as_str().unwrap();
        let ast = parsed(query);
        let Ok(ir) = from_spargebra(&ast, &slots()) else {
            continue;
        };
        let data = case["dataset_ntriples"].as_str().unwrap_or("");
        let mut vars = BTreeMap::new();
        // Parse bindings independently of expression evaluation.
        if query.contains("VALUES ?n") {
            fn values(p: &GraphPattern) -> Option<Term> {
                match p {
                    GraphPattern::Values { bindings, .. } => {
                        bindings.first()?.first()?.clone().map(Into::into)
                    }
                    GraphPattern::Extend { inner, .. } | GraphPattern::Project { inner, .. } => {
                        values(inner)
                    }
                    GraphPattern::Join { left, right } => values(left).or_else(|| values(right)),
                    _ => None,
                }
            }
            let q = spargebra::SparqlParser::new().parse_query(query).unwrap();
            let Query::Select { pattern, .. } = q else {
                unreachable!()
            };
            vars.insert(1, values(&pattern).expect("VALUES fixture"));
        } else if !data.is_empty() {
            let (dict, triples) = sparq_core::Graph::parse_to_triples(data, "n-triples").unwrap();
            assert_eq!(triples.len(), 1);
            vars.insert(1, dict.term(triples[0][2]));
        }
        let expected = mirror::eval(&ir, &vars);
        let expected = match expected {
            Ok(t) => Some(mirror::normalized(&t)),
            Err(Error::Sparql) => None,
            Err(Error::Capacity) => {
                included.push((case["id"].as_str().unwrap(), "capacity"));
                continue;
            }
        };
        let rows = engine(query, data);
        assert_eq!(rows.len(), 1, "{}", case["id"]);
        assert_eq!(
            rows[0][0].as_ref().map(mirror::normalized),
            expected,
            "{}",
            case["id"]
        );
        let golden = &case["expected_rows"];
        assert_eq!(
            serde_json::json!(
                rows.iter()
                    .map(|r| r
                        .iter()
                        .map(|t| t.as_ref().map(ToString::to_string))
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            ),
            *golden,
            "{}",
            case["id"]
        );
        included.push((case["id"].as_str().unwrap(), "equal"));
    }
    assert!(included.iter().any(|(id, _)| *id == "division-error"));
    assert!(
        included
            .iter()
            .any(|(id, _)| *id == "unary-plus-identity-error-byte-stored")
    );
    assert!(
        included
            .iter()
            .any(|(id, _)| *id == "unary-plus-valid-identity-large")
    );
    eprintln!("builtin expression scope: {included:?}");
}

fn noir_numeric(term: &Term) -> std::result::Result<String, Error> {
    use mirror::Number;
    use num_traits::Signed;
    Ok(match mirror::number(term)? {
        Number::Exact(n, ty) => format!(
            "exact({}, {}, {})",
            if ty == LaneType::Integer { 0 } else { 1 },
            n.is_negative(),
            n.abs()
        ),
        Number::Float(n) => format!("floating(2, {})", n.to_bits()),
        Number::Double(n) => format!("floating(3, {})", n.to_bits()),
    })
}
fn noir_operand(term: &Term) -> String {
    match noir_numeric(term) {
        Ok(n) => format!("(lane({n}), false)"),
        Err(Error::Sparql) => "(lane(zero_numeric()), true)".into(),
        Err(Error::Capacity) => panic!("sample input outside bucket"),
    }
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("sparq_noir_expr_{}", std::process::id()));
        std::fs::create_dir(&p).expect("fresh Noir sample directory");
        std::fs::create_dir(p.join("src")).unwrap();
        Self(p)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove Noir sample directory");
    }
}

#[test]
fn noir_samples_match_host_mirror() {
    let Some(gate) = crate::support::gate("noir_samples_match_host_mirror", &["nargo", "bb"])
    else {
        return;
    };
    for name in ["nargo", "bb"] {
        let out = crate::support::live_tool(&gate, name)
            .arg("--version")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(toolchain::pinned_version_output(name, &out.stdout));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let core = root.join("zk/noir-query/core");
    for subcommand in ["check", "test"] {
        let out = crate::support::live_tool(&gate, "nargo")
            .arg(subcommand)
            .arg("--program-dir")
            .arg(&core)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let scratch = Scratch::new();
    std::fs::write(scratch.0.join("Nargo.toml"),format!("[package]\nname = \"expr_samples\"\ntype = \"lib\"\nauthors = [\"sparq\"]\n[dependencies]\ncore = {{ path = {:?} }}\n",core)).unwrap();
    let mut source = String::from(
        "use core::expr_core::{lane, exact, floating, zero_numeric};\nuse core::expr_arith::binary;\n",
    );
    let mut cases = Vec::new();
    for a in [-7, 0, 9] {
        for b in [-3, 0, 2] {
            for (op, tag) in [
                (ExprOp::Add, 0),
                (ExprOp::Sub, 1),
                (ExprOp::Mul, 2),
                (ExprOp::Div, 3),
            ] {
                cases.push((
                    mirror::typed(a.to_string(), "integer"),
                    mirror::typed(b.to_string(), "integer"),
                    op,
                    tag,
                ));
            }
        }
    }
    for (a, at, b, bt) in [
        ("0.1", "decimal", "0.2", "decimal"),
        ("16777217", "integer", "1", "float"),
        ("0.1", "decimal", "0", "double"),
        ("-0", "float", "0", "float"),
        ("NaN", "double", "2", "integer"),
        ("1", "double", "0", "double"),
        ("bad", "integer", "1", "integer"),
        ("1200", "byte", "2", "integer"),
        (
            "5192296858534827.628530496329220095",
            "decimal",
            "1",
            "integer",
        ),
        (
            "3.4028234663852886e38",
            "float",
            "3.4028234663852886e38",
            "float",
        ),
        ("1.401298464324817e-45", "float", "2", "float"),
        (
            "1.7976931348623157e308",
            "double",
            "1.7976931348623157e308",
            "double",
        ),
        ("4.9406564584124654e-324", "double", "2", "double"),
        ("INF", "double", "-INF", "double"),
        ("-0", "double", "-0", "double"),
        ("34359738368.000003814697265625", "decimal", "0", "double"),
    ] {
        for (op, tag) in [
            (ExprOp::Add, 0),
            (ExprOp::Sub, 1),
            (ExprOp::Mul, 2),
            (ExprOp::Div, 3),
        ] {
            cases.push((mirror::typed(a, at), mirror::typed(b, bt), op, tag));
        }
    }
    for (i, (a, b, op, tag)) in cases.iter().enumerate() {
        compare_engine(&format!("({a}) {} ({b})", op.name()), None);
        let expr = call(
            op.clone(),
            vec![Expr::Constant(a.clone()), Expr::Constant(b.clone())],
        );
        let result = mirror::eval(&expr, &BTreeMap::new());
        let error = result == Err(Error::Sparql);
        let cap = result == Err(Error::Capacity);
        source.push_str(&format!("#[test]\nfn sample_{i}() {{ let (v,e)=binary({}, {}, {tag}); assert_eq(e,{error},\"sample error lane\"); assert_eq(v.capacity,{cap},\"sample capacity lane\");",noir_operand(a),noir_operand(b)));
        if let Ok(term) = result {
            let expected = noir_numeric(&term).unwrap();
            source.push_str(&format!("let expected={expected}; assert_eq(v.value.ty,expected.ty); assert_eq(v.value.negative,expected.negative); assert_eq(v.value.magnitude,expected.magnitude); assert_eq(v.value.bits,expected.bits);"));
        } else {
            source.push_str("assert_eq(v.value.magnitude,0); assert_eq(v.value.bits,0);");
        }
        source.push_str("}\n");
    }
    std::fs::write(scratch.0.join("src/lib.nr"), source).unwrap();
    let out = crate::support::live_tool(&gate, "nargo")
        .arg("test")
        .arg("--program-dir")
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(&format!("{} tests passed", cases.len()))
    );
    // A raw cell has no implementation of the checked-opening contract.
    std::fs::write(scratch.0.join("src/lib.nr"),
        "use core::expr_core::variable;\nuse core::expr_term_contract::TermCell;\npub fn raw(cell:TermCell) { let _=variable::<TermCell,4>(cell); }\n").unwrap();
    let out = crate::support::live_tool(&gate, "nargo")
        .arg("check")
        .arg("--program-dir")
        .arg(&scratch.0)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("CheckedOpening"));
    crate::support::complete(gate);
}
