//! sq-3dyje.6 — mutation-kill: DIRECT behavioral assertions targeting the survivors of the
//! honest **feature-on** cargo-mutants baseline.
//!
//! ## Why this file exists (the honest story)
//!
//! The committed `bench/mutants-baseline.json` entry for this crate (0 caught / 553
//! survived) was measured with the crate's features OFF: the whole surface is
//! `#[cfg(feature = "fedclient")]`-gated, so cargo-mutants generated mutants from the
//! source but built + tested an EMPTY crate — every mutant trivially "survived" without
//! any test ever compiling. The nightly lane now passes `--features
//! fedclient,fedclient-adaptive` (the same per-crate quirk as sparq-canon/sparq-prov), and
//! a real feature-on baseline shows the existing inline + integration suites already kill
//! the large majority of the set. This file adds exact-value assertions for the classes
//! that GENUINELY survive feature-on:
//!
//!  * **error `Display` strings** — every variant of [`FedError`], [`WireError`],
//!    [`ResolveError`] pinned byte-for-byte (a `fmt` replaced by `Ok(Default::default())`
//!    prints nothing);
//!  * **SPARQL literal escaping** — the `escape_literal` twins in `planner` and `pushdown`,
//!    pinned through the public [`lower_leaf`] / [`push_group`] renderings (each deleted
//!    escape arm changes the exact rendered sub-query);
//!  * **SRJ base-direction parsing** — the `"ltr"` arm of the RDF-1.2 `its:dir` token
//!    (the inline suite pinned only `"rtl"`);
//!  * **SSRF address classification** — a boundary-value table for BOTH `is_forbidden_ip`
//!    copies (`source` + `discovery`), so every `||`→`&&` / range-edge mutation flips at
//!    least one row;
//!  * **[`EgressGuard`] decisions** — allow/deny outcomes with exact reasons, port-scoped
//!    entries, IP-literal + userinfo + scheme-default-port endpoint parsing;
//!  * **native transport observables** over a raw in-process loopback TCP server (no
//!    external network, no DNS): the configured [`HttpTransport::with_timeout`] /
//!    `HttpFetcher::with_timeout` actually bounds a stalled request, the exact response
//!    body round-trips (killing the body-cap arithmetic mutants with a >3 MiB body), and a
//!    non-2xx status maps to the exact error string.
//!
//! Non-vacuity discipline: every assertion pins a SPECIFIC value/variant. Where a mutant is
//! genuinely equivalent (e.g. `EgressGuard::deny_private` vs `Default::default()` — both
//! construct the empty allowlist) it is documented in the PR/baseline note instead of a
//! pretend-test.
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! [FABLE-5] sq-3dyje.6 — SPARQ agent.

#![cfg(feature = "fedclient")]

use oxrdf::{Literal, Term as RdfTerm};
use sparq_fedclient::{
    bind_block_size, is_forbidden_ip, lower_leaf, parse_srj, push_group, BindJoin, Capability,
    EgressGuard, ExclusiveGroup, FedError, Filter, FilterClass, InterpError, ResolveError,
    SourceResolver, WireError,
};
use sparq_fedplan::{Bgp, Term, TriplePattern, Var};
use std::net::IpAddr;

fn v(name: &str) -> Term {
    Term::Var(Var(name.to_string()))
}
fn iri(s: &str) -> Term {
    Term::Iri(s.to_string())
}

// ─── Error Display strings: every variant, byte-for-byte ────────────────────────────────

#[test]
fn fed_error_display_pins_every_variant() {
    // A `fmt` body replaced by `Ok(Default::default())` prints the empty string; each
    // variant's exact text is the observable contract callers log/match on.
    assert_eq!(
        FedError::BadEndpoint("no host".into()).to_string(),
        "federated source: bad endpoint: no host"
    );
    assert_eq!(
        FedError::EgressRefused("private".into()).to_string(),
        "federated source: egress refused: private"
    );
    assert_eq!(
        FedError::Transport("boom".into()).to_string(),
        "federated source: transport error: boom"
    );
    assert_eq!(
        FedError::Unsupported("nope".into()).to_string(),
        "federated source: unsupported: nope"
    );
}

#[test]
fn wire_error_display_pins_every_variant() {
    assert_eq!(
        WireError::Truncated.to_string(),
        "binary brTPF block: truncated / length past end"
    );
    assert_eq!(
        WireError::BadMagic.to_string(),
        "binary brTPF block: bad magic (not a binding block)"
    );
    assert_eq!(
        WireError::UnsupportedVersion(9).to_string(),
        "binary brTPF block: unsupported format version 9"
    );
    assert_eq!(
        WireError::EmptyMapping.to_string(),
        "binary brTPF block: a mapping constrained no position (μ₀)"
    );
    assert_eq!(
        WireError::BadTermKind(7).to_string(),
        "binary brTPF block: bad term-kind tag 7"
    );
    assert_eq!(
        WireError::NonUtf8.to_string(),
        "binary brTPF block: term payload is not UTF-8"
    );
    assert_eq!(
        WireError::BadVarint.to_string(),
        "binary brTPF block: malformed length varint"
    );
}

#[test]
fn resolve_error_display_pins_both_variants() {
    assert_eq!(
        ResolveError::PatternOutOfRange {
            index: 5,
            patterns: 2
        }
        .to_string(),
        "planner bridge: pattern index 5 out of range (BGP has 2 patterns)"
    );
    assert_eq!(
        ResolveError::SourceOutOfRange {
            index: 3,
            sources: 1
        }
        .to_string(),
        "planner bridge: source index 3 out of range (1 sources)"
    );
}

#[test]
fn interp_error_display_pins_every_variant() {
    // A `fmt` replaced by `Ok(Default::default())` prints nothing; pin each variant's text.
    assert_eq!(
        InterpError::Resolve(ResolveError::PatternOutOfRange {
            index: 2,
            patterns: 1
        })
        .to_string(),
        "interpreter: planner bridge: pattern index 2 out of range (BGP has 1 patterns)"
    );
    assert_eq!(
        InterpError::Source(FedError::Transport("down".into())).to_string(),
        "interpreter: source error: federated source: transport error: down"
    );
    assert_eq!(
        InterpError::BadSrj("not json".into()).to_string(),
        "interpreter: malformed SRJ: not json"
    );
    assert_eq!(
        InterpError::MultiSource {
            pattern: 3,
            sources: 2
        }
        .to_string(),
        "interpreter: pattern 3 has 2 retained sources; the Phase-3 single-source \
         interpreter answers one source per leaf (multi-source UNION is Phase 5)"
    );
}

// ─── SPARQL literal escaping (planner::escape_literal via the public lower_leaf) ─────────

#[test]
fn lower_leaf_escapes_every_bare_literal_control_char_exactly() {
    // Object literal exercising EVERY escape arm: backslash, quote, LF, CR, TAB, plus a
    // plain char. Deleting any single match arm in `escape_literal` changes the exact
    // rendered SPARQL below (e.g. a raw CR instead of `\r`).
    let tp = TriplePattern {
        subject: v("s"),
        predicate: iri("http://ex/p"),
        object: Term::Literal("a\\b\"c\nd\re\tf".to_string()),
    };
    let sub = lower_leaf(&tp).unwrap();
    assert_eq!(
        sub.sparql,
        r#"SELECT ?s WHERE { ?s <http://ex/p> "a\\b\"c\nd\re\tf" }"#
    );
    assert_eq!(sub.project, vec!["s".to_string()]);
}

#[test]
fn lower_leaf_emits_prerendered_literal_verbatim() {
    // A literal that already carries SPARQL syntax (leading `"`) is emitted verbatim —
    // the OTHER branch of render_term's literal arm.
    let tp = TriplePattern {
        subject: v("s"),
        predicate: iri("http://ex/p"),
        object: Term::Literal(r#""30"^^<http://www.w3.org/2001/XMLSchema#integer>"#.to_string()),
    };
    assert_eq!(
        lower_leaf(&tp).unwrap().sparql,
        r#"SELECT ?s WHERE { ?s <http://ex/p> "30"^^<http://www.w3.org/2001/XMLSchema#integer> }"#
    );
}

// ─── pushdown::escape_literal / render_term via the public push_group ────────────────────

#[test]
fn push_group_renders_bare_literal_escapes_filter_order_limit_exactly() {
    // One endpoint-capability group over a single pattern with a bare (unquoted) literal
    // object carrying every escapable char; one pushable FILTER; ORDER BY + LIMIT pushed.
    // The WHOLE rendered sub-query is pinned, so a deleted escape arm, a dropped FILTER
    // append, a broken ORDER/LIMIT branch, or a mutated projection all fail here.
    let bgp = Bgp {
        patterns: vec![TriplePattern {
            subject: v("s"),
            predicate: iri("http://ex/p"),
            object: Term::Literal("x\\y\"z\nq\rr\ts".to_string()),
        }],
    };
    let group = ExclusiveGroup {
        source: 0,
        patterns: vec![0],
    };
    let filters = [Filter::new(
        vec!["s".to_string()],
        "?s != <http://ex/no>",
        FilterClass::Full,
    )];
    let pushed = push_group(
        &group,
        &bgp,
        &Capability::endpoint(),
        &["s".to_string()],
        &filters,
        &["?s".to_string()],
        Some(5),
    )
    .expect("a well-formed group pushes");
    assert_eq!(
        pushed.sub.sparql,
        r#"SELECT ?s WHERE { ?s <http://ex/p> "x\\y\"z\nq\rr\ts" FILTER(?s != <http://ex/no>) } ORDER BY ?s LIMIT 5"#
    );
    assert_eq!(pushed.sub.project, vec!["s".to_string()]);
    assert_eq!(pushed.pushed_filters, vec![0]);
    assert_eq!(pushed.local_filters, Vec::<usize>::new());
}

#[test]
fn push_group_keeps_uncovered_filter_local_with_exact_indices() {
    // A Full-class filter against an Equality-class capability is kept LOCAL; a pushed
    // equality filter is not. The exact index partition is the observable planner decision.
    let bgp = Bgp {
        patterns: vec![TriplePattern {
            subject: v("s"),
            predicate: iri("http://ex/p"),
            object: v("o"),
        }],
    };
    let group = ExclusiveGroup {
        source: 0,
        patterns: vec![0],
    };
    let mut cap = Capability::endpoint();
    cap.pushable_filters = FilterClass::Equality;
    let filters = [
        Filter::new(vec!["o".to_string()], "REGEX(?o, \"x\")", FilterClass::Full),
        Filter::new(
            vec!["o".to_string()],
            "?o = <http://ex/a>",
            FilterClass::Equality,
        ),
        // References a variable the group does NOT bind — the exact common-variable check
        // must keep it local even though the class is covered.
        Filter::new(
            vec!["other".to_string()],
            "?other = <http://ex/b>",
            FilterClass::Equality,
        ),
    ];
    let pushed = push_group(&group, &bgp, &cap, &["s".to_string()], &filters, &[], None)
        .expect("a well-formed group pushes");
    assert_eq!(pushed.pushed_filters, vec![1]);
    assert_eq!(pushed.local_filters, vec![0, 2]);
    assert_eq!(
        pushed.sub.sparql,
        "SELECT ?s WHERE { ?s <http://ex/p> ?o FILTER(?o = <http://ex/a>) }"
    );
}

#[test]
fn push_group_projection_clause_by_output_var_membership() {
    // The proj_clause decision has three arms driven by `project.is_empty() && output_vars.is_empty()`:
    //   (a) both empty            → SELECT *            (caller wants whatever the group projects)
    //   (b) output_vars NON-empty but names NO group var → project the GROUP's own vars (not *)
    //   (c) project non-empty     → SELECT the intersection
    // Arm (b) is the discriminator for the `&&`→`||` mutation at the proj_clause site: with `||`
    // it collapses to `SELECT *`, over-returning columns the caller did not ask for.
    let bgp = Bgp {
        patterns: vec![TriplePattern {
            subject: v("s"),
            predicate: iri("http://ex/p"),
            object: v("o"),
        }],
    };
    let group = ExclusiveGroup {
        source: 0,
        patterns: vec![0],
    };
    let cap = Capability::endpoint();
    // (a) empty output_vars ⇒ SELECT *.
    let a = push_group(&group, &bgp, &cap, &[], &[], &[], None).expect("pushes");
    assert_eq!(a.sub.sparql, "SELECT * WHERE { ?s <http://ex/p> ?o }");
    assert_eq!(a.sub.project, Vec::<String>::new());
    // (b) output_vars names ONLY a var the group does NOT produce ⇒ project the group's own
    // vars (?s ?o), NOT `*`. Under `&&`→`||` this wrongly becomes `SELECT *`.
    let b = push_group(&group, &bgp, &cap, &["absent".to_string()], &[], &[], None).expect("pushes");
    assert_eq!(
        b.sub.sparql, "SELECT ?s ?o WHERE { ?s <http://ex/p> ?o }",
        "output_vars naming no group var ⇒ project the group's vars, never SELECT *"
    );
    assert_eq!(b.sub.project, Vec::<String>::new(), "no group var is in output_vars");
    // (c) output_vars names a produced var ⇒ project exactly it.
    let c = push_group(&group, &bgp, &cap, &["o".to_string()], &[], &[], None).expect("pushes");
    assert_eq!(c.sub.sparql, "SELECT ?o WHERE { ?s <http://ex/p> ?o }");
    assert_eq!(c.sub.project, vec!["o".to_string()]);
}

#[test]
fn push_group_without_order_limit_capability_pushes_neither() {
    // A source whose capability does NOT advertise order_limit must get NO ORDER BY / LIMIT
    // even when the caller supplies keys + a limit (the `!fragment && cap.order_limit`
    // guard; its `&&`→`||` mutation pushes them for every non-fragment source).
    let bgp = Bgp {
        patterns: vec![TriplePattern {
            subject: v("s"),
            predicate: iri("http://ex/p"),
            object: v("o"),
        }],
    };
    let group = ExclusiveGroup {
        source: 0,
        patterns: vec![0],
    };
    let mut cap = Capability::endpoint();
    cap.order_limit = false;
    let pushed = push_group(
        &group,
        &bgp,
        &cap,
        &["s".to_string()],
        &[],
        &["?s".to_string()],
        Some(9),
    )
    .expect("a well-formed group pushes");
    assert_eq!(
        pushed.sub.sparql, "SELECT ?s WHERE { ?s <http://ex/p> ?o }",
        "no ORDER BY / LIMIT may be pushed to a source that cannot evaluate them"
    );
}

// ─── SRJ its:dir "ltr" arm (the inline suite pinned only "rtl") ───────────────────────────

#[test]
fn srj_directional_literal_ltr_decodes_exactly() {
    let rel = parse_srj(
        r#"{"head":{"vars":["d"]},"results":{"bindings":[{
            "d":{"type":"literal","value":"hello","xml:lang":"en","its:dir":"ltr"}
        }]}}"#,
    )
    .unwrap();
    assert_eq!(
        rel.rows[0][0],
        Some(RdfTerm::Literal(
            Literal::new_directional_language_tagged_literal(
                "hello",
                "en",
                oxrdf::BaseDirection::Ltr
            )
            .unwrap()
        )),
        "the its:dir \"ltr\" token must produce the LTR directional literal"
    );
}

// ─── SSRF classification boundary table (BOTH copies: source + discovery) ────────────────

/// Boundary-value rows chosen so each `||`→`&&`, `==`→`!=`, `&`→`|`/`^` and deleted-range
/// mutation in `is_forbidden_ip` flips at least one row: each forbidden range has an
/// in-range representative AND an adjacent allowed neighbour.
fn forbidden_ip_table() -> Vec<(&'static str, bool)> {
    vec![
        // loopback / unspecified / broadcast
        ("127.0.0.1", true),
        ("0.0.0.0", true),
        ("255.255.255.255", true),
        // RFC1918
        ("10.0.0.1", true),
        ("172.16.0.1", true),
        ("172.31.255.254", true),
        ("172.32.0.1", false), // just past 172.16/12
        ("192.168.1.1", true),
        ("192.169.0.1", false), // just past 192.168/16
        // link-local incl. the cloud-metadata IP
        ("169.254.169.254", true),
        ("169.253.0.1", false),
        // CGNAT 100.64/10 boundaries
        ("100.64.0.1", true),
        ("100.127.255.254", true),
        ("100.63.255.254", false),
        ("100.128.0.1", false),
        // plain public
        ("93.184.216.34", false),
        // v6: loopback / unspecified
        ("::1", true),
        ("::", true),
        // v6 link-local fe80::/10 boundaries
        ("fe80::1", true),
        ("febf::1", true),
        ("fe7f::1", false),
        ("fec0::1", false),
        // v6 unique-local fc00::/7 boundaries
        ("fc00::1", true),
        ("fdff::1", true),
        ("fbff::1", false),
        ("fe00::1", false),
        // v4-mapped v6 unwraps to the embedded v4
        ("::ffff:10.0.0.1", true),
        ("::ffff:8.8.8.8", false),
        // global v6
        ("2001:db8::1", false),
    ]
}

#[test]
fn source_is_forbidden_ip_boundary_table() {
    for (ip, want) in forbidden_ip_table() {
        let parsed: IpAddr = ip.parse().unwrap();
        assert_eq!(
            is_forbidden_ip(parsed),
            want,
            "source::is_forbidden_ip({}) must be {}",
            ip,
            want
        );
    }
}

#[test]
fn discovery_is_forbidden_ip_boundary_table() {
    // The discovery module carries its own equivalent copy (the engine's is pub(crate));
    // pin it independently so a mutation in either copy is caught.
    for (ip, want) in forbidden_ip_table() {
        let parsed: IpAddr = ip.parse().unwrap();
        assert_eq!(
            sparq_fedclient::discovery::is_forbidden_ip(parsed),
            want,
            "discovery::is_forbidden_ip({}) must be {}",
            ip,
            want
        );
    }
}

// ─── EgressGuard decisions: allowlist keys, port scoping, endpoint parsing ────────────────

#[test]
fn egress_guard_allowed_hosts_returns_exact_entries() {
    let guard = EgressGuard::deny_private()
        .allow_host("Sparql.Internal")
        .allow_host("127.0.0.1:8053");
    let hosts = guard.allowed_hosts();
    assert_eq!(hosts.len(), 2);
    assert!(hosts.contains("sparql.internal"), "entries are lowercased");
    assert!(hosts.contains("127.0.0.1:8053"));
}

#[test]
fn egress_guard_is_allowed_and_port_scoping() {
    let guard = EgressGuard::deny_private()
        .allow_host("open.example")
        .allow_host("scoped.example:8443");
    // Host-level entry: any port; case-insensitive host.
    assert!(guard.is_allowed("OPEN.example"));
    assert!(guard.is_allowed_port("open.example", 80));
    assert!(guard.is_allowed_port("open.example", 65535));
    // Port-scoped entry: exactly that port, no other.
    assert!(guard.is_allowed("scoped.example"), "on the list at SOME port");
    assert!(guard.is_allowed_port("scoped.example", 8443));
    assert!(!guard.is_allowed_port("scoped.example", 8444));
    // Absent host: nothing.
    assert!(!guard.is_allowed("absent.example"));
    assert!(!guard.is_allowed_port("absent.example", 8443));
}

#[test]
fn egress_guard_check_addr_decisions_and_reason() {
    let guard = EgressGuard::deny_private().allow_host("in.example:9000");
    let private: IpAddr = "10.0.0.9".parse().unwrap();
    let public: IpAddr = "93.184.216.34".parse().unwrap();
    // Allowlisted host:port re-opens a private address on that port only.
    assert_eq!(guard.check_addr("in.example", 9000, private), Ok(()));
    // Same host on ANOTHER port: refused, with the exact diagnostic ingredients.
    let err = guard
        .check_addr("in.example", 9001, private)
        .expect_err("port-scoped entry must not re-open other ports");
    assert!(
        err.contains("10.0.0.9") && err.contains("9001") && err.contains("default-deny"),
        "refusal must name the address, port and policy: {}",
        err
    );
    // A public address needs no allowlist.
    assert_eq!(guard.check_addr("other.example", 80, public), Ok(()));
}

#[test]
fn check_endpoint_ip_literal_paths() {
    // Default-deny: a loopback IP-literal endpoint is refused as EgressRefused (never
    // BadEndpoint — the IRI is fine, the address is not).
    let deny = EgressGuard::deny_private();
    match deny.check_endpoint("http://127.0.0.1:9999/sparql") {
        Err(FedError::EgressRefused(m)) => {
            assert!(m.contains("127.0.0.1"), "refusal names the address: {}", m)
        }
        other => panic!("expected EgressRefused, got {:?}", other),
    }
    // Allowlisted bare host: Ok with the EXACT bare lowercased host returned.
    let allow = EgressGuard::deny_private().allow_host("127.0.0.1");
    assert_eq!(
        allow.check_endpoint("http://127.0.0.1:9999/sparql"),
        Ok("127.0.0.1".to_string())
    );
    // No scheme/authority at all: BadEndpoint.
    match deny.check_endpoint("not-a-url") {
        Err(FedError::BadEndpoint(_)) => {}
        other => panic!("expected BadEndpoint, got {:?}", other),
    }
}

#[test]
fn check_endpoint_scheme_default_ports_and_userinfo() {
    // A port-scoped entry for 443 admits an https endpoint with NO explicit port — proving
    // the scheme default is 443 (an `https`→80 default mutation fails here)…
    let g443 = EgressGuard::deny_private().allow_host("127.0.0.1:443");
    assert_eq!(
        g443.check_endpoint("https://127.0.0.1/q"),
        Ok("127.0.0.1".to_string())
    );
    // …and must NOT admit the http form (default 80 ≠ 443).
    match g443.check_endpoint("http://127.0.0.1/q") {
        Err(FedError::EgressRefused(_)) => {}
        other => panic!("expected EgressRefused for port 80, got {:?}", other),
    }
    // The http scheme defaults to 80.
    let g80 = EgressGuard::deny_private().allow_host("127.0.0.1:80");
    assert_eq!(
        g80.check_endpoint("http://127.0.0.1/q"),
        Ok("127.0.0.1".to_string())
    );
    // Userinfo is stripped before the host is keyed.
    let g = EgressGuard::deny_private().allow_host("127.0.0.1");
    assert_eq!(
        g.check_endpoint("http://user:secret@127.0.0.1:7000/q"),
        Ok("127.0.0.1".to_string())
    );
    // Bracketed IPv6 literal: brackets stripped, port honoured.
    let g6 = EgressGuard::deny_private().allow_host("::1");
    assert_eq!(
        g6.check_endpoint("http://[::1]:8080/q"),
        Ok("::1".to_string())
    );
}

// ─── bind_block_size: the whole decision table ────────────────────────────────────────────

#[test]
fn bind_block_size_full_decision_table() {
    let mut cap = Capability::endpoint();
    cap.bind_join = BindJoin::Values;
    assert_eq!(bind_block_size(&cap), 50, "Values → DEFAULT_BIND_BLOCK");
    cap.bind_join = BindJoin::MaxMpR(7);
    assert_eq!(bind_block_size(&cap), 7, "MaxMpR(n) → n");
    cap.bind_join = BindJoin::MaxMpR(0);
    assert_eq!(bind_block_size(&cap), 1, "MaxMpR(0) clamps to 1");
    cap.bind_join = BindJoin::None;
    assert_eq!(bind_block_size(&cap), 0, "None disables the bind-join");
}

// ─── SourceResolver range checks with exact error payloads ────────────────────────────────

#[test]
fn source_resolver_range_checks_carry_exact_payloads() {
    let bgp = Bgp {
        patterns: vec![TriplePattern {
            subject: v("s"),
            predicate: iri("http://ex/p"),
            object: v("o"),
        }],
    };
    let adapters: Vec<&dyn sparq_fedclient::FederatedSource> = Vec::new();
    let resolver = SourceResolver::new(&bgp, &adapters);
    assert_eq!(resolver.source_count(), 0);
    assert_eq!(
        resolver.pattern(1).err(),
        Some(ResolveError::PatternOutOfRange {
            index: 1,
            patterns: 1
        })
    );
    assert!(resolver.pattern(0).is_ok());
    assert_eq!(
        resolver.source(0).err().map(|e| e.to_string()),
        Some("planner bridge: source index 0 out of range (0 sources)".to_string())
    );
}

// ─── Service-Description supportedLanguage arm precedence ────────────────────────────────

#[test]
fn sd_update_only_language_sets_update_flag_not_a_version() {
    // An SD advertising ONLY SPARQL11Update: the update flag must be set and NO query
    // version must be inferred. The `SPARQL10Query && version != Sparql11` match guard
    // mutated to `||` makes the Sparql10 arm swallow ANY language IRI (including this
    // Update one, which precedes the Update arm), flipping BOTH assertions.
    let nt = concat!(
        "<http://host/sparql> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n",
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#SPARQL11Update> .\n",
    );
    let cap = sparq_fedclient::discovery::parse_service_description(nt)
        .expect("well-formed SD parses")
        .expect("document has an sd:Service");
    assert!(cap.update, "SPARQL11Update must set the update flag");
    assert_eq!(
        cap.sparql_version, None,
        "an update-only SD advertises no QUERY language version"
    );
}

#[test]
fn sd_non_update_language_does_not_set_update_flag() {
    // The update flag is set ONLY by the EXACT SPARQL11Update IRI. cargo-mutants showed the
    // SPARQL11Update match guard mutated to `true` survived — no test asserted that a language
    // IRI reaching that arm but NOT equal to SPARQL11Update leaves update=false. The IRI must
    // NOT match the earlier SPARQL11Query / SPARQL10Query arms (those are tried first), so use
    // an UNKNOWN language IRI that falls THROUGH to the update arm — with the guard forced true
    // it would wrongly flip update; unmutated it is ignored. Pair it with a real SPARQL11Query
    // triple so the capability still parses and the version is asserted. (Read-only vs
    // read-write is a real capability distinction the planner keys on.)
    let nt = concat!(
        "<http://host/sparql> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n",
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#SPARQL11Query> .\n",
        // An UNKNOWN language IRI (not Query, not Update) — reaches the update arm and must be ignored.
        "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://example.org/some-other-language> .\n",
    );
    let cap = sparq_fedclient::discovery::parse_service_description(nt)
        .expect("well-formed SD parses")
        .expect("document has an sd:Service");
    assert!(
        !cap.update,
        "only the exact SPARQL11Update IRI sets update; an unknown language must not"
    );
    assert_eq!(
        cap.sparql_version,
        Some(sparq_fedclient::discovery::SparqlVersion::Sparql11)
    );
}

#[test]
fn sd_service_subject_needs_both_type_and_service_object() {
    // The sd:Service detection is `pred == rdf:type AND object == sd:Service`. A document
    // where each condition holds on a DIFFERENT triple (a non-Service rdf:type, and an
    // sd:Service object under a non-type predicate) has NO sd:Service subject, so the parser
    // must return Ok(None) — the caller then falls back to an ASK probe. The `&&`→`||`
    // mutation would wrongly register a service subject from EITHER half and return Some(..).
    let nt = concat!(
        // rdf:type, but to something that is NOT sd:Service.
        "<http://host/thing> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Dataset> .\n",
        // an sd:Service OBJECT, but under a non-rdf:type predicate.
        "<http://host/other> <http://www.w3.org/ns/sparql-service-description#endpoint> <http://www.w3.org/ns/sparql-service-description#Service> .\n",
    );
    assert_eq!(
        sparq_fedclient::discovery::parse_service_description(nt),
        Ok(None),
        "no triple satisfies BOTH rdf:type AND object=sd:Service ⇒ not a Service Description"
    );
}

#[test]
fn sd_sparql10_never_downgrades_an_advertised_11() {
    // Both orders: SPARQL11Query wins over SPARQL10Query regardless of triple order (the
    // `version != Sparql11` guard; `!=`→`==` or a deleted arm flips one of these).
    for (first, second) in [
        ("SPARQL11Query", "SPARQL10Query"),
        ("SPARQL10Query", "SPARQL11Query"),
    ] {
        let nt = format!(
            concat!(
                "<http://host/sparql> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/sparql-service-description#Service> .\n",
                "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#{}> .\n",
                "<http://host/sparql> <http://www.w3.org/ns/sparql-service-description#supportedLanguage> <http://www.w3.org/ns/sparql-service-description#{}> .\n",
            ),
            first, second
        );
        let cap = sparq_fedclient::discovery::parse_service_description(&nt)
            .expect("well-formed SD parses")
            .expect("document has an sd:Service");
        assert_eq!(
            cap.sparql_version,
            Some(sparq_fedclient::discovery::SparqlVersion::Sparql11),
            "1.1 must win regardless of order ({} then {})",
            first,
            second
        );
    }
}

// ─── Binary wire: a DUPLICATED position pair must not be dropped or overwrite ─────────────

#[test]
fn binary_wire_duplicate_position_pairs_ride_extra_not_overwrite() {
    use sparq_fedclient::{decode_bindings, encode_bindings, FragTerm};
    // A mapping repeating each canonical position: the FIRST pair wins the header slot,
    // every duplicate rides the EXTRA section — nothing dropped, nothing overwritten.
    // The `subject.is_none()` (etc.) match-guard mutated to `true` lets the SECOND pair
    // overwrite the first and drops it from the wire, breaking the exact round-trip.
    let block = vec![vec![
        ("s".to_string(), FragTerm::Iri("http://ex/first-s".into())),
        ("s".to_string(), FragTerm::Iri("http://ex/second-s".into())),
        ("p".to_string(), FragTerm::Iri("http://ex/first-p".into())),
        ("p".to_string(), FragTerm::Iri("http://ex/second-p".into())),
        ("o".to_string(), FragTerm::Literal("\"first-o\"".into())),
        ("o".to_string(), FragTerm::Literal("\"second-o\"".into())),
    ]];
    let back = decode_bindings(&encode_bindings(&block)).expect("decode");
    // Decode order: position slots in canonical s→p→o order (the FIRST pair of each), then
    // the EXTRA pairs (the duplicates) in encoded order.
    assert_eq!(
        back,
        vec![vec![
            ("s".to_string(), FragTerm::Iri("http://ex/first-s".into())),
            ("p".to_string(), FragTerm::Iri("http://ex/first-p".into())),
            ("o".to_string(), FragTerm::Literal("\"first-o\"".into())),
            ("s".to_string(), FragTerm::Iri("http://ex/second-s".into())),
            ("p".to_string(), FragTerm::Iri("http://ex/second-p".into())),
            ("o".to_string(), FragTerm::Literal("\"second-o\"".into())),
        ]]
    );
}

// ─── Native transport observables over a raw loopback TCP server ─────────────────────────
//
// No external network, no DNS: the server is an in-process std::net::TcpListener on
// 127.0.0.1:0; the SSRF guard/resolver admits it via an explicit allowlist entry. These
// tests kill the `with_timeout → Default::default()` builder mutants (the configured
// timeout must actually bound a stalled request — AND the builder must preserve the
// allowlist it chains after), the body-cap arithmetic mutants (`1024*1024*1024` /
// `256*1024*1024` degraded by `*`→`/`/`+` to ≤2 MiB caps fail a >3 MiB body), and the
// whole-method `fetch`/`get` → `Ok("")` replacements (exact body pinned).

mod loopback {
    use super::*;
    use sparq_fedclient::discovery::{Fetcher, HttpFetcher};
    use sparq_fedclient::{
        FragPattern, FragmentTransport, HttpFragmentTransport, HttpTransport, PatternTerm, Transport,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Serve exactly one connection: consume the WHOLE request (headers, then exactly
    /// `Content-Length` body bytes — responding before the client finishes writing would
    /// race a TCP RST into its pending read), then write `response` and close. Returns the
    /// bound address; the thread self-terminates.
    fn one_shot_server(response: Vec<u8>) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
                let mut req: Vec<u8> = Vec::new();
                let mut buf = [0u8; 65536];
                // Read until the header terminator is seen.
                let header_end = loop {
                    match sock.read(&mut buf) {
                        Ok(0) | Err(_) => break None,
                        Ok(n) => {
                            req.extend_from_slice(&buf[..n]);
                            if let Some(pos) =
                                req.windows(4).position(|w| w == b"\r\n\r\n")
                            {
                                break Some(pos + 4);
                            }
                        }
                    }
                };
                if let Some(header_end) = header_end {
                    // Honour Content-Length so the whole body is drained before replying.
                    let headers = String::from_utf8_lossy(&req[..header_end]).to_ascii_lowercase();
                    let want_body: usize = headers
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let mut have_body = req.len() - header_end;
                    while have_body < want_body {
                        match sock.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => have_body += n,
                        }
                    }
                    let _ = sock.write_all(&response);
                    let _ = sock.flush();
                }
            }
        });
        addr
    }

    /// A server that accepts and then NEVER responds (stalls), for the timeout tests.
    fn stalling_server() -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");
        std::thread::spawn(move || {
            if let Ok((sock, _)) = listener.accept() {
                // Hold the socket open, sending nothing, long past any test deadline.
                std::thread::sleep(Duration::from_secs(20));
                drop(sock);
            }
        });
        addr
    }

    fn http_ok(body: &[u8]) -> Vec<u8> {
        let mut r = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        r.extend_from_slice(body);
        r
    }

    #[test]
    fn http_transport_returns_exact_large_body_within_the_cap() {
        // >3 MiB: every degraded MAX_BODY_BYTES arithmetic (1024/1024*1024 = 1 KiB,
        // (1024+1024)*1024 = 2 MiB, 1024*1024+1024 ≈ 1 MiB) rejects or truncates it,
        // while the real 1 GiB cap passes it through byte-identically.
        let body = "x".repeat(3 * 1024 * 1024 + 17);
        let addr = one_shot_server(http_ok(body.as_bytes()));
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1");
        let transport = HttpTransport::from_guard(&guard);
        let url = format!("http://127.0.0.1:{}/sparql", addr.port());
        let got = transport
            .fetch(&url, "SELECT * WHERE { ?s ?p ?o }")
            .expect("a 200 with a 3 MiB body is within the 1 GiB cap");
        assert_eq!(got.len(), body.len(), "body length must round-trip exactly");
        assert_eq!(got, body, "body must round-trip byte-identically");
    }

    #[test]
    fn http_transport_maps_non_2xx_to_exact_error_string() {
        let addr = one_shot_server(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec(),
        );
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1");
        let transport = HttpTransport::from_guard(&guard);
        let url = format!("http://127.0.0.1:{}/sparql", addr.port());
        assert_eq!(
            transport.fetch(&url, "SELECT * WHERE { ?s ?p ?o }"),
            Err(format!("fedclient: endpoint {} returned HTTP 500", url))
        );
    }

    #[test]
    fn http_transport_with_timeout_bounds_a_stalled_request() {
        // The configured 200 ms timeout must fail a stalled request promptly, with ureq's
        // timeout error (NOT an egress refusal — the mutant that replaces `with_timeout`
        // with `Default::default()` ALSO drops the allowlist, turning this into an
        // immediate "private/internal" refusal; and the default 30 s timeout would blow
        // the 5 s deadline below).
        let addr = stalling_server();
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1");
        let transport = HttpTransport::from_guard(&guard).with_timeout(Duration::from_millis(200));
        let url = format!("http://127.0.0.1:{}/sparql", addr.port());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(transport.fetch(&url, "SELECT * WHERE { ?s ?p ?o }"));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the 200 ms timeout must fire well inside 5 s (with_timeout not applied?)");
        let err = result.expect_err("a stalled endpoint must not succeed");
        assert!(
            err.contains("timeout"),
            "the failure must be the configured timeout, not an egress refusal: {}",
            err
        );
        assert!(
            !err.contains("private/internal"),
            "the allowlist must survive the with_timeout builder: {}",
            err
        );
    }

    #[test]
    fn http_fetcher_returns_exact_large_body_within_the_cap() {
        // Same >3 MiB discipline for the discovery fetcher's 256 MiB cap: the degraded
        // arithmetic variants (0 B, 256 B, ~257 KiB, ~1.25 MiB) all reject it.
        let body = "y".repeat(3 * 1024 * 1024 + 5);
        let addr = one_shot_server(http_ok(body.as_bytes()));
        let fetcher = HttpFetcher::new().allow_private_host("127.0.0.1");
        let url = format!("http://127.0.0.1:{}/void.nt", addr.port());
        let got = fetcher
            .get(&url, "application/n-triples")
            .expect("a 200 with a 3 MiB body is within the 256 MiB cap");
        assert_eq!(got.len(), body.len());
        assert_eq!(got, body);
    }

    #[test]
    fn http_fetcher_maps_non_2xx_to_exact_error_string() {
        let addr = one_shot_server(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec(),
        );
        let fetcher = HttpFetcher::new().allow_private_host("127.0.0.1");
        let url = format!("http://127.0.0.1:{}/void.nt", addr.port());
        assert_eq!(
            fetcher.get(&url, "application/n-triples"),
            Err(format!("discovery: {} returned HTTP 503", url))
        );
    }

    #[test]
    fn http_fetcher_with_timeout_bounds_a_stalled_request() {
        let addr = stalling_server();
        let fetcher = HttpFetcher::new()
            .allow_private_host("127.0.0.1")
            .with_timeout(Duration::from_millis(200));
        let url = format!("http://127.0.0.1:{}/void.nt", addr.port());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetcher.get(&url, "application/n-triples"));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the 200 ms timeout must fire well inside 5 s (with_timeout not applied?)");
        let err = result.expect_err("a stalled endpoint must not succeed");
        assert!(
            err.contains("timeout"),
            "the failure must be the configured timeout, not an egress refusal: {}",
            err
        );
        assert!(
            !err.contains("private/internal"),
            "the allowlist must survive the with_timeout builder: {}",
            err
        );
    }

    fn all_var_pattern() -> FragPattern {
        FragPattern::new(
            PatternTerm::Var("s".to_string()),
            PatternTerm::Var("p".to_string()),
            PatternTerm::Var("o".to_string()),
        )
    }

    #[test]
    fn fragment_transport_returns_parsed_page_from_a_turtle_fragment() {
        // A real GET → Turtle fragment body → parsed FragmentPage: one data triple, the
        // hydra:totalItems count, no next page. Drives the whole native fragment path (URL
        // build + GET + body parse) over the loopback, killing the `fetch_fragment → Ok("")`
        // class and pinning the parse output exactly.
        let body = concat!(
            "<http://frag> <http://www.w3.org/ns/hydra/core#totalItems> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/a> <http://ex/p> <http://ex/b> .\n",
        );
        let addr = one_shot_server(http_ok(body.as_bytes()));
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1");
        let transport = HttpFragmentTransport::from_guard(&guard);
        let url = format!("http://127.0.0.1:{}/fragment", addr.port());
        let page = transport
            .fetch_fragment(&url, &all_var_pattern(), &[], None)
            .expect("a 200 Turtle fragment parses");
        assert_eq!(page.total_items, 1);
        assert_eq!(page.next, None);
        assert_eq!(page.triples.len(), 1);
    }

    #[test]
    fn fragment_transport_maps_non_2xx_to_exact_error_string() {
        let addr = one_shot_server(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1");
        let transport = HttpFragmentTransport::from_guard(&guard);
        let url = format!("http://127.0.0.1:{}/fragment", addr.port());
        let err = transport
            .fetch_fragment(&url, &all_var_pattern(), &[], None)
            .expect_err("a 404 is a transport error");
        assert!(
            err.contains("returned HTTP 404"),
            "the exact non-2xx error string must be reported: {}",
            err
        );
    }

    #[test]
    fn fragment_transport_with_timeout_bounds_a_stalled_request() {
        // The fragment transport's with_timeout must bound a stalled request AND preserve the
        // allowlist chained in by from_guard — kills the `with_timeout → Default::default()`
        // survivor (which drops both the timeout and the allowlist).
        let addr = stalling_server();
        let guard = EgressGuard::deny_private().allow_host("127.0.0.1");
        let transport =
            HttpFragmentTransport::from_guard(&guard).with_timeout(Duration::from_millis(200));
        let url = format!("http://127.0.0.1:{}/fragment", addr.port());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(transport.fetch_fragment(&url, &all_var_pattern(), &[], None));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the 200 ms timeout must fire well inside 5 s (with_timeout not applied?)");
        let err = result.expect_err("a stalled fragment server must not succeed");
        assert!(
            err.contains("timeout"),
            "the failure must be the configured timeout, not an egress refusal: {}",
            err
        );
        assert!(
            !err.contains("private/internal"),
            "the allowlist must survive the with_timeout builder: {}",
            err
        );
    }
}
