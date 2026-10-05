//! sq-g6b6 — RDF-1.2 N-Quads **canonical-token tracking suite** for the
//! NON-STANDARD `rdf12-triple-terms` profile (follow-up from sq-hslb).
//!
//! The profile single-sources its canonical token rules — the `<<( … )>>`
//! triple-term form, the `"…"@lang--dir` directional-language form, and the
//! literal escaping — from oxrdf 0.3's `Display` (and re-parses them with
//! oxttl 0.2). W3C **rdf12-n-quads** is NOT a Recommendation, so the token
//! rules must still be RE-VERIFIED against the final REC when one is
//! published, and against any canonical-escaping change upstream.
//!
//! ## Spec-status check — 2026-08-01 (sq-g6b6 residue, issue #3455)
//!
//! Checked against the live documents on 2026-08-01:
//!
//! - **`https://www.w3.org/TR/rdf12-n-quads/` — W3C Working Draft, 23 July 2026**
//!   (`WD-rdf12-n-quads-20260723`). Its publication history lists 40 entries,
//!   **all Working Drafts** — no Candidate, Proposed, or final Recommendation
//!   has ever been published. `https://www.w3.org/TR/rdf12-n-triples/`, which
//!   Canonical N-Quads is defined as an extension of, is at the same status
//!   and date.
//! - So the "once final" hedge above **stays**. It is not stale prose; it
//!   states the document's actual status. Do not drop it until a REC exists.
//!
//! The byte-exact expectations below were nevertheless compared against that
//! draft's grammar, and **all of them match it**:
//!
//! | Expectation here | Draft rule |
//! |---|---|
//! | `<<( s p o )>>`, single spaces | N-Quads `[12] tripleTerm ::= '<<(' subject predicate object ')>>'`; Canonical N-Triples §3 permits white space "after subject, predicate, object, and the terminal `<<(`", each a single space |
//! | nesting only via the object slot | N-Quads `[9] object ::= IRIREF \| BLANK_NODE_LABEL \| literal \| tripleTerm` |
//! | `@he--rtl` / `@en--ltr`, lowercase | `[16] LANG_DIR ::= '@' [a-zA-Z]+ ('-' [a-zA-Z0-9]+)* ('--' [a-zA-Z]+)?`; canonical form case-maps LANG_DIR alphabetics to lowercase |
//! | no `^^<…#string>` on plain literals | canonical form: `xsd:string` literals MUST NOT carry the datatype IRI |
//! | `\"` `\\` `\n` `\r` `\t` `\b` `\f` | canonical form: BS, HT, LF, FF, CR, `"`, `\` MUST use ECHAR (`[19] ECHAR ::= '\' [tbnrf\"']`) |
//! | `\u0001` and `\u007F` (lowercase `u`, uppercase hex) | canonical form: U+0000–U+0007, VT, U+000E–U+001F, DEL, and non-XML11-`Char` MUST use UCHAR — lowercase `\u` plus 4 HEX, with HEX restricted to `[0-9A-F]` |
//! | VT (U+000B) and every U+000E–U+001F code point likewise UCHAR, while VT's LF/FF neighbours keep ECHAR | the same two rules read as *sets*, not ranges: VT and U+000E–U+001F are in the UCHAR set, while LF (U+000A) and FF (U+000C) — the code points either side of VT — are in the ECHAR set |
//! | a fourth term after the object, one space before it (`… <http://ex/g> .`, `… _:c14n0 .`) | N-Quads §3: Canonical N-Quads "extends Canonical N-Triples … to include graphLabel", with white space permitted "after subject, predicate, object, and graphLabel" |
//!
//! One thing found in the draft is worth carrying forward to the REC re-check,
//! and it does not change an expectation today:
//!
//! 1. **An editorial inconsistency between the two documents.** N-Quads §3
//!    restates the white-space list as "after subject, predicate, object, and
//!    graphLabel" — it omits the "and the terminal `<<(`" clause that
//!    N-Triples §3 carries. Read on its own that would forbid the space after
//!    `<<(` and make every triple-term line here non-canonical. It is read as
//!    an inheritance, not a contradiction: N-Quads §3 says Canonical N-Quads
//!    "extends Canonical N-Triples … to include graphLabel", and the draft's
//!    own Example 5 prints `<<( … )>>` with those spaces. If the REC resolves
//!    it the other way, every expectation in this file moves.
//!
//! The two draft rules this file left untested when that comparison was first
//! recorded — the UCHAR mandate for VT (U+000B) and for U+000E–U+001F, and the
//! `graphLabel` position — are covered byte-exactly as of issue #5515, by
//! [`literal_escaping_vt_and_upper_c0_controls_byte_exact`] and
//! [`graph_label_token_byte_exact`] respectively.
//!
//! This suite makes that re-verification impossible to miss:
//!
//! 1. [`serializer_versions_pinned`] pins the oxrdf/oxttl versions **resolved
//!    by sparq-canon itself** (its dependency edges in the workspace
//!    `Cargo.lock`). Any bump fails the
//!    test with instructions: re-check the byte-exact token expectations below
//!    against the upstream changelog + the (then-final) W3C rdf12-n-quads
//!    grammar, then update the pin.
//! 2. The remaining tests assert **byte-exact** canonical lines for the token
//!    edge cases, so even a same-version behavioural drift (or a patched
//!    vendored copy) is caught, and a future serializer bump that changes the
//!    canonical bytes goes red here rather than silently changing every
//!    canonical hash downstream.
//!
//! If a failure here is a *deliberate* upstream alignment with the final spec,
//! updating the expectations is a **canonical-output break** for this profile
//! (hashes over canonical documents change) — call that out in the PR.
#![cfg(feature = "rdf12-triple-terms")]

use oxrdf::{GraphName, Literal, NamedNode, NamedOrBlankNode, Quad, Term, Triple};
use std::path::Path;

/// The oxrdf line whose `Display` the profile serializes canonical tokens with.
const PINNED_OXRDF: &str = "0.3.3";
/// The oxttl line the profile re-parses its own canonical lines with.
const PINNED_OXTTL: &str = "0.2.3";
/// The oxrdf line the rdf-canon bridge speaks (dep alias `oxrdf02`).
const PINNED_OXRDF_BRIDGE: &str = "0.2.4";
/// The oxttl line the rdf-canon bridge speaks (dep alias `oxttl01`).
const PINNED_OXTTL_BRIDGE: &str = "0.1.8";

fn iri(s: &str) -> NamedNode {
    NamedNode::new(s).unwrap()
}

fn tt(s: NamedOrBlankNode, p: NamedNode, o: Term) -> Term {
    Term::Triple(Box::new(Triple::new(s, p, o)))
}

/// One ground triple whose object is `object`, canonicalized; returns the
/// single canonical line (ground input ⇒ serialization is pure `Display`).
fn canon_line(object: Term) -> String {
    let t = Triple::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        object,
    );
    let c = sparq_canon::canonicalize_triples_rdf12(&[t]).unwrap();
    assert_eq!(c.lines.len(), 1, "one input triple, one canonical line");
    c.lines[0].clone()
}

/// One ground quad in the named/blank graph `graph`, canonicalized; returns the
/// whole canonical N-Quads **document** (so the trailing newline is part of the
/// byte-exact comparison). [`canon_line`] cannot serve here: it goes through the
/// `Triple`-taking entry point, which has no `graphLabel` to place — the
/// `Quad`-taking, document-returning entry points are the only ones that do.
fn canon_quad_doc(object: Term, graph: GraphName) -> String {
    let q = Quad::new(
        NamedOrBlankNode::NamedNode(iri("http://ex/a")),
        iri("http://ex/says"),
        object,
        graph,
    );
    sparq_canon::canonicalize_rdf12(&[q]).unwrap()
}

/// Every resolved version of `package` in the workspace `Cargo.lock`.
/// Fallback resolver for [`sparq_canon_dep_versions`] when a dependency edge
/// carries no version (Cargo omits it when only one version exists graph-wide).
fn locked_versions(lock: &str, package: &str) -> Vec<String> {
    let mut versions = Vec::new();
    let mut last_name: Option<String> = None;
    for line in lock.lines() {
        let line = line.trim();
        if let Some(n) = line.strip_prefix("name = \"") {
            last_name = Some(n.trim_end_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("version = \"") {
            if last_name.as_deref() == Some(package) {
                versions.push(v.trim_end_matches('"').to_string());
            }
            last_name = None;
        }
    }
    versions.sort();
    versions
}

/// The versions of `package` that **sparq-canon itself** resolves, read from
/// the `dependencies` edges of sparq-canon's own `[[package]]` entry in the
/// workspace `Cargo.lock` (edges look like `"oxrdf 0.3.3"` when several
/// versions coexist, bare `"oxrdf"` when only one does). Binding to this
/// crate's edges — rather than filtering every lockfile entry by an expected
/// version prefix — means a version move by sparq-canon cannot hide behind
/// another workspace crate that still resolves the previously-pinned line.
fn sparq_canon_dep_versions(package: &str) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let lock = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read workspace Cargo.lock at {:?}: {e}", path));
    let mut in_canon = false;
    let mut in_deps = false;
    let mut versions = Vec::new();
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_canon = false;
            in_deps = false;
        } else if line == "name = \"sparq-canon\"" {
            in_canon = true;
        } else if in_canon && line == "dependencies = [" {
            in_deps = true;
        } else if in_deps {
            if line == "]" {
                break;
            }
            let edge = line.trim_end_matches(',').trim_matches('"');
            let mut parts = edge.split_whitespace();
            if parts.next() == Some(package) {
                match parts.next() {
                    Some(v) => versions.push(v.to_string()),
                    // Unversioned edge: unique version graph-wide.
                    None => versions.extend(locked_versions(&lock, package)),
                }
            }
        }
    }
    assert!(
        !versions.is_empty(),
        "no {package:?} dependency edge found on sparq-canon's [[package]] \
         entry in {path:?} — the lockfile format or the dependency set changed; \
         fix this resolver rather than weakening the pin (sq-g6b6)"
    );
    versions.sort();
    versions
}

/// The tracking pin (sq-g6b6): the resolved oxrdf-0.3.x / oxttl-0.2.x
/// serializer versions must match the versions the byte-exact token
/// expectations in this file were verified against.
///
/// **If this fails after a dependency bump:** (1) diff the upstream
/// oxrdf/oxttl changelog for canonical-token / escaping changes and re-check
/// it against the W3C rdf12-n-quads grammar (final REC, once published);
/// (2) confirm every byte-exact test in this file still passes (if one
/// changed, that is a canonical-output break — say so in the PR); (3) update
/// `PINNED_OXRDF` / `PINNED_OXTTL`.
#[test]
fn serializer_versions_pinned() {
    // The COMPLETE set of oxrdf/oxttl versions sparq-canon resolves — the
    // serializer line the profile prints with plus the rdf-canon bridge line —
    // with no prefix filtering, so a move to any other major/minor (e.g.
    // oxrdf 0.4) fails here even while another workspace crate still resolves
    // the previously-pinned line.
    assert_eq!(
        sparq_canon_dep_versions("oxrdf"),
        vec![PINNED_OXRDF_BRIDGE.to_string(), PINNED_OXRDF.to_string()],
        "oxrdf versions resolved by sparq-canon changed: re-verify the \
         canonical-token edge cases in this file against upstream + W3C \
         rdf12-n-quads, then update the pin (sq-g6b6; see module docs)"
    );
    assert_eq!(
        sparq_canon_dep_versions("oxttl"),
        vec![PINNED_OXTTL_BRIDGE.to_string(), PINNED_OXTTL.to_string()],
        "oxttl versions resolved by sparq-canon changed: re-verify the \
         canonical-token edge cases in this file against upstream + W3C \
         rdf12-n-quads, then update the pin (sq-g6b6; see module docs)"
    );
}

/// Triple-term token form: `<<(` SP … SP `)>>`, inner terms space-separated.
#[test]
fn triple_term_token_byte_exact() {
    let line = canon_line(tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal("v")),
    ));
    assert_eq!(
        line,
        r#"<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> "v" )>> ."#
    );
}

/// Nested triple-term token form: the inner `<<( … )>>` sits in the object
/// slot of the outer token with the same single-space separation.
#[test]
fn nested_triple_term_token_byte_exact() {
    let inner = tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s2")),
        iri("http://ex/p2"),
        Term::Literal(Literal::new_simple_literal("w")),
    );
    let line = canon_line(tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        inner,
    ));
    assert_eq!(
        line,
        r#"<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> <<( <http://ex/s2> <http://ex/p2> "w" )>> )>> ."#
    );
}

/// Directional-language token form: `@lang--dir`, direction lowercase
/// `ltr`/`rtl`, no explicit datatype IRI — both nested in a triple term and
/// at the top level.
#[test]
fn directional_language_token_byte_exact() {
    let rtl = Literal::new_directional_language_tagged_literal(
        "שלום",
        "he",
        oxrdf::BaseDirection::Rtl,
    )
    .unwrap();
    let line = canon_line(tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Literal(rtl),
    ));
    assert_eq!(
        line,
        "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> \"שלום\"@he--rtl )>> ."
    );

    let ltr =
        Literal::new_directional_language_tagged_literal("hello", "en", oxrdf::BaseDirection::Ltr)
            .unwrap();
    let line = canon_line(Term::Literal(ltr));
    assert_eq!(
        line,
        r#"<http://ex/a> <http://ex/says> "hello"@en--ltr ."#
    );
}

/// Canonical string escaping inside a triple-term literal: ECHAR forms for
/// `"` `\` LF CR TAB (plus `\b`/`\f`), `\uXXXX` for other C0 controls and
/// DEL. This is the surface most likely to move if the final rdf12-n-quads
/// canonical form tightens escaping — keep it byte-exact.
#[test]
fn literal_escaping_in_triple_term_byte_exact() {
    let value = "q:\" b:\\ nl:\n cr:\r tab:\t bs:\u{08} ff:\u{0C} c0:\u{01} del:\u{7F}";
    let line = canon_line(tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal(value)),
    ));
    assert_eq!(
        line,
        "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> \
         \"q:\\\" b:\\\\ nl:\\n cr:\\r tab:\\t bs:\\b ff:\\f c0:\\u0001 del:\\u007F\" )>> ."
    );
}

/// Expectation of [`literal_escaping_vt_and_upper_c0_controls_byte_exact`],
/// shared with [`canonical_token_lines_round_trip_through_oxttl`].
const VT_AND_UPPER_C0_LINE: &str = "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> \
     \"lf:\\n vt:\\u000B ff:\\f hi:\\u000E\\u000F\\u0010\\u0011\\u0012\\u0013\\u0014\\u0015\\u0016\
     \\u0017\\u0018\\u0019\\u001A\\u001B\\u001C\\u001D\\u001E\\u001F\" )>> .";

/// The rest of the canonical UCHAR set: **VT (U+000B)** and the whole
/// **U+000E–U+001F** range, which [`literal_escaping_in_triple_term_byte_exact`]
/// leaves out (it covers only U+0001 and DEL from that set).
///
/// VT is the discriminating case, and is why LF (U+000A) and FF (U+000C) are
/// asserted on the same line immediately either side of it: the canonical form
/// defines ECHAR and UCHAR as *sets*, and VT is the one UCHAR code point
/// wedged between two ECHAR ones, so a serializer that escaped by range rather
/// than by set would emit an ECHAR for VT (an LF/FF form, or a non-existent
/// `\v`) while every other test in this file stayed green.
#[test]
fn literal_escaping_vt_and_upper_c0_controls_byte_exact() {
    let value = "lf:\u{0A} vt:\u{0B} ff:\u{0C} hi:\u{0E}\u{0F}\u{10}\u{11}\u{12}\u{13}\u{14}\
                 \u{15}\u{16}\u{17}\u{18}\u{19}\u{1A}\u{1B}\u{1C}\u{1D}\u{1E}\u{1F}";
    let line = canon_line(tt(
        NamedOrBlankNode::NamedNode(iri("http://ex/s")),
        iri("http://ex/p"),
        Term::Literal(Literal::new_simple_literal(value)),
    ));
    assert_eq!(line, VT_AND_UPPER_C0_LINE);
}

/// Expectations of [`graph_label_token_byte_exact`], shared with
/// [`canonical_token_lines_round_trip_through_oxttl`] (which compares lines, so
/// these carry no trailing newline).
const NAMED_GRAPH_LABEL_LINE: &str =
    r#"<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> "v" )>> <http://ex/g> ."#;
const BNODE_GRAPH_LABEL_LINE: &str = r#"<http://ex/a> <http://ex/says> "v" _:c14n0 ."#;

/// The `graphLabel` position — the fourth term that is Canonical N-Quads' whole
/// difference from Canonical N-Triples, and the reason this crate tracks the
/// N-Quads document. Every other case in this file is a three-term line, so
/// without this one the position the profile exists to serialize is untested.
///
/// Both label kinds are asserted: a named node prints as its IRIREF, and a
/// blank-node label goes through the same `c14nN` relabelling as a blank node
/// in any other position.
#[test]
fn graph_label_token_byte_exact() {
    let named = canon_quad_doc(
        tt(
            NamedOrBlankNode::NamedNode(iri("http://ex/s")),
            iri("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
        ),
        GraphName::NamedNode(iri("http://ex/g")),
    );
    assert_eq!(named, format!("{}\n", NAMED_GRAPH_LABEL_LINE));

    let bnode = canon_quad_doc(
        Term::Literal(Literal::new_simple_literal("v")),
        GraphName::BlankNode(oxrdf::BlankNode::new("g0").unwrap()),
    );
    assert_eq!(bnode, format!("{}\n", BNODE_GRAPH_LABEL_LINE));
}

/// Parser/serializer agreement on the canonical tokens: every canonical line
/// above must re-parse through oxttl's N-Quads parser (the parser the profile
/// itself uses) and `Display` back to the identical bytes. A one-sided
/// upstream change (parser xor serializer) breaks here even if the serialized
/// form alone still looks plausible.
#[test]
fn canonical_token_lines_round_trip_through_oxttl() {
    let lines = [
        r#"<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> "v" )>> ."#,
        r#"<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> <<( <http://ex/s2> <http://ex/p2> "w" )>> )>> ."#,
        "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> \"שלום\"@he--rtl )>> .",
        r#"<http://ex/a> <http://ex/says> "hello"@en--ltr ."#,
        "<http://ex/a> <http://ex/says> <<( <http://ex/s> <http://ex/p> \
         \"q:\\\" b:\\\\ nl:\\n cr:\\r tab:\\t bs:\\b ff:\\f c0:\\u0001 del:\\u007F\" )>> .",
        VT_AND_UPPER_C0_LINE,
        NAMED_GRAPH_LABEL_LINE,
        BNODE_GRAPH_LABEL_LINE,
    ];
    for line in lines {
        let quads: Vec<oxrdf::Quad> = oxttl::NQuadsParser::new()
            .for_slice(line.as_bytes())
            .collect::<Result<_, _>>()
            .unwrap_or_else(|e| panic!("canonical line must re-parse: {line:?}: {e}"));
        assert_eq!(quads.len(), 1, "exactly one quad per line: {line:?}");
        let q = &quads[0];
        // Graph-labelled lines carry a fourth term; default-graph lines must
        // keep the three-term spelling. The match is load-bearing rather than
        // cosmetic: oxrdf `Display`s `GraphName::DefaultGraph` as the literal
        // `DEFAULT`, which is not N-Quads syntax at all, so a uniform
        // four-field format string would corrupt every default-graph line here.
        let reserialized = match &q.graph_name {
            GraphName::DefaultGraph => format!("{} {} {} .", q.subject, q.predicate, q.object),
            g => format!("{} {} {} {} .", q.subject, q.predicate, q.object, g),
        };
        assert_eq!(
            reserialized, line,
            "parse ∘ print must be the identity on canonical token lines"
        );
    }
}
