//! Engine-level regression for the SPARQL parse-depth bound
//! (bead sq-1ukn, cert gap ASVS-G4 / ASVS V5.5.2; threat-model boundary B2,
//! T-PARSE-DoS).
//!
//! `spargebra` is a `peg` recursive-descent parser: each level of `{ }` / `( )` /
//! `[ ]` / `<<( )>>` syntactic nesting (group graph patterns, sub-SELECTs,
//! bracketed/unary expressions, property paths, RDF collections, blank-node
//! property lists, RDF-1.2 triple terms) maps onto native call-stack frames. The
//! vendored crate already caps that recursion (`MAX_RECURSION_DEPTH`, bead
//! sq-v5dg) and `vendor/spargebra/tests/recursion_depth.rs` proves the cap fires
//! at the `SparqlParser` layer. THIS file pins the same guarantee at the seam the
//! cert gap actually names — `sparq_engine::PreparedQuery::parse`, through which
//! every string query entry point (`query` / `ask` / `count` / `construct` / …)
//! funnels — and at the concrete attack envelope the bead calls out: a request
//! body filled to the server's default `--max-body-bytes` cap (1 MiB, see
//! `sparq_server::http` `ServerConfig::default`). Without the depth bound, that
//! body of nested delimiters would deep-recurse the parser until the native stack
//! overflows and ABORTS the process — an unauthenticated DoS. With it, the parse
//! returns a clean `Err` and the body cap is the only thing standing between an
//! attacker and the parser.
//!
//! Each case parses on a deliberately small 2 MiB thread stack — the size of a
//! tokio worker / blocking-pool thread, where the server actually parses (NOT the
//! larger main-thread `ulimit -s`). A stack overflow there aborts the whole test
//! process (SIGABRT), so reaching the `join()` at all is itself part of the
//! assertion: the parser must have RETURNED, not recursed to death.

use sparq_engine::PreparedQuery;

/// The server's default request-body cap (`sparq_server::http` `ServerConfig`:
/// `max_body_bytes: 1024 * 1024`). A query string up to this length is exactly
/// what an unauthenticated client can push at the parser, so the depth guard must
/// hold for a body of nested delimiters filled right to this cap.
const BODY_CAP: usize = 1024 * 1024;

/// Run `f` on a 2 MiB stack (tokio's default worker/blocking-pool stack). A stack
/// overflow there aborts the process, so reaching the `join()` at all proves the
/// parser returned rather than recursing past the native stack.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn 2 MiB-stack thread")
        .join()
        .expect("parser thread must return, not overflow the stack")
}

/// Build a query whose `WHERE` clause is `open`-then-`close` delimiters nested
/// `depth` levels deep around `?s ?p ?o`, padded so the whole string is as close
/// to (but not over) `BODY_CAP` as the delimiter count allows.
fn deeply_nested_where(open: char, close: char, depth: usize) -> String {
    let mut q = String::with_capacity(depth * 2 + 32);
    q.push_str("SELECT * WHERE ");
    for _ in 0..depth {
        q.push(open);
    }
    q.push_str(" ?s ?p ?o ");
    for _ in 0..depth {
        q.push(close);
    }
    q
}

/// A 1 MiB body that is *entirely* nested group graph patterns — the maximal
/// recursion a default server will admit — must come back as a clean parse `Err`
/// and must NOT overflow the 2 MiB parse stack. This is the headline cert
/// assertion: it exercises `PreparedQuery::parse` (the engine seam every string
/// query funnels through) at the exact `--max-body-bytes` envelope.
#[test]
fn body_cap_nested_groups_is_clean_err_not_overflow() {
    let result = on_small_stack(|| {
        // Half the budget for '{', half for '}', leaving the "SELECT * WHERE "
        // prefix and " ?s ?p ?o " infix headroom under the cap.
        let depth = (BODY_CAP - 32) / 2;
        let q = deeply_nested_where('{', '}', depth);
        assert!(
            q.len() <= BODY_CAP,
            "test query must fit the body cap (got {} bytes)",
            q.len()
        );
        PreparedQuery::parse(&q).is_err()
    });
    assert!(
        result,
        "a body-cap-sized stack of nested groups must parse to Err, never Ok"
    );
}

/// Same envelope, bracketed-expression nesting (the FILTER expression axis is a
/// distinct recursive production from group patterns and must be bounded too).
#[test]
fn body_cap_nested_expressions_is_clean_err_not_overflow() {
    let result = on_small_stack(|| {
        let depth = (BODY_CAP - 32) / 2;
        let mut q = String::with_capacity(depth * 2 + 32);
        q.push_str("SELECT * WHERE { FILTER(");
        for _ in 0..depth {
            q.push('(');
        }
        q.push('1');
        for _ in 0..depth {
            q.push(')');
        }
        q.push_str(") }");
        assert!(
            q.len() <= BODY_CAP + 16,
            "test query must be near the body cap (got {} bytes)",
            q.len()
        );
        PreparedQuery::parse(&q).is_err()
    });
    assert!(
        result,
        "a body-cap-sized stack of nested expressions must parse to Err, never Ok"
    );
}

/// The depth error must surface as a clean `Err(String)` (the engine maps the
/// parser's `SparqlSyntaxError` through `to_string()`), never a panic or abort.
/// A modest-but-past-the-cap depth is enough to assert the wording without paying
/// for a full 1 MiB allocation, and confirms `PreparedQuery::parse` propagates the
/// guard's message rather than swallowing it.
#[test]
fn past_cap_depth_reports_too_deeply_nested() {
    let q = deeply_nested_where('{', '}', 4000);
    let err = PreparedQuery::parse(&q).expect_err("a 4000-deep group query must be rejected");
    assert!(
        err.contains("too deeply nested"),
        "expected the recursion-depth message from the engine parse seam, got: {err}"
    );
}

/// Positive control: a legitimately deep query (well under the cap, far deeper
/// than any real-world or W3C-conformance query) must still parse through the
/// engine seam. The guard must reject pathological nesting WITHOUT rejecting any
/// query a real client would send.
#[test]
fn legitimately_deep_query_still_parses_through_engine() {
    let q = deeply_nested_where('{', '}', 64);
    assert!(
        PreparedQuery::parse(&q).is_ok(),
        "a 64-deep group query (within the cap) must still parse through PreparedQuery::parse"
    );
}
