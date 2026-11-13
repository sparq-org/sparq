// Host-only issuance measurements. The model's guest code is unchanged.
use ed25519_dalek::Signer;
use sha2::{Digest, Sha256};
use sparq_proved_evaluator_model::{
    authenticated_rdf::{Cryptosuite, SignedCredential},
    merkle_suite,
};
use std::time::Instant;

#[derive(Default)]
pub(super) struct Timings {
    pub inclusive_ns: u128,
    pub signature_ns: u128,
    pub tree_ns: Option<u128>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Canonicalize,
    Tree,
    Prepare,
    Signature,
    Encode,
}

trait Stages {
    fn run<T>(&mut self, stage: Stage, work: impl FnOnce() -> T) -> T;
}

impl Stages for Timings {
    fn run<T>(&mut self, stage: Stage, work: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let result = work();
        let ns = start.elapsed().as_nanos();
        match stage {
            Stage::Signature => self.signature_ns += ns,
            Stage::Tree => *self.tree_ns.get_or_insert(0) += ns,
            _ => {}
        }
        result
    }
}

pub(super) fn issue(
    document: &str,
    config: &str,
    suite: Cryptosuite,
    salt: [u8; 32],
) -> (SignedCredential, Timings) {
    let start = Instant::now();
    let key = super::key();
    let mut timings = Timings::default();
    let credential = issue_staged(document, config, suite, salt, &key, &mut timings);
    timings.inclusive_ns = start.elapsed().as_nanos();
    (credential, timings)
}

fn issue_staged(
    document: &str,
    config: &str,
    suite: Cryptosuite,
    salt: [u8; 32],
    key: &impl Signer<ed25519_dalek::Signature>,
    stages: &mut impl Stages,
) -> SignedCredential {
    let (canonical_document, canonical_config) = stages.run(Stage::Canonicalize, || {
        (
            sparq_canon::canonicalize_nquads(document).unwrap(),
            sparq_canon::canonicalize_nquads(config).unwrap(),
        )
    });
    let (issued_document, issued_config, message) = match suite {
        Cryptosuite::EddsaRdfc2022 => stages.run(Stage::Prepare, || {
            let config_hash = Sha256::digest(&canonical_config);
            let document_hash = Sha256::digest(&canonical_document);
            (
                document.to_owned(),
                config.to_owned(),
                [config_hash.as_slice(), document_hash.as_slice()].concat(),
            )
        }),
        Cryptosuite::EddsaSha256Merkle2026 => {
            // Parsing is preparation. The tree sample starts with parsed,
            // canonical quads and ends at the root, including leaf ordering.
            let quads = stages.run(Stage::Prepare, || {
                sparq_canon::parse_nquads(&canonical_document).unwrap()
            });
            let (leaves, root) = stages.run(Stage::Tree, || {
                let mut leaves: Vec<_> = quads
                    .iter()
                    .zip(canonical_document.lines())
                    .map(|(quad, line)| (merkle_suite::leaf(quad).unwrap(), line))
                    .collect();
                leaves.sort_unstable();
                let row: Vec<_> = leaves.iter().map(|(leaf, _)| *leaf).collect();
                let root = merkle_suite::root(&row).unwrap();
                (leaves, root)
            });
            stages.run(Stage::Prepare, || {
                let config_digest = Sha256::digest(&canonical_config).into();
                let message =
                    merkle_suite::signed_message(&salt, leaves.len() as u32, &root, &config_digest);
                let document = leaves.iter().map(|(_, line)| format!("{line}\n")).collect();
                (document, canonical_config, message.to_vec())
            })
        }
    };
    // Only the primitive signs within this interval. Key setup, hashing,
    // message preparation, signature byte conversion and salt framing are outside.
    let signature = stages.run(Stage::Signature, || key.sign(&message));
    stages.run(Stage::Encode, || {
        let mut signature = signature.to_bytes().to_vec();
        if suite == Cryptosuite::EddsaSha256Merkle2026 {
            signature.extend_from_slice(&salt);
        }
        SignedCredential {
            document: issued_document,
            proof_config: issued_config,
            signature,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Trace {
        stages: Vec<Stage>,
        active: std::rc::Rc<std::cell::Cell<Option<Stage>>>,
    }
    struct ObservedSigner<'a> {
        key: &'a ed25519_dalek::SigningKey,
        active: std::rc::Rc<std::cell::Cell<Option<Stage>>>,
    }
    impl Signer<ed25519_dalek::Signature> for ObservedSigner<'_> {
        fn try_sign(
            &self,
            message: &[u8],
        ) -> Result<ed25519_dalek::Signature, ed25519_dalek::SignatureError> {
            assert_eq!(self.active.get(), Some(Stage::Signature));
            self.key.try_sign(message)
        }
    }
    impl Stages for Trace {
        fn run<T>(&mut self, stage: Stage, work: impl FnOnce() -> T) -> T {
            assert!(self.active.replace(Some(stage)).is_none());
            self.stages.push(stage);
            let result = work();
            self.active.set(None);
            result
        }
    }

    #[test]
    fn issuance_boundaries_and_complete_credential_encoding_match_both_suites() {
        for suite in [
            Cryptosuite::EddsaRdfc2022,
            Cryptosuite::EddsaSha256Merkle2026,
        ] {
            let doc = super::super::payment_document(0);
            let config = super::super::proof_config(super::super::PAYMENT_VM, suite);
            let key = super::super::key();
            let salt = [0x40; 32];
            let mut trace = Trace::default();
            let observed = ObservedSigner {
                key: &key,
                active: trace.active.clone(),
            };
            let issued = issue_staged(&doc, &config, suite, salt, &observed, &mut trace);
            let expected = if suite == Cryptosuite::EddsaSha256Merkle2026 {
                assert_eq!(
                    trace.stages,
                    [
                        Stage::Canonicalize,
                        Stage::Prepare,
                        Stage::Tree,
                        Stage::Prepare,
                        Stage::Signature,
                        Stage::Encode
                    ]
                );
                merkle_suite::issue(&doc, &config, &key, salt).unwrap()
            } else {
                assert_eq!(
                    trace.stages,
                    [
                        Stage::Canonicalize,
                        Stage::Prepare,
                        Stage::Signature,
                        Stage::Encode
                    ]
                );
                let message = [
                    Sha256::digest(sparq_canon::canonicalize_nquads(&config).unwrap()).to_vec(),
                    Sha256::digest(sparq_canon::canonicalize_nquads(&doc).unwrap()).to_vec(),
                ]
                .concat();
                SignedCredential {
                    document: doc,
                    proof_config: config,
                    signature: key.sign(&message).to_bytes().to_vec(),
                }
            };
            let encoded = serde_json::to_vec(&issued).unwrap();
            assert_eq!(encoded, serde_json::to_vec(&expected).unwrap());
            let decoded: SignedCredential = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded.signature, expected.signature);
            assert_eq!(decoded.document, expected.document);
            assert_eq!(decoded.proof_config, expected.proof_config);
            assert_eq!(super::super::credential_encoded_len(&issued), encoded.len());
            assert!(
                encoded.len()
                    > issued.document.len() + issued.proof_config.len() + issued.signature.len()
            );
            let (_, timing) = issue(&issued.document, &issued.proof_config, suite, salt);
            assert_eq!(
                timing.tree_ns.is_some(),
                suite == Cryptosuite::EddsaSha256Merkle2026
            );
            assert!(timing.signature_ns > 0);
            if let Some(ns) = timing.tree_ns {
                assert!(ns > 0);
            }
            assert!(timing.inclusive_ns >= timing.signature_ns + timing.tree_ns.unwrap_or(0));
        }
    }
}
