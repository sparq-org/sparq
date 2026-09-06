//! Behavioral tests for the object chunker: multi-block seal/open round-trips,
//! the Merkle-link authentication, and the fail-closed rejections a tampered,
//! truncated, padded, or reordered block set must produce.

use sparq_e2ee_ng::cbor::Limits;
use sparq_e2ee_ng::envelope::{BlockContext, BlockEnvelope, ObjectKind};
use sparq_e2ee_ng::error::Error;
use sparq_e2ee_ng::ids::{BranchId, Epoch, ObjectId, RepoId, Secret32};
use sparq_e2ee_ng::object::{
    open_object, seal_object, seal_object_with, ObjectLayout, ObjectLimits, ROOT_CHUNK_INDEX,
};

fn key() -> Secret32 {
    Secret32([4u8; 32])
}

fn ctx() -> BlockContext {
    BlockContext {
        repo: RepoId::from_bytes([1u8; 32]),
        branch: BranchId::from_bytes([2u8; 32]),
        epoch: Epoch(7),
        kind: ObjectKind::Snapshot,
    }
}

/// A deliberately tiny layout so a few hundred bytes already build a multi-level
/// Merkle tree (leaves of 8 bytes, fan-out 2).
fn tiny() -> ObjectLayout {
    ObjectLayout { chunk_size: 8, arity: 2 }
}

fn sample(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

// ============================================================================
// Round-trips
// ============================================================================

#[test]
fn object_roundtrip_spans_many_blocks() {
    let (k, c) = (key(), ctx());
    let plaintext = sample(400_000); // > 6 default-size leaves
    let sealed = seal_object(&k, &c, &ObjectId::random(), &plaintext).unwrap();

    assert!(sealed.block_count() > 1, "a large object must span blocks");
    assert_eq!(sealed.open(&k, &c, ObjectLimits::default()).unwrap(), plaintext);
}

#[test]
fn object_roundtrip_across_sizes_and_depths() {
    let (k, c) = (key(), ctx());
    // 0 -> one empty leaf; 8/9 -> the leaf boundary; 600 -> a 7-level tree
    // under `tiny()` (75 leaves, fan-out 2).
    for len in [0usize, 1, 7, 8, 9, 16, 17, 100, 600] {
        let plaintext = sample(len);
        let sealed =
            seal_object_with(&k, &c, &ObjectId::random(), &plaintext, tiny()).unwrap();
        let got = sealed.open(&k, &c, ObjectLimits::default()).unwrap();
        assert_eq!(got, plaintext, "round-trip failed at len {len}");
        assert!(sealed.block_count() >= 2, "an object always has a node root");
    }
}

#[test]
fn object_structure_invariants() {
    let (k, c) = (key(), ctx());
    let sealed =
        seal_object_with(&k, &c, &ObjectId::random(), &sample(300), tiny()).unwrap();

    let root = sealed.root().unwrap();
    assert_eq!(root.chunk_index, ROOT_CHUNK_INDEX);
    assert_eq!(root.chunk_count as usize, sealed.block_count());

    let mut positions: Vec<u64> = Vec::new();
    for b in &sealed.blocks {
        assert_eq!(b.object_id, sealed.object_id);
        assert_eq!(b.chunk_count, root.chunk_count, "all blocks share chunk_count");
        positions.push(b.chunk_index);
    }
    positions.sort_unstable();
    let expected: Vec<u64> = (0..sealed.block_count() as u64).collect();
    assert_eq!(positions, expected, "chunk positions are 0..n with no duplicates");

    // Only the root sits at the reserved position.
    assert_eq!(
        sealed.blocks.iter().filter(|b| b.chunk_index == ROOT_CHUNK_INDEX).count(),
        1
    );
}

#[test]
fn object_ids_are_random_not_content_derived() {
    let (k, c) = (key(), ctx());
    let plaintext = sample(200);
    let object = ObjectId::random();
    let a = seal_object_with(&k, &c, &object, &plaintext, tiny()).unwrap();
    let b = seal_object_with(&k, &c, &object, &plaintext, tiny()).unwrap();

    assert_eq!(a.block_count(), b.block_count());
    for (x, y) in a.blocks.iter().zip(b.blocks.iter()) {
        assert_ne!(x.block_id, y.block_id, "block ids must be freshly random");
        assert_ne!(x.ciphertext, y.ciphertext, "sealing must be randomized");
    }
    assert_ne!(a.merkle_root().unwrap(), b.merkle_root().unwrap());
}

#[test]
fn object_merkle_root_is_the_root_block_digest() {
    let (k, c) = (key(), ctx());
    let sealed =
        seal_object_with(&k, &c, &ObjectId::random(), &sample(50), tiny()).unwrap();
    let root = sealed.root().unwrap();
    assert_eq!(sealed.merkle_root().unwrap(), root.digest());
    assert_eq!(sealed.commit_id().unwrap().as_bytes(), &root.digest());
    // The digest really is over the canonical encrypted envelope.
    let reencoded = BlockEnvelope::decode(&root.encode(), Limits::default()).unwrap();
    assert_eq!(reencoded.digest(), root.digest());
}

// ============================================================================
// Fail-closed rejections
// ============================================================================

/// Open a block set that is not the one `seal_object_with` produced.
fn open_mutated(
    sealed: &sparq_e2ee_ng::object::SealedObject,
    mutate: impl FnOnce(&mut Vec<BlockEnvelope>),
) -> Result<Vec<u8>, Error> {
    let (k, c) = (key(), ctx());
    let mut blocks = sealed.blocks.clone();
    mutate(&mut blocks);
    let root = blocks[0].clone();
    open_object(&k, &c, &root, &blocks, ObjectLimits::default())
}

fn sealed_sample() -> sparq_e2ee_ng::object::SealedObject {
    seal_object_with(&key(), &ctx(), &ObjectId::random(), &sample(300), tiny()).unwrap()
}

#[test]
fn object_tampered_child_ciphertext_fails_closed() {
    let sealed = sealed_sample();
    // Flip a bit in a non-root block: its digest no longer matches the digest
    // its encrypted parent authenticated.
    let got = open_mutated(&sealed, |b| b[1].ciphertext[0] ^= 0x01);
    assert!(matches!(got, Err(Error::Integrity(_))), "got {got:?}");
}

#[test]
fn object_tampered_root_ciphertext_fails_closed() {
    let sealed = sealed_sample();
    // The root has no parent to authenticate it, so the AEAD catches it.
    let got = open_mutated(&sealed, |b| b[0].ciphertext[0] ^= 0x01);
    assert!(matches!(got, Err(Error::Decrypt)), "got {got:?}");
}

#[test]
fn object_moved_child_position_fails_closed() {
    let sealed = sealed_sample();
    // Re-labelling a child's chunk position changes its canonical envelope, so
    // the parent's recorded digest no longer matches.
    let got = open_mutated(&sealed, |b| b[1].chunk_index += 100);
    assert!(matches!(got, Err(Error::Integrity(_))), "got {got:?}");
}

#[test]
fn object_missing_block_fails_closed() {
    let sealed = sealed_sample();
    let got = open_mutated(&sealed, |b| {
        b.pop();
    });
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");
}

#[test]
fn object_extra_block_fails_closed() {
    let sealed = sealed_sample();
    let got = open_mutated(&sealed, |b| {
        let mut extra = b[1].clone();
        extra.block_id = sparq_e2ee_ng::ids::BlockId::random();
        b.push(extra);
    });
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");
}

#[test]
fn object_duplicated_block_fails_closed() {
    let sealed = sealed_sample();
    let got = open_mutated(&sealed, |b| {
        b.pop();
        let dup = b[1].clone();
        b.push(dup);
    });
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");
}

#[test]
fn object_foreign_block_fails_closed() {
    let sealed = sealed_sample();
    let other = sealed_sample();
    let got = open_mutated(&sealed, |b| {
        let n = b.len();
        b[n - 1] = other.blocks[1].clone();
    });
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");
}

#[test]
fn object_leaf_presented_as_root_fails_closed() {
    let (k, c) = (key(), ctx());
    let sealed = sealed_sample();
    let leaf = sealed.blocks.iter().find(|b| b.chunk_index == 1).unwrap().clone();
    let got = open_object(&k, &c, &leaf, &sealed.blocks, ObjectLimits::default());
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");
}

#[test]
fn object_root_outside_the_block_set_fails_closed() {
    let (k, c) = (key(), ctx());
    let sealed = sealed_sample();
    let root = sealed.blocks[0].clone();

    // Drop the real root and swap in a fresh same-object block so the set still
    // has `chunk_count` entries, then present the root separately. Every
    // descendant still authenticates, so only the root's own membership in the
    // set can reject this.
    let mut blocks = sealed.blocks.clone();
    blocks.remove(0);
    let mut extra = blocks[0].clone();
    extra.block_id = sparq_e2ee_ng::ids::BlockId::random();
    blocks.push(extra);
    assert_eq!(blocks.len() as u64, root.chunk_count);
    let got = open_object(&k, &c, &root, &blocks, ObjectLimits::default());
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");

    // Same block id, different envelope: the set's entry is not the root that
    // was passed, so the traversed root would not be the one in the set.
    let mut blocks = sealed.blocks.clone();
    blocks[0].nonce[0] ^= 0x01;
    let got = open_object(&k, &c, &root, &blocks, ObjectLimits::default());
    assert!(matches!(got, Err(Error::Schema(_))), "got {got:?}");
}

#[test]
fn object_wrong_context_fails_closed() {
    let k = key();
    let sealed = sealed_sample();
    let mut wrong = ctx();
    wrong.epoch = Epoch(8);
    let got = sealed.open(&k, &wrong, ObjectLimits::default());
    assert!(matches!(got, Err(Error::Decrypt)), "got {got:?}");

    let mut wrong_kind = ctx();
    wrong_kind.kind = ObjectKind::Commit;
    assert!(matches!(
        sealed.open(&k, &wrong_kind, ObjectLimits::default()),
        Err(Error::Decrypt)
    ));
}

#[test]
fn object_wrong_read_secret_fails_closed() {
    let sealed = sealed_sample();
    let other = Secret32([9u8; 32]);
    assert!(matches!(
        sealed.open(&other, &ctx(), ObjectLimits::default()),
        Err(Error::Decrypt)
    ));
}

// ============================================================================
// Layout + limit validation
// ============================================================================

#[test]
fn object_layout_is_validated() {
    let (k, c) = (key(), ctx());
    let o = ObjectId::random();
    let bad = |layout| seal_object_with(&k, &c, &o, b"x", layout);

    assert!(matches!(
        bad(ObjectLayout { chunk_size: 0, arity: 2 }),
        Err(Error::Schema(_))
    ));
    assert!(matches!(
        bad(ObjectLayout { chunk_size: 8, arity: 1 }),
        Err(Error::Schema(_))
    ));
    assert!(matches!(
        bad(ObjectLayout { chunk_size: 8, arity: 100_000 }),
        Err(Error::LimitExceeded(_))
    ));
    assert!(matches!(
        bad(ObjectLayout { chunk_size: 1 << 30, arity: 2 }),
        Err(Error::LimitExceeded(_))
    ));
    ObjectLayout::default().validate().unwrap();
}

#[test]
fn object_limits_are_enforced() {
    let (k, c) = (key(), ctx());
    let sealed = sealed_sample();
    let root = sealed.root().unwrap();

    let few_blocks = ObjectLimits { max_blocks: 2, ..ObjectLimits::default() };
    assert!(matches!(
        open_object(&k, &c, root, &sealed.blocks, few_blocks),
        Err(Error::LimitExceeded(_))
    ));

    let short = ObjectLimits { max_plaintext_len: 16, ..ObjectLimits::default() };
    assert!(matches!(
        open_object(&k, &c, root, &sealed.blocks, short),
        Err(Error::LimitExceeded(_))
    ));

    let shallow = ObjectLimits { max_level: 1, ..ObjectLimits::default() };
    assert!(matches!(
        open_object(&k, &c, root, &sealed.blocks, shallow),
        Err(Error::LimitExceeded(_))
    ));

    // The honest layout is accepted under the defaults.
    open_object(&k, &c, root, &sealed.blocks, ObjectLimits::default()).unwrap();
}

// ============================================================================
// Randomized round-trip
// ============================================================================

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(64))]

    /// Any plaintext under any valid layout reassembles byte-for-byte, and the
    /// tree is always exactly `chunk_count` blocks with the root at chunk 0.
    #[test]
    fn object_roundtrip_arbitrary(
        plaintext in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2000),
        chunk_size in 1usize..64,
        arity in 2usize..8,
    ) {
        let (k, c) = (key(), ctx());
        let layout = ObjectLayout { chunk_size, arity };
        let sealed =
            seal_object_with(&k, &c, &ObjectId::random(), &plaintext, layout).unwrap();
        proptest::prop_assert_eq!(sealed.root().unwrap().chunk_index, ROOT_CHUNK_INDEX);
        proptest::prop_assert_eq!(
            sealed.root().unwrap().chunk_count,
            sealed.block_count() as u64
        );
        proptest::prop_assert_eq!(
            sealed.open(&k, &c, ObjectLimits::default()).unwrap(),
            plaintext
        );
    }
}
