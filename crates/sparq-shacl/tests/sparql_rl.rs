#![cfg(feature = "sparql-rl")]

use sparq_shacl::sparql_rl::{parse, Element, ErrorKind, Statement};

const BASE: &str = "http://example.org/rules/";

#[test]
fn preserves_directive_scope_and_ordered_rule_elements() {
    let source = r#"BASE <../>
PREFIX ex: <vocab/>
VERSION '1.2'
IMPORTS <library.srl>
DATA { ex:a ex:p 1 }
RULE ex:rule { ?s ex:count ?n } WHERE DATA {
    ?s ex:p ?o .
    FILTER(?o > 0)
    SET($x := ?o + 1)
    NOT DATA { ?s ex:excluded true FILTER(isIRI(?s)) }
    GROUP BY ?s { ?s ex:p ?v SET(?w := ?v * 2) }
        [ ?n := COUNT(DISTINCT ?v), $m := SUM(?w) ]
}
PREFIX ex: <other/>
RULE {} WHERE {}
"#;
    let doc = parse(source, BASE).unwrap();
    assert_eq!(doc.statements.len(), 8);
    assert_eq!(doc.statements[0], Statement::Base("<../>"));
    assert_eq!(
        doc.statements[1],
        Statement::Prefix {
            prefix: "ex:",
            iri: "<vocab/>"
        }
    );
    assert_eq!(doc.statements[2], Statement::Version("'1.2'"));
    assert_eq!(doc.statements[3], Statement::Imports("<library.srl>"));
    assert_eq!(doc.statements[4], Statement::Data("{ ex:a ex:p 1 }"));
    let Statement::Rule(rule) = &doc.statements[5] else {
        panic!("rule")
    };
    assert_eq!(rule.name, Some("ex:rule"));
    assert_eq!(rule.head, "{ ?s ex:count ?n }");
    assert!(rule.base_only);
    assert_eq!(rule.body.len(), 5);
    assert_eq!(rule.body[0], Element::Triples("?s ex:p ?o ."));
    assert_eq!(rule.body[1], Element::Filter("(?o > 0)"));
    assert_eq!(
        rule.body[2],
        Element::Assignment {
            variable: "$x",
            expression: "?o + 1"
        }
    );
    let Element::Negation { base_only, body } = &rule.body[3] else {
        panic!("negation")
    };
    assert!(base_only);
    assert_eq!(body.len(), 2);
    let Element::Group {
        keys,
        body,
        assignments,
    } = &rule.body[4]
    else {
        panic!("group")
    };
    assert_eq!(keys, &["?s"]);
    assert_eq!(body.len(), 2);
    assert_eq!(assignments[0].variable, "?n");
    assert_eq!(assignments[0].function, "COUNT(DISTINCT ?v)");
    assert_eq!(assignments[1].variable, "$m");
    assert_eq!(assignments[1].function, "SUM(?w)");
    assert_eq!(
        doc.statements[6],
        Statement::Prefix {
            prefix: "ex:",
            iri: "<other/>"
        }
    );
}

#[test]
fn lexical_boundaries_escapes_and_comments() {
    for source in [
        "# comment at EOF",
        "bAsE <http://example/> pReFiX : <v/> rUlE {} wHeRe { ?é :p-1 'x#y'@EN--rtl }",
        r#"PREFIX : <http://example/> DATA { :a :p '''long ' " # text''' ; :q "\u0041\\uD800" }"#,
        "PREFIX : <http://example/> DATA { :a :p 1.0e+2, -.5, +3, true, FALSE }",
        "PREFIX : <http://example/> RULE {} WHERE { ?s :p ?x FILTER(?x+1*2 < 10 && ?x NOT IN (2,3)) }",
        "PREFIX : <http://example/> DATA { :a\\~b :p%20x :o. }",
        "PREFIX : <http://example/> RULE { :a :p <<( :a :p <<( :b :p :c )>> )>> } WHERE {}",
        "PREFIX : <http://example/> RULE {} WHERE { ?s ^(:p/^:q) ?o }",
        "PREFIX : <http://example/> RULE {} WHERE { SET(?t := <<( ?s :p <<( ?a :q ?b )>> )>>) }",
        "PREFIX : <http://example/> DATA { << :a :p :b ~:r >> :q [ :p (1 2) ] }",
    ] { assert!(parse(source, BASE).is_ok(), "{source}: {:?}", parse(source, BASE)); }
    for source in [
        "RULE {} WHERE {} garbage",
        "RULE {} WHERE { :s :p :o } PREFIX : <http://example/>",
        "PREFIX : <http://example/> DATA { :a :p 1. . }",
        "PREFIX : <http://example/> DATA { :a :p _:b. ; :q 2 }",
        "PREFIX : <http://example/> DATA { :a :p 'a'@en--LTR }",
        "PREFIX : <http://example/> DATA { :a :p 'a'@en--sideways }",
        r"DATA { <urn:a> <urn:p> '\uD800' }",
        r"DATA { <urn:a> <urn:p> '\U00110000' }",
        r"DATA { <urn:a> <urn:p> '\q' }",
        r"DATA { <urn:\uDFFF> <urn:p> 1 }",
        "DATA { <urn:a> <urn:p> 'unterminated }",
        "DATA { <space iri> <urn:p> 1 }",
        "DATA { <urn:a> <urn:p> 1 }\u{a0}",
    ] {
        assert_eq!(
            parse(source, BASE).unwrap_err().kind,
            ErrorKind::Syntax,
            "{source}"
        );
    }
}

#[test]
fn grammar_modes_exclude_broader_sparql_and_data_variables() {
    for body in [
        "OPTIONAL { ?s ?p ?o }",
        "{ ?s ?p ?o } UNION { ?s ?p ?o }",
        "BIND(1 AS ?v)",
        "VALUES ?v { 1 }",
        "SERVICE <urn:remote> { ?s ?p ?o }",
        "?s <urn:p>* ?o",
        "?s <urn:p>+ ?o",
        "?s <urn:p>|<urn:q> ?o",
        "FILTER(BOUND(?v))",
        "SET(?v := RAND())",
        "FILTER(COUNT(?x)>1)",
        "NOT { NOT {} }",
        "NOT { SET(?v := 1) }",
        "NOT { GROUP {} [] }",
        "GROUP { NOT {} } []",
        "GROUP { GROUP {} [] } []",
        "GROUP BY {} []",
        "GROUP {} [?n := SUM(DISTINCT ?v)]",
        "GROUP {} [?n := MIN(*)]",
        "GROUP {} [?n := COUNT(?v+1)]",
        "GROUP {} [?n := COUNT(*),]",
        "GROUP {} [?n := AVG(?v)]",
        "FILTER STR()",
        "FILTER STR(1,2)",
        "SET(?x := (1<2<3))",
        "SET(?x := <<( <<( ?s <urn:p> ?o )>> <urn:p> ?o )>>)",
    ] {
        let source = format!("RULE {{}} WHERE {{ {body} }}");
        assert_eq!(
            parse(&source, BASE).unwrap_err().kind,
            ErrorKind::Syntax,
            "{source}"
        );
    }
    for data in [
        "?s <urn:p> 1",
        "<urn:s> ?p 1",
        "<urn:s> <urn:p> ?o",
        "<urn:s> <urn:p> <<( ?s <urn:p> 1 )>>",
        "<urn:s> <urn:p> 1 ~?r",
    ] {
        assert!(
            parse(&format!("DATA {{ {data} }}"), BASE).is_err(),
            "{data}"
        );
    }
    assert!(parse("RULE { <urn:s> <urn:p>/<urn:q> ?o } WHERE {}", BASE).is_err());
    // Syntax succeeds even when later stages must reject the rule.
    assert!(parse(
        "RULE { ?unbound <urn:p> 1 } WHERE { SET(?x := ?missing) SET(?x := 2) }",
        BASE
    )
    .is_ok());
    assert!(parse(
        "RULE { ?s <urn:p> 1 } WHERE { ?s <urn:q> 1 NOT { ?s <urn:p> 1 } }",
        BASE
    )
    .is_ok());
}

#[test]
fn builtin_arity_and_aggregate_grammar() {
    for expr in [
        "NOW()",
        "UUID()",
        "STRUUID()",
        "BNODE()",
        "BNODE('x')",
        "CONCAT()",
        "CONCAT('a','b')",
        "SUBSTR('abc',1)",
        "REGEX('x','x','i')",
        "REPLACE('a','a','b','i')",
        "IF(true,1,2)",
        "STRLANGDIR('x','en','ltr')",
        "TRIPLE(<urn:s>,<urn:p>,1)",
        "<urn:custom>()",
    ] {
        assert!(
            parse(&format!("RULE {{}} WHERE {{ SET(?v := {expr}) }}"), BASE).is_ok(),
            "{expr}"
        );
    }
    assert!(parse("RULE {} WHERE { GROUP {} [?a := COUNT(DISTINCT *), ?b := MIN(?x), ?c := MAX(?x)] GROUP BY ?x {} [] }", BASE).is_ok());
}

#[test]
fn errors_have_original_positions_and_limits_are_distinct() {
    let error = parse("# é\r\nRULE {}\rWHERE {\n  @\n}", BASE).unwrap_err();
    assert_eq!((error.line, error.column, error.offset), (4, 3, 24));
    assert_eq!(error.kind, ErrorKind::Syntax);
    assert!(parse("", "relative").is_err());
    let deep = format!(
        "RULE {{}} WHERE {{ FILTER({}1{}) }}",
        "(".repeat(100),
        ")".repeat(100)
    );
    assert_eq!(parse(&deep, BASE).unwrap_err().kind, ErrorKind::Limit);
    let long = " ".repeat(1_048_577);
    assert_eq!(parse(&long, BASE).unwrap_err().kind, ErrorKind::Limit);
    assert!(parse(&" ".repeat(1_048_576), BASE).is_ok());
    let flat = "RULE {} WHERE {}\n".repeat(2000);
    assert_eq!(parse(&flat, BASE).unwrap().statements.len(), 2000);
}

// A test-only concrete-syntax writer: no serialization API is added in this round.
fn write_body(elements: &[Element<'_>]) -> String {
    elements
        .iter()
        .map(|element| match element {
            Element::Triples(text) => text.to_string(),
            Element::Filter(text) => format!("FILTER {text}"),
            Element::Assignment {
                variable,
                expression,
            } => format!("SET({variable} := {expression})"),
            Element::Negation { base_only, body } => format!(
                "NOT {}{{ {} }}",
                if *base_only { "DATA " } else { "" },
                write_body(body)
            ),
            Element::Group {
                keys,
                body,
                assignments,
            } => format!(
                "GROUP {}{{ {} }} [{}]",
                if keys.is_empty() {
                    String::new()
                } else {
                    format!("BY {} ", keys.join(" "))
                },
                write_body(body),
                assignments
                    .iter()
                    .map(|a| format!("{} := {}", a.variable, a.function))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn write_document(document: &sparq_shacl::sparql_rl::Document<'_>) -> String {
    document
        .statements
        .iter()
        .map(|statement| match statement {
            Statement::Base(iri) => format!("BASE {iri}"),
            Statement::Prefix { prefix, iri } => format!("PREFIX {prefix} {iri}"),
            Statement::Version(version) => format!("VERSION {version}"),
            Statement::Imports(iri) => format!("IMPORTS {iri}"),
            Statement::Data(text) => format!("DATA {text}"),
            Statement::Rule(rule) => format!(
                "RULE {}{} WHERE {}{{ {} }}",
                rule.name.map_or(String::new(), |name| format!("{name} ")),
                rule.head,
                if rule.base_only { "DATA " } else { "" },
                write_body(&rule.body)
            ),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn generated_documents_reparse_with_the_same_tree_and_canonical_spelling() {
    for i in 0..64 {
        let text = format!("BASE <../{i}/> PREFIX : <v/> VERSION '1.2' IMPORTS <lib.srl> DATA {{ :s :p {i} }} RULE :r {{ ?s :q ?n }} WHERE {}{{ ?s :p ?v . FILTER(?v >= {i}) SET(?z := ?v+1) NOT {}{{ ?s :blocked true }} GROUP {}{{ ?s :p ?x SET(?y := ?x*2) }} [?n := COUNT(DISTINCT *), ?m := SUM(?y)] }}",
            if i & 1 != 0 { "DATA " } else { "" },
            if i & 2 != 0 { "DATA " } else { "" },
            if i & 4 != 0 { "BY ?s " } else { "" });
        let first = parse(&text, BASE).unwrap();
        let canonical = write_document(&first);
        let second = parse(&canonical, BASE).unwrap();
        assert_eq!(first, second);
        assert_eq!(write_document(&second), canonical);
    }
}

#[test]
fn generated_token_noise_and_all_truncations_return_without_panicking() {
    let example = "PREFIX : <http://example/> RULE { ?s :q ?n } WHERE { ?s :p ?v GROUP BY ?s { ?s :p ?o } [?n := COUNT(*)] }";
    for end in 0..=example.len() {
        let _ = parse(&example[..end], BASE);
    }
    let alphabet: Vec<_> = "{}[]()<>~|#\\\"'?$:+-*/@_%é中\n\r\t0129az"
        .chars()
        .collect();
    let mut state = 0x83f1_a295_u32;
    for _ in 0..1024 {
        let mut text = String::new();
        for _ in 0..128 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            text.push(alphabet[state as usize % alphabet.len()]);
        }
        let _ = parse(&text, BASE);
    }
}
