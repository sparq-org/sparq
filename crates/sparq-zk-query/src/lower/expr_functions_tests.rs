use super::expr::*;
use super::expr_semantics::{self as mirror, Error};
use super::expr_tests::{compare_engine, expression, parsed, slots};
use oxrdf::{Literal, NamedNode, Term};
use spargebra::algebra::{Expression, GraphPattern};
use std::collections::BTreeMap;

#[test]
fn function_lowering_types_and_refusals() {
    for (text, ty) in [
        ("isIRI(<urn:x>)", LaneType::Boolean),
        ("isBlank(<urn:x>)", LaneType::Boolean),
        ("isLiteral(1)", LaneType::Boolean),
        ("isNumeric(1)", LaneType::Boolean),
        ("STR(1)", LaneType::String),
        ("LANG(\"x\")", LaneType::String),
        ("DATATYPE(1)", LaneType::Term),
        ("LANGMATCHES(\"en-US\",\"en\")", LaneType::Boolean),
        ("STRDT(\"1\",<urn:dt>)", LaneType::Term),
        ("STRLANG(\"x\",\"en\")", LaneType::Term),
        ("YEAR(?n)", LaneType::Integer),
        ("SECONDS(?n)", LaneType::Decimal),
        ("TIMEZONE(?n)", LaneType::Duration),
        ("TZ(?n)", LaneType::String),
        ("ABS(-2)", LaneType::Integer),
        ("ROUND(-2.5)", LaneType::Decimal),
        ("CEIL(1e0)", LaneType::Double),
        ("FLOOR(?n)", LaneType::Dynamic),
    ] {
        let dag = lower(&expression(text)).unwrap();
        assert_eq!(dag.nodes[dag.root].ty, ty, "{text}");
    }
    let temporal=lower(&expression("\"2024-01-01T00:00:00.000000000000000001Z\"^^xsd:dateTime > \"2024-01-01T00:00:00Z\"^^xsd:dateTime")).unwrap();
    assert_eq!(
        temporal.nodes[temporal.root].gadget,
        Gadget::TemporalCompare(ExprOp::Gt)
    );
    let d = lower(&expression("DATATYPE(1+2.0)")).unwrap();
    assert_eq!(d.nodes[d.root].gadget, Gadget::NumericDatatype);
    assert!(
        lower(&expression("LANGMATCHES(LANG(\"x\"@en),\"en\")"))
            .unwrap()
            .nodes
            .iter()
            .any(|n| n.gadget == Gadget::AsString)
    );
    assert!(
        lower(&expression("YEAR(?n)"))
            .unwrap()
            .nodes
            .iter()
            .any(|n| n.gadget == Gadget::ReadTemporal)
    );
    for (text, name) in [
        ("NOW()", "NOW"),
        ("STRLEN(\"x\")", "STRLEN"),
        ("SHA256(\"x\")", "SHA256"),
        (
            "xsd:date(\"2024-01-01\")",
            "http://www.w3.org/2001/XMLSchema#date",
        ),
        ("IRI(\"x\")", "IRI"),
    ] {
        let query =
            format!("PREFIX xsd:<http://www.w3.org/2001/XMLSchema#> SELECT ({text} AS ?v) {{}}");
        assert_eq!(
            from_spargebra(&parsed(&query), &slots())
                .unwrap_err()
                .function,
            name
        );
    }
    for op in [
        ExprOp::IsIri,
        ExprOp::Str,
        ExprOp::StrDt,
        ExprOp::Iri,
        ExprOp::Year,
        ExprOp::Round,
    ] {
        assert_eq!(
            lower(&Expr::Call {
                op: op.clone(),
                args: vec![]
            })
            .unwrap_err()
            .function,
            op.name()
        );
    }
}
#[test]
fn term_functions_against_engine() {
    for text in [
        "isIRI(<urn:x>)",
        "isIRI(1)",
        "isBlank(<urn:x>)",
        "isLiteral(1)",
        "isLiteral(<urn:x>)",
        "isIRI(?missing)",
        "isBlank(?missing)",
        "isLiteral(?missing)",
        "isNumeric(?missing)",
        "isNumeric(\"bad\"^^xsd:integer)",
        "isNumeric(\"999999999999999999999999999999999999999999999999\"^^xsd:integer)",
        "isNumeric(\"0.000000000000000000000000000000001\"^^xsd:decimal)",
        "STR(<urn:x>)",
        "STR(\"+005\"^^xsd:integer)",
        "STR(?missing)",
        "LANG(\"chat\"@fr)",
        "LANG(\"chat\")",
        "LANG(<urn:x>)",
        "DATATYPE(\"bad\"^^xsd:integer)",
        "DATATYPE(\"chat\"@en)",
        "DATATYPE(1+2.0)",
        "DATATYPE(\"1\"^^xsd:float+2.0)",
        "STRDT(\"123\",xsd:integer)",
        "STRDT(\"bad\",xsd:integer)",
        "STRDT(1,xsd:integer)",
        "STRDT(\"x\",\"urn:dt\")",
        "STRLANG(\"chat\",\"EN-us\")",
        "STRLANG(\"chat\"@en,\"fr\")",
        "STRLANG(\"x\",\"\")",
        "STRLANG(\"x\",\"-en\")",
        "STRLANG(\"x\",\"abcd-USA\")",
        "STRLANG(\"x\",\"i-klingon\")",
        "LANGMATCHES(\"en-US\",\"en\")",
        "LANGMATCHES(\"eng\",\"en\")",
        "LANGMATCHES(\"\",\"*\")",
        "LANGMATCHES(\"en\",\"\")",
        "LANGMATCHES(\"en\\u0000\",\"en\")",
    ] {
        compare_engine(text, None);
    }
    for text in [
        "LANGMATCHES(\"en\"@en,\"en\")",
        "LANGMATCHES(\"en\",\"en\"@en)",
        "LANGMATCHES(\"en\"@en,\"en\"@fr)",
    ] {
        compare_engine(
            text,
            Some(
                "LANGMATCHES requires simple string arguments; the engine accepts language-tagged arguments",
            ),
        );
    }
    let blank = Term::BlankNode(oxrdf::BlankNode::new("b").unwrap());
    for (op, expected) in [
        (ExprOp::IsBlank, true),
        (ExprOp::IsIri, false),
        (ExprOp::IsLiteral, false),
    ] {
        let ir = Expr::Call {
            op,
            args: vec![Expr::Constant(blank.clone())],
        };
        assert_eq!(
            mirror::eval(&ir, &BTreeMap::new()),
            Ok(mirror::bool_term(expected))
        );
    }
}
#[test]
fn numeric_functions_generated_against_engine() {
    for name in ["ABS", "ROUND", "CEIL", "FLOOR"] {
        for value in -12..=12 {
            for suffix in ["", ".0", ".25", ".5", ".75"] {
                compare_engine(&format!("{name}({value}{suffix})"), None);
            }
        }
        for ty in ["float", "double"] {
            for value in [
                "-0",
                "-0.25",
                "-0.5",
                "-2.5",
                "0.5",
                "2.5",
                "INF",
                "-INF",
                "NaN",
                "1.401298464324817e-45",
                "16777216",
            ] {
                compare_engine(&format!("{name}(\"{value}\"^^xsd:{ty})"), None);
            }
        }
        for value in [
            "?missing",
            "<urn:x>",
            "\"bad\"^^xsd:integer",
            "\"1200\"^^xsd:byte",
        ] {
            compare_engine(&format!("{name}({value})"), None);
        }
    }
}
#[test]
fn date_functions_generated_against_engine() {
    for function in [
        "YEAR", "MONTH", "DAY", "HOURS", "MINUTES", "SECONDS", "TIMEZONE", "TZ",
    ] {
        for value in [
            "2011-01-10T14:45:13.815-05:00",
            "2024-02-29T23:59:59Z",
            "1999-12-31T24:00:00-05:00",
            "2024-02-28T24:00:00+14:00",
            "1970-01-01T00:00:00+00:00",
            "1900-03-01T00:00:00",
            "2024-01-01T00:00:00.000001Z",
            "2023-02-29T00:00:00Z",
            "2024-01-01T25:00:00Z",
            "2024-01-01T00:00:00+14:01",
            "0000-01-01T00:00:00Z",
        ] {
            compare_engine(
                &format!("{function}(\"{value}\"^^xsd:dateTime)"),
                if value.starts_with("0000-") {
                    Some("XML Schema 1.0 dateTime excludes year zero; the engine accepts it")
                } else {
                    None
                },
            );
        }
        for arg in [
            "?missing",
            "1",
            "\"2024-01-01T00:00:00Z\"",
            "\"2024-01-01\"^^xsd:date",
        ] {
            compare_engine(&format!("{function}({arg})"), None);
        }
    }
    for day in 1..=28 {
        compare_engine(
            &format!("DAY(\"2024-02-{day:02}T01:02:03Z\"^^xsd:dateTime)"),
            None,
        );
    }
}
#[test]
fn iri_public_base_resolution_against_engine() {
    let base = NamedNode::new("http://example.org/a/b").unwrap();
    for value in [
        "../c",
        "?q",
        "#f",
        "/root",
        "//other/x",
        "",
        "http://other/a/../b",
        "bad space",
        "bad%XZ",
        "http://host:no",
    ] {
        let query = format!(
            "BASE <{}> SELECT (IRI({}) AS ?v) {{}}",
            base.as_str(),
            Literal::new_simple_literal(value)
        );
        let ir = from_spargebra_with_context(&parsed(&query), &slots(), &[], Some(&base)).unwrap();
        let dag = lower(&ir).unwrap();
        assert!(
            dag.constants
                .iter()
                .any(|p| p.value == Term::NamedNode(base.clone()))
        );
        let expected = match mirror::eval(&ir, &BTreeMap::new()) {
            Ok(t) => Some(t),
            Err(Error::Sparql) => None,
            Err(Error::Capacity) => panic!("ASCII sample capacity"),
        };
        let rows = sparq_engine::query(&sparq_core::Graph::new(), &query)
            .unwrap()
            .rows;
        assert_eq!(rows, vec![vec![expected]], "{query}");
    }
}
#[test]
fn builtin_temporal_multi_projection_entries() {
    fn walk(
        p: &GraphPattern,
        expressions: &mut BTreeMap<String, Expression>,
        value: &mut Option<Term>,
    ) {
        match p {
            GraphPattern::Project { inner, .. } => walk(inner, expressions, value),
            GraphPattern::Extend {
                inner,
                variable,
                expression,
            } => {
                expressions.insert(variable.as_str().to_owned(), expression.clone());
                walk(inner, expressions, value);
            }
            GraphPattern::Values {
                variables,
                bindings,
            } => {
                assert_eq!(variables[0].as_str(), "d");
                *value = bindings[0][0].clone().map(Into::into);
            }
            _ => panic!("temporal fixture shape"),
        }
    }
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../sparq-engine/tests/fixtures/builtin_edges.json"
    ))
    .unwrap();
    for case in corpus["cases"].as_array().unwrap().iter().filter(|c| {
        matches!(
            c["id"].as_str(),
            Some("date-end-of-year" | "date-end-of-leap-month" | "date-end-before-leap-day")
        )
    }) {
        let query = case["query"].as_str().unwrap();
        let spargebra::Query::Select { pattern, .. } =
            spargebra::SparqlParser::new().parse_query(query).unwrap()
        else {
            panic!("SELECT");
        };
        let GraphPattern::Project { variables, .. } = &pattern else {
            panic!("projection");
        };
        let mut expressions = BTreeMap::new();
        let mut value = None;
        walk(&pattern, &mut expressions, &mut value);
        let mut s = slots();
        s.insert("d".into(), (4, LaneType::DateTime));
        let vars = BTreeMap::from([(4, value.unwrap())]);
        let expected = variables
            .iter()
            .map(|v| {
                let ir = from_spargebra(&expressions[v.as_str()], &s).unwrap();
                lower(&ir).unwrap();
                mirror::eval(&ir, &vars).ok()
            })
            .collect::<Vec<_>>();
        let rows = sparq_engine::query(&sparq_core::Graph::new(), query)
            .unwrap()
            .rows;
        assert_eq!(rows, vec![expected], "{}", case["id"]);
        assert_eq!(
            serde_json::json!(
                rows.iter()
                    .map(|r| r
                        .iter()
                        .map(|t| t.as_ref().map(ToString::to_string))
                        .collect::<Vec<_>>())
                    .collect::<Vec<_>>()
            ),
            case["expected_rows"]
        );
    }
}

fn noir_bytes(value: &str) -> String {
    format!(
        "[{}]",
        value
            .as_bytes()
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn noir_term(term: &Term) -> String {
    match term {
        Term::NamedNode(n) => format!("fixture(KIND_IRI,0,{})", noir_bytes(n.as_str())),
        Term::BlankNode(b) => format!("fixture(KIND_BLANK_NODE,0,{})", noir_bytes(b.as_str())),
        Term::Literal(l) => {
            let local = l
                .datatype()
                .as_str()
                .strip_prefix(mirror::XSD)
                .unwrap_or("")
                .to_ascii_uppercase();
            let dt = if l.language().is_some() {
                "expressions::expr_term_contract::DT_LANGSTRING".to_owned()
            } else {
                match local.as_str() {
                    "STRING" | "BOOLEAN" | "INTEGER" | "DECIMAL" | "FLOAT" | "DOUBLE"
                    | "DATETIME" | "DATE" | "BYTE" | "UNSIGNEDLONG" => {
                        format!("expressions::expr_term_contract::DT_{local}")
                    }
                    _ => "777".into(),
                }
            };
            format!(
                "{{let mut a=literal({},{});a.value.datatype=bytes({});a.value.language=bytes({});a}}",
                noir_bytes(l.value()),
                dt,
                noir_bytes(l.datatype().as_str()),
                noir_bytes(l.language().unwrap_or(""))
            )
        }
        _ => panic!("RDF 1.1 sample"),
    }
}
fn noir_text_argument(term: &Term) -> String {
    let Term::Literal(l) = term else {
        return "(lane(text([])),true)".into();
    };
    let error = l.datatype().as_str() != format!("{}string", mirror::XSD) && l.language().is_none();
    format!(
        "{{let mut s=text({});s.lang={};(lane(s),{})}}",
        noir_bytes(l.value()),
        u8::from(l.language().is_some()),
        error
    )
}
fn sample_call(op: &ExprOp, args: &[Term]) -> String {
    let a = || format!("(lane({}),false)", noir_term(&args[0]));
    match op {
        ExprOp::Abs | ExprOp::Round | ExprOp::Ceil | ExprOp::Floor => format!(
            "numeric_fn::numeric({},{})",
            super::expr_tests::noir_operand(&args[0]),
            match op {
                ExprOp::Abs => 0,
                ExprOp::Round => 1,
                ExprOp::Ceil => 2,
                _ => 3,
            }
        ),
        ExprOp::Year
        | ExprOp::Month
        | ExprOp::Day
        | ExprOp::Hours
        | ExprOp::Minutes
        | ExprOp::Seconds => format!(
            "date_fn::component(date_fn::read_date({}),{})",
            a(),
            match op {
                ExprOp::Year => 0,
                ExprOp::Month => 1,
                ExprOp::Day => 2,
                ExprOp::Hours => 3,
                ExprOp::Minutes => 4,
                _ => 5,
            }
        ),
        ExprOp::Tz | ExprOp::Timezone => format!(
            "date_fn::{}(date_fn::read_date({}))",
            if *op == ExprOp::Tz { "tz" } else { "timezone" },
            a()
        ),
        ExprOp::LangMatches => format!(
            "term_fn::lang_matches({},{})",
            noir_text_argument(&args[0]),
            noir_text_argument(&args[1])
        ),
        ExprOp::StrDt | ExprOp::StrLang => format!(
            "term_fn::{}({},(lane({}),false))",
            if *op == ExprOp::StrDt {
                "strdt"
            } else {
                "strlang"
            },
            a(),
            noir_term(&args[1])
        ),
        ExprOp::Iri => {
            let Term::NamedNode(base) = &args[1] else {
                panic!("base");
            };
            format!("term_fn::iri({},text({}))", a(), noir_bytes(base.as_str()))
        }
        _ => format!(
            "term_fn::{}({})",
            match op {
                ExprOp::IsIri => "is_iri",
                ExprOp::IsBlank => "is_blank",
                ExprOp::IsLiteral => "is_literal",
                ExprOp::IsNumeric => "is_numeric",
                ExprOp::Str => "str_value",
                ExprOp::Lang => "lang",
                ExprOp::Datatype => "datatype",
                _ => panic!("sample function"),
            },
            a()
        ),
    }
}
fn assert_noir_text(source: &mut String, value: &str, expected: &str) {
    source.push_str(&format!("check_text({value},{});", noir_bytes(expected)));
}
#[test]
fn noir_function_samples_match_host_mirror() {
    let Some(gate) =
        crate::support::gate("noir_function_samples_match_host_mirror", &["nargo", "bb"])
    else {
        return;
    };
    for name in ["nargo", "bb"] {
        let out = crate::support::live_tool(&gate, name)
            .arg("--version")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(super::expr_tests::toolchain::pinned_version_output(
            name,
            &out.stdout
        ));
    }
    let core_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../zk/noir-query/core")
        .canonicalize()
        .unwrap();
    let scratch = super::expr_test_support::Scratch::new("sparq_noir_functions");
    std::fs::create_dir(scratch.0.join("src")).unwrap();
    std::fs::write(scratch.0.join("Nargo.toml"),format!("[package]\nname=\"nd_function_samples\"\ntype=\"lib\"\n[dependencies]\nexpressions={{path={:?}}}\n",core_dir)).unwrap();
    let core = include_str!("../../../../zk/noir-query/core/src/expr_functions_tests.nr");
    let mut source = core
        .split("#[test]")
        .next()
        .unwrap()
        .replace("crate::", "expressions::");
    source.push_str("use expressions::expr_core::zero_numeric;\n");
    let mut cases = Vec::new();
    for (op, name) in [
        (ExprOp::Abs, "ABS"),
        (ExprOp::Round, "ROUND"),
        (ExprOp::Ceil, "CEIL"),
        (ExprOp::Floor, "FLOOR"),
    ] {
        for term in [
            mirror::typed("-2.5", "decimal"),
            mirror::typed("-0.25", "float"),
            mirror::typed("-0.25", "double"),
            mirror::typed("NaN", "double"),
            mirror::typed("-INF", "float"),
            mirror::typed("281474976710656", "integer"),
            mirror::typed("5192296858534827.628530496329220095", "decimal"),
            mirror::typed("bad", "integer"),
        ] {
            compare_engine(&format!("{name}({term})"), None);
            cases.push((op.clone(), vec![term]));
        }
    }
    for op in [
        ExprOp::IsIri,
        ExprOp::IsBlank,
        ExprOp::IsLiteral,
        ExprOp::IsNumeric,
        ExprOp::Str,
        ExprOp::Lang,
        ExprOp::Datatype,
    ] {
        for term in [
            Literal::new_simple_literal("x").into(),
            mirror::typed("bad", "integer"),
        ] {
            cases.push((op.clone(), vec![term]));
        }
    }
    cases.push((ExprOp::IsIri, vec![NamedNode::new("urn:x").unwrap().into()]));
    cases.push((
        ExprOp::IsBlank,
        vec![oxrdf::BlankNode::new("b").unwrap().into()],
    ));
    cases.push((
        ExprOp::IsNumeric,
        vec![mirror::typed(
            "999999999999999999999999999999999999999999999999",
            "integer",
        )],
    ));
    cases.push((ExprOp::IsNumeric, vec![mirror::typed("1200", "byte")]));
    cases.push((
        ExprOp::Lang,
        vec![
            Literal::new_language_tagged_literal("chat", "fr")
                .unwrap()
                .into(),
        ],
    ));
    cases.push((
        ExprOp::StrDt,
        vec![
            Literal::new_simple_literal("123").into(),
            NamedNode::new(format!("{}integer", mirror::XSD))
                .unwrap()
                .into(),
        ],
    ));
    cases.push((
        ExprOp::StrLang,
        vec![
            Literal::new_simple_literal("chat").into(),
            Literal::new_simple_literal("EN-us").into(),
        ],
    ));
    cases.push((
        ExprOp::StrLang,
        vec![
            Literal::new_simple_literal("chat").into(),
            Literal::new_simple_literal("-en").into(),
        ],
    ));
    for (tag, range) in [("en-US", "en"), ("eng", "en"), ("", "*"), ("en\0", "en")] {
        cases.push((
            ExprOp::LangMatches,
            vec![
                Literal::new_simple_literal(tag).into(),
                Literal::new_simple_literal(range).into(),
            ],
        ));
    }
    for op in [
        ExprOp::Year,
        ExprOp::Month,
        ExprOp::Day,
        ExprOp::Hours,
        ExprOp::Minutes,
        ExprOp::Seconds,
        ExprOp::Timezone,
        ExprOp::Tz,
    ] {
        for value in [
            "2011-01-10T14:45:13.815-05:00",
            "1999-12-31T24:00:00Z",
            "2024-01-01T00:00:00",
            "2023-02-29T00:00:00Z",
        ] {
            cases.push((op.clone(), vec![mirror::typed(value, "dateTime")]));
        }
    }
    for (i, (op, args)) in cases.iter().enumerate() {
        let result = super::expr_functions_semantics::function(
            op,
            &args.iter().cloned().map(Ok).collect::<Vec<_>>(),
        );
        let error = result == Err(Error::Sparql);
        let capacity = result == Err(Error::Capacity);
        source.push_str(&format!("#[test]\nfn function_sample_{i}() {{let (v,e)={};assert_eq(e,{error},\"function error\");assert_eq(v.capacity,{capacity},\"function capacity\");",sample_call(op,args)));
        if let Ok(term) = result {
            match op {
                ExprOp::Abs
                | ExprOp::Round
                | ExprOp::Ceil
                | ExprOp::Floor
                | ExprOp::Year
                | ExprOp::Month
                | ExprOp::Day
                | ExprOp::Hours
                | ExprOp::Minutes
                | ExprOp::Seconds => {
                    source.push_str(&format!("let expected={};assert_eq(v.value.ty,expected.ty);assert_eq(v.value.negative,expected.negative);assert_eq(v.value.magnitude,expected.magnitude);assert_eq(v.value.bits,expected.bits);",super::expr_tests::noir_numeric(&term).unwrap()));
                }
                ExprOp::IsIri
                | ExprOp::IsBlank
                | ExprOp::IsLiteral
                | ExprOp::IsNumeric
                | ExprOp::LangMatches => {
                    source.push_str(&format!(
                        "assert_eq(v.value,{});",
                        term == mirror::bool_term(true)
                    ));
                }
                ExprOp::Str | ExprOp::Lang | ExprOp::Tz => {
                    let Term::Literal(l) = term else {
                        panic!("string");
                    };
                    assert_noir_text(&mut source, "v.value", l.value());
                }
                ExprOp::Datatype => {
                    let Term::NamedNode(n) = term else {
                        panic!("IRI");
                    };
                    source.push_str("assert_eq(v.value.kind,KIND_IRI);");
                    assert_noir_text(&mut source, "v.value.lexical", n.as_str());
                }
                ExprOp::StrDt | ExprOp::StrLang => {
                    let Term::Literal(l) = term else {
                        panic!("literal");
                    };
                    source.push_str("assert_eq(v.value.kind,KIND_LITERAL);");
                    assert_noir_text(&mut source, "v.value.lexical", l.value());
                    assert_noir_text(&mut source, "v.value.datatype", l.datatype().as_str());
                    assert_noir_text(&mut source, "v.value.language", l.language().unwrap_or(""));
                }
                ExprOp::Timezone => {
                    let d = super::expr_functions_semantics::date(&args[0]).unwrap();
                    let minutes = d.offset.unwrap();
                    source.push_str(&format!(
                        "assert_eq(v.value.negative,{});assert_eq(v.value.microseconds,{});",
                        minutes < 0,
                        u64::from(minutes.unsigned_abs()) * 60 * 1_000_000
                    ));
                }
                _ => panic!("sample result"),
            }
        }
        source.push_str("}\n");
    }
    std::fs::write(scratch.0.join("src/lib.nr"), source).unwrap();
    let out = crate::support::live_tool(&gate, "nargo")
        .args(["test", "--silence-warnings", "--program-dir"])
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
    crate::support::complete(gate);
}
