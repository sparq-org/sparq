// [OPUS-5] sq-txg1y (issue #3234) — follow-up to sq-9c5e.
//! **Additive** JSON-LD VC *envelope* entry point layered on top of the RDF-native
//! [`crate::vc_bridge`] (design `research/zk-configurable-commitment-design.md` §5).
//!
//! # What this adds, and what it deliberately does not
//! [`crate::vc_bridge`] takes the credential's **RDF** (triples) plus raw key and
//! signature bytes, because that is the form the W3C Data-Integrity `rdfc` suites
//! actually hash. Every real VC, though, arrives as a **JSON-LD document with an
//! attached `proof`**, and turning one into those arguments is a fiddly, easy-to-
//! get-subtly-wrong ritual: split the `proof` off, rebuild the proof
//! configuration, copy the document's `@context` onto it, multibase-decode
//! `proofValue`, expand both halves to RDF. This module does exactly that ritual
//! and nothing else — it then hands the results straight to
//! [`verify_source_proof_by_token`].
//!
//! It is **strictly additive**: the RDF-native functions are unchanged and remain
//! the entry point for a caller who already has the credential's triples. Nothing
//! here weakens or bypasses a check — [`verify_vc_json`] performs the *same*
//! off-circuit verification, just from a different starting shape.
//!
//! ```text
//! VC JSON-LD document
//!    ├─ split           → unsecured document | proof
//!    ├─ proof config    = proof − proofValue, @context ← document.@context   (DI §3.2.2)
//!    ├─ multibase       → proofValue (`z`, base58-btc) → raw signature bytes
//!    ├─ expand (oxjsonld, caller-supplied contexts, NO network) → RDF triples
//!    └─ verify_source_proof_by_token(credential, proof_config, cryptosuite, pk, sig)
//! ```
//!
//! # No ambient network — the `contexts` allowlist (load-bearing)
//! `oxjsonld` is driven with a `LoadDocumentCallback` backed **exclusively** by the
//! caller-supplied `contexts` slice, and a referenced `@context` URL that is not in
//! it is refused by name. That slice *is* the allowlist: this crate opens no socket
//! and links no HTTP client, matching `sparq_jsonld::NoopLoader`'s deny-by-default
//! posture and sparq-wasm's `jsonld-contexts` binding. Practically this means a VC
//! quoting `"@context": "https://www.w3.org/ns/credentials/v2"` verifies here only
//! if the host passes that context document in — resolving it is the host's policy
//! decision, not this module's.
//!
//! Honest consequence: because the RDF is produced by *this* expansion, an
//! attacker-supplied context document changes the bytes that get hashed. Supplying
//! a context you do not trust is equivalent to supplying a credential you do not
//! trust — the DI signature is still checked, so a mismatched context makes
//! verification **fail**, it does not make a forged credential pass.
//!
//! # Honest scope boundary
//! - **Default graph only.** The credential and the proof config must expand to
//!   triples in the default graph. A `@graph` container that yields quads in a
//!   *named* graph is refused ([`VcBridgeError::NamedGraphUnsupported`]) rather
//!   than flattened, because flattening would change the very N-Quads the DI hash
//!   covers. (Strip/normalise such a document before calling.)
//! - **One `proof`.** A `proof` array (proof sets / chains) is refused; the bridge
//!   verifies a single Data-Integrity proof.
//! - **`z` multibase only.** `proofValue` must be base58-btc, which is what
//!   `eddsa-rdfc-2022` and `ecdsa-rdfc-2019` specify. Any other multibase prefix
//!   is refused rather than guessed at.
//! - **No `did:` resolution.** As in [`crate::vc_bridge`], the caller supplies the
//!   *resolved* issuer key bytes. `proof.verificationMethod` is returned verbatim
//!   in the [`VcEnvelope`] so the caller can check it names the key it resolved —
//!   this module does **not** check that binding for you.
//! - **`proof.type` is not pre-checked**, deliberately. It is *covered* by the
//!   signature: `type` expands into the proof configuration's RDF, so altering it
//!   changes `proofConfigHash` and verification fails. Re-checking the string
//!   before expansion would add a second, weaker copy of a check the hash already
//!   makes, and would reject documents whose context aliases the term differently.
//! - **Not externally audited** (sq-qhy4). Off-circuit, ingest-time host
//!   verification; asserts no in-circuit or query-soundness property.

use crate::vc_bridge::{verify_source_proof_by_token, VcBridgeError};
use oxrdf::{GraphName, Triple};
use serde_json::{Map, Value};

/// The JSON key holding the Data-Integrity proof on a secured VC.
const PROOF_KEY: &str = "proof";
/// The JSON key holding the multibase-encoded signature inside a proof.
const PROOF_VALUE_KEY: &str = "proofValue";
/// The `@context` key, copied from the document onto the proof configuration.
const CONTEXT_KEY: &str = "@context";

/// A DI-secured VC JSON document decomposed into everything the RDF-native bridge
/// needs, plus the proof metadata a caller must inspect for itself.
///
/// Produced by [`parse_vc_json`]; the fields are exactly the arguments of
/// [`verify_source_proof_by_token`] plus `verification_method`.
#[derive(Debug, Clone)]
pub struct VcEnvelope {
    /// The **unsecured document** (the VC with `proof` removed) expanded to RDF.
    /// This is the `credential` argument of the RDF-native bridge, and the graph a
    /// caller re-commits with [`crate::vc_bridge::ingest_verified_vc`].
    pub credential: Vec<Triple>,
    /// The **proof configuration** — the proof node without `proofValue`, carrying
    /// the document's `@context` — expanded to RDF (DI §3.2.2).
    pub proof_config: Vec<Triple>,
    /// The verbatim `proof.cryptosuite` token, NOT yet resolved to a
    /// [`crate::vc_bridge::VcCryptosuite`] (resolution is fail-closed and happens
    /// inside [`verify_source_proof_by_token`], so an out-of-scope suite like
    /// `bbs-2023` parses here and is rejected there).
    pub cryptosuite: String,
    /// The verbatim `proof.verificationMethod` IRI. **The caller must check this
    /// names the key it resolved** — this module performs no `did:` dereference
    /// and no binding check.
    pub verification_method: String,
    /// The multibase-decoded `proofValue`: the raw signature bytes.
    pub signature: Vec<u8>,
}

/// Split a DI-secured VC JSON-LD document into a [`VcEnvelope`] **without
/// verifying anything**.
///
/// `contexts` is the `@context`-URL allowlist: an ordered slice of
/// `(url, context_document_text)` pairs the host has already retrieved. A URL the
/// document references but the slice does not carry is refused by name; nothing
/// here touches the network. A duplicate URL takes its FIRST value.
///
/// Every failure is a fail-closed `Err`, never a panic: malformed JSON, a missing
/// or non-object `proof`, a `proof` array, a missing `cryptosuite` /
/// `verificationMethod` / `proofValue`, a non-`z` multibase, an unavailable
/// `@context`, or an expansion that escapes the default graph.
///
/// Use this when you want the pieces (e.g. to re-commit via
/// [`crate::vc_bridge::ingest_verified_vc`]); use [`verify_vc_json`] when you want
/// the verification too.
pub fn parse_vc_json(
    vc_json: &str,
    contexts: &[(&str, &str)],
) -> Result<VcEnvelope, VcBridgeError> {
    let document: Value = serde_json::from_str(vc_json)
        .map_err(|e| VcBridgeError::MalformedVcJson(format!("not valid JSON: {}", e)))?;
    let mut document = match document {
        Value::Object(map) => map,
        _ => {
            return Err(VcBridgeError::MalformedVcJson(
                "the VC document must be a JSON object".to_string(),
            ))
        }
    };

    // 1. Split the proof off. `remove` is what makes the remainder the *unsecured*
    //    document — the DI transform hashes the credential WITHOUT its proof.
    let proof = document.remove(PROOF_KEY).ok_or_else(|| {
        VcBridgeError::MalformedVcJson("the VC document has no `proof`".to_string())
    })?;
    let mut proof = match proof {
        Value::Object(map) => map,
        // A proof SET/CHAIN is a real DI feature and deliberately out of scope:
        // verifying "the" proof of a document with several is a policy choice
        // (which one? all? in what order?) this bridge does not make for the caller.
        Value::Array(_) => {
            return Err(VcBridgeError::MalformedVcJson(
                "`proof` is an array (proof set/chain); this bridge verifies a single \
                 Data-Integrity proof — select one and present it as an object"
                    .to_string(),
            ))
        }
        _ => {
            return Err(VcBridgeError::MalformedVcJson(
                "`proof` must be a JSON object".to_string(),
            ))
        }
    };

    // 2. Read the proof metadata, then remove `proofValue` — the signature is not
    //    part of what the signature covers.
    let cryptosuite = required_string(&proof, "cryptosuite")?;
    let verification_method = required_string(&proof, "verificationMethod")?;
    let proof_value = match proof.remove(PROOF_VALUE_KEY) {
        Some(Value::String(s)) => s,
        Some(_) => {
            return Err(VcBridgeError::MalformedVcJson(
                "`proof.proofValue` must be a string".to_string(),
            ))
        }
        None => {
            return Err(VcBridgeError::MalformedVcJson(
                "`proof.proofValue` is missing".to_string(),
            ))
        }
    };
    let signature = decode_multibase_base58btc(&proof_value)?;

    // 3. DI §3.2.2 step 4: the proof configuration takes the *document's*
    //    `@context`, replacing whatever the proof node carried. Without this the
    //    proof config expands under a different context and its canonical N-Quads
    //    — hence `proofConfigHash` — differ from what the issuer signed.
    match document.get(CONTEXT_KEY) {
        Some(ctx) => {
            proof.insert(CONTEXT_KEY.to_string(), ctx.clone());
        }
        None => {
            proof.remove(CONTEXT_KEY);
        }
    }

    Ok(VcEnvelope {
        credential: expand_to_triples(&Value::Object(document), contexts, "credential")?,
        proof_config: expand_to_triples(&Value::Object(proof), contexts, "proof configuration")?,
        cryptosuite,
        verification_method,
        signature,
    })
}

/// Parse a DI-secured VC JSON-LD document **and** verify its Data-Integrity proof
/// off-circuit under `issuer_pk`.
///
/// Exactly [`parse_vc_json`] followed by [`verify_source_proof_by_token`] — the
/// same fail-closed RDF-native verification the bridge has always done, reached
/// from a JSON envelope instead of from triples. In particular the
/// `ecdsa-rdfc-2019` curve profile (P-256/SHA-256 vs P-384/SHA-384) is still
/// resolved from `issuer_pk`, not from anything in the JSON.
///
/// `issuer_pk` is the **already-resolved** verification key (Ed25519 32B; ECDSA
/// SEC1). Check [`VcEnvelope::verification_method`] on the returned envelope names
/// the key you resolved — this function does not, and a proof that verifies under
/// a key you chose says nothing about the issuer you *expected*.
///
/// Returns the envelope on success so the caller can go on to re-commit with
/// [`crate::vc_bridge::ingest_verified_vc`]. Asserts no in-circuit /
/// query-soundness property (sq-qhy4).
pub fn verify_vc_json(
    vc_json: &str,
    contexts: &[(&str, &str)],
    issuer_pk: &[u8],
) -> Result<VcEnvelope, VcBridgeError> {
    let envelope = parse_vc_json(vc_json, contexts)?;
    verify_source_proof_by_token(
        &envelope.credential,
        &envelope.proof_config,
        &envelope.cryptosuite,
        issuer_pk,
        &envelope.signature,
    )?;
    Ok(envelope)
}

/// Read a required string member of the proof object, fail-closed on absent or
/// wrong-typed.
fn required_string(proof: &Map<String, Value>, key: &str) -> Result<String, VcBridgeError> {
    match proof.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(_) => Err(VcBridgeError::MalformedVcJson(format!(
            "`proof.{}` must be a string",
            key
        ))),
        None => Err(VcBridgeError::MalformedVcJson(format!(
            "`proof.{}` is missing",
            key
        ))),
    }
}

/// Decode a multibase `proofValue`. Only `z` (base58-btc) is accepted — the
/// encoding both in-scope `rdfc` suites specify. Any other prefix is refused
/// rather than guessed at, because guessing an alphabet silently changes the
/// signature bytes.
fn decode_multibase_base58btc(proof_value: &str) -> Result<Vec<u8>, VcBridgeError> {
    let mut chars = proof_value.chars();
    match chars.next() {
        Some('z') => {}
        Some(other) => {
            return Err(VcBridgeError::MalformedVcJson(format!(
                "`proof.proofValue` uses multibase prefix {:?}; only `z` (base58-btc) is \
                 implemented for the `rdfc` suites",
                other
            )))
        }
        None => {
            return Err(VcBridgeError::MalformedVcJson(
                "`proof.proofValue` is empty".to_string(),
            ))
        }
    }
    bs58::decode(chars.as_str())
        .into_vec()
        .map_err(|e| VcBridgeError::MalformedVcJson(format!("`proof.proofValue`: {}", e)))
}

/// Expand a JSON-LD document to the RDF triples the DI transform canonicalises.
///
/// Remote `@context` resolution is restricted to `contexts` (see the module docs);
/// `what` names the half being expanded so an error says which one failed.
fn expand_to_triples(
    document: &Value,
    contexts: &[(&str, &str)],
    what: &str,
) -> Result<Vec<Triple>, VcBridgeError> {
    // The callback must be `'static`, so the allowlist is copied into it.
    let allowlist: Vec<(String, String)> = contexts
        .iter()
        .map(|(url, doc)| ((*url).to_string(), (*doc).to_string()))
        .collect();

    let json = serde_json::to_vec(document).map_err(|e| {
        VcBridgeError::MalformedVcJson(format!("could not re-serialize the {}: {}", what, e))
    })?;

    let parser = oxjsonld::JsonLdParser::new()
        .for_slice(&json)
        .with_load_document_callback(move |url, _options| {
            match allowlist.iter().find(|(iri, _)| iri == url) {
                Some((iri, document)) => Ok(oxjsonld::JsonLdRemoteDocument {
                    document: document.as_bytes().to_vec(),
                    document_url: iri.clone(),
                }),
                // Fail-closed: the supplied slice IS the allowlist. Name the URL so
                // the caller knows exactly which context it has to pass in.
                None => Err(format!(
                    "remote @context <{}> was not supplied to the VC bridge (it performs no \
                     network access; retrieve the context yourself and pass it in `contexts`)",
                    url
                )
                .into()),
            }
        });

    let mut triples = Vec::new();
    for quad in parser {
        let quad = quad
            .map_err(|e| VcBridgeError::JsonLdExpansion(format!("expanding the {}: {}", what, e)))?;
        // Flattening a named graph into the default graph would change the very
        // N-Quads the DI hash covers, so refuse instead.
        if quad.graph_name != GraphName::DefaultGraph {
            return Err(VcBridgeError::NamedGraphUnsupported);
        }
        triples.push(Triple::new(quad.subject, quad.predicate, quad.object));
    }
    Ok(triples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vc_bridge::{
        hash_data_from_triples, hash_data_from_triples_sha384, ingest_verified_vc, VcCryptosuite,
    };
    use crate::encode::salt_from_bytes;
    use oxrdf::NamedNode;

    /// A self-contained JSON-LD `@context` for the fixtures below.
    ///
    /// Deliberately INLINE (not a URL) for most tests, so they exercise the
    /// expansion + hashing + verification path without any context-resolution
    /// policy in the way. It maps the Data-Integrity proof terms onto the real
    /// `https://w3id.org/security#` vocabulary the `rdfc` suites canonicalise, so
    /// the proof config expands to the same SHAPE the W3C vectors publish.
    const CONTEXT: &str = r#"{
        "id": "@id",
        "type": "@type",
        "sec": "https://w3id.org/security#",
        "xsd": "http://www.w3.org/2001/XMLSchema#",
        "DataIntegrityProof": "sec:DataIntegrityProof",
        "cryptosuite": "sec:cryptosuite",
        "proofPurpose": {"@id": "sec:proofPurpose", "@type": "@vocab"},
        "assertionMethod": "sec:assertionMethod",
        "verificationMethod": {"@id": "sec:verificationMethod", "@type": "@id"},
        "created": {"@id": "http://purl.org/dc/terms/created", "@type": "xsd:dateTime"},
        "alumniOf": "https://example.org/vc#alumniOf",
        "issuer": {"@id": "https://example.org/vc#issuer", "@type": "@id"},
        "credentialSubject": {"@id": "https://example.org/vc#credentialSubject", "@type": "@id"}
    }"#;

    /// The URL the by-reference variants quote for [`CONTEXT`].
    const CONTEXT_URL: &str = "https://example.org/contexts/test-v1";

    /// [`CONTEXT`] as a retrievable *context document* — a JSON-LD context
    /// document is the `{"@context": …}` wrapper, not the bare term map.
    fn context_document() -> String {
        format!(r#"{{"@context": {}}}"#, CONTEXT)
    }

    /// A DI-secured VC with an INLINE `@context`, parameterised by suite token and
    /// `proofValue` so a test can sign it for real.
    fn vc_json(context: &str, cryptosuite: &str, proof_value: &str) -> String {
        // Positional args (not inline captures) — the repo-wide convention that
        // dodges the CodeQL `rust/unused-variable` false positive.
        format!(
            r#"{{
              "@context": {},
              "id": "urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33",
              "issuer": "https://vc.example/issuers/5678",
              "credentialSubject": {{
                "id": "did:example:abcdefgh",
                "alumniOf": "The School of Examples"
              }},
              "proof": {{
                "type": "DataIntegrityProof",
                "cryptosuite": "{}",
                "created": "2023-02-24T23:36:38Z",
                "verificationMethod": "did:example:issuer#key-1",
                "proofPurpose": "assertionMethod",
                "proofValue": "{}"
              }}
            }}"#,
            context, cryptosuite, proof_value
        )
    }

    /// base58-btc encode, so a test can build a real `proofValue` from a real
    /// signature (the decode side is `bs58`, so this is a genuine round trip
    /// through the module's own decoder).
    fn multibase_z(bytes: &[u8]) -> String {
        format!("z{}", bs58::encode(bytes).into_string())
    }

    /// Sign the envelope's own `hashData` with a fresh Ed25519 key and rebuild the
    /// VC around the resulting `proofValue`. Returns `(vc_json, public_key)`.
    fn signed_eddsa_vc(context: &str) -> (String, Vec<u8>) {
        use ed25519_dalek::{Signer, SigningKey};
        // Parse once with a PLACEHOLDER proofValue purely to derive the triples;
        // `proofValue` is stripped before the proof config is built, so the
        // placeholder cannot affect the hashData.
        let draft = vc_json(context, "eddsa-rdfc-2022", &multibase_z(&[0u8; 64]));
        let ctx_doc = context_document();
        let env = parse_vc_json(&draft, &[(CONTEXT_URL, &ctx_doc)]).expect("draft must parse");
        let hd = hash_data_from_triples(&env.credential, &env.proof_config).unwrap();

        let sk = SigningKey::from_bytes(&[11u8; 32]);
        let sig = sk.sign(&hd);
        (
            vc_json(context, "eddsa-rdfc-2022", &multibase_z(&sig.to_bytes())),
            sk.verifying_key().as_bytes().to_vec(),
        )
    }

    // --- the happy path, end to end ---------------------------------------------

    #[test]
    fn signed_vc_json_verifies_and_reaches_the_commitment_pipeline() {
        let (vc, pk) = signed_eddsa_vc(CONTEXT);
        let env = verify_vc_json(&vc, &[], &pk).expect("a genuine VC envelope must verify");

        // The envelope carries the proof metadata verbatim, for the caller's own
        // verification-method binding check.
        assert_eq!(env.cryptosuite, "eddsa-rdfc-2022");
        assert_eq!(env.verification_method, "did:example:issuer#key-1");
        assert_eq!(env.signature.len(), 64);

        // The credential half is the UNSECURED document: the proof node's triples
        // must not be in it.
        assert!(
            !env.credential.iter().any(|t| t
                .predicate
                .as_str()
                .starts_with("https://w3id.org/security#")),
            "the proof must be split off before the credential is expanded"
        );
        assert!(!env.credential.is_empty() && !env.proof_config.is_empty());

        // ...and it feeds the RDF-native bridge unchanged.
        let ingested = ingest_verified_vc(
            NamedNode::new("urn:uuid:58172aac-d8ba-11ed-83dd-0b3aef56cc33").unwrap(),
            &env.credential,
            &env.proof_config,
            VcCryptosuite::EddsaRdfc2022,
            &pk,
            &env.signature,
            salt_from_bytes(&[5u8; 32]),
        )
        .expect("a verified JSON VC must ingest");
        assert_eq!(
            ingested.registry_entry().source_cryptosuite.as_deref(),
            Some("eddsa-rdfc-2022")
        );
    }

    /// The proof config must carry the DOCUMENT's `@context` (DI §3.2.2 step 4),
    /// so it expands to the security vocabulary rather than to nothing.
    #[test]
    fn proof_config_expands_under_the_documents_context() {
        let (vc, _) = signed_eddsa_vc(CONTEXT);
        let env = parse_vc_json(&vc, &[]).unwrap();
        let predicates: Vec<&str> = env
            .proof_config
            .iter()
            .map(|t| t.predicate.as_str())
            .collect();
        assert!(
            predicates.contains(&"https://w3id.org/security#cryptosuite"),
            "proof config predicates: {:?}",
            predicates
        );
        assert!(predicates.contains(&"https://w3id.org/security#verificationMethod"));
        assert!(predicates.contains(&"http://purl.org/dc/terms/created"));
        // `proofValue` is NOT part of what the signature covers.
        assert!(
            !predicates.iter().any(|p| p.ends_with("proofValue")),
            "proofValue must be stripped from the proof configuration"
        );
    }

    /// The P-384 profile reaches the same envelope path, and the curve is still
    /// resolved from the KEY — nothing in the JSON names it. [OPUS-5] sq-txg1y.
    #[test]
    fn ecdsa_p384_vc_json_verifies_through_the_envelope() {
        use p384::ecdsa::{signature::Signer, Signature, SigningKey};
        let draft = vc_json(CONTEXT, "ecdsa-rdfc-2019", &multibase_z(&[0u8; 96]));
        let env = parse_vc_json(&draft, &[]).unwrap();
        let hd = hash_data_from_triples_sha384(&env.credential, &env.proof_config).unwrap();

        let sk = SigningKey::from_slice(&[13u8; 48]).unwrap();
        let sig: Signature = sk.sign(&hd);
        let pk = sk.verifying_key().to_encoded_point(true).as_bytes().to_vec();
        let vc = vc_json(CONTEXT, "ecdsa-rdfc-2019", &multibase_z(&sig.to_bytes()));

        let verified = verify_vc_json(&vc, &[], &pk).expect("P-384 VC envelope must verify");
        assert_eq!(verified.signature.len(), 96);

        // Mutation: the SAME JSON under a P-256 key cannot verify (and the widths
        // no longer line up), so the profile really is key-driven here too.
        let p256_pk = p256::ecdsa::SigningKey::from_slice(&[13u8; 32])
            .unwrap()
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec();
        assert!(matches!(
            verify_vc_json(&vc, &[], &p256_pk),
            Err(VcBridgeError::MalformedSignature)
        ));
    }

    // --- the context allowlist ---------------------------------------------------

    #[test]
    fn context_by_url_resolves_only_from_the_allowlist() {
        // The same document, but quoting the context by URL.
        let (vc, pk) = signed_eddsa_vc(&format!("\"{}\"", CONTEXT_URL));

        // Supplied: verifies, and produces the SAME RDF as the inline form (the
        // context is the same document, so the hashed bytes must agree).
        let ctx_doc = context_document();
        let env = verify_vc_json(&vc, &[(CONTEXT_URL, &ctx_doc)], &pk)
            .expect("an allowlisted context must resolve");
        let (inline_vc, _) = signed_eddsa_vc(CONTEXT);
        let inline = parse_vc_json(&inline_vc, &[]).unwrap();
        assert_eq!(
            hash_data_from_triples(&env.credential, &env.proof_config).unwrap(),
            hash_data_from_triples(&inline.credential, &inline.proof_config).unwrap(),
            "by-URL and inline forms of the same context must hash identically"
        );

        // Not supplied: fail-closed, naming the URL. No network is attempted.
        let err = verify_vc_json(&vc, &[], &pk).expect_err("an unlisted context must fail closed");
        match err {
            VcBridgeError::JsonLdExpansion(msg) => assert!(
                msg.contains(CONTEXT_URL),
                "the error must name the missing context: {}",
                msg
            ),
            other => panic!("expected JsonLdExpansion, got {:?}", other),
        }
    }

    // --- fail-closed envelope parsing --------------------------------------------

    #[test]
    fn malformed_envelopes_fail_closed() {
        let cases: [(&str, &str); 7] = [
            ("not json at all", "not valid JSON"),
            ("[]", "must be a JSON object"),
            (r#"{"id": "urn:x"}"#, "no `proof`"),
            (r#"{"proof": [{}]}"#, "proof set/chain"),
            (r#"{"proof": "z1"}"#, "`proof` must be a JSON object"),
            (
                r#"{"proof": {"verificationMethod": "did:example:1", "proofValue": "z1"}}"#,
                "`proof.cryptosuite` is missing",
            ),
            (
                r#"{"proof": {"cryptosuite": "eddsa-rdfc-2022", "verificationMethod": "did:example:1"}}"#,
                "`proof.proofValue` is missing",
            ),
        ];
        for (json, expected) in cases {
            match parse_vc_json(json, &[]) {
                Err(VcBridgeError::MalformedVcJson(msg)) => assert!(
                    msg.contains(expected),
                    "for {:?}: expected a message containing {:?}, got {:?}",
                    json,
                    expected,
                    msg
                ),
                other => panic!("for {:?}: expected MalformedVcJson, got {:?}", json, other),
            }
        }
    }

    #[test]
    fn non_base58_multibase_proof_value_fails_closed() {
        // `u` is base64url in multibase — a real alphabet, but not the one the
        // `rdfc` suites use, so it is refused rather than decoded as base58.
        for pv in ["uAAAA", "f00ff", "", "z!!not-base58!!"] {
            let vc = vc_json(CONTEXT, "eddsa-rdfc-2022", pv);
            assert!(
                matches!(
                    parse_vc_json(&vc, &[]),
                    Err(VcBridgeError::MalformedVcJson(_))
                ),
                "proofValue {:?} must fail closed",
                pv
            );
        }
    }

    /// A tampered credential body must not verify — the whole point of routing
    /// through the RDF-native check rather than trusting the envelope.
    #[test]
    fn tampered_json_body_fails_closed() {
        let (vc, pk) = signed_eddsa_vc(CONTEXT);
        let tampered = vc.replace("The School of Examples", "The School of Forgeries");
        assert_ne!(vc, tampered, "the mutation must actually apply");
        assert!(matches!(
            verify_vc_json(&tampered, &[], &pk),
            Err(VcBridgeError::VerificationFailed)
        ));
    }

    /// An out-of-scope cryptosuite still fails closed through the envelope — the
    /// token is passed through verbatim and resolved by the RDF-native layer.
    #[test]
    fn out_of_scope_cryptosuite_fails_closed_through_the_envelope() {
        let vc = vc_json(CONTEXT, "bbs-2023", &multibase_z(&[0u8; 64]));
        assert!(matches!(
            verify_vc_json(&vc, &[], &[0u8; 32]),
            Err(VcBridgeError::UnsupportedCryptosuite(t)) if t == "bbs-2023"
        ));
    }

    /// A document whose expansion escapes the default graph is refused, not
    /// flattened — flattening would silently change the hashed N-Quads.
    #[test]
    fn named_graph_expansion_is_refused() {
        let vc = r#"{
          "@context": {"contains": {"@id": "https://example.org/vc#contains", "@container": "@graph"}},
          "@id": "urn:uuid:outer",
          "contains": {"@id": "urn:uuid:inner", "https://example.org/vc#p": "v"},
          "proof": {
            "cryptosuite": "eddsa-rdfc-2022",
            "verificationMethod": "did:example:issuer#key-1",
            "proofValue": "z11111111111111111111111111111111111111111111111111111111111111111111111111111111111111"
          }
        }"#;
        assert!(matches!(
            parse_vc_json(vc, &[]),
            Err(VcBridgeError::NamedGraphUnsupported)
        ));
    }
}
