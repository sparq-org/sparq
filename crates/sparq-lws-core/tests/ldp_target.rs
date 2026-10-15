// AUTHORED-BY Claude Opus 4.8
//! LDP target / request-URL parsing tests (pure value logic).

use sparq_lws_core::ldp::target::parse_target;
use sparq_lws_core::ServerError;

const BASE: &str = "https://pod.example";

#[test]
fn parses_a_simple_resource_path() {
    let t = parse_target(BASE, "/alice/data").unwrap();
    assert_eq!(t.iri, "https://pod.example/alice/data");
    assert_eq!(t.htu, "https://pod.example/alice/data");
    assert!(!t.is_container);
}

#[test]
fn the_root_is_a_valid_target_and_is_a_container() {
    let t = parse_target(BASE, "/").unwrap();
    assert_eq!(t.iri, "https://pod.example/");
    // "/" is the storage ROOT container (an `ldp:BasicContainer`), so it IS a container — a `GET /`
    // must render its `ldp:contains` listing.
    assert!(t.is_container);
}

#[test]
fn a_trailing_slash_marks_a_container() {
    let t = parse_target(BASE, "/alice/").unwrap();
    assert_eq!(t.iri, "https://pod.example/alice/");
    assert!(t.is_container);
}

#[test]
fn query_and_fragment_are_stripped_for_htu_and_iri() {
    let t = parse_target(BASE, "/alice/data?foo=bar#frag").unwrap();
    assert_eq!(t.iri, "https://pod.example/alice/data");
    assert_eq!(t.htu, "https://pod.example/alice/data");
}

#[test]
fn a_trailing_slash_on_base_is_normalised_away() {
    let t = parse_target("https://pod.example/", "/alice").unwrap();
    assert_eq!(t.iri, "https://pod.example/alice");
}

#[test]
fn dotdot_traversal_is_rejected() {
    let err = parse_target(BASE, "/alice/../etc/passwd").unwrap_err();
    assert_eq!(err.status().as_u16(), 400);
}

#[test]
fn single_dot_segment_is_rejected() {
    let err = parse_target(BASE, "/alice/./data").unwrap_err();
    assert_eq!(err.status().as_u16(), 400);
}

#[test]
fn interior_double_slash_is_rejected() {
    let err = parse_target(BASE, "/alice//data").unwrap_err();
    assert_eq!(err.status().as_u16(), 400);
}

#[test]
fn a_non_absolute_path_is_rejected() {
    let err = parse_target(BASE, "alice/data").unwrap_err();
    assert_eq!(err.status().as_u16(), 400);
}

#[test]
fn an_empty_base_is_rejected() {
    let err = parse_target("", "/alice").unwrap_err();
    assert_eq!(err.status().as_u16(), 400);
}

#[test]
fn a_deep_path_parses() {
    let t = parse_target(BASE, "/a/b/c/d.ttl").unwrap();
    assert_eq!(t.iri, "https://pod.example/a/b/c/d.ttl");
    assert!(!t.is_container);
}

#[test]
fn a_linkset_record_name_is_not_a_target() {
    for path in [
        "/alice/doc.meta",
        "/alice/doc.META",
        "/alice/c/.meta",
        "/alice/doc%2Emeta",
        "/alice/doc%2emeta?x=1",
        "/alice/doc.meta/",
        "/alice/x.meta/y",
        "/alice/X.Meta/y/",
    ] {
        assert!(
            matches!(parse_target(BASE, path), Err(ServerError::NotFound)),
            "{path}"
        );
    }
    for path in ["/alice/doc.metadata", "/alice/meta", "/alice/x.metas/y"] {
        assert!(parse_target(BASE, path).is_ok(), "{path}");
    }
}
