//! SERVICE bind-join (VALUES pushdown) — differential + request-reduction tests.
//! (bead sq-sjkj — research candidate C1)
//!
//! The whole correctness claim of the bound-join is: pushing the left side's bound
//! join variables into the remote query as a `VALUES` block yields results IDENTICAL
//! to evaluating the SERVICE unbound and joining locally. These tests prove that by
//! running BOTH paths through ONE mock transport and asserting equal result sets, and
//! they prove the *point* of the optimisation — fewer / smaller remote requests — by
//! inspecting what the mock was actually asked.
//!
//! The mock is a real second `sparq` `Graph` standing in for the remote endpoint: it
//! answers each `fetch` by running the received query (VALUES and all) through the
//! engine's own `query_json`, so the injected `VALUES` is genuinely evaluated remotely
//! — not faked. It records every query string + a request count.
//!
//!   cargo test -p sparq-engine --features service --test service_bound_join

#![cfg(feature = "service")]

use std::sync::Mutex;

use sparq_core::Graph;
use sparq_engine::{query, with_service_bound_join_block_size, QueryResult};

// Everything here is driven through the PUBLIC API (`query` + `query_json` as the
// mock's own evaluator) against a real loopback endpoint, so it exercises the whole
// production path: VALUES injection → ureq POST → SRJ parse → local join. The
// transport-seam (mocked-transport) differential — equal results bound vs verbatim —
// lives in the in-crate `service_exec_tests` module (exec.rs), where the `pub(crate)`
// injection seam is reachable without a socket.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use sparq_engine::with_service_egress_allow;

/// A loopback SPARQL endpoint backed by a real `Graph`: it parses each incoming
/// `query=` form field, runs it through the engine (so an injected `VALUES` block is
/// genuinely evaluated), replies with SPARQL-Results-JSON, and records every query
/// string it served. Serves `n` requests then exits (freeing the port).
struct Recorder {
    queries: Arc<Mutex<Vec<String>>>,
}

fn serve_graph(remote: Graph, n: usize) -> (String, Recorder) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let queries = Arc::new(Mutex::new(Vec::<String>::new()));
    let q_thread = Arc::clone(&queries);
    let remote = Arc::new(remote);
    thread::spawn(move || {
        tx.send(()).ok();
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = Vec::new();
            let mut tmp = [0u8; 8192];
            // Read until we have the FULL request: the headers, then exactly
            // Content-Length bytes of body. A short read mid-body would lose the
            // `query=` form value and make the mock answer the wrong question.
            loop {
                let have_headers = buf.windows(4).position(|w| w == b"\r\n\r\n");
                if let Some(hdr_end) = have_headers {
                    let body_start = hdr_end + 4;
                    let head = String::from_utf8_lossy(&buf[..hdr_end]);
                    let clen = head
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            if k.eq_ignore_ascii_case("content-length") {
                                v.trim().parse::<usize>().ok()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0);
                    if buf.len() >= body_start + clen {
                        break; // full body in hand
                    }
                }
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(k) => buf.extend_from_slice(&tmp[..k]),
                    Err(_) => break,
                }
            }
            let req = String::from_utf8_lossy(&buf);
            let q = extract_query(&req);
            q_thread.lock().unwrap().push(q.clone());
            let body = sparq_engine::query_json(&remote, &q)
                .unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#));
            let resp = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: application/sparql-results+json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    rx.recv().unwrap();
    (format!("http://{addr}/"), Recorder { queries })
}

/// Pull the URL-encoded `query=` form value out of a raw HTTP request body.
fn extract_query(req: &str) -> String {
    let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
    for pair in body.split('&') {
        if let Some(v) = pair.strip_prefix("query=") {
            return url_decode(v);
        }
    }
    String::new()
}

/// Minimal application/x-www-form-urlencoded decode (enough for SPARQL: `+`→space,
/// `%XX`→byte).
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn local_graph() -> Graph {
    Graph::load_str(
        "@prefix ex: <http://ex/> .\n\
         ex:alice a ex:Person .\n\
         ex:bob   a ex:Person .\n\
         ex:carol a ex:Person .\n",
        "turtle",
    )
    .unwrap()
}

/// The remote endpoint knows names for alice and bob (not carol), plus a lot of
/// unrelated noise the bound-join should NOT pull back.
fn remote_graph() -> Graph {
    let mut ttl = String::from("@prefix ex: <http://ex/> .\n");
    ttl.push_str("ex:alice ex:name \"Alice\" .\n");
    ttl.push_str("ex:bob ex:name \"Bob\" .\n");
    // Noise: 200 other subjects with names — these must never come back when the
    // bound-join restricts to {alice, bob, carol}.
    for i in 0..200 {
        ttl.push_str(&format!("ex:noise{i} ex:name \"N{i}\" .\n"));
    }
    Graph::load_str(&ttl, "turtle").unwrap()
}

fn names(res: &QueryResult) -> Vec<String> {
    let i = res.vars.iter().position(|v| v.as_str() == "name");
    let mut v: Vec<String> = res
        .rows
        .iter()
        .filter_map(|r| i.and_then(|i| r[i].as_ref()).map(|t| t.to_string()))
        .collect();
    v.sort();
    v
}

#[test]
fn bound_join_results_match_and_restrict_remote() {
    // One request expected (3 distinct subjects < default block 50 => a single block).
    let (url, rec) = serve_graph(remote_graph(), 1);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res =
        with_service_egress_allow(["127.0.0.1".to_string()], || query(&g, &q)).expect("federated");

    // Exactly alice + bob matched (carol has no remote name); NONE of the 200 noise
    // names leaked through — the remote was restricted to the pushed subjects.
    assert_eq!(
        names(&res),
        vec!["\"Alice\"".to_string(), "\"Bob\"".to_string()]
    );

    // The remote was asked ONE question, and it carried a VALUES pushdown naming the
    // local subjects — proof the bindings were pushed, not the bare pattern.
    let qs = rec.queries.lock().unwrap();
    assert_eq!(
        qs.len(),
        1,
        "a single bound-join block => one remote request"
    );
    let sent = &qs[0];
    assert!(
        sent.contains("VALUES"),
        "the pushed query must inject VALUES: {sent}"
    );
    assert!(sent.contains("http://ex/alice"), "must push alice: {sent}");
    assert!(
        sent.contains("http://ex/carol"),
        "must push carol (even with no match): {sent}"
    );
    assert!(
        !sent.contains("noise"),
        "must not mention remote-only noise: {sent}"
    );
}

#[test]
fn bound_join_blocks_split_across_requests() {
    // 3 distinct subjects, block size 2 => two requests (one of 2, one of 1). The
    // result set must STILL be identical to the single-block case.
    let (url, rec) = serve_graph(remote_graph(), 2);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res = with_service_egress_allow(["127.0.0.1".to_string()], || {
        with_service_bound_join_block_size(2, || query(&g, &q))
    })
    .expect("federated");

    assert_eq!(
        names(&res),
        vec!["\"Alice\"".to_string(), "\"Bob\"".to_string()]
    );
    let qs = rec.queries.lock().unwrap();
    assert_eq!(
        qs.len(),
        2,
        "3 tuples at block size 2 => two remote requests"
    );
    assert!(
        qs.iter().all(|s| s.contains("VALUES")),
        "both blocks push VALUES"
    );
}

#[test]
fn bound_join_optional_left_join_keeps_unmatched_left_rows() {
    // OPTIONAL { SERVICE } — carol has no remote name and must survive UNBOUND.
    let (url, _rec) = serve_graph(remote_graph(), 1);
    let g = local_graph();
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . OPTIONAL {{ SERVICE <{url}> {{ ?s ex:name ?name }} }} }}"
    );
    let res =
        with_service_egress_allow(["127.0.0.1".to_string()], || query(&g, &q)).expect("federated");
    // All three local persons survive; only alice/bob have a name, carol's is unbound.
    assert_eq!(res.rows.len(), 3, "left-join keeps every local Person");
    assert_eq!(
        names(&res),
        vec!["\"Alice\"".to_string(), "\"Bob\"".to_string()]
    );
}

#[test]
fn bound_join_multiple_join_vars() {
    // Two shared join variables (?s and ?city) pushed as a paired VALUES tuple.
    let local = Graph::load_str(
        "@prefix ex: <http://ex/> .\n\
         ex:alice ex:livesIn ex:paris .\n\
         ex:bob   ex:livesIn ex:rome .\n",
        "turtle",
    )
    .unwrap();
    let remote = Graph::load_str(
        "@prefix ex: <http://ex/> .\n\
         ex:alice ex:cityName ex:paris .\n\
         ex:bob   ex:cityName ex:milan .\n",
        "turtle",
    )
    .unwrap();
    let (url, rec) = serve_graph(remote, 1);
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?c WHERE {{ ?s ex:livesIn ?c . \
         SERVICE <{url}> {{ ?s ex:cityName ?c }} }}"
    );
    let res = with_service_egress_allow(["127.0.0.1".to_string()], || query(&local, &q))
        .expect("federated");
    // Only alice's (s,c)=(alice,paris) pair matches remotely; bob's (bob,rome) does not.
    assert_eq!(
        res.rows.len(),
        1,
        "only the matching (s,c) pair survives the paired bound-join"
    );
    let qs = rec.queries.lock().unwrap();
    assert!(
        qs[0].contains("VALUES (?s ?c)") || qs[0].contains("VALUES ( ?s ?c )"),
        "paired VALUES head: {}",
        qs[0]
    );
}

#[test]
fn bound_join_empty_left_side_yields_empty() {
    // No local Person at all => the join is empty; the remote should not even be asked
    // a bound query (the verbatim fallback is used, but with an empty left the whole
    // join is empty regardless). Serve 1 just in case; assert the result is empty.
    let empty_local =
        Graph::load_str("@prefix ex: <http://ex/> . ex:x ex:y ex:z .", "turtle").unwrap();
    let (url, _rec) = serve_graph(remote_graph(), 1);
    let q = format!(
        "PREFIX ex: <http://ex/>\n\
         SELECT ?s ?name WHERE {{ ?s a ex:Person . SERVICE <{url}> {{ ?s ex:name ?name }} }}"
    );
    let res = with_service_egress_allow(["127.0.0.1".to_string()], || query(&empty_local, &q))
        .expect("federated");
    assert_eq!(res.rows.len(), 0, "no local Person => empty join");
}
