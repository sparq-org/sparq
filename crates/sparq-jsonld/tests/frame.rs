//! sq-oy1f.29 — integration tests for the native document-level Framing
//! Algorithm (`sparq_jsonld::frame`), the bead's acceptance lane
//! (`cargo test -p sparq-jsonld --test frame`). Each test targets one of the
//! divergence classes the bead names: value patterns over `@value` alternative
//! arrays, `@explicit`/`@default` fill, named-graph `@graph` framing shapes,
//! `@list`/`@set` re-emit, blank-node `@embed` handling, and the framing error
//! codes (`invalid frame`, `invalid @embed value`).

use sparq_jsonld::frame::{frame, FrameOptions};
use sparq_jsonld::{Json, JsonLdError, JsonLdErrorCode, JsonLdOptions, NoopLoader, ProcessingMode};

fn parse(s: &str) -> Json {
    Json::parse(s).expect("valid JSON fixture")
}

fn run(input: &str, frame_doc: &str) -> Json {
    run_opts(input, frame_doc, &JsonLdOptions::default(), &FrameOptions::default())
        .expect("frame ok")
}

fn run_opts(
    input: &str,
    frame_doc: &str,
    options: &JsonLdOptions,
    fopts: &FrameOptions,
) -> Result<Json, JsonLdError> {
    frame(&parse(input), &parse(frame_doc), options, fopts, &NoopLoader)
}

fn text(j: &Json) -> String {
    let mut s = String::new();
    j.write(&mut s);
    s
}

/// Type matching selects the one matching node and embeds its referenced child once
/// (`@embed: @once` default).
#[test]
fn type_match_embeds_referenced_node() {
    let out = run(
        r#"{"@context": {"ex": "http://ex/"},
            "@graph": [
              {"@id": "ex:a", "@type": "ex:T", "ex:child": {"@id": "ex:b"}},
              {"@id": "ex:b", "ex:name": "leaf"}
            ]}"#,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T"}"#,
    );
    assert_eq!(out.get("@id").and_then(Json::as_str), Some("ex:a"));
    let child = out.get("ex:child").expect("embedded child");
    assert_eq!(child.get("ex:name"), Some(&Json::Str("leaf".to_string())));
}

/// A value pattern over `@value` alternatives selects only nodes carrying a matching
/// value; the `@language` alternative compares case-insensitively.
#[test]
fn value_pattern_alternatives() {
    let input = r#"{"@context": {"ex": "http://ex/"},
        "@graph": [
          {"@id": "ex:a", "ex:p": {"@value": "hit", "@language": "EN"}},
          {"@id": "ex:b", "ex:p": {"@value": "miss", "@language": "en"}}
        ]}"#;
    let out = run(
        input,
        r#"{"@context": {"ex": "http://ex/"},
            "ex:p": {"@value": ["hit", "other"], "@language": ["en"]}}"#,
    );
    // Only ex:a matches; its value is retained (language lowercased by expansion).
    assert_eq!(out.get("@id").and_then(Json::as_str), Some("ex:a"));
    let p = out.get("ex:p").expect("value kept");
    assert_eq!(p.get("@value"), Some(&Json::Str("hit".to_string())));
    assert_eq!(p.get("@language"), Some(&Json::Str("en".to_string())));
}

/// `@explicit` prunes properties absent from the frame; `@default` fills a missing
/// property; `@omitDefault` suppresses the fill.
#[test]
fn explicit_and_default_fill() {
    let input = r#"{"@context": {"ex": "http://ex/"},
        "@id": "ex:a", "@type": "ex:T", "ex:keep": "k", "ex:drop": "d"}"#;
    let out = run(
        input,
        r#"{"@context": {"ex": "http://ex/"},
            "@type": "ex:T",
            "@explicit": true,
            "ex:keep": {},
            "ex:missing": {"@default": "filled"},
            "ex:gone": {"@omitDefault": true}}"#,
    );
    assert_eq!(out.get("ex:keep"), Some(&Json::Str("k".to_string())));
    assert!(out.get("ex:drop").is_none(), "@explicit prunes: {}", text(&out));
    assert_eq!(out.get("ex:missing"), Some(&Json::Str("filled".to_string())));
    assert!(out.get("ex:gone").is_none(), "@omitDefault suppresses the fill");
    // An unmatched frame property without @default fills as JSON null.
    let out2 = run(
        input,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T", "ex:missing": {}}"#,
    );
    assert_eq!(out2.get("ex:missing"), Some(&Json::Raw("null".to_string())));
}

/// Named-graph framing: a matched subject that names a graph embeds the graph's
/// content under `@graph`; a graph-level subject already embedded inside a sibling is
/// not re-emitted (the 1.1 embedded-flag rule).
#[test]
fn named_graph_framing() {
    let out = run(
        r#"{"@id": "ex:cred",
            "ex:proof": {"@graph": {
              "@type": "ex:Proof",
              "ex:signer": [{"@id": "ex:S", "ex:name": "inner"}]
            }}}"#,
        r#"{"ex:proof": {"@graph": {}}}"#,
    );
    let proof = out.get("ex:proof").expect("graph node kept");
    let graph = proof.get("@graph").expect("graph content embedded");
    // Exactly ONE graph node (the Proof) — the signer is embedded inside it, not
    // re-emitted as a graph-level reference.
    let node = match graph {
        Json::Arr(items) => {
            assert_eq!(items.len(), 1, "one graph node: {}", text(graph));
            &items[0]
        }
        obj => obj,
    };
    let signer = node.get("ex:signer").expect("signer embedded");
    assert_eq!(signer.get("ex:name"), Some(&Json::Str("inner".to_string())));
}

/// `@list` re-emit: list values re-emit entry-wise (duplicates retained), and node
/// entries are re-framed (embedded) inside the list.
#[test]
fn list_reemit_keeps_duplicates_and_embeds_nodes() {
    let out = run(
        r#"{"@context": {"ex": "http://ex/", "ex:l": {"@container": "@list"}},
            "@graph": [
              {"@id": "ex:a", "@type": "ex:T", "ex:l": [1, 2, 2, {"@id": "ex:b"}]},
              {"@id": "ex:b", "ex:name": "in-list"}
            ]}"#,
        r#"{"@context": {"ex": "http://ex/", "ex:l": {"@container": "@list"}}, "@type": "ex:T"}"#,
    );
    let l = out.get("ex:l").expect("list kept");
    let Json::Arr(items) = l else {
        panic!("list array: {}", text(&out))
    };
    assert_eq!(items.len(), 4, "duplicates retained: {}", text(l));
    assert_eq!(items[1], items[2], "the duplicate 2s survive");
    let node = items[3].get("ex:name");
    assert_eq!(node, Some(&Json::Str("in-list".to_string())), "node entry embedded");
}

/// Blank-node `@embed`: a single-use blank node loses its `@id`
/// (`pruneBlankNodeIdentifiers`, 1.1 default) while a shared blank node keeps it.
#[test]
fn bnode_embed_pruning() {
    let out = run(
        r#"{"@context": {"ex": "http://ex/"},
            "@id": "ex:a", "@type": "ex:T",
            "ex:single": {"ex:name": "once"},
            "ex:shared": [{"@id": "_:s", "ex:name": "twice"}],
            "ex:also": [{"@id": "_:s"}]}"#,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T"}"#,
    );
    let single = out.get("ex:single").expect("single-use bnode kept");
    assert!(single.get("@id").is_none(), "single-use bnode @id pruned: {}", text(&out));
    // The shared bnode keeps a label so the two references stay linked.
    let shared_text = text(&out);
    assert!(shared_text.contains("_:"), "shared bnode label kept: {shared_text}");
}

/// `@embed: @never` emits a node reference; `@embed: @always` re-embeds at every
/// reference.
#[test]
fn embed_never_and_always() {
    let input = r#"{"@context": {"ex": "http://ex/"},
        "@graph": [
          {"@id": "ex:a", "@type": "ex:T", "ex:p": {"@id": "ex:c"}, "ex:q": {"@id": "ex:c"}},
          {"@id": "ex:c", "ex:name": "child"}
        ]}"#;
    let never = run(
        input,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T", "@embed": "@never"}"#,
    );
    let p = never.get("ex:p").expect("reference kept");
    assert!(p.get("ex:name").is_none(), "@never emits a bare reference: {}", text(&never));

    let always = run(
        input,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T", "@embed": "@always"}"#,
    );
    for prop in ["ex:p", "ex:q"] {
        let v = always.get(prop).expect("property kept");
        assert_eq!(
            v.get("ex:name"),
            Some(&Json::Str("child".to_string())),
            "@always re-embeds under {prop}: {}",
            text(&always)
        );
    }
}

/// `@embed: @last` (JSON-LD 1.0): the LAST reference keeps the embed; earlier ones
/// demote to node references.
#[test]
fn embed_last_keeps_only_last() {
    let mut opts = JsonLdOptions::default();
    opts.processing_mode = ProcessingMode::JsonLd10;
    let out = run_opts(
        r#"{"@context": {"ex": "http://ex/"},
            "@graph": [
              {"@id": "ex:a", "@type": "ex:T", "ex:e1": {"@id": "ex:c"}, "ex:e2": {"@id": "ex:c"}},
              {"@id": "ex:c", "ex:name": "embedded"}
            ]}"#,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:T", "@embed": "@last"}"#,
        &opts,
        &FrameOptions::default(),
    )
    .expect("frame ok");
    let node = graph_single(&out);
    let e1 = node.get("ex:e1").expect("e1 kept");
    let e2 = node.get("ex:e2").expect("e2 kept");
    assert!(e1.get("ex:name").is_none(), "earlier embed demoted: {}", text(&out));
    assert_eq!(
        e2.get("ex:name"),
        Some(&Json::Str("embedded".to_string())),
        "last embed kept: {}",
        text(&out)
    );
}

/// The framing error codes: a blank-node `@id`/`@type` pattern raises
/// `invalid frame`; an out-of-range `@embed` raises `invalid @embed value`.
#[test]
fn framing_error_codes() {
    let input = r#"{"@id": "ex:a", "ex:p": "v"}"#;
    let e = run_opts(
        input,
        r#"{"@id": ["_:b0"]}"#,
        &JsonLdOptions::default(),
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert_eq!(e.code(), JsonLdErrorCode::InvalidFrame);
    assert_eq!(e.code().as_str(), "invalid frame");

    let e = run_opts(
        input,
        r#"{"@embed": "@sometimes"}"#,
        &JsonLdOptions::default(),
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert_eq!(e.code(), JsonLdErrorCode::InvalidEmbedValue);
    assert_eq!(e.code().as_str(), "invalid @embed value");
}

/// `omitGraph: false` (the JSON-LD 1.0 default) wraps a single match in a `@graph`
/// envelope; the 1.1 default returns it bare.
#[test]
fn omit_graph_shaping() {
    let input = r#"{"@id": "http://ex/a", "http://ex/p": "v"}"#;
    let bare = run(input, "{}");
    assert!(bare.get("@graph").is_none(), "1.1 default omits @graph: {}", text(&bare));
    assert_eq!(bare.get("@id").and_then(Json::as_str), Some("http://ex/a"));

    let mut opts10 = JsonLdOptions::default();
    opts10.processing_mode = ProcessingMode::JsonLd10;
    let enveloped = run_opts(input, "{}", &opts10, &FrameOptions::default()).expect("frame ok");
    let g = enveloped.get("@graph").expect("1.0 default keeps @graph");
    assert!(matches!(g, Json::Arr(items) if items.len() == 1));
}

/// `@requireAll` demands every frame property; without it any one suffices.
#[test]
fn require_all_matching() {
    let input = r#"{"@context": {"ex": "http://ex/"},
        "@graph": [
          {"@id": "ex:both", "ex:p": "1", "ex:q": "2"},
          {"@id": "ex:only-p", "ex:p": "1"}
        ]}"#;
    let strict = run(
        input,
        r#"{"@context": {"ex": "http://ex/"}, "@requireAll": true,
            "ex:p": {}, "ex:q": {}, "@explicit": true}"#,
    );
    assert_eq!(
        strict.get("@id").and_then(Json::as_str),
        Some("ex:both"),
        "only the node with BOTH properties matches: {}",
        text(&strict)
    );
}

/// `@reverse` framing attaches referrers under the reverse property.
#[test]
fn reverse_framing() {
    let out = run(
        r#"{"@context": {"ex": "http://ex/"},
            "@graph": [
              {"@id": "ex:child", "@type": "ex:C"},
              {"@id": "ex:parent", "ex:has": {"@id": "ex:child"}, "ex:name": "p"}
            ]}"#,
        r#"{"@context": {"ex": "http://ex/"}, "@type": "ex:C",
            "@reverse": {"ex:has": {}}}"#,
    );
    let rev = out.get("@reverse").expect("@reverse attached");
    let referrer = rev.get("ex:has").expect("reverse property");
    let referrer = match referrer {
        Json::Arr(items) => &items[0],
        obj => obj,
    };
    assert_eq!(referrer.get("ex:name"), Some(&Json::Str("p".to_string())));
}

/// The single node of a `@graph` envelope (or the object itself when bare).
fn graph_single(out: &Json) -> &Json {
    match out.get("@graph") {
        Some(Json::Arr(items)) if items.len() == 1 => &items[0],
        Some(other) => other,
        None => out,
    }
}
