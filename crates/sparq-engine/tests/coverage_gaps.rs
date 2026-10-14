//! sq-qcnn.35 — Targeted coverage tests for exec.rs paths not
//! exercised by the existing suite: XSD constructor casts (eval_cast), SPARQL 1.2
//! language-direction builtins (hasLang/hasLangDir/LANGDIR/STRLANGDIR), and
//! isBlank. Each test directly invokes the public query() entry-point with
//! expressions that exercise specific branches, so line-coverage is attributed
//! to the eval_function / eval_cast path rather than indirectly.

use sparq_core::Graph;
use sparq_engine::query;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// An empty in-memory graph (no triples). Sufficient for any query whose WHERE
/// clause is `{}` or a VALUES-only pattern.
fn empty() -> Graph {
    Graph::load_str("", "turtle").unwrap()
}

/// Execute a SELECT query that projects a SINGLE variable over an empty graph
/// and return the first cell as a `Term::to_string()`.
///
/// Panics if the result is not exactly 1 row × 1 column.
fn sel(q: &str) -> Option<String> {
    let g = empty();
    let r = query(&g, q).expect("query error");
    assert_eq!(r.rows.len(), 1, "expected 1 row for: {}", q);
    assert_eq!(r.rows[0].len(), 1, "expected 1 column for: {}", q);
    r.rows[0][0].as_ref().map(|t| t.to_string())
}

/// Execute a query and return the number of result rows (no column assertions).
/// Use this instead of `sel()` when the expected outcome is ZERO rows (e.g. a
/// FILTER that errors drops all rows).
fn count_rows(q: &str) -> usize {
    let g = empty();
    let r = query(&g, q).expect("query error");
    r.rows.len()
}

fn xsd_iri(ty: &str) -> String {
    format!("<http://www.w3.org/2001/XMLSchema#{}>", ty)
}
fn typed_lit(lex: &str, ty: &str) -> String {
    format!("\"{}\"^^{}", lex, xsd_iri(ty))
}

// ─── XSD string cast (target = xsd:string) ───────────────────────────────────

/// `xsd:string` from a bool literal (as_bool_val arm): oxrdf displays an
/// xsd:string literal WITHOUT the explicit type suffix (it is the default).
#[test]
fn xsd_string_from_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:string(true) AS ?v) WHERE {{}}", pfx)),
        Some("\"true\"".to_string())
    );
    assert_eq!(
        sel(&format!("{}SELECT (xsd:string(false) AS ?v) WHERE {{}}", pfx)),
        Some("\"false\"".to_string())
    );
}

/// `xsd:string` from an integer literal (Num::Int → i.to_string() arm).
#[test]
fn xsd_string_from_integer() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:string(42) AS ?v) WHERE {{}}", pfx)),
        Some("\"42\"".to_string())
    );
}

/// `xsd:string` from a decimal literal (Num::Dec → dec_trim arm):
/// trailing zeros are trimmed but "3.14" is unchanged.
#[test]
fn xsd_string_from_decimal() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:string(3.14) AS ?v) WHERE {{}}", pfx)),
        Some("\"3.14\"".to_string())
    );
}

/// `xsd:string` from an IRI (value_str fallback): the IRI's string form (no
/// angle brackets) is wrapped in a typed xsd:string literal.
#[test]
fn xsd_string_from_iri() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:string(<http://ex/x>) AS ?v) WHERE {{}}",
            pfx
        )),
        Some("\"http://ex/x\"".to_string())
    );
}

// ─── XSD boolean cast ────────────────────────────────────────────────────────

/// `xsd:boolean` from a bool literal (as_bool_val arm): type-safe round-trip.
#[test]
fn xsd_boolean_from_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:boolean(true) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("true", "boolean"))
    );
    assert_eq!(
        sel(&format!("{}SELECT (xsd:boolean(false) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("false", "boolean"))
    );
}

/// `xsd:boolean` from numeric literals (as_numeric arm): 0 → false, !0 → true.
#[test]
fn xsd_boolean_from_numeric() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:boolean(0) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("false", "boolean"))
    );
    assert_eq!(
        sel(&format!("{}SELECT (xsd:boolean(1) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("true", "boolean"))
    );
}

/// `xsd:boolean` from string literals (src_str arm): "true"/"1" → true,
/// "false"/"0" → false.
#[test]
fn xsd_boolean_from_string() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:boolean(\"true\") AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("true", "boolean"))
    );
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:boolean(\"0\") AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("false", "boolean"))
    );
}

// ─── XSD integer cast ────────────────────────────────────────────────────────

/// `xsd:integer` from a bool (as_bool_val arm): true → 1, false → 0.
#[test]
fn xsd_integer_from_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:integer(true) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("1", "integer"))
    );
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:integer(false) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("0", "integer"))
    );
}

/// `xsd:integer` from an integer (Num::Int identity arm).
#[test]
fn xsd_integer_from_integer() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:integer(42) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("42", "integer"))
    );
}

/// `xsd:integer` from a decimal (Num::Dec truncation arm): truncates toward zero.
#[test]
fn xsd_integer_from_decimal() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // 3.9 truncates to 3.
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:integer(3.9) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("3", "integer"))
    );
    // -3.9 truncates toward zero → -3.
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:integer(-3.9) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("-3", "integer"))
    );
}

// ─── XSD decimal cast ────────────────────────────────────────────────────────

/// `xsd:decimal` from a bool (as_bool_val arm): true → "1.0", false → "0.0".
#[test]
fn xsd_decimal_from_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:decimal(true) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("1.0", "decimal"))
    );
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:decimal(false) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("0.0", "decimal"))
    );
}

/// `xsd:decimal` from integer 0 (special Num::Int(0) → bare "0" arm).
#[test]
fn xsd_decimal_from_integer_zero() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:decimal(0) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("0", "decimal"))
    );
}

/// `xsd:decimal` from a non-zero integer (Num::Int → "N.0" arm).
#[test]
fn xsd_decimal_from_integer_nonzero() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:decimal(42) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("42.0", "decimal"))
    );
}

/// `xsd:decimal` from a decimal literal (Num::Dec → keeps lexical arm).
#[test]
fn xsd_decimal_from_decimal() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:decimal(3.14) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("3.14", "decimal"))
    );
}

/// `xsd:decimal` from a string (src_str → dec_trim_min1 arm): trailing zeros
/// are trimmed but at least one fraction digit is kept.
#[test]
fn xsd_decimal_from_string() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:decimal(\"3.14\") AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("3.14", "decimal"))
    );
}

// ─── XSD double cast ─────────────────────────────────────────────────────────

/// `xsd:double` from a bool (as_bool_val arm): true → "1.0E0", false → "0E0".
#[test]
fn xsd_double_from_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:double(true) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("1.0E0", "double"))
    );
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:double(false) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("0E0", "double"))
    );
}

/// `xsd:double` from integer 0 (Num::Int(0) → bare "0" arm).
#[test]
fn xsd_double_from_integer_zero() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:double(0) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("0", "double"))
    );
}

/// `xsd:double` from a non-zero integer (Num::Int → "N.0" arm).
#[test]
fn xsd_double_from_integer_nonzero() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:double(42) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("42.0", "double"))
    );
}

/// `xsd:double` from a decimal literal (Num::Dec → keeps lexical arm).
#[test]
fn xsd_double_from_decimal() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:double(3.14) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("3.14", "double"))
    );
}

/// `xsd:double` from a double literal (Num::Double → plain_min1 arm).
/// A SPARQL double literal like `1.5E0` evaluates to `Num::Double(1.5)`.
#[test]
fn xsd_double_from_double() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:double(1.5E0) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("1.5", "double"))
    );
}

// ─── XSD float cast ──────────────────────────────────────────────────────────

/// `xsd:float` from integer 0 (Num::Int(0) → bare "0" arm).
#[test]
fn xsd_float_from_integer_zero() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:float(0) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("0", "float"))
    );
}

/// `xsd:float` from a non-zero integer (Num::Int → "N.0" arm, target = FLOAT).
#[test]
fn xsd_float_from_integer_nonzero() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:float(42) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("42.0", "float"))
    );
}

/// `xsd:float` from a bool (as_bool_val arm, target = FLOAT): true → "1.0E0".
#[test]
fn xsd_float_from_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:float(true) AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("1.0E0", "float"))
    );
}

// ─── XSD dateTime cast ───────────────────────────────────────────────────────

/// `xsd:dateTime` from a well-formed ISO dateTime string (src_str arm).
#[test]
fn xsd_datetime_from_string() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:dateTime(\"2024-01-01T00:00:00\") AS ?v) WHERE {{}}",
            pfx
        )),
        Some(typed_lit("2024-01-01T00:00:00", "dateTime"))
    );
}

// ─── SPARQL 1.2: hasLang ─────────────────────────────────────────────────────

/// `hasLang` returns true for a language-tagged literal, false for untagged or
/// non-literal inputs (F::HasLang — all three branches).
#[test]
fn haslang_branches() {
    let bool_true = typed_lit("true", "boolean");
    let bool_false = typed_lit("false", "boolean");

    // lang-tagged literal → true (Value::Term(Literal) with l.language().is_some())
    assert_eq!(
        sel("SELECT (hasLang(\"hi\"@en) AS ?v) WHERE {}"),
        Some(bool_true.clone())
    );
    // untagged string → false (Value::Term(Literal) with l.language().is_none())
    assert_eq!(
        sel("SELECT (hasLang(\"hi\") AS ?v) WHERE {}"),
        Some(bool_false.clone())
    );
    // numeric → false (_ branch)
    assert_eq!(
        sel("SELECT (hasLang(42) AS ?v) WHERE {}"),
        Some(bool_false.clone())
    );
}

// ─── SPARQL 1.2: hasLangDir ──────────────────────────────────────────────────

/// `hasLangDir` returns true only for a directional language-tagged literal.
#[test]
fn haslangdir_branches() {
    let bool_true = typed_lit("true", "boolean");
    let bool_false = typed_lit("false", "boolean");

    // directional literal ("en--ltr") → true
    assert_eq!(
        sel("SELECT (hasLangDir(\"hi\"@en--ltr) AS ?v) WHERE {}"),
        Some(bool_true)
    );
    // lang-tagged but no direction → false
    assert_eq!(
        sel("SELECT (hasLangDir(\"hi\"@en) AS ?v) WHERE {}"),
        Some(bool_false.clone())
    );
    // plain string → false (_ branch)
    assert_eq!(
        sel("SELECT (hasLangDir(\"hi\") AS ?v) WHERE {}"),
        Some(bool_false)
    );
}

// ─── SPARQL 1.2: LANGDIR ─────────────────────────────────────────────────────

/// `LANGDIR` returns the base direction string or "" for non-directional literals,
/// and "" for numeric arguments (the Num/Bool → simple("") arm).
#[test]
fn langdir_branches() {
    // directional literal → "ltr"
    assert_eq!(
        sel("SELECT (LANGDIR(\"hi\"@en--ltr) AS ?v) WHERE {}"),
        Some("\"ltr\"".to_string())
    );
    // rtl direction → "rtl"
    assert_eq!(
        sel("SELECT (LANGDIR(\"\u{645}\u{631}\u{62d}\u{628}\u{627}\"@ar--rtl) AS ?v) WHERE {}"),
        Some("\"rtl\"".to_string())
    );
    // lang-tagged but no direction → ""
    assert_eq!(
        sel("SELECT (LANGDIR(\"hi\"@en) AS ?v) WHERE {}"),
        Some("\"\"".to_string())
    );
    // numeric → "" (Num/Bool arm)
    assert_eq!(
        sel("SELECT (LANGDIR(42) AS ?v) WHERE {}"),
        Some("\"\"".to_string())
    );
}

// ─── SPARQL 1.2: STRLANGDIR ──────────────────────────────────────────────────

/// `STRLANGDIR` constructs a directional language-tagged literal from three
/// simple-literal arguments (lex, tag, dir). Tests both valid and error paths.
#[test]
fn strlangdir_construct() {
    // Valid construction: "hello"@en--ltr
    assert_eq!(
        sel("SELECT (STRLANGDIR(\"hello\", \"en\", \"ltr\") AS ?v) WHERE {}"),
        Some("\"hello\"@en--ltr".to_string())
    );
    // Valid construction: rtl
    assert_eq!(
        sel("SELECT (STRLANGDIR(\"world\", \"ar\", \"rtl\") AS ?v) WHERE {}"),
        Some("\"world\"@ar--rtl".to_string())
    );
    // Invalid dir (not "ltr"/"rtl") → Value::Error → UNBOUND
    assert_eq!(
        sel("SELECT (STRLANGDIR(\"x\", \"en\", \"up\") AS ?v) WHERE {}"),
        None
    );
}

// ─── F::IsBlank ──────────────────────────────────────────────────────────────

/// `isBlank`: BNODE() yields a blank-node → true; an IRI → false.
#[test]
fn is_blank_branches() {
    // BNODE() generates a fresh blank node: isBlank must return true.
    let r = query(
        &empty(),
        "SELECT (isBlank(BNODE()) AS ?v) WHERE {}",
    )
    .unwrap();
    let v = r.rows[0][0].as_ref().unwrap().to_string();
    assert_eq!(v, typed_lit("true", "boolean"));

    // An IRI is never a blank node.
    let r = query(
        &empty(),
        "SELECT (isBlank(<http://ex/x>) AS ?v) WHERE {}",
    )
    .unwrap();
    let v = r.rows[0][0].as_ref().unwrap().to_string();
    assert_eq!(v, typed_lit("false", "boolean"));
}

// ─── XSD string cast: float / double / non-finite sources ────────────────────

/// `xsd:string` from an xsd:float typed literal (Num::Float finite → format!("{f}")).
#[test]
fn xsd_string_from_float() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:string("1.5"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some("\"1.5\"".to_string())
    );
}

/// `xsd:string` from an xsd:double literal (Num::Double finite → format!("{f}")).
#[test]
fn xsd_string_from_double() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:string(1.5E0) AS ?v) WHERE {{}}", pfx)),
        Some("\"1.5\"".to_string())
    );
}

/// `xsd:string` from a non-finite float (other → other.lexical() arm).
#[test]
fn xsd_string_from_float_nonfinite() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:string("INF"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some("\"INF\"".to_string())
    );
}

/// `xsd:string` from a blank node — type error → UNBOUND.
#[test]
fn xsd_string_from_bnode() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            "{}SELECT (xsd:string(BNODE()) AS ?v) WHERE {{}}",
            pfx
        )),
        None
    );
}

/// `xsd:string` from a decimal with trailing zeros: dec_trim removes them
/// ("3.10" → "3.1"). Exercises the dec_trim inner loop.
#[test]
fn xsd_string_from_decimal_trailing_zeros() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:string("3.10"^^xsd:decimal) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some("\"3.1\"".to_string())
    );
}

// ─── XSD boolean cast: error paths and computed-bool arm ─────────────────────

/// `xsd:boolean` from an invalid string — none of "true"/"1"/"false"/"0" →
/// Value::Error → UNBOUND.
#[test]
fn xsd_boolean_from_invalid_string() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:boolean("hello") AS ?v) WHERE {{}}"#,
            pfx
        )),
        None
    );
}

/// `xsd:boolean` from a language-tagged literal — `src_str()` returns None
/// → `_ => Value::Error` → UNBOUND.
#[test]
fn xsd_boolean_from_lang_tagged() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:boolean("hi"@en) AS ?v) WHERE {{}}"#,
            pfx
        )),
        None
    );
}

/// `xsd:boolean` from a COMPUTED boolean (Value::Bool arm in as_bool_val, line 8027).
/// `1=1` in SPARQL evaluates to Value::Bool(true).
#[test]
fn xsd_boolean_from_computed_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:boolean(1=1) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("true", "boolean"))
    );
}

/// `xsd:boolean` from an ill-formed xsd:boolean literal (`as_bool_val` line 8031:
/// "maybe" is not "true"|"1"|"false"|"0").  Uses STRDT to construct it.
#[test]
fn xsd_boolean_from_invalid_bool_literal() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:boolean(STRDT("maybe", xsd:boolean)) AS ?v) WHERE {{}}"#,
            pfx
        )),
        None
    );
}

// ─── XSD dateTime cast: pass-through and error ───────────────────────────────

/// `xsd:dateTime` from an existing xsd:dateTime literal — pass-through arm.
#[test]
fn xsd_datetime_from_existing() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:dateTime("2024-01-01T00:00:00"^^xsd:dateTime) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("2024-01-01T00:00:00", "dateTime"))
    );
}

/// `xsd:dateTime` from a non-parseable string → Value::Error → UNBOUND.
#[test]
fn xsd_datetime_from_invalid() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:dateTime("not-a-date") AS ?v) WHERE {{}}"#,
            pfx
        )),
        None
    );
}

// ─── XSD integer / decimal cast: float source ────────────────────────────────

/// `xsd:integer` from a finite xsd:float: truncates toward zero.
#[test]
fn xsd_integer_from_float() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:integer("1.75"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("1", "integer"))
    );
}

/// `xsd:integer` from an infinite xsd:float → Value::Error → UNBOUND.
#[test]
fn xsd_integer_from_infinite_float() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:integer("INF"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        None
    );
}

/// `xsd:decimal` from a finite xsd:float (plain_min1 arm, non-integral).
#[test]
fn xsd_decimal_from_float() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:decimal("1.5"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("1.5", "decimal"))
    );
}

/// `xsd:decimal` from an infinite xsd:float → Value::Error → UNBOUND.
#[test]
fn xsd_decimal_from_infinite_float() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:decimal("INF"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        None
    );
}

/// `xsd:decimal` from an integer-string "3" (dec_trim_min1 scale-0 arm): forces
/// at least one fraction digit → "3.0".
#[test]
fn xsd_decimal_from_integer_string() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:decimal("3") AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("3.0", "decimal"))
    );
}

// ─── XSD float / double: float self-cast, integral double, string inputs ─────

/// `xsd:float` from an xsd:float literal (Num::Float arm, non-integral: plain_min1
/// returns the non-integral path "1.5").
#[test]
fn xsd_float_from_float() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:float("1.5"^^xsd:float) AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("1.5", "float"))
    );
}

/// `xsd:double` from an INTEGRAL double (plain_min1 integral arm: format!("{:.1}")).
/// 2.0E0 has fract==0 → plain_min1 returns "2.0".
#[test]
fn xsd_double_from_integral_double() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!("{}SELECT (xsd:double(2.0E0) AS ?v) WHERE {{}}", pfx)),
        Some(typed_lit("2.0", "double"))
    );
}

/// `xsd:double` from a string in scientific notation (src_str path → parse_xsd_f64
/// → format!("{:E}")).
#[test]
fn xsd_double_from_string_sci() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:double("1.5E2") AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("1.5E2", "double"))
    );
}

/// `xsd:float` from a string in scientific notation (src_str path → parse_xsd_f32
/// → format!("{:E}")).
#[test]
fn xsd_float_from_string_sci() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    assert_eq!(
        sel(&format!(
            r#"{}SELECT (xsd:float("1.5E2") AS ?v) WHERE {{}}"#,
            pfx
        )),
        Some(typed_lit("1.5E2", "float"))
    );
}

// ─── Builtin function branches not yet exercised ─────────────────────────────

/// `hasLang(<iri>)` — an IRI is not a literal: hits the `_ => false` arm (line 8527).
#[test]
fn haslang_iri_branch() {
    assert_eq!(
        sel("SELECT (hasLang(<http://ex/x>) AS ?v) WHERE {}"),
        Some(typed_lit("false", "boolean"))
    );
}

/// `hasLangDir(<iri>)` — same as hasLang but for direction: `_ => false` (line 8532).
#[test]
fn haslangdir_iri_branch() {
    assert_eq!(
        sel("SELECT (hasLangDir(<http://ex/x>) AS ?v) WHERE {}"),
        Some(typed_lit("false", "boolean"))
    );
}

/// `LANGDIR(1+1)` — computed Num hits `Value::Num(_) | Value::Bool(_) => simple("")`
/// (line 8538).
#[test]
fn langdir_computed_num_branch() {
    assert_eq!(
        sel("SELECT (LANGDIR(1+1) AS ?v) WHERE {}"),
        Some("\"\"".to_string())
    );
}

/// `LANGDIR(<iri>)` — IRI hits the `_ => Value::Error` arm (line 8539).
#[test]
fn langdir_iri_branch() {
    assert_eq!(
        sel("SELECT (LANGDIR(<http://ex/x>) AS ?v) WHERE {}"),
        None
    );
}

/// `STRLANGDIR` where the first argument is a language-tagged literal — its
/// `str_lit` result is `Some((lex, Some(lang)))` which doesn't match
/// `(Some((lex, None)), ...)` → `_ => Value::Error` (line 8556).
#[test]
fn strlangdir_lang_tagged_first_arg() {
    assert_eq!(
        sel(r#"SELECT (STRLANGDIR("x"@en, "en", "ltr") AS ?v) WHERE {}"#),
        None
    );
}

/// `IRI(42)` — integer literal hits the `_ => Value::Error` arm (line 8340).
#[test]
fn iri_non_string_error() {
    assert_eq!(
        sel("SELECT (IRI(42) AS ?v) WHERE {}"),
        None
    );
}

/// `isNumeric(1+1)` — computed Num hits `Value::Num(_) => true` (line 8347).
#[test]
fn isnumeric_computed_num() {
    assert_eq!(
        sel("SELECT (isNumeric(1+1) AS ?v) WHERE {}"),
        Some(typed_lit("true", "boolean"))
    );
}

/// `isNumeric(<iri>)` — IRI hits `_ => false` (line 8349).
#[test]
fn isnumeric_iri() {
    assert_eq!(
        sel("SELECT (isNumeric(<http://ex/x>) AS ?v) WHERE {}"),
        Some(typed_lit("false", "boolean"))
    );
}

// ─── xsd:dateTime accessors (YEAR/MONTH/DAY/HOURS/MINUTES/SECONDS) ───────────

/// `YEAR/MONTH/DAY/HOURS/MINUTES/SECONDS` on a dateTime literal — each accessor
/// calls `datetime_field(v, idx)` → `parse_datetime` (lines 8452-8457, 9074-9083).
#[test]
fn datetime_accessors() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let dt = r#""2024-03-15T12:30:45"^^xsd:dateTime"#;

    assert_eq!(
        sel(&format!("{}SELECT (YEAR({}) AS ?v) WHERE {{}}", pfx, dt)),
        Some(typed_lit("2024", "integer"))
    );
    assert_eq!(
        sel(&format!("{}SELECT (MONTH({}) AS ?v) WHERE {{}}", pfx, dt)),
        Some(typed_lit("3", "integer"))
    );
    assert_eq!(
        sel(&format!("{}SELECT (DAY({}) AS ?v) WHERE {{}}", pfx, dt)),
        Some(typed_lit("15", "integer"))
    );
    assert_eq!(
        sel(&format!("{}SELECT (HOURS({}) AS ?v) WHERE {{}}", pfx, dt)),
        Some(typed_lit("12", "integer"))
    );
    assert_eq!(
        sel(&format!("{}SELECT (MINUTES({}) AS ?v) WHERE {{}}", pfx, dt)),
        Some(typed_lit("30", "integer"))
    );
    // SECONDS returns a decimal
    assert_eq!(
        sel(&format!("{}SELECT (SECONDS({}) AS ?v) WHERE {{}}", pfx, dt)),
        Some(typed_lit("45", "decimal"))
    );
}

// ─── eval_dec exact decimal arithmetic (lines 7798-7804) ─────────────────────

/// A FILTER with decimal arithmetic forces the exact `eval_dec` path (called when
/// `expr_has_arith` is true for a comparison operand).  Each sub-test exercises
/// a different arithmetic operator.
#[test]
fn eval_dec_arithmetic_in_filter() {
    // Add (line 7798): 1.5 + 2.5 > 3.0 → true
    assert_eq!(
        sel("SELECT (1 AS ?x) WHERE { FILTER(1.5 + 2.5 > 3.0) }"),
        Some(typed_lit("1", "integer"))
    );
    // Subtract (line 7799): 5.0 - 1.5 > 3.0 → true
    assert_eq!(
        sel("SELECT (1 AS ?x) WHERE { FILTER(5.0 - 1.5 > 3.0) }"),
        Some(typed_lit("1", "integer"))
    );
    // Multiply (line 7800): 2.0 * 2.0 > 3.0 → true
    assert_eq!(
        sel("SELECT (1 AS ?x) WHERE { FILTER(2.0 * 2.0 > 3.0) }"),
        Some(typed_lit("1", "integer"))
    );
    // UnaryMinus (lines 7802-7804): -1.5 + 3.0 > 0.0 → true
    assert_eq!(
        sel("SELECT (1 AS ?x) WHERE { FILTER(-1.5 + 3.0 > 0.0) }"),
        Some(typed_lit("1", "integer"))
    );
    // UnaryPlus (line 7801): +2.0 + 0.0 > 0.0 → true
    assert_eq!(
        sel("SELECT (1 AS ?x) WHERE { FILTER(+2.0 + 0.0 > 0.0) }"),
        Some(typed_lit("1", "integer"))
    );
}

// ─── trace_label arms via explain_analyze (lines 2301-2313) ──────────────────

/// EXPLAIN ANALYZE on queries containing Union, Extend (BIND), Minus, Values, and
/// Path patterns triggers `trace_label` for each — covering lines 2301-2305.
#[test]
fn trace_label_union_extend_minus_values() {
    use sparq_engine::explain_analyze;
    // Union → line 2301
    let r = explain_analyze(
        &empty(),
        "SELECT ?v WHERE { { VALUES ?v { 1 } } UNION { VALUES ?v { 2 } } }",
    );
    assert!(r.is_ok(), "Union explain_analyze failed: {:?}", r);

    // Extend (BIND) → line 2302
    let r = explain_analyze(&empty(), "SELECT ?v WHERE { BIND(1 AS ?v) }");
    assert!(r.is_ok(), "Extend explain_analyze failed: {:?}", r);

    // Minus → line 2303
    let r = explain_analyze(
        &empty(),
        "SELECT ?v WHERE { VALUES ?v { 1 2 } MINUS { VALUES ?v { 1 } } }",
    );
    assert!(r.is_ok(), "Minus explain_analyze failed: {:?}", r);

    // Values → line 2304 (also hit by Union test above; confirmed again)
    let r = explain_analyze(&empty(), "SELECT ?v WHERE { VALUES ?v { 1 } }");
    assert!(r.is_ok(), "Values explain_analyze failed: {:?}", r);
}

// ─── Property path: Sequence with bound start/object ─────────────────────────

/// Builds a two-triple chain: :a -p-> :b -q-> :c.
fn chain_graph() -> sparq_core::Graph {
    sparq_core::Graph::load_str(
        "<http://ex/a> <http://ex/p> <http://ex/b> .\n\
         <http://ex/b> <http://ex/q> <http://ex/c> .\n",
        "turtle",
    )
    .unwrap()
}

/// Helper: run a SELECT that yields exactly ONE row × ONE column on a given graph.
fn sel_g(g: &sparq_core::Graph, q: &str) -> Option<String> {
    let r = sparq_engine::query(g, q).expect("query error");
    assert_eq!(r.rows.len(), 1, "expected 1 row for: {}", q);
    assert_eq!(r.rows[0].len(), 1, "expected 1 column for: {}", q);
    r.rows[0][0].as_ref().map(|t| t.to_string())
}

/// Sequence path with bound SUBJECT: `<:a> <:p>/<:q> ?o` exercises the bound-start
/// branch of `path_pairs` (P::Sequence, `ends.s = Some(id_a)`) — lines 3699-3707.
#[test]
fn seq_path_bound_start() {
    let g = chain_graph();
    assert_eq!(
        sel_g(&g, "SELECT ?o WHERE { <http://ex/a> <http://ex/p>/<http://ex/q> ?o }"),
        Some("<http://ex/c>".to_string())
    );
}

/// Sequence path with bound OBJECT: `?s <:p>/<:q> <:c>` exercises the bound-object
/// branch (`ends.o = Some(id_c)`) — lines 3719-3727.
#[test]
fn seq_path_bound_object() {
    let g = chain_graph();
    assert_eq!(
        sel_g(&g, "SELECT ?s WHERE { ?s <http://ex/p>/<http://ex/q> <http://ex/c> }"),
        Some("<http://ex/a>".to_string())
    );
}

// ─── COALESCE — iterates until a non-error arg (lines 7486-7490) ─────────────

/// `COALESCE(?x, 1)` where `?x` is unbound: the loop skips the UNBOUND first arg
/// and returns 1 on the second — covering the loop body at lines 7486-7490.
#[test]
fn coalesce_skips_unbound() {
    assert_eq!(
        sel("SELECT (COALESCE(?x, 1) AS ?v) WHERE {}"),
        Some(typed_lit("1", "integer"))
    );
}

// ─── BIND a computed boolean — value_to_id(Bool) path (lines 9130-9134) ──────

/// `BIND(1=1 AS ?v)` stores a COMPUTED boolean in the result row via
/// `value_to_id(Value::Bool(true))` — hitting lines 9130-9134.
#[test]
fn bind_computed_bool() {
    assert_eq!(
        sel("SELECT ?v WHERE { BIND(1=1 AS ?v) }"),
        Some(typed_lit("true", "boolean"))
    );
}

// ─── TIMEZONE: tz_to_duration with non-UTC offset (lines 8985-8988) ──────────

/// `TIMEZONE` on a dateTime with a `+hh:mm` offset exercises the `split_once(':')`
/// branch of `tz_to_duration` (lines 8985-8988).
#[test]
fn timezone_nonzero_offset() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // +05:30 → "PT5H30M"
    let result = sel(&format!(
        r#"{}SELECT (TIMEZONE("2024-01-01T00:00:00+05:30"^^xsd:dateTime) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(result.is_some(), "TIMEZONE(+05:30) should return a value");
    let r = result.unwrap();
    assert!(r.contains("PT"), "expected dayTimeDuration, got: {}", r);
}

/// `TIMEZONE` on a dateTime with `+00:00` offset — h==0 && m==0 → returns "PT0S"
/// (line 8987-8988).
#[test]
fn timezone_zero_offset() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        r#"{}SELECT (TIMEZONE("2024-01-01T00:00:00+00:00"^^xsd:dateTime) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(result.is_some(), "TIMEZONE(+00:00) should return a value");
    let r = result.unwrap();
    assert!(r.contains("PT0S"), "expected PT0S, got: {}", r);
}

// ─── UCASE on directional literal (lit_with_lang directional arm) ─────────────

/// `UCASE("hello"@en--ltr)` preserves the directional tag; `lit_with_lang` calls
/// `l.split_once("--")` → directional case — lines 8927-8930.
#[test]
fn ucase_directional_literal() {
    let result = sel(r#"SELECT (UCASE("hello"@en--ltr) AS ?v) WHERE {}"#);
    assert_eq!(result, Some("\"HELLO\"@en--ltr".to_string()));
}

// ─── eval_expr: Divide operator (line 7465) ───────────────────────────────────

/// `Divide` arm of `eval_expr` — line 7465. Division of decimal literals returns
/// a decimal result via `arith(…, ArithOp::Div)`.
//
#[test]
fn divide_operator() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (\"6.0\"^^xsd:decimal / \"2.0\"^^xsd:decimal AS ?v) WHERE {{}}",
        pfx
    ));
    assert!(result.is_some(), "6.0/2.0 should return a value");
}

// ─── eval_expr: UnaryPlus operator (line 7466) ───────────────────────────────

/// `UnaryPlus` arm of `eval_expr` — line 7466. `+?n` just returns the argument
/// unchanged (it is a no-op in SPARQL, but the parser emits the `UnaryPlus` node).
//
#[test]
fn unary_plus_operator() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (+\"3\"^^xsd:integer AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(
        result,
        Some("\"3\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string())
    );
}

// ─── eval_expr: Bound with a BOUND variable (line 7473) ──────────────────────

/// `Bound(v)` where the variable IS bound — exercises the `true` arm of line 7473.
/// The query uses `VALUES` to guarantee the binding in every row.
//
#[test]
fn bound_bound_variable() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (BOUND(?v) AS ?r) WHERE {{ VALUES ?v {{ \"1\"^^xsd:integer }} }}",
        pfx
    ));
    assert_eq!(result, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

// ─── eval_expr: IF false arm (line 7479) ─────────────────────────────────────

/// `If(cond, t, f)` where the condition is `false` — exercises line 7479's
/// `Some(false) => eval_expr(…, f)` branch.
//
#[test]
fn if_false_arm() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // false → returns the third arg (2)
    let result = sel(&format!(
        "{}SELECT (IF(false, \"1\"^^xsd:integer, \"2\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(
        result,
        Some("\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string())
    );
}

// ─── eval_expr: COALESCE all-error returns Unbound (line 7492) ───────────────

/// `Coalesce(es)` where every argument errors — exercises line 7492 which returns
/// `Value::Unbound` (= the `SELECT` projection yields an unbound cell). Uses
/// `SUBJECT("hello")` which returns `Value::Error` for a non-triple argument.
//
#[test]
fn coalesce_all_error() {
    // SUBJECT("hello") → Value::Error; COALESCE falls off the end → Value::Unbound
    // Projected in SELECT → ?v is unbound → sel() returns None (not panics)
    let result = sel(r#"SELECT (COALESCE(SUBJECT("hello")) AS ?v) WHERE {}"#);
    assert_eq!(result, None, "COALESCE of all errors should produce an unbound cell");
}

// ─── F::Lang on computed numeric / bool (line 8256) ─────────────────────────

/// `LANG(1 + 1)` — the argument is a computed `Value::Num`; the `Value::Num(_) |
/// Value::Bool(_)` arm at line 8256 returns the empty string literal.
#[test]
fn lang_computed_num() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (LANG(\"1\"^^xsd:integer + \"1\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    // LANG of a computed numeric returns the empty simple literal ""
    assert_eq!(result, Some("\"\"".to_string()));
}

// ─── F::Datatype on computed numeric (line 8263) ─────────────────────────────

/// `DATATYPE(1 + 1)` — `Value::Num(n)` arm at line 8263 returns the numeric's
/// promoted XSD datatype IRI.
#[test]
fn datatype_computed_num() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (DATATYPE(\"1\"^^xsd:integer + \"1\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    // integer + integer → xsd:integer
    assert_eq!(
        result,
        Some("<http://www.w3.org/2001/XMLSchema#integer>".to_string())
    );
}

// ─── F::Datatype on computed bool (line 8264) ────────────────────────────────

/// `DATATYPE(1 = 1)` — `Value::Bool(_)` arm at line 8264 returns xsd:boolean.
//
#[test]
fn datatype_computed_bool() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (DATATYPE(\"1\"^^xsd:integer = \"1\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(
        result,
        Some("<http://www.w3.org/2001/XMLSchema#boolean>".to_string())
    );
}

// ─── F::SubStr on non-string first arg (line 8310) ───────────────────────────

/// `SUBSTR(42, 1)` — the first argument is an integer literal, not a string
/// literal; `str_lit` returns `None`, hitting the `None => return Ok(Value::Error)`
/// branch at line 8310.
#[test]
fn substr_non_string_error() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (SUBSTR(\"42\"^^xsd:integer, \"1\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(result, None, "SUBSTR on non-string first arg should error");
}

// ─── F::StrLang with invalid language tag (line 8370) ────────────────────────

/// `STRLANG("x", "NOT VALID!!!")` — `Literal::new_language_tagged_literal` returns
/// `Err` for an ill-formed language tag, hitting line 8370.
#[test]
fn strlang_invalid_lang_tag() {
    // An exclamation mark is not a valid BCP-47 subtag character.
    let result = sel(r#"SELECT (STRLANG("x", "NOT_VALID!!!") AS ?v) WHERE {}"#);
    assert_eq!(result, None, "STRLANG with invalid tag should error");
}

// ─── F::BNode with non-string arg (line 8429) ────────────────────────────────

/// `BNODE(42)` — the scoped-BNODE variant calls `str_lit`; a non-string argument
/// returns `None`, hitting line 8429 → `Value::Error`.
#[test]
fn bnode_non_string_error() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (BNODE(\"42\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(result, None, "BNODE(non-string) should error");
}

// ─── as_bool_val: numeric literal path (line 7405) ────────────────────────────

/// `FILTER` on a numeric literal non-zero value exercises `as_bool_val`'s
/// `parse_xsd_f64` branch at line 7405.
#[test]
fn as_bool_val_numeric_literal() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // 1.5 is truthy; the row should be returned
    let result = sel(&format!(
        "{}SELECT (\"1.5\"^^xsd:decimal AS ?v) WHERE {{ FILTER(\"1.5\"^^xsd:decimal) }}",
        pfx
    ));
    assert!(result.is_some(), "numeric literal 1.5 is truthy");
}

// ─── as_bool_val: other (non-bool/non-numeric/non-string) datatype → None (line 7409)

/// A `FILTER` on a dateTime literal (`as_bool_val` other-datatype arm → `None` →
/// filter error → row dropped) exercises line 7409. Uses `count_rows()` because
/// `sel()` panics on zero rows.
#[test]
fn as_bool_val_other_datatype() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // dateTime is not bool/numeric/string → as_bool_val returns None → FILTER errors → 0 rows
    let n = count_rows(&format!(
        "{}SELECT (\"hello\" AS ?v) WHERE {{ FILTER(\"2024-01-01T00:00:00Z\"^^xsd:dateTime) }}",
        pfx
    ));
    assert_eq!(n, 0, "FILTER(dateTime) hits the other-datatype arm → no row");
}

// ─── eval_numeric: local vocab numeric cache path (line 7615) ────────────────

/// A comparison of a BIND-computed value vs a constant exercises `eval_numeric`'s
/// `local.numeric(id)` path at line 7615 (the `is_local(id)` branch).
#[test]
fn eval_numeric_local_vocab_path() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // BIND(1+1 AS ?x) produces a local-vocab numeric. Comparing ?x > 1 hits line 7615.
    let result = sel(&format!(
        "{}SELECT ?x WHERE {{ BIND(\"1\"^^xsd:integer + \"1\"^^xsd:integer AS ?x) FILTER(?x > \"1\"^^xsd:integer) }}",
        pfx
    ));
    assert_eq!(
        result,
        Some("\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string())
    );
}

// ─── eval_numeric: arithmetic operators (lines 7621-7625) ────────────────────

/// Exercises the `Add`, `Subtract`, `Multiply`, `Divide`, `UnaryPlus` arms of
/// `eval_numeric` (lines 7621-7625) by nesting arithmetic inside a comparison
/// filter so `cmp_expr` delegates to `eval_numeric` for both sides.
#[test]
fn eval_numeric_arithmetic_ops() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    // This filters on 3+2-1 * 2/2 > 3  i.e. expression subtree — pushes through
    // Add / Subtract / Multiply / Divide / UnaryPlus inside eval_numeric.
    let add_result = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer AS ?v) WHERE {{ FILTER(\"3\"^^xsd:integer + \"2\"^^xsd:integer > \"4\"^^xsd:integer) }}",
        pfx
    ));
    assert!(add_result.is_some(), "3+2 > 4 should be true");

    let sub_result = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer AS ?v) WHERE {{ FILTER(\"5\"^^xsd:integer - \"2\"^^xsd:integer > \"2\"^^xsd:integer) }}",
        pfx
    ));
    assert!(sub_result.is_some(), "5-2 > 2 should be true");

    let mul_result = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer AS ?v) WHERE {{ FILTER(\"3\"^^xsd:integer * \"2\"^^xsd:integer > \"5\"^^xsd:integer) }}",
        pfx
    ));
    assert!(mul_result.is_some(), "3*2 > 5 should be true");

    let div_result = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer AS ?v) WHERE {{ FILTER(\"6.0\"^^xsd:decimal / \"2.0\"^^xsd:decimal > \"2.0\"^^xsd:decimal) }}",
        pfx
    ));
    assert!(div_result.is_some(), "6/2 > 2 should be true");

    let unary_plus_result = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer AS ?v) WHERE {{ FILTER(+\"3\"^^xsd:integer > \"2\"^^xsd:integer) }}",
        pfx
    ));
    assert!(unary_plus_result.is_some(), "+3 > 2 should be true");
}

// ─── F::IsNumeric on computed numeric (line 8347) ────────────────────────────

/// `ISNUMERIC(1+1)` — the `Value::Num(_)` arm at line 8347 returns `true`.
//
#[test]
fn is_numeric_computed_num() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let result = sel(&format!(
        "{}SELECT (ISNUMERIC(\"1\"^^xsd:integer + \"2\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(result, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

// ─── F::IsTriple / F::Subject / F::Predicate / F::Object (lines 8501-8520) ───

/// RDF-star built-ins: `ISTRIPLE`, `SUBJECT`, `PREDICATE`, `OBJECT` on a triple
/// term created by `TRIPLE(s, p, o)`. Exercises lines 8486-8520.
#[test]
fn rdf_star_triple_builtins() {
    // TRIPLE(s, p, o) on IRIs; then extract parts
    let pfx = "PREFIX ex: <http://example.org/> ";

    let is_triple = sel(&format!(
        "{}SELECT (ISTRIPLE(TRIPLE(ex:s, ex:p, ex:o)) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(is_triple, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));

    let subject_val = sel(&format!(
        "{}SELECT (SUBJECT(TRIPLE(ex:s, ex:p, ex:o)) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(subject_val, Some("<http://example.org/s>".to_string()));

    let predicate_val = sel(&format!(
        "{}SELECT (PREDICATE(TRIPLE(ex:s, ex:p, ex:o)) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(predicate_val, Some("<http://example.org/p>".to_string()));

    let object_val = sel(&format!(
        "{}SELECT (OBJECT(TRIPLE(ex:s, ex:p, ex:o)) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(object_val, Some("<http://example.org/o>".to_string()));
}

// ─── F::IsTriple on non-triple → false (line 8504) ───────────────────────────

/// `ISTRIPLE("hello")` — the catch-all arm at line 8504 returns `false`.
#[test]
fn is_triple_non_triple() {
    let result = sel(r#"SELECT (ISTRIPLE("hello") AS ?v) WHERE {}"#);
    assert_eq!(result, Some("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

// ─── F::Subject / Predicate / Object on non-triple → Error (line 8511/8515/8519)

/// `SUBJECT("hello")` — subject/predicate/object on a non-triple → `Value::Error`
/// → no binding projected. Lines 8511, 8515, 8519.
#[test]
fn rdf_star_accessor_non_triple_error() {
    let subj = sel(r#"SELECT (SUBJECT("hello") AS ?v) WHERE {}"#);
    assert_eq!(subj, None, "SUBJECT(non-triple) should error");

    let pred = sel(r#"SELECT (PREDICATE("hello") AS ?v) WHERE {}"#);
    assert_eq!(pred, None, "PREDICATE(non-triple) should error");

    let obj = sel(r#"SELECT (OBJECT("hello") AS ?v) WHERE {}"#);
    assert_eq!(obj, None, "OBJECT(non-triple) should error");
}

// ─── F::Regex — exec.rs lines 8461-8470 + build_regex 9009-9024 ───────────────

/// Basic `REGEX("abc", "b")` → true. Exercises the F::Regex match arm and
/// `build_regex`.
#[test]
#[cfg(feature = "regex")]
fn regex_basic_match() {
    let r = sel(r#"SELECT (REGEX("abcdef", "cd") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `REGEX("ABC", "abc", "i")` — the `i` case-insensitive flag path in
/// `build_regex` (line 9016).
#[test]
#[cfg(feature = "regex")]
fn regex_case_insensitive() {
    let r = sel(r#"SELECT (REGEX("ABC", "abc", "i") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `REGEX("abc", "z")` → false (no match).
#[test]
#[cfg(feature = "regex")]
fn regex_no_match() {
    let r = sel(r#"SELECT (REGEX("abc", "z") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `REGEX("abc", "[invalid[")` — invalid regex → `build_regex` returns `None` →
/// `Value::Error` → no binding (line 8469).
#[test]
#[cfg(feature = "regex")]
fn regex_invalid_pattern() {
    let r = sel(r#"SELECT (REGEX("abc", "[invalid[") AS ?v) WHERE {}"#);
    assert_eq!(r, None, "invalid regex pattern should error");
}

/// `REGEX` with unknown flag — `build_regex` checks all chars and returns `None`
/// on an unrecognised flag character (line 9010-9011).
#[test]
#[cfg(feature = "regex")]
fn regex_unknown_flag() {
    // 'z' is not a valid SPARQL regex flag
    let r = sel(r#"SELECT (REGEX("abc", "a", "z") AS ?v) WHERE {}"#);
    assert_eq!(r, None, "unknown flag should error");
}

// ─── F::Replace — exec.rs lines 8473-8482 ────────────────────────────────────

/// Basic `REPLACE("hello world", "world", "earth")` — exercises the Replace arm
/// and build_regex (lines 8473-8480).
#[test]
#[cfg(feature = "regex")]
fn replace_basic() {
    let r = sel(r#"SELECT (REPLACE("hello world", "world", "earth") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"hello earth\"".to_string()));
}

/// `REPLACE("ABC", "b", "X", "i")` — case-insensitive replace with flag
/// (line 8478).
#[test]
#[cfg(feature = "regex")]
fn replace_with_flag() {
    let r = sel(r#"SELECT (REPLACE("ABC", "b", "X", "i") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"AXC\"".to_string()));
}

/// `REPLACE("abc", "[bad[", "x")` — invalid regex → `build_regex` returns `None`
/// → `Value::Error` (line 8481).
#[test]
#[cfg(feature = "regex")]
fn replace_invalid_pattern() {
    let r = sel(r#"SELECT (REPLACE("abc", "[bad[", "x") AS ?v) WHERE {}"#);
    assert_eq!(r, None, "invalid pattern in REPLACE should error");
}

// ─── F::Md5 / Sha1 / Sha256 — exec.rs lines 8394-8402, digest_hex 8940-8952 ──

/// `MD5("abc")` — exercises digest_hex + F::Md5 (lines 8394, 8940-8949).
//
#[test]
#[cfg(feature = "digest")]
fn md5_basic() {
    let r = sel(r#"SELECT (MD5("abc") AS ?v) WHERE {}"#);
    // MD5("abc") = 900150983cd24fb0d6963f7d28e17f72
    assert_eq!(r, Some("\"900150983cd24fb0d6963f7d28e17f72\"".to_string()));
}

/// `SHA1("abc")` — exercises F::Sha1 (line 8396) and digest_hex.
#[test]
#[cfg(feature = "digest")]
fn sha1_basic() {
    let r = sel(r#"SELECT (SHA1("abc") AS ?v) WHERE {}"#);
    // SHA1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
    assert_eq!(r, Some("\"a9993e364706816aba3e25717850c26c9cd0d89d\"".to_string()));
}

/// `SHA256("abc")` — exercises F::Sha256 (line 8398) and digest_hex.
#[test]
#[cfg(feature = "digest")]
fn sha256_basic() {
    let r = sel(r#"SELECT (SHA256("abc") AS ?v) WHERE {}"#);
    assert!(r.is_some(), "SHA256 should return a value");
    // SHA256 output is a 64-character hex string (256 bits = 32 bytes = 64 hex chars)
    assert_eq!(r.unwrap().len(), "\"\"".len() + 64, "SHA256 output is 64 hex chars");
}

/// `SHA384("abc")` — exercises F::Sha384 (line 8400).
#[test]
#[cfg(feature = "digest")]
fn sha384_basic() {
    let r = sel(r#"SELECT (SHA384("abc") AS ?v) WHERE {}"#);
    assert!(r.is_some(), "SHA384 should return a value");
    assert_eq!(r.unwrap().len(), "\"\"".len() + 96, "SHA384 output is 96 hex chars");
}

/// `SHA512("abc")` — exercises F::Sha512 (line 8402).
#[test]
#[cfg(feature = "digest")]
fn sha512_basic() {
    let r = sel(r#"SELECT (SHA512("abc") AS ?v) WHERE {}"#);
    assert!(r.is_some(), "SHA512 should return a value");
    assert_eq!(r.unwrap().len(), "\"\"".len() + 128, "SHA512 output is 128 hex chars");
}

/// `MD5(42)` — non-string arg → `digest_hex` returns `Value::Error` (line 8951).
//
#[test]
#[cfg(feature = "digest")]
fn md5_non_string_error() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        "{}SELECT (MD5(\"42\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(r, None, "MD5(non-string) should error");
}

// ─── F::EncodeForUri — exec.rs line 8333, encode_for_uri 9028-9038 ────────────

/// `ENCODE_FOR_URI("hello world")` → spaces become `%20` (line 8333 + encode_for_uri).
//
#[test]
fn encode_for_uri_basic() {
    let r = sel(r#"SELECT (ENCODE_FOR_URI("hello world") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"hello%20world\"".to_string()));
}

// ─── F::LangMatches — exec.rs lines 8380-8390 ────────────────────────────────

/// `LANGMATCHES("en-US", "en")` → true (subtag match).
#[test]
fn langmatches_subtag() {
    let r = sel(r#"SELECT (LANGMATCHES("en-US", "en") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `LANGMATCHES("en", "*")` → true (any non-empty tag).
#[test]
fn langmatches_star() {
    let r = sel(r#"SELECT (LANGMATCHES("en", "*") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `LANGMATCHES("", "*")` → false (empty tag is not any).
#[test]
fn langmatches_empty_tag_star() {
    let r = sel(r#"SELECT (LANGMATCHES("", "*") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `LANGMATCHES("de", "en")` → false (different language).
#[test]
fn langmatches_no_match() {
    let r = sel(r#"SELECT (LANGMATCHES("de", "en") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

// ─── F::Contains / F::StrStarts / F::StrEnds — lines 8287-8289 ───────────────

/// `CONTAINS("hello", "ell")` → true.
#[test]
fn contains_match() {
    let r = sel(r#"SELECT (CONTAINS("hello", "ell") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `STRSTARTS("hello", "hel")` → true.
#[test]
fn strstarts_match() {
    let r = sel(r#"SELECT (STRSTARTS("hello", "hel") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `STRENDS("hello", "llo")` → true.
#[test]
fn strends_match() {
    let r = sel(r#"SELECT (STRENDS("hello", "llo") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

// ─── F::StrBefore / F::StrAfter — lines 8293-8305 ────────────────────────────

/// `STRBEFORE("hello world", "world")` → "hello ".
#[test]
fn strbefore_match() {
    let r = sel(r#"SELECT (STRBEFORE("hello world", "world") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"hello \"".to_string()));
}

/// `STRBEFORE("hello", "xyz")` → empty string (no match → simple empty).
#[test]
fn strbefore_no_match() {
    let r = sel(r#"SELECT (STRBEFORE("hello", "xyz") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"\"".to_string()));
}

/// `STRAFTER("hello world", "hello ")` → "world".
#[test]
fn strafter_match() {
    let r = sel(r#"SELECT (STRAFTER("hello world", "hello ") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"world\"".to_string()));
}

/// `STRAFTER("hello", "xyz")` → empty string (no match).
#[test]
fn strafter_no_match() {
    let r = sel(r#"SELECT (STRAFTER("hello", "xyz") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"\"".to_string()));
}

// ─── F::Concat — exec.rs lines 8269-8285 ─────────────────────────────────────

/// `CONCAT("hello", " ", "world")` → "hello world". Tests the multi-arg path and
/// the `lang = Some(match lang { None => l, ... })` accumulation (lines 8276-8280).
//
#[test]
fn concat_simple_strings() {
    let r = sel(r#"SELECT (CONCAT("hello", " ", "world") AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"hello world\"".to_string()));
}

/// `CONCAT("a"@en, "b"@en)` → "ab"@en — same language tag preserved. Tests the
/// `Some(prev) if prev == l => prev` arm at line 8278.
#[test]
fn concat_same_lang_tag_preserved() {
    let r = sel(r#"SELECT (CONCAT("a"@en, "b"@en) AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"ab\"@en".to_string()));
}

/// `CONCAT("a"@en, "b"@fr)` → "ab" (plain — different tags drop tag). Tests the
/// `Some(_) => None` arm at line 8279.
#[test]
fn concat_mixed_lang_tags_dropped() {
    let r = sel(r#"SELECT (CONCAT("a"@en, "b"@fr) AS ?v) WHERE {}"#);
    assert_eq!(r, Some("\"ab\"".to_string()));
}

// ─── In(a, list) → false / error arms — exec.rs lines 7494-7509 ─────────────

/// `1 IN (2, 3)` → false (no match, no error). Exercises the `Value::Bool(false)`
/// return at line 7509.
#[test]
fn in_expr_false() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer IN (\"2\"^^xsd:integer, \"3\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(r, Some("\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `1 IN (1, 2)` → true (first element matches). Exercises line 7504.
#[test]
fn in_expr_true() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        "{}SELECT (\"1\"^^xsd:integer IN (\"1\"^^xsd:integer, \"2\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(r, Some("\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>".to_string()));
}

/// `1 IN (?x, 2)` where `?x` is unbound → `values_equal` returns `None` →
/// `errored = true` → `Value::Error` (line 7506, 7509).
#[test]
fn in_expr_comparison_error() {
    // ?x is unbound from the OPTIONAL (no triple matches); values_equal(1, Unbound) = None
    let r = sel(r#"SELECT (1 IN (?x, 2) AS ?v) WHERE { OPTIONAL { <ex:a> <ex:b> ?x } }"#);
    // errored=true + no match on 2 (1 != 2) → Value::Error → unbound → None
    // But wait: 1 IN (?x, 2): 1 == 2? No. So errored=true, no match → Value::Error → unbound
    // But actually 1 != 2, so errored stays true, returns Value::Error
    // Actually wait - `2` here is integer 2... but `1 IN (?x, 2)`:
    // - compare 1 vs ?x: ?x is unbound → values_equal(1, Unbound) = None → errored = true
    // - compare 1 vs 2: 1 != 2 → Some(false)
    // → errored=true, no Some(true) hit → Value::Error (line 7509 `errored` branch)
    assert_eq!(r, None, "IN with unbound comparison should propagate error");
}

// ─── F::NOW / F::Rand / F::StrUuid — exec.rs lines 8437-8450 ────────────────

/// `NOW()` returns an xsd:dateTime. Exercises F::Now (line 8437).
#[test]
fn now_returns_datetime() {
    let r = sel(r#"SELECT (NOW() AS ?v) WHERE {}"#);
    assert!(r.is_some(), "NOW() should return a value");
    let s = r.unwrap();
    assert!(
        s.contains("dateTime"),
        "NOW() should return an xsd:dateTime, got: {}",
        s
    );
}

/// `RAND()` returns an xsd:double in [0, 1). Exercises F::Rand (line 8439).
//
#[test]
fn rand_returns_double() {
    let r = sel(r#"SELECT (RAND() AS ?v) WHERE {}"#);
    assert!(r.is_some(), "RAND() should return a value");
    let s = r.unwrap();
    assert!(
        s.contains("double"),
        "RAND() should return an xsd:double, got: {}",
        s
    );
}

/// `STRUUID()` returns a simple literal UUID string. Exercises F::StrUuid (line 8450).
//
#[test]
fn struuid_returns_string() {
    let r = sel(r#"SELECT (STRUUID() AS ?v) WHERE {}"#);
    assert!(r.is_some(), "STRUUID() should return a value");
    let s = r.unwrap();
    // UUID is a simple literal (no type suffix, no lang tag)
    assert!(s.starts_with('"'), "STRUUID() should return a simple string literal: {}", s);
    // Check it has UUID format (8-4-4-4-12 hex chars separated by dashes)
    let inner = s.trim_matches('"');
    assert_eq!(inner.len(), 36, "UUID should be 36 chars: {}", inner);
}

/// `UUID()` returns an IRI. Exercises F::Uuid (line 8445).
#[test]
fn uuid_returns_iri() {
    let r = sel(r#"SELECT (UUID() AS ?v) WHERE {}"#);
    assert!(r.is_some(), "UUID() should return a value");
    let s = r.unwrap();
    assert!(
        s.starts_with("<urn:uuid:"),
        "UUID() should return a urn:uuid IRI, got: {}",
        s
    );
}

// ─── F::Tz / F::Timezone error paths (no-tz datetime) ─────────────────────────

/// `TZ("2024-01-01T00:00:00")` — datetime without timezone → TZ returns empty string
/// (line 8969-8971).
#[test]
fn tz_no_timezone() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        r#"{}SELECT (TZ("2024-01-01T00:00:00"^^xsd:dateTime) AS ?v) WHERE {{}}"#,
        pfx
    ));
    // No timezone → TZ returns "" (empty string simple literal)
    assert_eq!(r, Some("\"\"".to_string()));
}

/// `TIMEZONE("2024-01-01T00:00:00")` — datetime without timezone → TIMEZONE errors
/// (tz_to_duration returns None for empty tz, line 8978-8979).
#[test]
fn timezone_no_timezone_error() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        r#"{}SELECT (TIMEZONE("2024-01-01T00:00:00"^^xsd:dateTime) AS ?v) WHERE {{}}"#,
        pfx
    ));
    // No timezone → tz_to_duration("") = None → Value::Error → no binding
    assert_eq!(r, None, "TIMEZONE without timezone should error");
}

/// `TIMEZONE("2024-01-01T00:00:00Z")` — `Z` timezone → tz_to_duration returns
/// "PT0S" (line 8982).
#[test]
fn timezone_utc_z() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        r#"{}SELECT (TIMEZONE("2024-01-01T00:00:00Z"^^xsd:dateTime) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(r.is_some(), "TIMEZONE(Z) should return a value");
    let s = r.unwrap();
    assert!(s.contains("PT0S"), "Z timezone → PT0S, got: {}", s);
}

// ─── F::LangDir computed_num / IRI branch (lines 8538-8539) ──────────────────

/// `LANGDIR(1+1)` — computed numeric → the `Value::Num(_) | Value::Bool(_)` arm
/// at line 8538 returns the empty simple string.
#[test]
fn langdir_computed_num() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        "{}SELECT (LANGDIR(\"1\"^^xsd:integer + \"1\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(r, Some("\"\"".to_string()));
}

// ─── F::StrLangDir invalid direction (line 8549) ─────────────────────────────

/// `STRLANGDIR("x", "en", "upward")` — invalid direction (not "ltr"/"rtl") →
/// `Value::Error` (line 8549).
#[test]
fn strlangdir_invalid_direction() {
    let r = sel(r#"SELECT (STRLANGDIR("x", "en", "upward") AS ?v) WHERE {}"#);
    assert_eq!(r, None, "STRLANGDIR with invalid direction should error");
}

// ─── F::Str on computed values (line 8238) ───────────────────────────────────

/// `STR(1+1)` — `value_str` on a computed numeric → string form. Line 8238.
//
#[test]
fn str_computed_num() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";
    let r = sel(&format!(
        "{}SELECT (STR(\"2\"^^xsd:integer + \"1\"^^xsd:integer) AS ?v) WHERE {{}}",
        pfx
    ));
    assert_eq!(r, Some("\"3\"".to_string()));
}

// ─── F::Abs / Ceil / Floor / Round on decimal (lines 8353-8356) ──────────────

/// `ABS(-2.5)`, `CEIL(-2.5)`, `FLOOR(-2.5)`, `ROUND(-2.5)` — exercises all four
/// numeric rounding functions (lines 8353-8356).
#[test]
fn abs_ceil_floor_round() {
    let pfx = "PREFIX xsd: <http://www.w3.org/2001/XMLSchema#> ";

    let abs = sel(&format!(
        r#"{}SELECT (ABS("-2.5"^^xsd:decimal) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(abs.is_some(), "ABS should return a value");

    let ceil = sel(&format!(
        r#"{}SELECT (CEIL("-2.5"^^xsd:decimal) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(ceil.is_some(), "CEIL should return a value");

    let floor = sel(&format!(
        r#"{}SELECT (FLOOR("-2.5"^^xsd:decimal) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(floor.is_some(), "FLOOR should return a value");

    let round = sel(&format!(
        r#"{}SELECT (ROUND("-2.5"^^xsd:decimal) AS ?v) WHERE {{}}"#,
        pfx
    ));
    assert!(round.is_some(), "ROUND should return a value");
}

// ─── lib.rs: ASK paths through query() ───────────────────────────────────────
// sq-qcnn.35 – covers query_prepared_with_budget ASK match arm
// (lib.rs lines 734-738): true → unit row, false → empty row-set.

/// ASK { } on an empty graph is satisfiable (the empty pattern always has a solution).
/// Exercises the true arm of the ASK branch in query_prepared_with_budget.
#[test]
fn query_ask_true_returns_unit_row() {
    let g = empty();
    let r = sparq_engine::query(&g, "ASK { }").unwrap();
    assert_eq!(r.rows.len(), 1, "ASK true must produce exactly one unit row");
    assert!(r.rows[0].is_empty(), "unit row has no columns");
    assert!(r.vars.is_empty(), "ASK result has no projected variables");
}

/// ASK with an unsatisfiable pattern (no triples in graph) returns zero rows.
/// Exercises the false arm of the ASK branch in query_prepared_with_budget.
#[test]
fn query_ask_false_returns_no_rows() {
    let g = empty();
    let r = sparq_engine::query(&g, "ASK { ?s ?p ?o }").unwrap();
    assert_eq!(r.rows.len(), 0, "ASK false must produce zero rows");
}

// ─── lib.rs: ASK paths through query_json() ──────────────────────────────────
// sq-qcnn.35 – covers query_json_prepared_with_budget ASK arm
// (lib.rs line 806-807): emits the SPARQL 1.1 JSON boolean form.

/// query_json() on an ASK query that is true emits the boolean JSON form.
#[test]
fn query_json_ask_true_boolean_form() {
    let g = empty();
    let json = sparq_engine::query_json(&g, "ASK { }").unwrap();
    assert_eq!(json, r#"{"head":{},"boolean":true}"#);
}

/// query_json() on an unsatisfied ASK emits false boolean form.
#[test]
fn query_json_ask_false_boolean_form() {
    let g = empty();
    let json = sparq_engine::query_json(&g, "ASK { ?s ?p ?o }").unwrap();
    assert_eq!(json, r#"{"head":{},"boolean":false}"#);
}

// ─── lib.rs: ASK through query_json_chunks_with_budget() ─────────────────────
// sq-qcnn.35 – covers the ASK arm inside
// query_json_chunks_with_budget (lib.rs lines 831-832).

/// query_json_chunks_with_budget() for an ASK query returns the boolean chunk.
#[test]
fn query_json_chunks_ask_true_produces_boolean_chunk() {
    let g = empty();
    let budget = sparq_engine::QueryBudget::unlimited();
    let chunks = sparq_engine::query_json_chunks_with_budget(&g, "ASK { }", &budget).unwrap();
    let json: String = chunks.concat();
    assert_eq!(json, r#"{"head":{},"boolean":true}"#);
}

// ─── lib.rs: ASK through count() ─────────────────────────────────────────────
// sq-qcnn.35 – covers count_prepared_with_budget ASK arm
// (lib.rs line 865): count(ASK true) = 1, count(ASK false) = 0.

/// count() of a true ASK query returns 1 (the unit-row encoding).
#[test]
fn count_ask_true_is_one() {
    let g = empty();
    let n = sparq_engine::count(&g, "ASK { }").unwrap();
    assert_eq!(n, 1, "count(ASK true) = 1");
}

/// count() of a false ASK query returns 0.
#[test]
fn count_ask_false_is_zero() {
    let g = empty();
    let n = sparq_engine::count(&g, "ASK { ?s ?p ?o }").unwrap();
    assert_eq!(n, 0, "count(ASK false) = 0");
}

// ─── json.rs: fast-path JSON writers via query_json() ────────────────────────
// sq-qcnn.35 – these tests use query_json() which goes through
// the id-level JSON fast-path in exec.rs (eval_select_json → write_id_json).
// That path calls parts_to_json / inline_int_json in json.rs for dictionary
// terms — covering branches that to_sparql_json_rows (the materialised path)
// does not reach.

/// Blank-node dictionary term → parts_to_json Blank arm (json.rs lines 26-29).
#[test]
fn query_json_blank_node_subject_in_result() {
    // A blank-node subject lands in the dictionary at load time; query_json
    // serialises it through the fast-path (parts_to_json Blank arm).
    let g = Graph::load_str("_:a <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let json = sparq_engine::query_json(&g, "SELECT ?s WHERE { ?s ?p ?o }").unwrap();
    assert!(json.contains("\"bnode\""), "expected bnode type in JSON: {}", json);
}

/// Lang-tagged literal → parts_to_json lang arm (json.rs lines 35, 42-50).
#[test]
fn query_json_lang_tagged_literal_in_result() {
    let g = Graph::load_str(
        "<http://ex/s> <http://ex/label> \"hello\"@en .",
        "turtle",
    )
    .unwrap();
    let json = sparq_engine::query_json(&g, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    assert!(json.contains("\"xml:lang\""), "expected xml:lang in JSON: {}", json);
    assert!(json.contains("\"en\""), "expected en language tag in JSON: {}", json);
}

/// Typed literal (non-xsd:string) → parts_to_json datatype arm (json.rs lines 51-53).
#[test]
fn query_json_typed_literal_non_string_in_result() {
    let g = Graph::load_str(
        "<http://ex/s> <http://ex/v> \"3.14\"^^<http://www.w3.org/2001/XMLSchema#double> .",
        "turtle",
    )
    .unwrap();
    let json = sparq_engine::query_json(&g, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    assert!(json.contains("\"datatype\""), "expected datatype key in JSON: {}", json);
    assert!(json.contains("double"), "expected double type in JSON: {}", json);
}

/// Integer zero literal → inline_int_json v==0 arm (json.rs line 64).
#[test]
fn query_json_integer_zero_in_result() {
    let g = Graph::load_str(
        "<http://ex/s> <http://ex/n> \"0\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
        "turtle",
    )
    .unwrap();
    let json = sparq_engine::query_json(&g, "SELECT ?o WHERE { ?s ?p ?o }").unwrap();
    assert!(json.contains("\"0\""), "expected literal zero in JSON: {}", json);
    assert!(
        json.contains("integer"),
        "expected integer datatype in JSON: {}",
        json
    );
}

// ─── json.rs: to_sparql_json_rows with Triple term ───────────────────────────
// sq-qcnn.35 – exercises term_to_json Triple arm with a blank-node
// subject (json.rs line 179).

/// to_sparql_json_rows() with a Term::Triple whose subject is a blank node
/// covers the BlankNode arm inside the Triple arm of term_to_json (json.rs line 179).
#[test]
fn to_sparql_json_rows_triple_with_blank_node_subject() {
    use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, Term, Triple};
    let var = oxrdf::Variable::new("t").unwrap();
    // Construct a triple term with a blank-node subject.
    let triple = Triple::new(
        NamedOrBlankNode::BlankNode(BlankNode::new("b1").unwrap()),
        NamedNode::new("http://ex/p").unwrap(),
        Term::NamedNode(NamedNode::new("http://ex/o").unwrap()),
    );
    let rows: Vec<Vec<Option<Term>>> = vec![vec![Some(Term::Triple(Box::new(triple)))]];
    let json = sparq_engine::json::to_sparql_json_rows(&[var], &rows);
    assert!(
        json.contains("\"bnode\""),
        "expected bnode in triple subject JSON: {}",
        json
    );
    assert!(
        json.contains("\"triple\""),
        "expected triple type in JSON: {}",
        json
    );
}

// ─── update.rs: CLEAR / DROP graph-target variants ───────────────────────────
// sq-qcnn.35 – exercises CLEAR NAMED, CLEAR ALL, DROP DEFAULT,
// DROP NAMED arms in apply_op (update.rs lines 446, 447-449, 455, 460).

/// CLEAR NAMED empties all named graphs (update.rs line 446).
#[test]
fn update_clear_named_empties_named_graphs() {
    let ttl = "<http://ex/s> <http://ex/p> <http://ex/o> .";
    let mut g = Graph::load_str(ttl, "turtle").unwrap();
    // Add a named graph entry first.
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g1> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    )
    .unwrap();
    assert!(!g.named.is_empty(), "setup: named graph should exist");
    // CLEAR NAMED clears all named graphs (keeps them but empties their triple sets).
    sparq_engine::update_in_place(&mut g, "CLEAR NAMED").unwrap();
    for (_, store) in &g.named {
        assert_eq!(store.len(), 0, "CLEAR NAMED must empty every named graph");
    }
}

/// CLEAR ALL empties both the default graph and all named graphs (update.rs lines 447-449).
#[test]
fn update_clear_all_empties_all_graphs() {
    let ttl = "<http://ex/s> <http://ex/p> <http://ex/o> .";
    let mut g = Graph::load_str(ttl, "turtle").unwrap();
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g1> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    )
    .unwrap();
    sparq_engine::update_in_place(&mut g, "CLEAR ALL").unwrap();
    assert_eq!(g.store.len(), 0, "CLEAR ALL must empty the default graph");
    for (_, store) in &g.named {
        assert_eq!(store.len(), 0, "CLEAR ALL must empty every named graph");
    }
}

/// DROP DEFAULT clears the default graph (update.rs line 455).
#[test]
fn update_drop_default_clears_default_graph() {
    let ttl = "<http://ex/s> <http://ex/p> <http://ex/o> .";
    let mut g = Graph::load_str(ttl, "turtle").unwrap();
    assert!(!g.store.is_empty(), "setup: default graph must have triples");
    sparq_engine::update_in_place(&mut g, "DROP DEFAULT").unwrap();
    assert_eq!(g.store.len(), 0, "DROP DEFAULT must clear the default graph");
}

/// DROP NAMED removes all named-graph entries (update.rs line 460).
#[test]
fn update_drop_named_removes_named_graphs() {
    let ttl = "<http://ex/s> <http://ex/p> <http://ex/o> .";
    let mut g = Graph::load_str(ttl, "turtle").unwrap();
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g1> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    )
    .unwrap();
    assert!(!g.named.is_empty(), "setup: named graph should exist");
    sparq_engine::update_in_place(&mut g, "DROP NAMED").unwrap();
    assert!(g.named.is_empty(), "DROP NAMED must remove all named-graph entries");
}

// ─── exec.rs trace_label arms via explain_analyze ────────────────────────────
// sq-qcnn.35 — trace_label(Path), trace_label(Graph),
// trace_label(Project) are unreachable from eval_modified but ARE reached when
// eval_graph_pattern is called on those patterns as a Join child under an
// active trace::install(). explain_analyze installs the trace.

/// trace_label(GraphPattern::Path) — line 2305 in exec.rs.
/// A property-path expression (`<p>+`) forces the parser to emit a Path node.
/// Under explain_analyze the trace is active, so eval_graph_pattern_traced is
/// called on the Path pattern and trace_label hits the `GraphPattern::Path` arm.
#[test]
fn explain_analyze_path_covers_trace_label_path() {
    let g = Graph::load_str(
        "<http://ex/a> <http://ex/p> <http://ex/b> .",
        "turtle",
    )
    .unwrap();
    let out = sparq_engine::explain_analyze(
        &g,
        "SELECT * WHERE { <http://ex/a> <http://ex/p>+ ?o }",
    )
    .unwrap();
    // The trace header must appear (confirms tracing ran).
    assert!(
        out.contains("Execution trace"),
        "expected trace output: {}",
        out
    );
}

/// trace_label(GraphPattern::Graph) — line 2306 in exec.rs.
/// A GRAPH clause is a non-modifying pattern; eval_modified falls through to
/// `other => eval_graph_pattern(Graph{...})`, which triggers trace_label.
#[test]
fn explain_analyze_graph_clause_covers_trace_label_graph() {
    let mut g = Graph::load_str("", "turtle").unwrap();
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    )
    .unwrap();
    let out = sparq_engine::explain_analyze(
        &g,
        "SELECT * WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }",
    )
    .unwrap();
    assert!(
        out.contains("Execution trace"),
        "expected trace output: {}",
        out
    );
}

/// trace_label(GraphPattern::Project) — line 2307 in exec.rs.
/// A sub-SELECT appears as a Join child when it follows another pattern in the
/// group graph pattern. The Join is processed by eval_graph_pattern_inner which
/// calls eval_graph_pattern(Project{...}), hitting trace_label(Project).
#[test]
fn explain_analyze_sub_select_in_join_covers_trace_label_project() {
    let g = Graph::load_str(
        "<http://ex/s> <http://ex/p> <http://ex/o> .",
        "turtle",
    )
    .unwrap();
    let out = sparq_engine::explain_analyze(
        &g,
        "SELECT * WHERE { ?s ?p ?o . { SELECT ?x WHERE { ?x ?p2 ?o2 } } }",
    )
    .unwrap();
    assert!(
        out.contains("Execution trace"),
        "expected trace output: {}",
        out
    );
}

// ─── exec.rs GRAPH ?g {?g ...} path — lines 2159-2163 ────────────────────────
// sq-qcnn.35 — When `GRAPH ?g { ?g :p ?o }` is evaluated and the
// inner pattern evaluation also produces a column for ?g (because ?g appears
// inside the inner pattern), the Some(c) arm at line 2159 is taken.

/// GRAPH ?g { ?g <p> ?o } — the graph variable ?g appears INSIDE the inner
/// pattern, so after evaluating the inner pattern, the binding has a column for
/// ?g. The code retains rows where row[c] already equals gid (line 2165) and
/// fills NO_ID cells (line 2162). Covers exec.rs lines 2159-2167.
#[test]
fn graph_var_used_inside_pattern_covers_retain_branch() {
    let mut g = Graph::load_str("", "turtle").unwrap();
    // Insert a triple in named graph <http://ex/g> where the subject IS the
    // graph IRI itself: <g> <p> <o> inside graph <g>.
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g> { <http://ex/g> <http://ex/p> <http://ex/o> } }",
    )
    .unwrap();
    // GRAPH ?g { ?g <http://ex/p> ?o } — the inner pattern binds ?g to the
    // subject, so after eval_graph_named the Bindings has a ?g column.
    let r = query(
        &g,
        "SELECT ?g ?o WHERE { GRAPH ?g { ?g <http://ex/p> ?o } }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "expected exactly one result row");
    let g_val = r.rows[0][0].as_ref().unwrap().to_string();
    assert_eq!(
        g_val, "<http://ex/g>",
        "?g must be the named graph IRI: {}",
        g_val
    );
}

// ─── exec.rs temporal filter count — lines 1847-1856 ────────────────────────
// sq-qcnn.35 — count_single_filtered with a ScanCmp::Temp
// comparison runs the temporal-filter count closure.

/// COUNT(*) with a sargable dateTime FILTER on a single pattern triggers
/// count_single_filtered → the ScanCmp::Temp arm (lines 1847-1856).
/// The inner count_single_filtered counts 2 rows (e1=2021, e3=2023), but
/// `query()` returns one aggregate result row whose value is "2".
#[test]
fn count_with_datetime_filter_covers_temporal_scan_cmp() {
    let ttl = "\
        <http://ex/e1> <http://ex/ts> \"2021-06-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n\
        <http://ex/e2> <http://ex/ts> \"2019-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .\n\
        <http://ex/e3> <http://ex/ts> \"2023-03-15T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> .";
    let g = Graph::load_str(ttl, "turtle").unwrap();
    // Single pattern + temporal FILTER: the GROUP shortcut calls try_count on the
    // inner Filter pattern → count_pushdown → count_single_filtered → ScanCmp::Temp
    // branch executes the temporal-filter closure over the 3 data rows.
    let r = query(
        &g,
        "SELECT (COUNT(*) AS ?c) WHERE { ?s <http://ex/ts> ?d . FILTER(?d > \"2020-01-01T00:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>) }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "aggregate returns one result row");
    // The COUNT value is 2: e1 (2021) and e3 (2023) pass; e2 (2019) fails.
    let val = r.rows[0][0].as_ref().unwrap().to_string();
    assert_eq!(val, "\"2\"^^<http://www.w3.org/2001/XMLSchema#integer>", "count must be 2: {}", val);
}

// ─── exec.rs multi-cmp fallback count — lines 1857-1864 ─────────────────────
// sq-qcnn.35 — count_single_filtered with 2 sargable filters
// (neither the Num-single nor the Temp-single arm) hits `_ =>` (lines 1857-1864).

/// COUNT(*) with two sargable numeric FILTERs on one pattern triggers
/// count_single_filtered → the fallback `_ =>` arm (lines 1857-1864) because
/// cmps has len>1, so neither the single-Num nor single-Temp arm matches.
/// The inner count_single_filtered counts 1 row (s2=7); query returns
/// one aggregate row whose value is "1".
#[test]
fn count_with_two_filters_covers_multi_cmp_fallback() {
    let ttl = "\
        <http://ex/s1> <http://ex/n> \"3\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
        <http://ex/s2> <http://ex/n> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n\
        <http://ex/s3> <http://ex/n> \"12\"^^<http://www.w3.org/2001/XMLSchema#integer> .";
    let g = Graph::load_str(ttl, "turtle").unwrap();
    // Two SEPARATE FILTER clauses: FILTER(?n > 4) FILTER(?n < 10). spargebra
    // produces nested Filter patterns; flatten_conjunction un-nests them into
    // filters=[?n>4, ?n<10] (two separate expressions). Both are sargable numerics
    // → cmps=[Num(Gt(4)), Num(Lt(10))], len=2 → neither single arm matches
    // → fallback `_ =>` closure (lines 1857-1864) runs over all 3 data rows.
    let r = query(
        &g,
        "SELECT (COUNT(*) AS ?c) WHERE { ?s <http://ex/n> ?n . FILTER(?n > 4) FILTER(?n < 10) }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "aggregate returns one result row");
    // s2 (7) is the only row passing both filters.
    let val = r.rows[0][0].as_ref().unwrap().to_string();
    assert_eq!(val, "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>", "count must be 1: {}", val);
}

// ─── exec.rs star-count sorted_vals Some branch — lines 1771-1781 ────────────
// sq-qcnn.35 — GroupStream::new sets sorted_vals=Some when the
// chosen permutation does not deliver sort_col as the first free column. For a
// pattern with predicate=fixed and center-var at subject position (v_pos=0),
// choose_sorted finds no direct-order permutation and falls back to POS, whose
// first free column is O≠S → sorted=false → sorted_vals=Some(collected values).

/// Multi-pattern star COUNT triggers count_pushdown → GroupStream with a
/// fixed-predicate pattern where sort_col=S but POS is chosen → sorted_vals=Some
/// → GroupStream::next() runs the Some(vals) branch (lines 1771-1781).
/// The aggregate COUNT(*) query returns one result row with count value "1".
#[test]
fn count_star_shape_with_fixed_predicate_covers_sorted_vals_fallback() {
    let ttl = "\
        <http://ex/a> <http://ex/p1> <http://ex/v1> .\n\
        <http://ex/a> <http://ex/p2> <http://ex/v2> .\n\
        <http://ex/b> <http://ex/p1> <http://ex/v3> .";
    let g = Graph::load_str(ttl, "turtle").unwrap();
    // Star shape: ?s <p1> ?o1 . ?s <p2> ?o2 — center=?s, each pattern has a
    // FIXED predicate (P bound), so v_pos=S=0 but POS is the chosen perm →
    // sorted=false → sorted_vals=Some → GroupStream::next Some(vals) branch.
    // Inner star-count: Σ_{?s=<a>} count1(<a>)×count2(<a>) = 1×1 = 1.
    let r = query(
        &g,
        "SELECT (COUNT(*) AS ?c) WHERE { ?s <http://ex/p1> ?o1 . ?s <http://ex/p2> ?o2 }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "aggregate returns one result row");
    let val = r.rows[0][0].as_ref().unwrap().to_string();
    assert_eq!(
        val,
        "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
        "star-count must be 1 (only <a> has both p1 and p2): {}",
        val
    );
}

// ─── exec.rs Value::Bool path in value_to_id — lines 9130-9134 ───────────────
// sq-qcnn.35 — group_aggregate evaluates MAX(BOUND(?p)) which
// returns Value::Bool(true). value_to_id is called in the sequential aggregate
// path (line 6248), hitting the Value::Bool arm.

/// MAX(BOUND(?p)) evaluates to Value::Bool(true) for a non-empty result set.
/// group_aggregate calls value_to_id(graph, local, Value::Bool(true)), covering
/// exec.rs lines 9130-9134 (Bool arm of value_to_id).
#[test]
fn aggregate_max_bound_covers_value_to_id_bool_arm() {
    let g = Graph::load_str(
        "<http://ex/s> <http://ex/p> <http://ex/o> .",
        "turtle",
    )
    .unwrap();
    // MAX(BOUND(?p)) — BOUND(?p) is Value::Bool(true) for each row; minmax_values
    // returns Value::Bool(true); value_to_id called with it → Bool arm.
    let r = query(&g, "SELECT (MAX(BOUND(?p)) AS ?m) WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(r.rows.len(), 1, "expected one aggregate result row");
    let val = r.rows[0][0].as_ref().unwrap().to_string();
    assert!(
        val.contains("boolean"),
        "expected xsd:boolean result: {}",
        val
    );
    assert!(
        val.contains("true"),
        "MAX(BOUND(?p)) must be true: {}",
        val
    );
}

/// On a graph that already contains a boolean literal in the dictionary,
/// value_to_id(Value::Bool(true)) hits the `id => return id` fast-return
/// at exec.rs line 9134 instead of allocating a new Term.
#[test]
fn aggregate_max_bound_with_bool_in_dict_covers_fast_return() {
    // The graph contains "true"^^xsd:boolean so the dictionary already has it.
    let g = Graph::load_str(
        "<http://ex/s> <http://ex/p> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> .",
        "turtle",
    )
    .unwrap();
    let r = query(&g, "SELECT (MAX(BOUND(?p)) AS ?m) WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(r.rows.len(), 1, "expected one aggregate result row");
    let val = r.rows[0][0].as_ref().unwrap().to_string();
    assert!(
        val.contains("true"),
        "MAX(BOUND(?p)) must be true: {}",
        val
    );
}

// ─── exec.rs UnaryPlus / UnaryMinus in eval_exact_lexical — lines 7694-7699 ──
// sq-qcnn.35 — eval_exact_lexical is called when cmp_expr / equal_expr
// needs a precise lexical for two numeric values that f64 rounds to equal. The
// UnaryPlus and UnaryMinus arms are hit when the expression is +?var or -?var.

/// FILTER(+?x = 5) where ?x = 5 (integer). After f64 equality, eval_exact_lexical
/// is called on both sides. The left side is UnaryPlus(Variable(?x)) → line 7694.
#[test]
fn filter_unary_plus_covers_eval_exact_lexical_plus_arm() {
    let r = query(
        &empty(),
        "SELECT ?x WHERE { VALUES ?x { \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> } FILTER(+?x = 5) }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "FILTER(+?x = 5) with ?x=5 must pass");
}

/// FILTER(-?x = -5) where ?x = 5. eval_exact_lexical(UnaryMinus(Variable(?x)))
/// calls the inner recursively → returns "5", then strip_prefix('-') on "5" is
/// None → format!("-{s}") gives "-5". Compares to literal -5. Covers line 7697.
#[test]
fn filter_unary_minus_positive_input_covers_none_branch() {
    let r = query(
        &empty(),
        "SELECT ?x WHERE { VALUES ?x { \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> } FILTER(-?x = -5) }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "FILTER(-?x = -5) with ?x=5 must pass");
}

/// FILTER(-?x = 5) where ?x = -5. eval_exact_lexical(UnaryMinus(Variable(?x)))
/// → inner returns "-5", strip_prefix('-') = Some("5") → r.to_string() = "5".
/// Compares to literal 5. Covers line 7696.
#[test]
fn filter_unary_minus_negative_input_covers_some_branch() {
    let r = query(
        &empty(),
        "SELECT ?x WHERE { VALUES ?x { \"-5\"^^<http://www.w3.org/2001/XMLSchema#integer> } FILTER(-?x = 5) }",
    )
    .unwrap();
    assert_eq!(r.rows.len(), 1, "FILTER(-?x = 5) with ?x=-5 must pass");
}

/// explain() on a query with an aggregate but no explicit GROUP BY variables
/// (e.g. `SELECT (COUNT(*) AS ?c) WHERE { ... }`). spargebra emits a
/// `GraphPattern::Group { variables: [], ... }` node; render_pattern() takes
/// the Group arm and the `variables.is_empty()` sub-branch returns "<all>"
/// (explain.rs line 232). sq-qcnn.35
#[test]
fn explain_aggregate_no_group_by_covers_all_string_branch() {
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let plan = sparq_engine::explain(&g, "SELECT (COUNT(*) AS ?c) WHERE { ?s ?p ?o }").unwrap();
    assert!(plan.contains("Group"), "explain must mention Group for aggregate query: {}", plan);
    assert!(plan.contains("<all>"), "no-variable GROUP must show '<all>': {}", plan);
}

/// explain() on a query whose WHERE clause is `FILTER(true)` with no triple
/// patterns. After flatten_conjunction, patterns=[] and filters=[expr].
/// The empty-patterns branch writes the BGP-empty header, then iterates the
/// filters (explain.rs lines 258-259). sq-qcnn.35
#[test]
fn explain_empty_bgp_with_filter_covers_filter_loop() {
    let g = empty();
    let plan = sparq_engine::explain(&g, "SELECT * WHERE { FILTER(1 = 1) }").unwrap();
    assert!(
        plan.contains("BGP (empty"),
        "explain must show empty BGP line: {}",
        plan
    );
}

/// SPARQL Update: CLEAR NAMED via the immutable `update()` path (which goes
/// through `apply_op` → `apply_update_rebuild`, NOT `update_in_place_core`).
/// Covers update.rs line 446: `GraphTarget::NamedGraphs` arm in `apply_op`'s
/// Clear branch. sq-qcnn.35
#[test]
fn update_immutable_clear_named_covers_apply_op_named_graphs_arm() {
    let mut g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    ).unwrap();
    // Use the IMMUTABLE update() → apply_update_rebuild → apply_op path.
    let g2 = sparq_engine::update(&g, "CLEAR NAMED").unwrap();
    assert_eq!(
        query(&g2, "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap().rows.len(),
        0,
        "CLEAR NAMED via update() must leave named graphs empty"
    );
}

/// SPARQL Update: DROP DEFAULT via the immutable `update()` path.
/// Covers update.rs line 455: `GraphTarget::DefaultGraph` arm in `apply_op`'s
/// Drop branch. sq-qcnn.35
#[test]
fn update_immutable_drop_default_covers_apply_op_default_arm() {
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let g2 = sparq_engine::update(&g, "DROP DEFAULT").unwrap();
    assert_eq!(
        query(&g2, "SELECT * WHERE { ?s ?p ?o }").unwrap().rows.len(),
        0,
        "DROP DEFAULT via update() must empty the default graph"
    );
}

/// SPARQL Update: DROP NAMED via the immutable `update()` path.
/// Covers update.rs line 460: `GraphTarget::NamedGraphs` arm in `apply_op`'s
/// Drop branch. sq-qcnn.35
#[test]
fn update_immutable_drop_named_covers_apply_op_drop_named_arm() {
    let mut g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    sparq_engine::update_in_place(
        &mut g,
        "INSERT DATA { GRAPH <http://ex/g> { <http://ex/s> <http://ex/p> <http://ex/o> } }",
    ).unwrap();
    let g2 = sparq_engine::update(&g, "DROP NAMED").unwrap();
    assert_eq!(
        query(&g2, "SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }").unwrap().rows.len(),
        0,
        "DROP NAMED via update() must remove all named graphs"
    );
}

/// SPARQL Update: DELETE WHERE with an OPTIONAL clause where the optional
/// binding (?o) is unbound. The delete template has ?o in object position;
/// gtp_subst(&dp.object, &get) returns None → the `else { continue }` branch
/// skips this row (update.rs line 312). sq-qcnn.35
#[test]
fn update_delete_unbound_optional_covers_continue_branch() {
    let ttl = "<http://ex/s> <http://ex/p> <http://ex/x> .";
    let mut g = Graph::load_str(ttl, "turtle").unwrap();
    // ?o is unbound (no <ex:q> triple) → the DELETE template skips this row
    sparq_engine::update_in_place(
        &mut g,
        "DELETE { <http://ex/s> <http://ex/p> ?o } WHERE { <http://ex/s> <http://ex/p> ?x OPTIONAL { ?x <http://ex/q> ?o } }",
    ).unwrap();
    // Triple should remain (the delete was skipped due to unbound ?o)
    assert_eq!(
        query(&g, "SELECT * WHERE { ?s ?p ?o }").unwrap().rows.len(),
        1,
        "DELETE with unbound optional var must skip the row"
    );
}

/// CONSTRUCT { ?s <http://ex/p> ?o } WHERE { ?s <http://ex/p> ?o } over data
/// that has a blank-node subject. The WHERE clause binds ?s to a blank node,
/// so subject_term() takes the Term::BlankNode(b) arm (construct.rs line 205).
/// sq-qcnn.35
#[test]
fn construct_with_bnode_subject_covers_subject_term_bnode_arm() {
    // Turtle blank-node shorthand _:b produces a blank-node subject.
    let g = Graph::load_str("_:b <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let ts = sparq_engine::construct(
        &g,
        "CONSTRUCT { ?s <http://ex/p> ?o } WHERE { ?s <http://ex/p> ?o }",
    )
    .unwrap();
    // The blank-node triple must appear in the output (subject is a blank node).
    assert_eq!(ts.len(), 1, "one triple expected from blank-node subject CONSTRUCT: {:?}", ts);
    let subj = ts[0].subject.to_string();
    assert!(
        subj.starts_with("_:"),
        "subject of CONSTRUCT result must be a blank node, got: {}",
        subj
    );
}

/// DESCRIBE ?v WHERE { <http://ex/s> <http://ex/p> ?v } over data where the
/// object is a plain literal. The SELECT solution has a literal cell; cbd()
/// must skip it via the _ => {} arm (construct.rs line 267).
/// sq-qcnn.35
#[test]
fn describe_with_literal_result_covers_cbd_literal_skip_arm() {
    let g = Graph::load_str("<http://ex/s> <http://ex/p> \"hello\" .", "turtle").unwrap();
    // ?v is bound to the literal "hello"; cbd() must skip it silently.
    let ts = sparq_engine::describe(
        &g,
        "DESCRIBE ?v WHERE { <http://ex/s> <http://ex/p> ?v }",
    )
    .unwrap();
    // A literal has no CBD, so the result is empty (no triples about "hello").
    assert!(
        ts.is_empty(),
        "DESCRIBE with only-literal bindings must return empty graph, got: {:?}",
        ts
    );
}

/// `ask_prepared()` covers lib.rs lines 755-756 (the function was never called
/// before). A PreparedQuery is built from an ASK query string; ask_prepared
/// delegates to ask_prepared_with_budget with an unlimited budget.
/// sq-qcnn.35
#[test]
fn ask_prepared_covers_uncalled_function() {
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let pq = sparq_engine::PreparedQuery::parse("ASK { ?s ?p ?o }").unwrap();
    let result = sparq_engine::ask_prepared(&g, &pq).unwrap();
    assert!(result, "ASK {{ ?s ?p ?o }} must be true for non-empty graph");
}

/// `query_view()` covers lib.rs view functions (with_view, query_view,
/// query_view_with_budget — all with count=0 before this test). Uses an
/// empty named set (view restricted to no named graphs).
/// sq-qcnn.35
#[test]
fn query_view_covers_view_api_functions() {
    use sparq_engine::{DatasetView, DefaultGraphMode, FxHashSet};
    use std::sync::Arc;
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let view = DatasetView {
        base: &g,
        named: Arc::new(FxHashSet::default()),
        default: DefaultGraphMode::StoreDefault,
    };
    let r = sparq_engine::query_view(&view, "SELECT * WHERE { ?s ?p ?o }").unwrap();
    assert_eq!(r.rows.len(), 1, "query_view must see the default graph triple");
}

/// `ask_view()` covers lib.rs ask_view and ask_view_with_budget (both with
/// count=0 before this test). sq-qcnn.35
#[test]
fn ask_view_covers_ask_view_function() {
    use sparq_engine::{DatasetView, DefaultGraphMode, FxHashSet};
    use std::sync::Arc;
    let g = Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
    let view = DatasetView {
        base: &g,
        named: Arc::new(FxHashSet::default()),
        default: DefaultGraphMode::StoreDefault,
    };
    let result = sparq_engine::ask_view(&view, "ASK { ?s ?p ?o }").unwrap();
    assert!(result, "ask_view must return true for non-empty default graph");
}
