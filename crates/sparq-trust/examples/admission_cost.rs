//! Deterministic admission and certification-closure cost evidence (`sq-r78pf`).
//!
//! This example deliberately records operation-independent counts only. It
//! contains no clock sampling, elapsed duration, allocator telemetry, or host metadata.

use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};
use sparq_trust::admit::{admit, PresentedCredential, Session};
use sparq_trust::graph::{
    certification_message, derive_effective_rules, explain_edge, CertScope, Certification,
    EdgeRejection,
};
use sparq_trust::policy::{ShapeRef, TrustRule};
use sparq_zk::sig::{PublicKey, SecretKey};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

const NOW: i64 = 1_700_000_000;
const FRESH: i64 = 86_400;
const TARGET: &str = "https://pod.example/resource";
const PREDICATE: &str = "https://schema.org/age";

fn iri(value: &str) -> NamedNode {
    NamedNode::new(value).expect("fixture IRIs are valid")
}

fn predicate_shape(predicate: &str, suffix: &str) -> ShapeRef {
    let root = BlankNode::new(format!("shape{suffix}")).expect("valid fixture blank node");
    let property = BlankNode::new(format!("property{suffix}")).expect("valid fixture blank node");
    ShapeRef {
        root: Term::BlankNode(root.clone()),
        triples: vec![
            Triple::new(
                root.clone(),
                iri("http://www.w3.org/ns/shacl#targetSubjectsOf"),
                iri(predicate),
            ),
            Triple::new(
                root,
                iri("http://www.w3.org/ns/shacl#property"),
                property.clone(),
            ),
            Triple::new(
                property.clone(),
                iri("http://www.w3.org/ns/shacl#path"),
                iri(predicate),
            ),
            Triple::new(
                property,
                iri("http://www.w3.org/ns/shacl#minCount"),
                Literal::new_simple_literal("1"),
            ),
        ],
    }
}

fn anchor(index: usize, key: PublicKey) -> TrustRule {
    TrustRule {
        source: iri(&format!("https://authority.example/{index}")),
        issuer_key: key,
        shape: predicate_shape(PREDICATE, &index.to_string()),
        scope: iri(TARGET),
        fresh_within_secs: FRESH,
    }
}

fn signed_cert(index: usize, signer: &SecretKey, certified_key: PublicKey) -> Certification {
    let mut cert = Certification {
        certifier: iri(&format!("https://authority.example/{index}")),
        certifier_key: signer.public_key(),
        certified_issuer: iri(&format!("https://issuer.example/{index}")),
        certified_key,
        scope: CertScope::AnyService,
        valid_from_unix_secs: NOW - 1,
        valid_until_unix_secs: NOW + FRESH,
        signature_hex: String::new(),
    };
    cert.signature_hex = signer.sign_commitment(&certification_message(&cert));
    cert
}

fn fixture(size: usize) -> (Vec<TrustRule>, Vec<Certification>) {
    let mut rules = Vec::with_capacity(size);
    let mut certifications = Vec::with_capacity(size);
    for index in 0..size {
        let signer = SecretKey::from_seed(index as u64 + 1);
        let certified_key = SecretKey::from_seed(index as u64 + 10_001).public_key();
        rules.push(anchor(index, signer.public_key()));
        let mut cert = signed_cert(index, &signer, certified_key);
        // Deterministically exercise each fail-closed reason reachable at depth one.
        match index % 6 {
            1 => cert.certifier = iri("https://unanchored.example/authority"),
            2 => cert.signature_hex = "00".to_owned(),
            3 => cert.valid_until_unix_secs = NOW - 1,
            4 => cert.certified_issuer = cert.certifier.clone(),
            5 => cert.scope = CertScope::Shape(predicate_shape("https://schema.org/name", "broad")),
            _ => {}
        }
        if index % 6 != 2 {
            cert.signature_hex = signer.sign_commitment(&certification_message(&cert));
        }
        certifications.push(cert);
    }
    (rules, certifications)
}

#[derive(Default)]
struct Rejections {
    no_anchor: usize,
    signature_invalid: usize,
    out_of_window: usize,
    cyclic: usize,
    broadening: usize,
    over_depth: usize,
}

impl Rejections {
    fn record(&mut self, reason: EdgeRejection) {
        match reason {
            EdgeRejection::NoAnchor => self.no_anchor += 1,
            EdgeRejection::SignatureInvalid => self.signature_invalid += 1,
            EdgeRejection::OutOfWindow => self.out_of_window += 1,
            EdgeRejection::Cyclic => self.cyclic += 1,
            EdgeRejection::Broadening => self.broadening += 1,
            EdgeRejection::OverDepth => self.over_depth += 1,
        }
    }
}

/// Render the fixed fixture suite as canonical, deterministic JSON.
///
/// Public solely so the integration test can include this example and mutation-witness
/// byte stability without adding anything to the `sparq-trust` library API.
pub fn render_fixture_suite() -> String {
    let mut output = String::from("{\n  \"schema_version\": 1,\n  \"cases\": [\n");
    let cases = [(1, 0_u32), (1, 1), (4, 1), (16, 1), (64, 1)];
    for (case_index, (size, depth_bound)) in cases.into_iter().enumerate() {
        let (rules, certifications) = fixture(size);
        let effective = derive_effective_rules(&rules, &certifications, NOW, depth_bound);
        let mut rejections = Rejections::default();
        for cert in &certifications {
            if let Err(reason) = explain_edge(&rules, cert, NOW, depth_bound) {
                rejections.record(reason);
            }
        }

        let visited_set_size = certifications
            .iter()
            .flat_map(|cert| [cert.certifier.as_str(), cert.certified_issuer.as_str()])
            .collect::<BTreeSet<_>>()
            .len();

        // Exercise the unchanged admission gate once per effective rule. The deliberately
        // malformed signature fails closed after canonicalisation; its result is not a
        // metric because this harness counts policy/closure work, not authorisation facts.
        let credential = PresentedCredential {
            graph: vec![Triple::new(
                iri("https://agent.example/alice"),
                iri(PREDICATE),
                Literal::from(42),
            )],
            issuer_signature_hex: "00".to_owned(),
            salt: [7; 32],
            issued_at_unix_secs: NOW,
            revoked: false,
        };
        let session = Session {
            agent: iri("https://agent.example/alice"),
            now_unix_secs: NOW,
        };
        let _ = admit(&credential, &effective, &session, &iri(TARGET));

        let derived_rule_count = effective.len().saturating_sub(rules.len());
        let max_closure_depth_reached = usize::from(depth_bound > 0 && !certifications.is_empty());
        write!(
            output,
            "    {{\n      \"fixture_size\": {size},\n      \"depth_bound\": {depth_bound},\n      \"direct_rule_count\": {},\n      \"certification_edges_considered\": {},\n      \"edges_rejected\": {{\n        \"no_anchor\": {},\n        \"signature_invalid\": {},\n        \"out_of_window\": {},\n        \"cyclic\": {},\n        \"broadening\": {},\n        \"over_depth\": {}\n      }},\n      \"max_closure_depth_reached\": {max_closure_depth_reached},\n      \"visited_set_size\": {visited_set_size},\n      \"derived_rule_count\": {derived_rule_count}\n    }}{}\n",
            rules.len(),
            certifications.len(),
            rejections.no_anchor,
            rejections.signature_invalid,
            rejections.out_of_window,
            rejections.cyclic,
            rejections.broadening,
            rejections.over_depth,
            if case_index + 1 == cases.len() { "" } else { "," },
        )
        .expect("writing to String is infallible");
    }
    output.push_str("  ]\n}\n");
    output
}

fn output_path(args: impl IntoIterator<Item = String>) -> Result<String, String> {
    let mut args = args.into_iter();
    match (args.next().as_deref(), args.next(), args.next()) {
        (Some("--emit"), Some(path), None) => Ok(path),
        _ => Err("usage: admission_cost --emit PATH".to_owned()),
    }
}

fn main() {
    let path = output_path(std::env::args().skip(1)).unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    if let Some(parent) = Path::new(&path).parent() {
        std::fs::create_dir_all(parent).expect("create output directory");
    }
    std::fs::write(path, render_fixture_suite()).expect("write deterministic metrics JSON");
}
