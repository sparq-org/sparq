use super::*;

// A datetime IRI written out long-hand keeps the probe strings readable.
const DT: &str = "^^<http://www.w3.org/2001/XMLSchema#dateTime>";

/// One BUILTIN's probes. `valid`/`boundary` carry the EXACT expected term in
/// `Term::to_string()` form (e.g. `"3"^^<…integer>`, `<http://ex/x>`, `"ab"@en`);
/// `None` boundary means "no distinct boundary case for this builtin".
struct Row {
    /// The `spargebra::algebra::Function` variant under test (documentation +
    /// stable label for failures; the SPARQL keyword in the probe selects it).
    builtin: &'static str,
    /// Well-typed call → expected term string.
    valid: (&'static str, &'static str),
    /// Wrong-typed call → MUST be an expression error (unbound `?out`).
    type_err: &'static str,
    /// Optional edge case → expected term string.
    boundary: Option<(&'static str, &'static str)>,
}

/// Single-row dataset: `?s = ex:s`, `?o = ex:o`. Probes that need a typed datum
/// embed their own literal; `?o` exists only so the BGP yields exactly one row.
fn one_row_graph() -> Graph {
    Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "ntriples").unwrap()
}

/// Evaluate `BIND(<expr> AS ?out)` over the one-row graph and return the single
/// row's `?out`: `Some(term-string)` when bound, `None` when the expression
/// errored (unbound). Panics if the row count is not exactly 1 (a BIND must never
/// add or drop rows — that would be a different bug than the value under test).
fn bind_out(g: &Graph, expr: &str) -> Option<String> {
    let q = format!("SELECT ?out WHERE {{ ?s <http://ex/p> ?o BIND({expr} AS ?out) }}");
    let r = crate::query(g, &q).unwrap_or_else(|e| panic!("query failed for `{expr}`: {e}"));
    assert_eq!(r.rows.len(), 1, "BIND must keep exactly one row for `{expr}`, got {}", r.rows.len());
    r.rows[0][0].as_ref().map(|t| t.to_string())
}

/// Drive every probe in `table`, accumulating ALL mismatches before failing so a
/// single run reports the full divergence set (not just the first).
fn run_table(table: &[Row]) {
    let g = one_row_graph();
    let mut fails: Vec<String> = Vec::new();
    let mut valid_n = 0usize;
    let mut err_n = 0usize;
    let mut bound_n = 0usize;
    for row in table {
        // (a) VALID — exact value.
        let (vexpr, vexp) = row.valid;
        match bind_out(&g, vexpr) {
            Some(got) if got == vexp => valid_n += 1,
            got => fails.push(format!("[{}] VALID `{vexpr}`: expected Some({vexp:?}), got {got:?}", row.builtin)),
        }
        // (b) TYPE ERROR — must be unbound (the engine's expression-error outcome).
        match bind_out(&g, row.type_err) {
            None => err_n += 1,
            Some(got) => fails.push(format!(
                "[{}] TYPE-ERROR `{}`: expected an expression error (unbound ?out), got Some({got:?})",
                row.builtin, row.type_err
            )),
        }
        // (c) BOUNDARY — exact value, when the builtin has a meaningful edge.
        if let Some((bexpr, bexp)) = row.boundary {
            match bind_out(&g, bexpr) {
                Some(got) if got == bexp => bound_n += 1,
                got => fails.push(format!("[{}] BOUNDARY `{bexpr}`: expected Some({bexp:?}), got {got:?}", row.builtin)),
            }
        }
    }
    assert!(
        fails.is_empty(),
        "{} builtin error-path probe(s) diverged from the SPARQL spec:\n  {}",
        fails.len(),
        fails.join("\n  ")
    );
    // The counts double as a coverage assertion: if a row is added/removed the
    // numbers move, so an accidental no-op table can't pass silently.
    eprintln!("builtin_error_paths: {} builtins, valid={valid_n} type_err={err_n} boundary={bound_n}", table.len());
}

const INT: &str = "^^<http://www.w3.org/2001/XMLSchema#integer>";
const BOOL: &str = "^^<http://www.w3.org/2001/XMLSchema#boolean>";

/// STRING builtins whose operand(s) must be STRING LITERALS (simple / lang-tagged /
/// xsd:string). An IRI / number / boolean operand is a type error.
#[test]
fn string_builtins() {
    // expected-term helpers as owned strings (the table holds &'static via Box::leak-free
    // const concat where possible; here we build at runtime then borrow).
    let table = vec![
        Row {
            builtin: "STR",
            // STR is the ONE string builtin that accepts ANY term (incl. IRIs/numbers):
            // so its "type error" probe is an UNBOUND operand, not a wrong type.
            valid: ("STR(123)", "\"123\""),
            type_err: "STR(?missing)",
            boundary: Some(("STR(<http://ex/o>)", "\"http://ex/o\"")),
        },
        Row {
            builtin: "STRLEN",
            valid: ("STRLEN(\"abc\")", "\"3\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
            // FIXED arm: an IRI is NOT a string literal → error (was wrongly 11).
            type_err: "STRLEN(<http://ex/o>)",
            // Counts CODEPOINTS, not bytes: "é" + a combining mark style multi-byte char.
            boundary: Some(("STRLEN(\"naïve\")", "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>")),
        },
        Row {
            builtin: "UCASE",
            valid: ("UCASE(\"abc\")", "\"ABC\""),
            type_err: "UCASE(123)",
            // Language tag is preserved (simple in → simple out; tagged in → tagged out).
            boundary: Some(("UCASE(\"abc\"@en)", "\"ABC\"@en")),
        },
        Row {
            builtin: "LCASE",
            valid: ("LCASE(\"ABC\")", "\"abc\""),
            type_err: "LCASE(<http://ex/o>)",
            boundary: Some(("LCASE(\"ABC\"@en)", "\"abc\"@en")),
        },
        Row {
            builtin: "CONCAT",
            valid: ("CONCAT(\"a\", \"b\")", "\"ab\""),
            // A non-string (numeric) operand makes the whole CONCAT a type error.
            type_err: "CONCAT(\"a\", 123)",
            // Same language tag on EVERY operand → result keeps it; a mismatch → simple.
            boundary: Some(("CONCAT(\"a\"@en, \"b\"@en)", "\"ab\"@en")),
        },
        Row {
            builtin: "CONCAT (lang mismatch → simple)",
            valid: ("CONCAT(\"a\"@en, \"b\"@fr)", "\"ab\""),
            type_err: "CONCAT(\"a\"@en, <http://ex/o>)",
            boundary: None,
        },
        Row {
            builtin: "CONTAINS",
            valid: ("CONTAINS(\"abc\", \"b\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            type_err: "CONTAINS(\"abc\", 1)",
            boundary: Some(("CONTAINS(\"abc\", \"z\")", "\"false\"^^<http://www.w3.org/2001/XMLSchema#boolean>")),
        },
        Row {
            builtin: "STRSTARTS",
            valid: ("STRSTARTS(\"abc\", \"ab\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            type_err: "STRSTARTS(123, \"ab\")",
            boundary: None,
        },
        Row {
            builtin: "STRENDS",
            valid: ("STRENDS(\"abc\", \"bc\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            type_err: "STRENDS(\"abc\", 3)",
            boundary: None,
        },
        Row {
            builtin: "STRBEFORE",
            valid: ("STRBEFORE(\"abc\", \"b\")", "\"a\""),
            type_err: "STRBEFORE(\"abc\", 1)",
            // No match → the EMPTY simple literal (not an error).
            boundary: Some(("STRBEFORE(\"abc\", \"z\")", "\"\"")),
        },
        Row {
            builtin: "STRAFTER",
            valid: ("STRAFTER(\"abc\", \"a\")", "\"bc\""),
            type_err: "STRAFTER(\"abc\", 1)",
            boundary: Some(("STRAFTER(\"abc\", \"z\")", "\"\"")),
        },
        Row {
            builtin: "ENCODE_FOR_URI",
            valid: ("ENCODE_FOR_URI(\"a b\")", "\"a%20b\""),
            // FIXED arm: a number is NOT a string literal → error (was wrongly "123").
            type_err: "ENCODE_FOR_URI(123)",
            // Unreserved set (RFC 3986) passes through untouched.
            boundary: Some(("ENCODE_FOR_URI(\"A-z_0.9~\")", "\"A-z_0.9~\"")),
        },
        Row {
            builtin: "SUBSTR",
            valid: ("SUBSTR(\"hello\", 2)", "\"ello\""),
            // A non-numeric start position is a type error.
            type_err: "SUBSTR(\"hello\", \"x\")",
            // 1-indexed by codepoint; a start past the end → empty.
            boundary: Some(("SUBSTR(\"hello\", 2, 2)", "\"el\"")),
        },
        Row {
            builtin: "SUBSTR (non-numeric length)",
            valid: ("SUBSTR(\"hello\", 1, 4)", "\"hell\""),
            type_err: "SUBSTR(\"hello\", 1, \"y\")",
            boundary: None,
        },
    ];
    run_table(&table);
}

/// NUMERIC builtins: the operand must be numeric; a string/IRI is a type error.
/// They PRESERVE the argument's numeric datatype.
#[test]
fn numeric_builtins() {
    let table = vec![
        Row {
            builtin: "ABS",
            valid: ("ABS(-3)", "\"3\"^^<http://www.w3.org/2001/XMLSchema#integer>"),
            type_err: "ABS(\"foo\")",
            boundary: Some(("ABS(0)", "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>")),
        },
        Row {
            builtin: "CEIL",
            valid: ("CEIL(2.1)", "\"3\"^^<http://www.w3.org/2001/XMLSchema#decimal>"),
            type_err: "CEIL(\"foo\")",
            boundary: Some(("CEIL(-2.1)", "\"-2\"^^<http://www.w3.org/2001/XMLSchema#decimal>")),
        },
        Row {
            builtin: "FLOOR",
            valid: ("FLOOR(2.9)", "\"2\"^^<http://www.w3.org/2001/XMLSchema#decimal>"),
            type_err: "FLOOR(<http://ex/o>)",
            boundary: Some(("FLOOR(-2.1)", "\"-3\"^^<http://www.w3.org/2001/XMLSchema#decimal>")),
        },
        Row {
            builtin: "ROUND",
            valid: ("ROUND(2.4)", "\"2\"^^<http://www.w3.org/2001/XMLSchema#decimal>"),
            type_err: "ROUND(\"foo\")",
            boundary: Some(("ROUND(2.5)", "\"3\"^^<http://www.w3.org/2001/XMLSchema#decimal>")),
        },
    ];
    run_table(&table);
    // STRLEN/CONTAINS-style probes already proved the bool/int datatype tags; here
    // the per-row datatype suffix (decimal vs integer) is the load-bearing anchor.
    let _ = (INT, BOOL, DT);
}

/// TYPE-TEST / accessor builtins (LANG, DATATYPE, isIRI, …) and the
/// literal-constructors (STRDT, STRLANG, STRLANGDIR).
#[test]
fn term_and_constructor_builtins() {
    let table = vec![
        Row {
            builtin: "LANG",
            valid: ("LANG(\"x\"@en)", "\"en\""),
            // LANG of an IRI is a type error (LANG of a non-tagged literal is "").
            type_err: "LANG(<http://ex/o>)",
            boundary: Some(("LANG(\"x\")", "\"\"")),
        },
        Row {
            builtin: "DATATYPE",
            valid: ("DATATYPE(\"x\")", "<http://www.w3.org/2001/XMLSchema#string>"),
            type_err: "DATATYPE(<http://ex/o>)",
            boundary: Some(("DATATYPE(42)", "<http://www.w3.org/2001/XMLSchema#integer>")),
        },
        Row {
            builtin: "STRDT",
            valid: (
                "STRDT(\"5\", <http://www.w3.org/2001/XMLSchema#integer>)",
                "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            ),
            // First arg must be a SIMPLE literal — a lang-tagged input is an error.
            type_err: "STRDT(\"5\"@en, <http://www.w3.org/2001/XMLSchema#integer>)",
            // A non-IRI second arg (a plain string) is also an error.
            boundary: None,
        },
        Row {
            builtin: "STRDT (non-IRI datatype)",
            valid: (
                "STRDT(\"x\", <http://ex/myType>)",
                "\"x\"^^<http://ex/myType>",
            ),
            type_err: "STRDT(\"5\", \"notaniri\")",
            boundary: None,
        },
        Row {
            builtin: "STRLANG",
            valid: ("STRLANG(\"hi\", \"en\")", "\"hi\"@en"),
            // Both args must be simple literals; a tagged first arg errors.
            type_err: "STRLANG(\"hi\"@fr, \"en\")",
            boundary: None,
        },
        Row {
            builtin: "STRLANGDIR",
            valid: ("STRLANGDIR(\"hi\", \"en\", \"ltr\")", "\"hi\"@en--ltr"),
            // The direction must be exactly "ltr"/"rtl"; anything else errors.
            type_err: "STRLANGDIR(\"hi\", \"en\", \"sideways\")",
            boundary: Some(("STRLANGDIR(\"hi\", \"en\", \"rtl\")", "\"hi\"@en--rtl")),
        },
        Row {
            builtin: "LANGMATCHES",
            valid: ("LANGMATCHES(\"en\", \"en\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            // FIXED arm: a non-string operand is a type error (was wrongly `false`).
            type_err: "LANGMATCHES(123, \"en\")",
            // "*" matches any non-empty tag.
            boundary: Some(("LANGMATCHES(\"en-GB\", \"*\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>")),
        },
        Row {
            builtin: "IRI",
            valid: ("IRI(\"http://valid/\")", "<http://valid/>"),
            // A lexical form that is not a legal IRI (contains spaces) is an error.
            type_err: "IRI(\"a b c\")",
            // An IRI argument passes through unchanged.
            boundary: Some(("IRI(<http://ex/o>)", "<http://ex/o>")),
        },
    ];
    run_table(&table);
}

/// DATE/TIME accessor builtins: the operand must be an xsd:dateTime; a string or
/// number is a type error.
#[test]
fn datetime_builtins() {
    let valid_dt = "\"2024-03-15T13:45:30\"";
    let mk = |kw: &str, exp: &str, ty: &str| Row {
        builtin: Box::leak(kw.to_string().into_boxed_str()),
        valid: (
            Box::leak(format!("{kw}({valid_dt}{DT})").into_boxed_str()),
            Box::leak(format!("\"{exp}\"^^<http://www.w3.org/2001/XMLSchema#{ty}>").into_boxed_str()),
        ),
        type_err: Box::leak(format!("{kw}(\"foo\")").into_boxed_str()),
        boundary: None,
    };
    let table = vec![
        mk("YEAR", "2024", "integer"),
        mk("MONTH", "3", "integer"),
        mk("DAY", "15", "integer"),
        mk("HOURS", "13", "integer"),
        mk("MINUTES", "45", "integer"),
        mk("SECONDS", "30", "decimal"),
        Row {
            builtin: "TZ",
            // No timezone in the lexical form → empty simple literal (NOT an error).
            valid: (Box::leak(format!("TZ({valid_dt}{DT})").into_boxed_str()), "\"\""),
            type_err: "TZ(\"foo\")",
            boundary: Some((
                Box::leak(format!("TZ(\"2024-03-15T13:45:30Z\"{DT})").into_boxed_str()),
                "\"Z\"",
            )),
        },
        Row {
            builtin: "TIMEZONE",
            // TIMEZONE on a dateTime WITHOUT a timezone is a type error (unlike TZ).
            valid: (
                Box::leak(format!("TIMEZONE(\"2024-03-15T13:45:30Z\"{DT})").into_boxed_str()),
                "\"PT0S\"^^<http://www.w3.org/2001/XMLSchema#dayTimeDuration>",
            ),
            type_err: Box::leak(format!("TIMEZONE({valid_dt}{DT})").into_boxed_str()),
            boundary: Some((
                Box::leak(format!("TIMEZONE(\"2024-03-15T13:45:30+05:00\"{DT})").into_boxed_str()),
                "\"PT5H\"^^<http://www.w3.org/2001/XMLSchema#dayTimeDuration>",
            )),
        },
    ];
    run_table(&table);
}

/// HASH builtins (feature `digest`): the operand must be a simple literal /
/// xsd:string; a number or IRI is a type error. Output is lowercase hex.
#[cfg(feature = "digest")]
#[test]
fn hash_builtins() {
    // Known vectors for the empty string and "abc".
    let table = vec![
        Row {
            builtin: "MD5",
            valid: ("MD5(\"abc\")", "\"900150983cd24fb0d6963f7d28e17f72\""),
            type_err: "MD5(123)",
            boundary: Some(("MD5(\"\")", "\"d41d8cd98f00b204e9800998ecf8427e\"")),
        },
        Row {
            builtin: "SHA1",
            valid: ("SHA1(\"abc\")", "\"a9993e364706816aba3e25717850c26c9cd0d89d\""),
            type_err: "SHA1(<http://ex/o>)",
            boundary: None,
        },
        Row {
            builtin: "SHA256",
            valid: (
                "SHA256(\"abc\")",
                "\"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\"",
            ),
            type_err: "SHA256(123)",
            boundary: None,
        },
        Row {
            builtin: "SHA384",
            valid: (
                "SHA384(\"abc\")",
                "\"cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7\"",
            ),
            type_err: "SHA384(<http://ex/o>)",
            boundary: None,
        },
        Row {
            builtin: "SHA512",
            valid: (
                "SHA512(\"abc\")",
                "\"ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f\"",
            ),
            type_err: "SHA512(42)",
            boundary: None,
        },
    ];
    run_table(&table);
}

/// REGEX / REPLACE (feature `regex`): the TEXT operand must be a string literal;
/// a malformed pattern OR an illegal flags string is a type error.
#[cfg(feature = "regex")]
#[test]
fn regex_builtins() {
    let table = vec![
        Row {
            builtin: "REGEX",
            valid: ("REGEX(\"abc\", \"^a\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            // A non-string text operand is a type error.
            type_err: "REGEX(123, \"a\")",
            // Case-insensitive flag.
            boundary: Some(("REGEX(\"ABC\", \"abc\", \"i\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>")),
        },
        Row {
            builtin: "REGEX (malformed pattern)",
            valid: ("REGEX(\"abc\", \"[a-c]\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            // An unclosed character class is a compile error → expression error.
            type_err: "REGEX(\"abc\", \"[\")",
            boundary: None,
        },
        Row {
            builtin: "REGEX (illegal flag)",
            valid: ("REGEX(\"abc\", \"a\", \"i\")", "\"true\"^^<http://www.w3.org/2001/XMLSchema#boolean>"),
            // 'z' is not a SPARQL/XPath regex flag → error.
            type_err: "REGEX(\"abc\", \"a\", \"z\")",
            boundary: None,
        },
        Row {
            builtin: "REPLACE",
            valid: ("REPLACE(\"abc\", \"b\", \"X\")", "\"aXc\""),
            type_err: "REPLACE(123, \"a\", \"X\")",
            // Keeps the text's language tag.
            boundary: Some(("REPLACE(\"abc\"@en, \"b\", \"X\")", "\"aXc\"@en")),
        },
        Row {
            builtin: "REPLACE (malformed pattern)",
            valid: ("REPLACE(\"abc\", \"a\", \"X\")", "\"Xbc\""),
            type_err: "REPLACE(\"abc\", \"[\", \"X\")",
            boundary: None,
        },
    ];
    run_table(&table);
}
