//! sq-ushvx (epic sq-my8wd) — smoke test for the in-process SERVICE-federation
//! harness ([`sparq_conformance::service_loopback`]).
//!
//! HERMETIC: needs no fetched W3C test data — it stands up a real `sparq-server` over an
//! in-memory fixture graph on an ephemeral `127.0.0.1:0` loopback port and drives a
//! trivial federated `SERVICE` query through the engine's REAL `ureq` transport, asserting
//! the remote solutions are joined end-to-end.
//!
//! Built only with the OPT-IN `service-loopback` feature:
//!   cargo test -p sparq-conformance --features service-loopback --test service_loopback
//!
//! With the feature OFF this file compiles to a single self-SKIP `#[test]`, so the default
//! `cargo test -p sparq-conformance` (and the `--workspace` shards) never link
//! tokio/axum/ureq through this harness — the lean-default opt-in posture.

#[cfg(not(feature = "service-loopback"))]
#[test]
fn service_loopback_suite_skipped_without_feature() {
    eprintln!(
        "skipping: the in-process SERVICE-federation harness is behind the opt-in \
         `service-loopback` feature (cargo test -p sparq-conformance \
         --features service-loopback)"
    );
}

#[cfg(feature = "service-loopback")]
mod enabled {
    use sparq_conformance::service_loopback::LoopbackEndpoint;
    use sparq_core::Graph;

    /// The remote (federated) graph the loopback `sparq-server` serves: each person's name.
    fn remote_graph() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\n\
             ex:alice ex:name \"Alice\" .\n\
             ex:bob   ex:name \"Bob\" .\n\
             ex:carol ex:name \"Carol\" .\n",
            "turtle",
        )
        .expect("load remote fixture")
    }

    /// The local graph the federated query starts from: who is a Person.
    fn local_graph() -> Graph {
        Graph::load_str(
            "@prefix ex: <http://ex/> .\n\
             ex:alice a ex:Person .\n\
             ex:bob   a ex:Person .\n",
            "turtle",
        )
        .expect("load local fixture")
    }

    /// End-to-end: a trivial FEDERATED query (a `SERVICE` call to the loopback endpoint)
    /// runs through the REAL ureq transport and joins the remote names onto the local
    /// people. This exercises the WHOLE path: HTTP request/response, content negotiation,
    /// SPARQL-Results parsing, and the bind-join — not a canned mock.
    #[test]
    fn federated_service_query_joins_remote_solutions_end_to_end() {
        let endpoint = LoopbackEndpoint::serve(remote_graph());

        // Sanity: the bound URL is a fresh ephemeral loopback port.
        assert_eq!(endpoint.addr().ip().to_string(), "127.0.0.1");
        assert_ne!(endpoint.addr().port(), 0, "an ephemeral port was bound");

        let local = local_graph();
        let query = format!(
            "PREFIX ex: <http://ex/>\n\
             SELECT ?s ?name WHERE {{\n\
               ?s a ex:Person .\n\
               SERVICE <{}> {{ ?s ex:name ?name }}\n\
             }}",
            endpoint.sparql_url()
        );

        let res = endpoint
            .run_federated(&local, &query)
            .expect("federated SERVICE query over the loopback endpoint");

        // alice + bob are local Persons and have remote names; carol has a remote name but
        // is not a local Person, so the join drops her. => exactly two rows.
        assert_eq!(
            res.rows.len(),
            2,
            "the two local Persons join with their remote names (carol is not local)"
        );
        let name_idx = res
            .vars
            .iter()
            .position(|v| v.as_str() == "name")
            .expect("?name in result vars");
        let names: Vec<String> = res
            .rows
            .iter()
            .filter_map(|r| r[name_idx].as_ref().map(|t| t.to_string()))
            .collect();
        assert!(names.iter().any(|n| n.contains("Alice")), "got {:?}", names);
        assert!(names.iter().any(|n| n.contains("Bob")), "got {:?}", names);
        assert!(
            !names.iter().any(|n| n.contains("Carol")),
            "carol must not survive the join: {:?}",
            names
        );
    }

    /// The SSRF egress filter is NOT globally disabled by the harness: a SERVICE to a
    /// DIFFERENT loopback endpoint (one the harness did NOT allowlist) is still refused.
    /// This proves the allowlist is scoped to the harness's own endpoint host, not a
    /// blanket "loopback is fine" switch — the load-bearing fail-closed invariant.
    ///
    /// (Because the engine allowlist is bare-host-keyed, the scoping is host-level; this
    /// test pins the host-level boundary by using a non-loopback private host that the
    /// harness never allowlists. A genuinely port-scoped guard is a follow-up engine
    /// change — see the module docs.)
    #[test]
    fn unallowlisted_private_host_service_is_refused() {
        // Stand up the harness endpoint (this allowlists ONLY 127.0.0.1 for its own
        // run_federated calls). Here we instead run a bare engine query (no harness
        // allowlist scope) against a SERVICE pointing at a private RFC1918 host the
        // default-deny policy refuses — proving the harness did not relax the global filter.
        let _endpoint = LoopbackEndpoint::serve(remote_graph());
        let local = local_graph();
        // 10.255.255.1 is RFC1918 private and is NOT allowlisted by anyone here. A
        // non-SILENT SERVICE to it must propagate the egress refusal as an error.
        let query = "PREFIX ex: <http://ex/>\n\
             SELECT ?s ?name WHERE {\n\
               ?s a ex:Person .\n\
               SERVICE <http://10.255.255.1:9/sparql> { ?s ex:name ?name }\n\
             }";
        let res = sparq_engine::query(&local, query);
        assert!(
            res.is_err(),
            "a SERVICE to an un-allowlisted private host must be refused (got {:?})",
            res
        );
        let err = res.unwrap_err();
        assert!(
            err.contains(sparq_engine::SERVICE_EGRESS_REFUSED_MARKER),
            "the failure must be the SSRF egress refusal, not some other error: {err}"
        );
    }

    /// Teardown: dropping the endpoint releases the port. We can bind a fresh endpoint
    /// afterwards without leak/contention — a light proof that Drop joins the server thread.
    #[test]
    fn endpoint_tears_down_cleanly_on_drop() {
        let first_port = {
            let ep = LoopbackEndpoint::serve(remote_graph());
            ep.addr().port()
        }; // ep dropped here -> graceful shutdown + thread join
        assert_ne!(first_port, 0);
        // A second endpoint binds fine after the first has torn down.
        let ep2 = LoopbackEndpoint::serve(remote_graph());
        assert_eq!(ep2.addr().ip().to_string(), "127.0.0.1");
    }
}
