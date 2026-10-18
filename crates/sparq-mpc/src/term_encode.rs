// sq-dl81 — the collision-resistant Term->Fp join-key encoder + the
// injectivity contract the hidden-value join rests on.
//! Domain-separated `Term -> Fp` encoding for PRIVATE join keys.
//!
//! ## Why this module exists (the soundness gap it closes)
//!
//! [`crate::join::HiddenValueJoin`] (and the hidden property-path chain,
//! [`crate::hidden_path`]) joins two holders on a key WITHOUT revealing it, by a
//! secret-shared **field-equality** test over [`Fp`]: it opens only the masked
//! product `m = (key_L − key_R)·r`, and `m == 0 ⇔ key_L == key_R` *in the field*.
//! That field equality is a SOUND stand-in for **RDF term equality** only if the
//! `Term -> Fp` encoding is **injective on the join domain**. A collision — two
//! distinct terms mapping to the same `Fp` — is a **false match**: the protocol
//! discloses a join row that does not exist in the plaintext answer, and the
//! differential tests CANNOT see it because they pick hand-injective small-int
//! keys by construction (the gap named in bead sq-dl81).
//!
//! Before this module the encoding was "the holder's untested responsibility".
//! This module replaces that with a **documented, uniform-output encoder** so the
//! injectivity question becomes a *quantified birthday bound over the field*
//! rather than an unstated assumption, plus a **collision-detection path**
//! ([`KeyEncoder`]) the holder can run over its own key domain to *prove*
//! injectivity for a concrete input set before any share crosses the wire.
//!
//! ## Construction
//!
//! `encode(term) = reduce_mod_p( SHA-512( DOMAIN_TAG ‖ ntriples(term) ) )`.
//!
//! - **Canonical pre-image.** The term is serialised in its **N-Triples**
//!   lexical form (oxrdf's [`std::fmt::Display`]): IRIs as `<…>`, literals as
//!   `"…"`, `"…"^^<dt>`, or `"…"@lang`, blank nodes as `_:id`, triple terms
//!   recursively. That serialisation is itself injective on terms *up to blank-
//!   node label* (see the caveat below), and crucially it disambiguates the
//!   VARIANTS — an IRI `<x>` and a plain literal `"x"` never share a pre-image —
//!   so the encoder cannot confuse a term-kind boundary.
//! - **Domain separation.** A fixed [`DOMAIN_TAG`] prefixes every pre-image so a
//!   `Term` hash can never coincide with some other SHA-512 use in the system
//!   (RFC-style domain separation; the same discipline the ZK estate uses).
//! - **Fold into the field.** The 64-byte digest is read as a big-endian `u128`
//!   over its first 16 bytes and reduced mod `p = 2^61 − 1`. SHA-512 output is
//!   (modelled as) uniform, so the reduced value is statistically uniform over
//!   the field; truncating to 128 bits before the reduction loses no security
//!   because the field is only 61 bits wide (see the birthday note).
//!
//! ## Injectivity & the birthday bound (state this honestly)
//!
//! The field has `p = 2^61 − 1` elements ([`crate::field::P`]). Under the random-
//! oracle model for SHA-512, the encoder's outputs are uniform over the field, so
//! over a set of `q` DISTINCT terms the probability that *some* two collide is the
//! classic birthday bound `≈ q² / (2·p) = q² / 2^62`. Concretely a collision
//! reaches 50 % only near `q ≈ 2^30.5 ≈ 1.86×10^9` distinct terms — **far** above
//! the viable hidden-join regime (`≤10³–10⁴ rows/holder`; see the `mpc-protocols`
//! skill). At `q = 10^4` the collision probability is `≈ 10^8 / 2^62 ≈ 2.2×10^-11`.
//!
//! This is a **statistical** bound, NOT a guarantee. Two honest consequences:
//!
//! 1. The encoder is **NOT a security claim by itself** — like the rest of the
//!    crate it is a substrate. The malicious-secure, in-circuit *encoding-
//!    correctness* proof (that the opened key equals `encode(term)` for the
//!    holder's actual term) is the M4 collaborative-proof job and is NOT done here
//!    (the bead calls this module its "on-ramp").
//! 2. A holder who needs an *exact* injectivity guarantee for its concrete key
//!    set — not just the birthday bound — runs [`KeyEncoder`], which detects any
//!    collision among the encoded terms and returns [`EncodeError::Collision`]
//!    (fail-closed) BEFORE the keys are secret-shared. This converts the residual
//!    birthday risk into a checkable precondition for the inputs actually used.
//!
//! ## Blank-node caveat
//!
//! Blank-node labels are scope-local: the same conceptual node may carry
//! different labels in different holders' graphs, and *different* nodes may share
//! a label. The N-Triples form encodes the **label**, so two holders joining on a
//! blank node will match iff their labels agree — which is the SPARQL term-
//! equality semantics for blank nodes anyway (no cross-graph blank-node identity).
//! Callers that need cross-graph blank-node identity must canonicalise labels
//! (e.g. via `sparq-canon`) BEFORE encoding; that is out of scope here and is
//! flagged, not silently assumed.

use crate::field::{Fp, P};
use oxrdf::Term;
use sha2::{Digest, Sha512};
use std::collections::HashMap;

/// Domain-separation prefix for the `Term -> Fp` join-key hash. Any change here
/// is a **breaking** change to the encoding (existing encoded keys would no
/// longer match), so it is a fixed, versioned constant.
pub const DOMAIN_TAG: &[u8] = b"sparq-mpc/term-join-key/v1\0";

/// Encode a single RDF [`Term`] into a field element for use as a PRIVATE join
/// key, via the domain-separated SHA-512 construction documented at the module
/// level: `reduce_mod_p(SHA-512(DOMAIN_TAG ‖ ntriples(term)))`.
///
/// The output is statistically uniform over `F_p` (random-oracle model), so the
/// probability that two DISTINCT terms collide is the birthday bound `≈ q²/2^62`
/// (module docs). This is the encoder the hidden-value join's field-equality test
/// stands on; for an EXACT injectivity check over a concrete key set, drive it
/// through [`KeyEncoder`] instead, which detects collisions and fails closed.
pub fn encode_term(term: &Term) -> Fp {
    let mut hasher = Sha512::new();
    hasher.update(DOMAIN_TAG);
    // N-Triples lexical form: variant-disambiguating and stable.
    hasher.update(term.to_string().as_bytes());
    let digest = hasher.finalize();
    // Read the leading 16 bytes big-endian as a u128, then reduce mod p. 128 bits
    // is far wider than the 61-bit field, so the reduction is statistically
    // uniform and the truncation costs nothing (the field is the bottleneck).
    let mut be = [0u8; 16];
    be.copy_from_slice(&digest[..16]);
    let wide = u128::from_be_bytes(be);
    // p < 2^61, so wide % p < 2^61 fits a u64 — exactly Fp's canonical range.
    Fp::new((wide % (P as u128)) as u64)
}

/// A collision in the `Term -> Fp` encoding: two DISTINCT terms that map to the
/// SAME field element. Surfacing this is the whole point of the [`KeyEncoder`]
/// detection path — a collision is a *false match* the hidden-value join cannot
/// otherwise see, so it is an error, never silently tolerated.
/// `Debug` is hand-written, not derived: the derive would print the colliding
/// terms, and both of this type's renderings are value-free (see the `Display`
/// impl below). The payload's fields stay public, so a caller that needs the
/// terms reads them from [`Collision`] directly.
#[derive(Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// Two distinct terms encoded to the same field element. The holder MUST
    /// resolve this (it is a soundness break for the hidden join) before any key
    /// is secret-shared — e.g. by canonicalising, or in the (astronomically
    /// unlikely, per the birthday bound) hash-collision case, by changing the
    /// domain tag / widening the field in a future revision. The two terms are
    /// boxed to keep `EncodeError` (hence `Result<Fp, EncodeError>`) small on the
    /// happy path — a collision is the rare error case.
    Collision(Box<Collision>),
}

/// The payload of an [`EncodeError::Collision`]: the colliding key and the two
/// distinct terms that produced it. Boxed inside the error so the common
/// `Ok(Fp)` result stays cheap.
///
/// The fields are public — the holder that hit the collision already holds both
/// terms and needs them to resolve it. What is deliberately NOT available is a
/// *rendering*: `Debug` and `Display` are hand-written and print neither the
/// terms nor the key, so the payload cannot reach a log line or a peer's error
/// body just by being formatted.
#[derive(Clone, PartialEq, Eq)]
pub struct Collision {
    /// The colliding field element both terms encode to.
    pub key: Fp,
    /// The first term (already seen) that produced `key`.
    pub first: Term,
    /// The second, DISTINCT term that also produced `key`.
    pub second: Term,
}

/// A fixed, value-free rendering shared by `Display` and `Debug` on both this
/// error and its payload. It names the condition and what to do about it, and
/// carries nothing derived from the terms or the key — not the terms, not the
/// key, and not a count or a hash prefix of either, since those are themselves
/// functions of the inputs.
const COLLISION_MESSAGE: &str = "Term->Fp join-key collision: two distinct terms encode to the \
                                 same join key (a false-match soundness break for the \
                                 hidden-value join). Resolve it before any key is \
                                 secret-shared; the colliding terms are on the Collision \
                                 payload, which this message deliberately does not render";

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::Collision(_) => f.write_str(COLLISION_MESSAGE),
        }
    }
}

impl std::fmt::Debug for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::Collision(_) => write!(f, "EncodeError::Collision({})", COLLISION_MESSAGE),
        }
    }
}

impl std::fmt::Display for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(COLLISION_MESSAGE)
    }
}

impl std::fmt::Debug for Collision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Collision({})", COLLISION_MESSAGE)
    }
}

impl std::error::Error for EncodeError {}

/// A stateful [`encode_term`] wrapper that DETECTS collisions over the set of
/// terms it has encoded, converting the encoder's statistical birthday bound into
/// a checkable, fail-closed precondition for the **concrete** key domain a holder
/// is about to secret-share.
///
/// Drive every join key for one holder through a single `KeyEncoder`; if any two
/// distinct terms collide it returns [`EncodeError::Collision`] (the false-match
/// soundness break) instead of a key, so the caller stops BEFORE the bad key
/// crosses the wire. With no collision the recorded map is also a proof obligation
/// the M4 in-circuit encoding-correctness check will later discharge.
/// `Debug` is hand-written: the derive printed the whole `key -> term` map, which
/// is every join key this holder has encoded together with the term that produced
/// it — the entire key domain in one log line.
///
/// The rendering carries no entry count either. The map is keyed by the ENCODED
/// key, so duplicate terms collapse into one entry: the count is the number of
/// DISTINCT keys, a statistic over the holder's data rather than the row count the
/// hidden-value join already discloses.
#[derive(Default, Clone)]
pub struct KeyEncoder {
    /// `key -> the (first) term that produced it`, for collision detection.
    seen: HashMap<u64, Term>,
}

impl std::fmt::Debug for KeyEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEncoder")
            .field("seen", &"<redacted>")
            .finish()
    }
}

impl KeyEncoder {
    /// A fresh encoder with an empty seen-set.
    pub fn new() -> Self {
        KeyEncoder {
            seen: HashMap::new(),
        }
    }

    /// Encode `term`, recording it for collision detection. Returns the encoded
    /// [`Fp`] on success, or [`EncodeError::Collision`] if a DISTINCT term already
    /// encoded to the same field element.
    ///
    /// Encoding the SAME term twice is NOT a collision — it returns the same key
    /// (the encoding is deterministic), which is exactly the equi-join behaviour a
    /// holder wants when a key recurs across its rows.
    pub fn encode(&mut self, term: &Term) -> Result<Fp, EncodeError> {
        let key = encode_term(term);
        match self.seen.get(&key.value()) {
            Some(prev) if prev != term => Err(EncodeError::Collision(Box::new(Collision {
                key,
                first: prev.clone(),
                second: term.clone(),
            }))),
            Some(_) => Ok(key), // same term re-encoded: deterministic, fine.
            None => {
                self.seen.insert(key.value(), term.clone());
                Ok(key)
            }
        }
    }

    /// How many DISTINCT terms have been encoded so far (the size `q` against
    /// which the birthday bound `≈ q²/2^62` is evaluated).
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether no term has been encoded yet.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};
    use sha2::Sha256;

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new(s).unwrap())
    }

    // === Determinism: the same term always encodes to the same key (equi-join
    // semantics depend on this) and the key is canonical in [0, P). ===
    #[test]
    fn encode_is_deterministic_and_canonical() {
        let t = iri("http://example.org/alice");
        let a = encode_term(&t);
        let b = encode_term(&t);
        assert_eq!(a, b, "encoding must be deterministic");
        assert!(a.value() < P, "encoded key must be canonical in [0, P)");
    }

    // === Variant disambiguation: an IRI <x> and a plain literal "x" with the
    // same INNER bytes must NOT collide — the false-match the N-Triples domain
    // separation is there to prevent. ===
    #[test]
    fn iri_and_literal_with_same_inner_bytes_differ() {
        let as_iri = iri("urn:x");
        let as_lit = Term::Literal(Literal::new_simple_literal("urn:x"));
        assert_ne!(
            encode_term(&as_iri),
            encode_term(&as_lit),
            "an IRI and a literal sharing inner text must encode differently"
        );
    }

    // === Datatype / language tags participate in the key: terms that differ
    // ONLY in datatype or language must encode differently (they are distinct
    // RDF terms, so a hidden join must NOT match them). ===
    #[test]
    fn datatype_and_language_distinguish_literals() {
        let plain = Term::Literal(Literal::new_simple_literal("1"));
        let typed = Term::Literal(Literal::new_typed_literal(
            "1",
            NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
        ));
        let lang_en = Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap());
        let lang_fr = Term::Literal(Literal::new_language_tagged_literal("hi", "fr").unwrap());
        let keys = [
            encode_term(&plain),
            encode_term(&typed),
            encode_term(&lang_en),
            encode_term(&lang_fr),
        ];
        for i in 0..keys.len() {
            for j in (i + 1)..keys.len() {
                assert_ne!(keys[i], keys[j], "distinct literals {i} vs {j} collided");
            }
        }
    }

    // === The KeyEncoder treats a repeated SAME term as a non-collision and
    // returns the same key (recurring equi-join key), and counts DISTINCT terms. ===
    #[test]
    fn same_term_reencoded_is_not_a_collision() {
        let mut enc = KeyEncoder::new();
        let t = iri("http://example.org/k");
        let k1 = enc.encode(&t).expect("first encode");
        let k2 = enc.encode(&t).expect("re-encode same term is fine");
        assert_eq!(k1, k2);
        assert_eq!(
            enc.len(),
            1,
            "re-encoding the same term adds no distinct key"
        );
    }

    // === NEAR-COLLISION test (the bead's required test): a large batch of
    // distinct, ADVERSARIALLY near-identical terms (URIs differing by one byte,
    // sequential indices, shared long prefixes) must produce all-distinct keys
    // with NO collision detected — the property the hidden join's soundness
    // rests on, exercised on the exact input shape that would break a naive
    // (e.g. truncating / prefix-folding) encoder. ===
    #[test]
    fn near_collision_batch_has_no_collisions() {
        let mut enc = KeyEncoder::new();
        let mut count = 0usize;
        // Adversarial near-identical families: long shared prefix, one varying
        // char; sequential integers; same digits in different term kinds.
        for i in 0..5000u32 {
            let prefix = "http://example.org/very/long/shared/namespace/path#node";
            // (a) IRIs differing only in a trailing index.
            enc.encode(&iri(&format!("{prefix}{i}")))
                .unwrap_or_else(|e| panic!("collision in IRI family: {e}"));
            // (b) IRIs differing only in ONE flipped byte at a fixed position.
            enc.encode(&iri(&format!("{prefix}-{:08x}", i ^ 0x5a5a)))
                .unwrap_or_else(|e| panic!("collision in flipped-byte family: {e}"));
            // (c) integer literals with the same lexical text as the IRI index.
            enc.encode(&Term::Literal(Literal::new_typed_literal(
                i.to_string(),
                NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
            )))
            .unwrap_or_else(|e| panic!("collision in integer-literal family: {e}"));
            count += 3;
        }
        // All distinct terms encoded; the seen-set size equals the number fed in
        // (no two distinct terms shared a key).
        assert_eq!(
            enc.len(),
            count,
            "near-collision batch produced a Fp collision: {} distinct terms but \
             only {} distinct keys",
            count,
            enc.len()
        );
    }

    // === The collision-detection path FIRES when fed a genuine collision: we
    // can't cheaply force a SHA-512 collision, so we assert the mechanism by
    // pre-seeding the encoder's seen-map with a DIFFERENT term at the key a real
    // term will hash to, then confirm the second encode is reported as a
    // collision (fail-closed), and that the same term at that key is NOT. ===
    #[test]
    fn collision_path_fires_and_is_fail_closed() {
        let real = iri("http://example.org/real");
        let key = encode_term(&real);
        let mut enc = KeyEncoder::new();
        // Pre-seed: pretend a DIFFERENT term already occupies `real`'s key slot.
        let other = iri("http://example.org/other-pre-seeded");
        enc.seen.insert(key.value(), other.clone());
        // Encoding `real` now must be reported as a collision against `other`.
        match enc.encode(&real) {
            Err(EncodeError::Collision(c)) => {
                assert_eq!(c.key, key);
                assert_eq!(c.first, other);
                assert_eq!(c.second, real);
            }
            Ok(_) => panic!("collision-detection path failed to fire"),
        }
        // Display is informative (exercises the Error/Display impls).
        let err = enc.encode(&real).unwrap_err();
        assert!(err.to_string().contains("collision"));
    }

    // === Blank-node labels participate in the key per the documented caveat:
    // distinct labels → distinct keys; identical labels → identical keys (SPARQL
    // term-equality for blank nodes). ===
    #[test]
    fn blank_nodes_keyed_by_label() {
        let b1 = Term::BlankNode(BlankNode::new("b1").unwrap());
        let b1_again = Term::BlankNode(BlankNode::new("b1").unwrap());
        let b2 = Term::BlankNode(BlankNode::new("b2").unwrap());
        assert_eq!(encode_term(&b1), encode_term(&b1_again));
        assert_ne!(encode_term(&b1), encode_term(&b2));
        // And a blank node never collides with a same-text literal.
        assert_ne!(
            encode_term(&b1),
            encode_term(&Term::Literal(Literal::new_simple_literal("b1")))
        );
    }

    // === END-TO-END: the encoder is a SOUND key source for the REAL hidden
    // join. Driving HiddenValueJoin on `encode_term` keys must match EXACTLY the
    // pairs whose join TERMS are equal — no false match (the soundness property
    // the encoder underwrites), no missed match. This wires the encoder into the
    // actual M3 secret-shared-equality path, not just standalone hashing. ===
    #[test]
    fn encoder_keys_drive_hidden_join_matching_on_term_equality() {
        use crate::join::{HiddenKeyedRows, HiddenValueJoin};
        use crate::partial::HolderId;
        use crate::shamir::ShamirBackend;
        use oxrdf::Variable;

        let lit = |s: &str| Some(Term::Literal(Literal::new_simple_literal(s)));
        // Left holder keys on person IRIs; right holder keys on the SAME IRI space.
        // Only the shared IRI <…/bob> should join.
        let alice = iri("http://example.org/people/alice");
        let bob = iri("http://example.org/people/bob");
        let carol = iri("http://example.org/people/carol");

        let left = HiddenKeyedRows {
            holder: HolderId::new("L"),
            payload_vars: vec![Variable::new_unchecked("name")],
            rows: vec![
                (encode_term(&alice), vec![lit("Alice")]),
                (encode_term(&bob), vec![lit("Bob")]),
            ],
        };
        let right = HiddenKeyedRows {
            holder: HolderId::new("R"),
            payload_vars: vec![Variable::new_unchecked("city")],
            rows: vec![
                (encode_term(&bob), vec![lit("Leeds")]),
                (encode_term(&carol), vec![lit("Paris")]),
            ],
        };
        let join = HiddenValueJoin::new(ShamirBackend::new_seeded(3, 0xD181).unwrap());
        let got = join.join(&left, &right).unwrap();
        // Exactly the <…/bob> pair joins (equal encoded keys ⇔ equal terms).
        assert_eq!(got.rows.len(), 1, "only the equal-term pair must match");
        let rendered = format!("{:?}", got.rows);
        assert!(rendered.contains("Bob") && rendered.contains("Leeds"));
        // And the disclosed-different terms (Alice/Paris) do NOT appear.
        assert!(!rendered.contains("Alice") && !rendered.contains("Paris"));
    }

    // === Domain separation: the tag actually prefixes the pre-image, so a bare
    // SHA-512 of the N-Triples form (no tag) reduces to a DIFFERENT field
    // element. This guards against cross-protocol hash reuse. ===
    #[test]
    fn domain_tag_changes_the_output() {
        let t = iri("http://example.org/x");
        let with_tag = encode_term(&t);
        // Recompute WITHOUT the domain tag.
        let mut h = Sha512::new();
        h.update(t.to_string().as_bytes());
        let d = h.finalize();
        let mut be = [0u8; 16];
        be.copy_from_slice(&d[..16]);
        let without_tag = Fp::new((u128::from_be_bytes(be) % (P as u128)) as u64);
        assert_ne!(
            with_tag, without_tag,
            "the domain tag must affect the encoded key"
        );
    }

    // === Byte-stability known-answer (KAT) guard. sq-jkcj.
    //
    // The `sha2` 0.10 -> 0.11 dependency bump changed the crate's *API* (the
    // `Digest`/`Output` types: `GenericArray` -> `hybrid-array::Array`), NOT the
    // SHA-2 *algorithm*. The other determinism tests above check only RELATIONAL
    // properties (same term => same key; distinct terms => distinct keys), which a
    // silent change in the underlying hash bytes would still satisfy. This test
    // pins the EXACT output bytes so any future drift in the hash line, the
    // `encode_term` construction, or oxrdf's N-Triples rendering fails closed.
    //
    // This is a dependency-maintenance regression guard. It is NOT a ZK/MPC
    // soundness or privacy claim: the hidden-value join's security rests on the
    // protocol and the (semi-honest, NOT externally audited -- see sq-qhy4)
    // analysis, not on these fixed vectors.
    #[test]
    fn sha2_byte_stability_known_answers() {
        use std::fmt::Write;
        // (1) Bare SHA-256 / SHA-512 algorithm-identity vectors (NIST / RFC 6234
        // empty-string KATs). These pin that the `sha2` crate behind the 0.11 API
        // still computes the standard SHA-2 cores.
        let mut hex256 = String::new();
        for b in <Sha256 as Digest>::digest(b"") {
            let _ = write!(hex256, "{:02x}", b);
        }
        assert_eq!(
            hex256, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "SHA-256(\"\") must be byte-identical across the sha2 0.10 -> 0.11 bump"
        );
        let mut hex512 = String::new();
        for b in <Sha512 as Digest>::digest(b"") {
            let _ = write!(hex512, "{:02x}", b);
        }
        assert_eq!(
            hex512,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce\
             47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
            "SHA-512(\"\") must be byte-identical across the sha2 0.10 -> 0.11 bump"
        );

        // (2) End-to-end `encode_term` field-element KATs for fixed terms. These
        // pin the full domain-separated construction
        // reduce_mod_p(SHA-512(DOMAIN_TAG || ntriples(term))) to exact values, so a
        // change to the tag, the truncation, the field reduction, or the oxrdf
        // N-Triples rendering is caught. Values computed independently (Python
        // hashlib) against the same construction.
        let cases: [(Term, u64); 4] = [
            (iri("http://example.org/alice"), 125_816_213_822_204_096),
            (
                Term::Literal(Literal::new_simple_literal("alice")),
                1_818_932_659_386_263_799,
            ),
            (
                Term::Literal(Literal::new_typed_literal(
                    "1",
                    NamedNode::new("http://www.w3.org/2001/XMLSchema#integer").unwrap(),
                )),
                672_429_347_926_891_200,
            ),
            (
                Term::Literal(Literal::new_language_tagged_literal("hi", "en").unwrap()),
                1_254_318_331_484_379_694,
            ),
        ];
        for (term, expected) in &cases {
            assert_eq!(
                encode_term(term).value(),
                *expected,
                "encode_term({}) drifted from its committed byte-stable value",
                term
            );
        }
    }

    /// A collision error renders NOTHING derived from the colliding terms — not
    /// the terms, not the key, and not a count or hash prefix of either — in
    /// `Display` or in `Debug`. The payload keeps both terms as public fields for
    /// the holder that must resolve the collision.
    #[test]
    fn collision_error_renders_no_term_or_key_content() {
        let real = iri("http://example.org/patients/alice-444555666");
        let key = encode_term(&real);
        let mut enc = KeyEncoder::new();
        let other = iri("http://example.org/patients/bob-777888999");
        enc.seen.insert(key.value(), other.clone());
        let err = enc.encode(&real).unwrap_err();

        for rendered in [format!("{}", err), format!("{:?}", err)] {
            assert!(
                !rendered.contains("444555666") && !rendered.contains("777888999"),
                "a colliding term's content must not be rendered: {}",
                rendered
            );
            assert!(
                !rendered.contains("alice") && !rendered.contains("bob"),
                "nor any part of it: {}",
                rendered
            );
            assert!(
                !rendered.contains(&key.value().to_string()),
                "nor the colliding key, which is a function of both terms: {}",
                rendered
            );
            assert!(
                rendered.contains("collision"),
                "the condition is still named: {}",
                rendered
            );
        }

        // Same for the payload rendered on its own, and the fields are still there.
        let EncodeError::Collision(c) = &err;
        for rendered in [format!("{}", c), format!("{:?}", c)] {
            assert!(
                !rendered.contains("444555666") && !rendered.contains("777888999"),
                "{}",
                rendered
            );
            assert!(
                !rendered.contains(&c.key.value().to_string()),
                "{}",
                rendered
            );
        }
        assert_eq!(c.first, other);
        assert_eq!(c.second, real);
        assert_eq!(c.key, key);
    }
}
