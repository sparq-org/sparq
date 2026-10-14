//! Emits registry entries signed through the issuance path of sparq commit
//! 5e7e008e (before the registry signature covered the commitment method).
//! Run against a checkout of that commit; prints N-Quads on stdout, one block
//! per entry, each preceded by a `# name` comment line.
use oxrdf::NamedNode;
use sparq_zk::commit::CommitmentMethod;
use sparq_zk::field::Fr;
use sparq_zk::registry::{RegistryEntry, Status, ZK_GRAPH};
use sparq_zk::sig::SecretKey;

fn emit(name: &str, e: &RegistryEntry) {
    println!("# {name}");
    let mut lines: Vec<String> = e
        .to_triples()
        .iter()
        .map(|[s, p, o]| format!("{s} {p} {o} <{ZK_GRAPH}> ."))
        .collect();
    lines.sort();
    for l in lines {
        println!("{l}");
    }
}

fn main() {
    let sk = SecretKey::from_seed(7064);
    let doc = |n: &str| NamedNode::new_unchecked(format!("https://example.org/legacy/{n}"));
    let list = NamedNode::new_unchecked("https://example.org/legacy/status-list");

    // Honest string-canonical entries, exactly as the base issuance path made them.
    let plain = RegistryEntry::issued(doc("string-canonical"), Fr::from(0x5c01u64), Fr::from(0x5a17u64), &sk);
    emit("string-canonical-plain", &plain);
    let status = RegistryEntry::issued_with_status(
        doc("string-canonical-status"),
        Fr::from(0x5c02u64),
        Fr::from(0x5a18u64),
        list.clone(),
        3,
        1,
        Status::Active,
        &sk,
    );
    emit("string-canonical-status", &status);

    // dual-leaf-v1 entries: signed by the same method-independent path, then
    // given their method with the base API.
    let v1 = RegistryEntry::issued(doc("dual-leaf-v1"), Fr::from(0xd101u64), Fr::from(0x5a19u64), &sk)
        .with_already_issued_method(CommitmentMethod::DualLeafV1);
    emit("dual-leaf-v1-plain", &v1);
    let v1s = RegistryEntry::issued_with_status(
        doc("dual-leaf-v1-status"),
        Fr::from(0xd102u64),
        Fr::from(0x5a1au64),
        list,
        4,
        1,
        Status::Active,
        &sk,
    )
    .with_already_issued_method(CommitmentMethod::DualLeafV1);
    emit("dual-leaf-v1-status", &v1s);

    // Sanity: under the base code every one of these verifies.
    assert!(plain.verify_commitment_signature());
    assert!(status.verify_commitment_signature_with_status());
    assert!(v1.verify_commitment_signature());
    assert!(v1s.verify_commitment_signature_with_status());
}
