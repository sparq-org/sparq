//! Per-term / per-triple field encoding (plan §2.2).
//!
//! `Enc_t(term) = h_2(type_code, h_s(value))` with `h_s` = Blake3
//! off-circuit and `h_2` = Poseidon2 (Q10: Pedersen retired). Blank nodes
//! gain graph scoping and a per-graph salt:
//!
//! `Enc_t(bnode) = h_2(BlankNode_code, h_2(salt_G, blake3(canonical_label)))`
//!
//! where `salt_G` is fixed at issuance/ingest and recorded as `zk:rdfc10Salt`
//! in `<urn:sparq:zk>`. The salt closes the Q6 cross-graph correlation
//! channel below the join layer: RDFC10's canonical labels (`c14n0`, …)
//! recur across unrelated graphs, and unsalted they would hash to equal leaf
//! components. Salted, bnodes from different graphs are distinct **by
//! construction** — which is exactly RDF *merge* semantics (plan §2.4).
//!
//! Conventions this module fixes (the circuits must recompute `h_2` layers
//! only; `h_s` outputs are witnesses):
//! - `h_s(IRI)`        = blake3 over the IRI string (no `<>` delimiters).
//! - `h_s(literal)`    = blake3 over the canonical N-Triples token (lexical
//!   form with escapes, `@lang` / `^^<datatype>` included) — one stable
//!   byte-serialization that folds datatype and language in.
//! - `h_s(bnode)`      = blake3 over the RDFC10 canonical label (no `_:`).
//! - 32-byte Blake3 digests map into the field by truncation to the low 31
//!   bytes (248 bits, bias-free; see [`crate::field::field_from_hash_bytes`]).
//! - Leaf hash of a triple: `h_3(Enc(s), Enc(p), Enc(o))` (one Poseidon2
//!   sponge call over three field elements).

use crate::field::{field_from_hash_bytes, Fr};
use crate::poseidon2;
use oxrdf::{NamedOrBlankNode, Term, Triple};

/// Type codes (field-element domain separators inside `h_2`).
pub const TYPE_CODE_IRI: u64 = 1;
pub const TYPE_CODE_LITERAL: u64 = 2;
pub const TYPE_CODE_BLANK_NODE: u64 = 3;

fn blake3_field(bytes: &[u8]) -> Fr {
    field_from_hash_bytes(blake3::hash(bytes).as_bytes())
}

/// Encodes a term under a graph salt. Blank-node labels are expected to be
/// the RDFC10 canonical labels (encode after canonicalization).
///
/// Returns `None` for RDF 1.2 triple terms (outside the committed data
/// model; callers fail closed).
pub fn encode_term(term: &Term, salt: &Fr) -> Option<Fr> {
    match term {
        Term::NamedNode(n) => Some(poseidon2::hash(&[
            Fr::from(TYPE_CODE_IRI),
            blake3_field(n.as_str().as_bytes()),
        ])),
        Term::Literal(l) => Some(poseidon2::hash(&[
            Fr::from(TYPE_CODE_LITERAL),
            blake3_field(l.to_string().as_bytes()),
        ])),
        Term::BlankNode(b) => {
            let inner = poseidon2::hash(&[*salt, blake3_field(b.as_str().as_bytes())]);
            Some(poseidon2::hash(&[Fr::from(TYPE_CODE_BLANK_NODE), inner]))
        }
        Term::Triple(_) => None,
    }
}

/// Encodes a triple to its commitment leaf: `h_3(Enc(s), Enc(p), Enc(o))`.
pub fn encode_triple(triple: &Triple, salt: &Fr) -> Option<Fr> {
    let s = match &triple.subject {
        NamedOrBlankNode::NamedNode(n) => encode_term(&Term::NamedNode(n.clone()), salt)?,
        NamedOrBlankNode::BlankNode(b) => encode_term(&Term::BlankNode(b.clone()), salt)?,
    };
    let p = encode_term(&Term::NamedNode(triple.predicate.clone()), salt)?;
    let o = encode_term(&triple.object, salt)?;
    Some(poseidon2::hash(&[s, p, o]))
}

/// Derives a salt field element from caller-supplied entropy (the ingest
/// path draws 32 OS-random bytes per graph; tests use fixed bytes).
pub fn salt_from_bytes(bytes: &[u8; 32]) -> Fr {
    field_from_hash_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};

    #[test]
    fn iris_and_literals_are_salt_independent() {
        let s1 = salt_from_bytes(&[1u8; 32]);
        let s2 = salt_from_bytes(&[2u8; 32]);
        let iri = Term::NamedNode(NamedNode::new("http://ex/a").unwrap());
        let lit = Term::Literal(Literal::new_language_tagged_literal("x", "en").unwrap());
        assert_eq!(encode_term(&iri, &s1), encode_term(&iri, &s2));
        assert_eq!(encode_term(&lit, &s1), encode_term(&lit, &s2));
    }

    #[test]
    fn bnodes_are_salt_separated() {
        let s1 = salt_from_bytes(&[1u8; 32]);
        let s2 = salt_from_bytes(&[2u8; 32]);
        let b = Term::BlankNode(BlankNode::new("c14n0").unwrap());
        assert_ne!(encode_term(&b, &s1), encode_term(&b, &s2));
    }

    #[test]
    fn literal_shapes_are_distinguished() {
        let s = salt_from_bytes(&[0u8; 32]);
        let plain = Term::Literal(Literal::new_simple_literal("1"));
        let int = Term::Literal(Literal::new_typed_literal(
            "1",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        ));
        let lang = Term::Literal(Literal::new_language_tagged_literal("1", "en").unwrap());
        let e: Vec<_> = [&plain, &int, &lang].iter().map(|t| encode_term(t, &s).unwrap()).collect();
        assert_ne!(e[0], e[1]);
        assert_ne!(e[0], e[2]);
        assert_ne!(e[1], e[2]);
    }

    // sq-qcnn.25: DETERMINISM value pins. These recompute the exact
    // `Enc_t` composition from the public primitives (independent of `encode_term`'s
    // body) and assert the encoder reproduces it EXACTLY — so a mutation of the
    // type-code, the `h_s`/`h_2` layering, or the argument order is caught, not just
    // a panic. They are the "same input -> same field element" determinism contract
    // (sq-qcnn.25). `blake3_field` is the module's own `h_s` helper (in scope via
    // `super::*`), so the recomputation stays byte-for-byte faithful to the encoder.
    // NO soundness claim is made (sq-qhy4).

    #[test]
    fn encode_term_iri_pins_type_code_and_hs_layering() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let iri = Term::NamedNode(NamedNode::new("http://ex/a").unwrap());
        // Enc_t(IRI) = h_2(TYPE_CODE_IRI, blake3(iri-string, no <> delimiters)).
        let expected = crate::poseidon2::hash(&[
            Fr::from(TYPE_CODE_IRI),
            blake3_field(b"http://ex/a"),
        ]);
        let got = encode_term(&iri, &salt).unwrap();
        assert_eq!(got, expected);
        // Byte-stable: the same input yields the same 32-byte field word every call.
        assert_eq!(
            crate::field::field_to_be_bytes_32(&got),
            crate::field::field_to_be_bytes_32(&encode_term(&iri, &salt).unwrap()),
        );
    }

    #[test]
    fn encode_term_literal_pins_canonical_ntriples_token() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let lit = Literal::new_language_tagged_literal("x", "en").unwrap();
        let term = Term::Literal(lit.clone());
        // Enc_t(literal) = h_2(TYPE_CODE_LITERAL, blake3(canonical N-Triples token)).
        // The token folds datatype/language: it is `Literal::to_string()`.
        let expected = crate::poseidon2::hash(&[
            Fr::from(TYPE_CODE_LITERAL),
            blake3_field(lit.to_string().as_bytes()),
        ]);
        assert_eq!(encode_term(&term, &salt).unwrap(), expected);
    }

    #[test]
    fn encode_term_bnode_pins_salted_two_layer_composition() {
        let salt = salt_from_bytes(&[7u8; 32]);
        let b = Term::BlankNode(BlankNode::new("c14n0").unwrap());
        // Enc_t(bnode) = h_2(BLANK_NODE, h_2(salt, blake3(canonical-label, no _:))).
        let inner = crate::poseidon2::hash(&[salt, blake3_field(b"c14n0")]);
        let expected = crate::poseidon2::hash(&[Fr::from(TYPE_CODE_BLANK_NODE), inner]);
        assert_eq!(encode_term(&b, &salt).unwrap(), expected);
    }

    #[test]
    fn encode_triple_pins_h3_of_s_p_o_and_is_position_sensitive() {
        let salt = salt_from_bytes(&[9u8; 32]);
        let s = NamedNode::new("http://ex/s").unwrap();
        let p = NamedNode::new("http://ex/p").unwrap();
        let o = NamedNode::new("http://ex/o").unwrap();
        let t = Triple::new(
            NamedOrBlankNode::NamedNode(s.clone()),
            p.clone(),
            Term::NamedNode(o.clone()),
        );
        // Leaf = h_3(Enc(s), Enc(p), Enc(o)) in exactly that argument order.
        let expected = crate::poseidon2::hash(&[
            encode_term(&Term::NamedNode(s.clone()), &salt).unwrap(),
            encode_term(&Term::NamedNode(p.clone()), &salt).unwrap(),
            encode_term(&Term::NamedNode(o.clone()), &salt).unwrap(),
        ]);
        assert_eq!(encode_triple(&t, &salt).unwrap(), expected);
        // Swapping predicate<->object roles must change the leaf: the position is
        // load-bearing (kills an s/p/o argument-swap in the sponge input).
        let swapped = Triple::new(
            NamedOrBlankNode::NamedNode(s),
            o, // predicate slot now holds what was the object IRI
            Term::NamedNode(p),
        );
        assert_ne!(encode_triple(&t, &salt).unwrap(), encode_triple(&swapped, &salt).unwrap());
    }

    #[test]
    fn encode_term_rejects_rdf12_triple_term_fail_closed() {
        // RDF 1.2 triple terms are outside the committed data model: encode_term
        // returns None so callers fail closed (covers the `Term::Triple(_) => None`
        // arm). No soundness claim (sq-qhy4).
        let salt = salt_from_bytes(&[1u8; 32]);
        let inner = Triple::new(
            NamedOrBlankNode::NamedNode(NamedNode::new("http://ex/s").unwrap()),
            NamedNode::new("http://ex/p").unwrap(),
            Term::NamedNode(NamedNode::new("http://ex/o").unwrap()),
        );
        let quoted = Term::Triple(Box::new(inner));
        assert_eq!(encode_term(&quoted, &salt), None);
    }

    #[test]
    fn salt_from_bytes_pins_field_from_hash_bytes() {
        // salt_from_bytes is exactly field_from_hash_bytes over the entropy word.
        let bytes = [3u8; 32];
        assert_eq!(salt_from_bytes(&bytes), crate::field::field_from_hash_bytes(&bytes));
        // Deterministic + entropy-sensitive.
        assert_eq!(salt_from_bytes(&bytes), salt_from_bytes(&bytes));
        assert_ne!(salt_from_bytes(&[3u8; 32]), salt_from_bytes(&[4u8; 32]));
    }
}
