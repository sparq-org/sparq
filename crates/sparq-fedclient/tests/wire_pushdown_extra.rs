//! sq-bif.2 — brTPF binding-block wire codec + capability-aware pushdown extra branches.
//!
//! The `wire` and `pushdown` modules carry strong inline unit tests. This file adds the
//! adversarial / edge branches the inline suites do not, exercised through the PUBLIC codec +
//! pushdown surface:
//!
//!  * the binary wire round-tripping a mapping that binds **all three** positions (s+p+o) plus
//!    an EXTRA non-position pair, and a literal carrying its `^^<dt>` decoration verbatim;
//!  * [`decode_bindings`] rejecting a buffer whose **term length points past the end** and one
//!    whose **EXTRA name length points past the end** — both clean [`WireError`]s, never a panic
//!    or out-of-bounds read (this crate is `forbid(unsafe_code)`);
//!  * the public [`BINARY_VERSION`] / [`BINARY_MAGIC`] contract: a future version byte is a
//!    clean [`WireError::UnsupportedVersion`], a wrong magic a clean [`WireError::BadMagic`];
//!  * pushdown's [`group_vars`] / [`common_variable_check`] with an EMPTY group (no group var ⇒
//!    no var-referencing filter is pushable; a constant-only filter still is), and
//!    [`push_group`] over a brTPF fragment source whose projection is `SELECT *` when the caller
//!    requests no narrowing (the empty-output-vars fragment branch);
//!  * [`bind_block_size`] for `MaxMpR(0)` (clamped to 1) and `BindJoin::None` (0).
//!
//! Gated on `fedclient`; the default build compiles this file to nothing.
//!
//! sq-bif.2 — flagged for Fable re-review when available.

#![cfg(feature = "fedclient")]

use sparq_fedclient::{
    bind_block_size, common_variable_check, decode_bindings, encode_bindings, encode_bindings_text,
    group_vars, push_group, Capability, ExclusiveGroup, Filter, FilterClass, FragTerm, WireError,
    BINARY_MAGIC, BINARY_VERSION,
};
use sparq_fedplan::{Bgp, Term, TriplePattern, Var};

type FragBinding = Vec<(String, FragTerm)>;

fn iri(s: &str) -> FragTerm {
    FragTerm::Iri(s.to_string())
}
fn lit(s: &str) -> FragTerm {
    FragTerm::Literal(s.to_string())
}

// ─── Binary wire: full s+p+o + extra round-trip, decorated literals ─────────────────────

#[test]
fn binary_round_trips_full_spo_plus_extra_and_typed_literal() {
    // A mapping that binds ALL THREE canonical positions (each rides a header flag, no name
    // bytes) PLUS a non-position variable (the EXTRA section), and a typed literal carrying its
    // ^^<dt> decoration verbatim — the densest mapping shape, exercised end-to-end.
    let block: Vec<FragBinding> = vec![vec![
        ("s".to_string(), iri("http://ex/alice")),
        ("p".to_string(), iri("http://xmlns.com/foaf/0.1/age")),
        (
            "o".to_string(),
            lit(r#""30"^^<http://www.w3.org/2001/XMLSchema#integer>"#),
        ),
        ("graph".to_string(), iri("http://ex/g1")),
    ]];
    let back = decode_bindings(&encode_bindings(&block)).expect("decode");
    // Position keys come back canonical s→p→o, then the EXTRA pair in encoded order.
    assert_eq!(back, block);
}

#[test]
fn binary_carries_literal_chars_the_text_wire_cannot() {
    // A literal with embedded '=', whitespace, and a newline survives the binary wire (the text
    // wire's one-mapping-per-line `position=term` grammar could not carry it).
    let nasty: Vec<FragBinding> = vec![vec![("o".to_string(), lit("\"a = b\tc\nd\""))]];
    assert_eq!(decode_bindings(&encode_bindings(&nasty)).unwrap(), nasty);
}

// ─── Binary wire: adversarial decode (length fields past the end) ───────────────────────

#[test]
fn decode_rejects_term_length_past_end() {
    // Hand-craft: magic + version + count=1 + header(subject) + term-kind(IRI) + a varint length
    // of 50 but NO payload bytes. The term length points past the buffer ⇒ a clean Truncated.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&BINARY_MAGIC);
    bytes.push(BINARY_VERSION);
    bytes.push(1); // count = 1
    bytes.push(0b0000_0001); // header: subject present
    bytes.push(0); // term kind = IRI
    bytes.push(50); // varint length = 50 (but no bytes follow)
    let err = decode_bindings(&bytes).unwrap_err();
    assert_eq!(err, WireError::Truncated, "a length past the end is Truncated");
}

#[test]
fn decode_rejects_extra_name_length_past_end() {
    // magic + version + count=1 + header(HAS_EXTRA only) + nextra=1 + name-length=99 with no
    // name bytes ⇒ a clean Truncated, never an out-of-bounds read.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&BINARY_MAGIC);
    bytes.push(BINARY_VERSION);
    bytes.push(1); // count = 1
    bytes.push(0b1000_0000); // header: HAS_EXTRA, no positions
    bytes.push(1); // nextra = 1
    bytes.push(99); // name length = 99 (no bytes follow)
    let err = decode_bindings(&bytes).unwrap_err();
    assert_eq!(err, WireError::Truncated);
}

#[test]
fn decode_version_and_magic_contract() {
    // The public BINARY_VERSION/BINARY_MAGIC contract: a wrong magic is BadMagic; a future
    // version byte is UnsupportedVersion (so a peer upgrade is detected, not misparsed).
    let good = encode_bindings(&[vec![("s".to_string(), iri("http://ex/a"))]]);
    let mut bad_magic = good.clone();
    bad_magic[0] ^= 0xff;
    assert_eq!(decode_bindings(&bad_magic), Err(WireError::BadMagic));

    let mut future = good;
    future[4] = BINARY_VERSION.wrapping_add(7);
    assert_eq!(
        decode_bindings(&future),
        Err(WireError::UnsupportedVersion(BINARY_VERSION.wrapping_add(7)))
    );
}

#[test]
fn decode_every_truncation_point_is_clean() {
    // No prefix of a valid block may panic — every cut is a clean error (forbid unsafe).
    let block: Vec<FragBinding> = vec![
        vec![
            ("s".to_string(), iri("http://ex/alice")),
            ("o".to_string(), lit("\"hi\"@en")),
        ],
        vec![("person".to_string(), iri("http://ex/dave"))],
    ];
    let bytes = encode_bindings(&block);
    for cut in 0..bytes.len() {
        // Must not panic; a partial buffer is always a clean Result.
        let _ = decode_bindings(&bytes[..cut]);
    }
    // The full buffer round-trips.
    assert_eq!(decode_bindings(&bytes).unwrap(), block);
}

#[test]
fn text_wire_drops_non_position_vars_binary_keeps_them() {
    // A binding over a non-position variable has no brTPF text slot (dropped), but the binary
    // wire carries it losslessly.
    let block: Vec<FragBinding> = vec![vec![("person".to_string(), iri("http://ex/dave"))]];
    assert_eq!(encode_bindings_text(&block), "");
    assert_eq!(decode_bindings(&encode_bindings(&block)).unwrap(), block);
}

// ─── Pushdown: empty-group + constant filter + fragment SELECT * ────────────────────────

fn var(s: &str) -> Term {
    Term::Var(Var::new(s))
}
fn tiri(s: &str) -> Term {
    Term::Iri(s.to_string())
}

#[test]
fn common_variable_check_over_empty_group() {
    // An empty group binds NO variable: a var-referencing filter is not pushable, but a
    // constant-only conjunct (no vars) trivially is.
    let none: Vec<String> = Vec::new();
    let with_var = Filter::new(vec!["x".to_string()], "?x > 1", FilterClass::Full);
    assert!(
        !common_variable_check(&with_var, &none),
        "no group var ⇒ a var-referencing filter is not pushable"
    );
    let constant = Filter::new(vec![], "1 = 1", FilterClass::Equality);
    assert!(
        common_variable_check(&constant, &none),
        "a constant-only conjunct is trivially pushable"
    );
}

#[test]
fn group_vars_collects_in_position_order_dedup() {
    // ?s :p ?o . ?o :q ?z — group vars are s, o, z (de-duplicated, pattern-then-position order).
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), tiri("http://ex/p"), var("o")),
        TriplePattern::new(var("o"), tiri("http://ex/q"), var("z")),
    ]);
    let group = ExclusiveGroup {
        source: 0,
        patterns: vec![0, 1],
    };
    assert_eq!(
        group_vars(&group, &bgp),
        vec!["s".to_string(), "o".to_string(), "z".to_string()]
    );
}

#[test]
fn push_group_fragment_with_no_output_vars_is_select_star_first_pattern() {
    // A brTPF source group of TWO patterns with NO requested output narrowing: pushdown emits
    // only the FIRST pattern as `SELECT *` (a fragment server answers one pattern; no FILTER /
    // ORDER / LIMIT), and the rest stays client-side.
    let bgp = Bgp::new(vec![
        TriplePattern::new(var("s"), tiri("http://ex/p"), var("o")),
        TriplePattern::new(var("o"), tiri("http://ex/q"), var("z")),
    ]);
    let group = ExclusiveGroup {
        source: 0,
        patterns: vec![0, 1],
    };
    let cap = Capability::brtpf(20);
    // Empty output_vars ⇒ "give whatever the group projects"; for a fragment source that is the
    // first pattern's open SELECT.
    let pushed = push_group(&group, &bgp, &cap, &[], &[], &[], None).unwrap();
    assert_eq!(
        pushed.sub.sparql, "SELECT * WHERE { ?s <http://ex/p> ?o }",
        "fragment + empty output ⇒ first pattern only, SELECT *"
    );
    assert!(pushed.pushed_filters.is_empty());
}

#[test]
fn push_group_empty_group_fails_closed() {
    // A group naming NO patterns is malformed ⇒ push_group returns None (fail closed).
    let bgp = Bgp::new(vec![TriplePattern::new(
        var("s"),
        tiri("http://ex/p"),
        var("o"),
    )]);
    let empty_group = ExclusiveGroup {
        source: 0,
        patterns: vec![],
    };
    let cap = Capability::endpoint();
    assert!(push_group(&empty_group, &bgp, &cap, &[], &[], &[], None).is_none());
}

// ─── Pushdown: bind-block size policy across all bind-join modes ────────────────────────

#[test]
fn bind_block_size_clamps_and_disables_correctly() {
    // brTPF maxMpR(0) is clamped to 1 (never a zero-sized block ⇒ no infinite loop), maxMpR(n)
    // is n, plain TPF (no bind-join) is 0, and a full endpoint is the default block.
    assert_eq!(bind_block_size(&Capability::brtpf(0)), 1, "maxMpR 0 clamps to 1");
    assert_eq!(bind_block_size(&Capability::brtpf(7)), 7);
    assert_eq!(bind_block_size(&Capability::tpf()), 0, "plain TPF has no bind-join");
    assert!(
        bind_block_size(&Capability::endpoint()) >= 1,
        "a full endpoint accepts a non-empty VALUES block"
    );
}
