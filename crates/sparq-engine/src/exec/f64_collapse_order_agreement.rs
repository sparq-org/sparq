use super::*;

fn lex(t: &Option<Term>) -> String {
    match t {
        Some(Term::Literal(l)) => l.value().to_string(),
        // Positional arg (not inline `{other:?}`) to dodge the CodeQL
        // `rust/unused-variable` false positive.
        other => panic!("expected a literal sort key, got {:?}", other),
    }
}

fn order_by(g: &Graph, clause: &str) -> Vec<String> {
    let q = format!("PREFIX ex: <http://ex/> SELECT ?v WHERE {{ ?s ex:v ?v }} ORDER BY {}", clause);
    crate::query(g, &q).unwrap().rows.iter().map(|r| lex(&r[0])).collect()
}

#[test]
fn order_by_agrees_with_minmax_on_big_integers_beyond_2_53() {
    // 2^53 (…992) and 2^53 + 1 (…993) share ONE f64 (…993 is not representable). Both
    // fit i64, so the graph interns them as exact `xsd:integer` terms; the numerics
    // cache stores only f64, collapsing them. The ids are laid out big-then-small so a
    // COLLAPSE (Equal) sort would keep the input order — the ASC/DESC checks catch it.
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> .\n\
             ex:a ex:v 9007199254740993 .\n\
             ex:b ex:v 9007199254740992 .\n",
        "turtle",
    )
    .unwrap();
    let small = "9007199254740992".to_string();
    let big = "9007199254740993".to_string();

    // ORDER BY orders by VALUE now — ASC and DESC are proper reverses (pre-fix they were
    // IDENTICAL, both the collapsed input order: THIS is the revert detector).
    assert_eq!(order_by(&g, "ASC(?v)"), vec![small.clone(), big.clone()]);
    assert_eq!(order_by(&g, "DESC(?v)"), vec![big.clone(), small.clone()]);

    // MIN/MAX (already exact via `num_compare`) agree with ORDER BY's endpoints.
    let agg = crate::query(
        &g,
        "PREFIX ex: <http://ex/> SELECT (MIN(?v) AS ?mn) (MAX(?v) AS ?mx) WHERE { ?s ex:v ?v }",
    )
    .unwrap();
    assert_eq!(lex(&agg.rows[0][0]), small, "MIN == ORDER BY ASC head");
    assert_eq!(lex(&agg.rows[0][1]), big, "MAX == ORDER BY DESC head");

    // And relational `<` sees them distinct (b < a): the FILTER keeps exactly the b row.
    let rel = crate::query(
        &g,
        "PREFIX ex: <http://ex/> SELECT ?vb WHERE { ex:b ex:v ?vb . ex:a ex:v ?va . FILTER(?vb < ?va) }",
    )
    .unwrap();
    assert_eq!(rel.rows.len(), 1, "b < a must hold exactly");
    assert_eq!(lex(&rel.rows[0][0]), small);
}

#[test]
fn order_by_orders_high_precision_decimals_by_value() {
    // Two xsd:decimals differing only in the 18th fraction digit — they share one f64.
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             ex:a ex:v \"0.123456789012345679\"^^xsd:decimal .\n\
             ex:b ex:v \"0.123456789012345678\"^^xsd:decimal .\n",
        "turtle",
    )
    .unwrap();
    let small = "0.123456789012345678".to_string();
    let big = "0.123456789012345679".to_string();
    assert_eq!(order_by(&g, "ASC(?v)"), vec![small.clone(), big.clone()]);
    assert_eq!(order_by(&g, "DESC(?v)"), vec![big, small]);
}

#[test]
fn compare_values_exact_rechecks_f64_collapsed_numeric_values() {
    // Directly exercises the substrate `exact_cmp` hook the engine implements for `Value`
    // (the general `compare_values` path — the MIN/MAX mixed-type fallback and any consumer
    // that orders numeric Values). Two integers beyond 2^53 share an f64 yet must order.
    let a = Value::Term(Term::Literal(Literal::new_typed_literal("9007199254740993", xsd::INTEGER)));
    let b = Value::Term(Term::Literal(Literal::new_typed_literal("9007199254740992", xsd::INTEGER)));
    assert_eq!(as_num(&a), as_num(&b), "the f64 arm COLLAPSES the pair");
    assert_eq!(compare_values(&a, &b), Some(Ordering::Greater));
    assert_eq!(compare_values(&b, &a), Some(Ordering::Less));

    // High-precision decimals sharing one f64.
    let c = Value::Term(Term::Literal(Literal::new_typed_literal("0.123456789012345679", xsd::DECIMAL)));
    let d = Value::Term(Term::Literal(Literal::new_typed_literal("0.123456789012345678", xsd::DECIMAL)));
    assert_eq!(as_num(&c), as_num(&d));
    assert_eq!(compare_values(&c, &d), Some(Ordering::Greater));

    // A genuinely equal-VALUED cross-type pair stays Equal (no spurious inequality):
    // 1 (integer) vs 1.0 (decimal).
    let one_int = Value::Term(Term::Literal(Literal::new_typed_literal("1", xsd::INTEGER)));
    let one_dec = Value::Num(Num::Dec(Dec { mant: 10, scale: 1 }));
    assert_eq!(compare_values(&one_int, &one_dec), Some(Ordering::Equal));

    // Two xsd:doubles that are f64-equal ARE equal: the engine's `exact_cmp` delegates to
    // `num_compare`, whose f64 fallback (doubles have no exact decimal tier) returns Equal
    // — so the collapse is correctly left in place, never a spurious inequality.
    let dbl = Value::Num(Num::Double(9_007_199_254_740_992.0));
    assert_eq!(dbl.exact_cmp(&dbl), Some(Ordering::Equal));
    // A non-numeric operand has no exact recheck at all -> `None` (the f64 verdict stands).
    let iri = Value::Term(Term::NamedNode(oxrdf::NamedNode::new("http://ex/x").unwrap()));
    assert_eq!(dbl.exact_cmp(&iri), None, "non-numeric -> no exact recheck");
}

#[test]
fn as_num_routes_through_parse_xsd_f64_agreeing_with_as_numeric() {
    // sq-rkzhr: the LENIENT `as_num` (the `CompareTerm::as_f64` seam that
    // drives ORDER BY / the lenient numeric compare) must accept EXACTLY the double /
    // float lexicals the EXACT `as_numeric` path accepts. Otherwise the sq-rikm7
    // f64-collapse recheck can see a literal that is "numeric-and-equal" under `as_f64`
    // yet yields no exact-recheck value (`exact_cmp` -> None) — the very asymmetry
    // sq-rikm7 removed. `str::parse::<f64>` swallowed the non-XSD `inf`/`infinity`/`nan`
    // spellings; `parse_xsd_f64` (shared with `as_numeric`) does not.
    let dbl = |s: &str| Value::Term(Term::Literal(Literal::new_typed_literal(s, xsd::DOUBLE)));
    let flt = |s: &str| Value::Term(Term::Literal(Literal::new_typed_literal(s, xsd::FLOAT)));

    // The XSD specials parse (this held via Rust's parser before, and still does).
    assert_eq!(as_num(&dbl("INF")), Some(f64::INFINITY));
    assert_eq!(as_num(&dbl("+INF")), Some(f64::INFINITY));
    assert_eq!(as_num(&dbl("-INF")), Some(f64::NEG_INFINITY));
    assert!(matches!(as_num(&dbl("NaN")), Some(n) if n.is_nan()));
    assert_eq!(as_num(&flt("INF")), Some(f64::INFINITY));
    assert!(matches!(as_num(&flt("NaN")), Some(n) if n.is_nan()));

    // Non-XSD spellings Rust's `str::parse::<f64>` accepts are now REJECTED, matching
    // `as_numeric` (before the fix `as_num` accepted them -> the two paths disagreed).
    for bad in ["inf", "+inf", "infinity", "-infinity", "nan", "Infinity"] {
        // Positional args dodge the CodeQL `rust/unused-variable` false positive.
        assert_eq!(as_num(&dbl(bad)), None, "as_num must reject non-XSD spelling {:?}", bad);
        assert!(as_numeric(&dbl(bad)).is_none(), "as_numeric already rejects {:?}", bad);
    }

    // The alignment invariant: the lenient and exact paths agree on presence for EVERY
    // lexical — valid specials, ordinary numbers, non-XSD spellings, and non-numerics.
    for s in ["INF", "+INF", "-INF", "NaN", "6", "6.0E0", "2.0E-1", "inf", "nan", "hello"] {
        assert_eq!(
            as_num(&dbl(s)).is_some(),
            as_numeric(&dbl(s)).is_some(),
            "as_num / as_numeric disagree on the validity of {:?}",
            s
        );
    }

    // sq-74oy4 / sq-6b1lj: the alignment now extends to PER-DATATYPE
    // well-formedness AND whitespace. `as_num` is datatype-aware and trimming, so it
    // agrees with `as_numeric` (`Num::of_literal`) on integer/decimal lexicals too — a
    // padded lexical is its trimmed value; a lexical ill-formed FOR its datatype is None.
    let ints = |s: &str| Value::Term(Term::Literal(Literal::new_typed_literal(s, xsd::INTEGER)));
    let decs = |s: &str| Value::Term(Term::Literal(Literal::new_typed_literal(s, xsd::DECIMAL)));
    assert_eq!(as_num(&ints(" 1 ")), Some(1.0), "padded integer trims to 1");
    assert_eq!(as_num(&decs(" 1.5 ")), Some(1.5), "padded decimal trims to 1.5");
    assert_eq!(as_num(&ints("1.5")), None, "fraction on integer is a type error");
    assert_eq!(as_num(&ints("1E2")), None, "exponent on integer is a type error");
    assert_eq!(as_num(&decs("1E2")), None, "exponent on decimal is a type error");
    // Every one agrees with the exact `as_numeric` seam.
    for (v, _lbl) in [
        (ints(" 1 "), "padded int"), (decs(" 1.5 "), "padded dec"),
        (ints("1.5"), "int fraction"), (ints("1E2"), "int exp"), (decs("1E2"), "dec exp"),
    ] {
        assert_eq!(
            as_num(&v).is_some(),
            as_numeric(&v).is_some(),
            "as_num / as_numeric disagree on datatype-aware validity"
        );
    }
}

#[test]
fn compare_orders_double_infinities_and_isolates_nan() {
    // sq-rkzhr: with `as_num` routed through `parse_xsd_f64`, the lenient
    // compare seam orders the XSD floating specials by value (-INF < finite < +INF).
    // sq-wjl8i: NaN is TOTALISED into the order — it sorts FIRST (before
    // -INF) and ties with itself, instead of the former `None` (which callers mapped
    // to Equal, making NaN "equal" to every numeric — the partiality witness).
    // Relational `<` / `=` keep their NaN type-error semantics untouched.
    let d = |s: &str| Value::Term(Term::Literal(Literal::new_typed_literal(s, xsd::DOUBLE)));
    let (neg_inf, inf, five, nan) = (d("-INF"), d("INF"), d("5.0E0"), d("NaN"));
    assert_eq!(compare_values(&neg_inf, &five), Some(Ordering::Less));
    assert_eq!(compare_values(&five, &inf), Some(Ordering::Less));
    assert_eq!(compare_values(&neg_inf, &inf), Some(Ordering::Less));
    assert_eq!(compare_values(&inf, &inf), Some(Ordering::Equal));
    assert_eq!(compare_values(&nan, &five), Some(Ordering::Less));
    assert_eq!(compare_values(&five, &nan), Some(Ordering::Greater));
    assert_eq!(compare_values(&nan, &neg_inf), Some(Ordering::Less));
    assert_eq!(compare_values(&nan, &nan), Some(Ordering::Equal));

    // End-to-end ORDER BY over stored double specials exercises the REAL query path
    // (NaN routes through the general sort cell: the numerics cache uses NaN as its
    // not-numeric sentinel, so the fast numeric cell never sees it).
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             ex:a ex:v \"-INF\"^^xsd:double .\n\
             ex:b ex:v \"5.0E0\"^^xsd:double .\n\
             ex:c ex:v \"INF\"^^xsd:double .\n\
             ex:d ex:v \"NaN\"^^xsd:double .\n",
        "turtle",
    )
    .unwrap();
    assert_eq!(order_by(&g, "ASC(?v)"), vec!["NaN", "-INF", "5.0E0", "INF"]);
    assert_eq!(order_by(&g, "DESC(?v)"), vec!["INF", "5.0E0", "-INF", "NaN"]);
}

#[test]
fn order_by_mixed_numeric_string_temporal_column_is_deterministic() {
    // A single ORDER BY column mixing a numeric graph term, a plain-string literal and an
    // xsd:dateTime exercises the `Num`-vs-`Val` / `Num`-vs-`Temp` sort-cell cross arms
    // (which reconstruct the `Val(Num::Double)` representation and defer to the shared
    // `compare_values`). sq-wjl8i: cross-KIND literal pairs rank by
    // `LiteralKind` (numeric < dateTime < string) — the exact order is additionally
    // pinned end-to-end by `order_by_cross_kind_ranks_and_collapse_is_exact`; this test
    // keeps the permutation/reversibility shape claims.
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             ex:n ex:v 5 .\n\
             ex:s ex:v \"apple\" .\n\
             ex:t ex:v \"2024-03-15T13:00:00Z\"^^xsd:dateTime .\n",
        "turtle",
    )
    .unwrap();
    // The result is a stable, deterministic permutation (the exact cross-type tie order is
    // spec-unconstrained, but must not panic and must round-trip every row exactly once).
    let asc = order_by(&g, "ASC(?v)");
    let desc = order_by(&g, "DESC(?v)");
    assert_eq!(asc.len(), 3);
    assert_eq!(desc.len(), 3);
    let mut sorted_asc = asc.clone();
    sorted_asc.sort();
    let mut sorted_desc = desc.clone();
    sorted_desc.sort();
    assert_eq!(sorted_asc, sorted_desc, "ASC and DESC are permutations of the same rows");
    // DESC is the exact reverse of ASC (total order over distinct-valued rows).
    let mut rev = asc.clone();
    rev.reverse();
    assert_eq!(rev, desc, "DESC(?v) is ASC(?v) reversed");
}

/// sq-wjl8i end-to-end: the kind-first rank and the exact mixed-tier
/// collapse recheck through the REAL query path (both the numeric sort-cell fast
/// path — all-numeric column — and the mixed-kind `compare_values` path).
#[test]
fn order_by_cross_kind_ranks_and_collapse_is_exact() {
    // All-NUMERIC column at the 2^53 collapse: int 2^53, int 2^53+1 and the double
    // carrying their shared f64 image. The exact order is 2^53 = 2^53E0 < 2^53+1
    // (the former three-way collapsed tie was intransitive against the exact
    // int/int order). The int/int and int/double ties are decided by
    // `cmp_sort_num`'s exact recheck (the fast numeric sort cells).
    let g = Graph::load_str(
        "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             ex:a ex:v \"9007199254740993\"^^xsd:integer .\n\
             ex:b ex:v \"9007199254740992E0\"^^xsd:double .\n\
             ex:c ex:v \"9007199254740992\"^^xsd:integer .\n",
        "turtle",
    )
    .unwrap();
    let asc = order_by(&g, "ASC(?v)");
    // The equal pair (2^53, 2^53E0) keeps input order (stable sort); both precede 2^53+1.
    assert_eq!(asc[2], "9007199254740993", "the collapsed neighbour sorts strictly last");
    assert!(asc[..2].contains(&"9007199254740992E0".to_string()));
    assert!(asc[..2].contains(&"9007199254740992".to_string()));

    // MIXED-KIND column: every numeric (NaN included) sorts before the boolean,
    // the dateTime before the date, both before the plain string, the string
    // before the language-tagged literal, and the unknown-datatype literal last —
    // the documented LiteralKind rank; the digit-string "11" can no longer
    // interleave lexically between numerics 10 and 2 (the sq-wjl8i witness).
    let g2 = Graph::load_str(
        "@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
             ex:s ex:v \"11\" .\n\
             ex:n1 ex:v 10 .\n\
             ex:n2 ex:v 2 .\n\
             ex:nan ex:v \"NaN\"^^xsd:double .\n\
             ex:bt ex:v true .\n\
             ex:dt ex:v \"2024-03-15T13:00:00Z\"^^xsd:dateTime .\n\
             ex:d ex:v \"2020-01-01\"^^xsd:date .\n\
             ex:lang ex:v \"chat\"@fr .\n\
             ex:odd ex:v \"weird\"^^ex:custom .\n",
        "turtle",
    )
    .unwrap();
    assert_eq!(
        order_by(&g2, "ASC(?v)"),
        vec!["NaN", "2", "10", "true", "2024-03-15T13:00:00Z", "2020-01-01", "11", "chat", "weird"]
    );
}
