//! **Golden test vectors** (research doc §9 requires test vectors before the
//! suite can be called production-ready). These pin the *exact* deterministic
//! wire bytes of every primitive so the encoding + the concrete v0 crypto
//! composition cannot silently drift. Ed25519 signing, HKDF, and (given a fixed
//! key/nonce) XChaCha20-Poly1305 are all deterministic, so every value below is
//! reproducible. Primitive inputs are in `cargo run --example gen_vectors`;
//! the multi-block object inputs are in `object_vector` below.
//!
//! A change to any of these strings is a **wire-format break** and must be a
//! deliberate, reviewed version bump — never an accident.

use sparq_e2ee_ng::capability::{base_grant, Authority, Capability, Validity};
use sparq_e2ee_ng::cbor::Limits;
use sparq_e2ee_ng::envelope::{
    open_block, seal_block, BlockContext, BlockEnvelope, Commit, ObjectKind,
};
use sparq_e2ee_ng::epoch::{EpochTransition, HistoryPolicy};
use sparq_e2ee_ng::ids::{
    AuthorKeyId, BlockId, BranchId, CommitId, Epoch, ObjectId, RepoId, Secret32, TopicId,
};
use sparq_e2ee_ng::keyschedule::{block_key, object_key};
use sparq_e2ee_ng::object::{seal_object_deterministic, ObjectLayout, ObjectLimits, SealedObject};
use sparq_e2ee_ng::sign::SecretSigningKey;
use sparq_e2ee_ng::suite::AEAD_NONCE_LEN;
use sparq_e2ee_ng::wrap::{wrap_with, RecipientSecretKey, WrappedSecret};

const OBJECT_KEY: &str = "f047a88d13eceb6c4ae04aab8c60097a9550a1f53a817a711bfd3482c04b2da8";
const BLOCK_KEY: &str = "285777cd44c36e35570b889f786fd170ff8571d0fab1cd2ae7c379c70b422942";
const CAP_ID: &str = "e915b3796456da74d18ca059c6e07b7711fdb37d3a26b50638f030a8348066f6";
const COMMIT_ID_OF_BLOCK: &str =
    "814a9a0164fe1573dd301684ebd319a980f817681a107463a5f58d6d23b53392";

fn common() -> (Secret32, RepoId, BranchId, ObjectId, SecretSigningKey) {
    (
        Secret32([4u8; 32]),
        RepoId::from_bytes([1u8; 32]),
        BranchId::from_bytes([2u8; 32]),
        ObjectId::from_bytes([5u8; 32]),
        SecretSigningKey::from_seed([1u8; 32]),
    )
}

// Captured from object_vector(); block order is root, leaves, then internal nodes.
const OBJECT_NODE_PLAINTEXT: [(u64, &str); 3] = [
    (0, concat!(
        "a4010002020382a301582014141414141414141414141414141414141414141414141414141414141414140204035820",
        "b18c31722c613c29f513ee7339e9b525cb0d14e5db763dd26d747a5662ca9688a3015820151515151515151515151515",
        "1515151515151515151515151515151515151515020503582078543e0c773c75d838e9ac9204c9eb76d1d8d5db3ddba6",
        "aaa2587accd0a73cc6040a",
    )),
    (4, concat!(
        "a4010002010382a301582011111111111111111111111111111111111111111111111111111111111111110201035820",
        "170899a5ab5067f3b55ddd51c01392791b3a445fb85e0f8898a1e98fe554e10aa3015820121212121212121212121212",
        "1212121212121212121212121212121212121212020203582014c36f0502b6892d049e27390eeb071fefe0b328f94e6f",
        "b6d048d52ca3dec8460408",
    )),
    (5, concat!(
        "a4010002010381a301582013131313131313131313131313131313131313131313131313131313131313130203035820",
        "a50030debc5f04568153c378e7c08e455a5ca03eeca60b1c59d6e7eec13dfe790402",
    )),
];

const OBJECT_BLOCKS: [&str; 6] = [
    concat!(
        "a90100025820101010101010101010101010101010101010101010101010101010101010101003582005050505050505",
        "050505050505050505050505050505050505050505050505050400050606785a75726e3a6a657377723a773369643a65",
        "3265652d6e673a64726166743a323032362d30372373756974652d76302d7832353531392d786368616368613230706f",
        "6c79313330352d686b64662d7368613235362d6564323535313907581820202020202020202020202020202020202020",
        "202020202008590110cdb979c1af5ad6bf4459c8dbef7f214f99c0421d2f880a8875d846044f285fcc88d5f2511e8679",
        "6d61a6add07c7f3042f9c3a96fdcb35a6babe7fe38d3187391777a8b10c1203884f70efe10a7236edb1651deb0999e3b",
        "d4101e58f3fd0fc099308e41b7cccce134a52318bed20661f369fff7a9424f8c1740420637de102ad20f701078b134c2",
        "8b13a65a0cb25ee1f52483d54ad1706a019fcde5dafa6010df523aeb99da3596e9037738cf0b3d19bb7f9bcfb1c3b303",
        "2b8503dca360481681f5ca60b0817736b3c9fcae4db6ccb213d8848133ddad27a773b2c7118e7867da959768e153aa8d",
        "9285c2e4fdeb3bfee4b3725a074178177bec18dae448d939a7b0b543778a55bb19a9b1f6e92241822e09190100",
    ),
    concat!(
        "a90100025820111111111111111111111111111111111111111111111111111111111111111103582005050505050505",
        "050505050505050505050505050505050505050505050505050401050606785a75726e3a6a657377723a773369643a65",
        "3265652d6e673a64726166743a323032362d30372373756974652d76302d7832353531392d786368616368613230706f",
        "6c79313330352d686b64662d7368613235362d6564323535313907581821212121212121212121212121212121212121",
        "212121212108590110389a86a5f22ae7b9610872dff0dcefa5a4e77d7cdd2f0f69e3b340b5d71a1561e1e7bff8948892",
        "3afaaba1b4c1dbcfbd96ab3c2ee54f73a765823dcb7f31ec3e3027084f0522e5836b156e9c42c20fef09d79e75f19d9e",
        "12afe4269dfb5e680c4adf1278bacf30dff7dbe368e70a7b236d30c5b03bacbe0fb3b8da9582772ad8f6893a77c4b032",
        "3eafe97581734de10803027bf4d4f6588669afcf0d5d3180401684f090bd9cd7961fe7ee3ef378a6b438ff6beba47677",
        "3b0455540de92b10a630887e573561c9e142010212b370898ab87475d30dc5ef28ae36daaf5faae3e598babaacd9f8e4",
        "54359f4b7ca3aa7ed09817f18ddb227239d51c3d85ecfb9f58ab3043e8b47842c0ea98c2b20569c01e09190100",
    ),
    concat!(
        "a90100025820121212121212121212121212121212121212121212121212121212121212121203582005050505050505",
        "050505050505050505050505050505050505050505050505050402050606785a75726e3a6a657377723a773369643a65",
        "3265652d6e673a64726166743a323032362d30372373756974652d76302d7832353531392d786368616368613230706f",
        "6c79313330352d686b64662d7368613235362d6564323535313907581822222222222222222222222222222222222222",
        "222222222208590110f9c82dbff359f1167e286b95ed3c284c0c39e76a6220975b519d06158c64df425486e03b6b96b8",
        "b56a07a38a000e80798e46ea87c3b6f8d52f2708a497ebfc3ae63a349e2e04f75a4932abf1588dd9f347b8c765aadb77",
        "39e9ce9cd468d4e29c306141e320e3694eba5cb4f8c3920441b575eb96c216079cb28fcf3a33e6469a85b209c3a9e9a6",
        "eb117541b6eb948fc73e9570947a846ddd2d82b0141815521f4a649211b600c3024293a10bf0989fa4f0d3d9ce893ec0",
        "1642402748f71f83ae336928f60ec1cd15e8763109650b5aaa0356ab99d0a1aa10d335ddca4263b88539db899e594179",
        "a546a58bcc2befc7fa942f51d539fba20e4c0f883af83a30b6cba16bfebc13136b3f726aa629806b4509190100",
    ),
    concat!(
        "a90100025820131313131313131313131313131313131313131313131313131313131313131303582005050505050505",
        "050505050505050505050505050505050505050505050505050403050606785a75726e3a6a657377723a773369643a65",
        "3265652d6e673a64726166743a323032362d30372373756974652d76302d7832353531392d786368616368613230706f",
        "6c79313330352d686b64662d7368613235362d6564323535313907581823232323232323232323232323232323232323",
        "232323232308590110c441d1f95870b056e66a04dcea3dd6ee643f38c599921870c85b95ea92bac66f474593b9ab65f4",
        "6bae78417385dd31021766a8f9e5f53fe7c5cf95b3c24f317c1770c58ec878491f4163ca4696e46180a06efe655e77be",
        "349f2ca1107cf72253ddb0df01c00fa56b82f34a7cdd20250239b18aa486b23db7c9e5bf3f5ff72f3e9143e65a5628b6",
        "ff0a6f17232f70682dccb5ffc6cb62540755db907e21d6818cb10cba9ac8c3ef162776abd98c3a81f0721ed49cfae6cd",
        "fab095607987b02c7f50041cf4bc83824f6085de0ee97bd240f8c1c3862be4dd985b68c7b23a190b7a107a61093b946f",
        "dc8941f4df4930df5aaf09099879519e4de80fba234292af3221337c25e178bff276ad05f4e554667809190100",
    ),
    concat!(
        "a90100025820141414141414141414141414141414141414141414141414141414141414141403582005050505050505",
        "050505050505050505050505050505050505050505050505050404050606785a75726e3a6a657377723a773369643a65",
        "3265652d6e673a64726166743a323032362d30372373756974652d76302d7832353531392d786368616368613230706f",
        "6c79313330352d686b64662d7368613235362d6564323535313907581824242424242424242424242424242424242424",
        "24242424240859011014d383f3fd6fb278b3846e354345fc24883cf4306b419e6c2abea6fa3bd15672b5d5ba7eba539b",
        "d4d4a70b8bda52f01897820f5496eb01e2a1fd3730baf8c6916d5ab161c38465c2f7868a841786c25cd12a9731e8a73d",
        "42a4c603dc4bbc5712c3600908b156284ef3b92486e729bdd53472edb03bd173a0b52d9e21d4952a01cbd4f943cb3915",
        "3e81767888c8793644bf73918f1ab7be9ce2453400f3fa7be84a2265cfc0f167cfc4839faaf3271e387b5a985f2bdf52",
        "d634ed243d3319c7a5a80c1f26ea82e9f025383beebc7782fbd5a8872e1f279a9175658087c421d620769f80306f6a47",
        "0619ce17109513b4d6b78d9609e0e6897e0c6f3daead950fe42b7be3d6851ea2589f2cff3f56419d4409190100",
    ),
    concat!(
        "a90100025820151515151515151515151515151515151515151515151515151515151515151503582005050505050505",
        "050505050505050505050505050505050505050505050505050405050606785a75726e3a6a657377723a773369643a65",
        "3265652d6e673a64726166743a323032362d30372373756974652d76302d7832353531392d786368616368613230706f",
        "6c79313330352d686b64662d7368613235362d6564323535313907581825252525252525252525252525252525252525",
        "25252525250859011085363c56d71de13f311a4c6e9542a0f6244bd38a47b87fa24765a27d5c055ade00b8a962925a07",
        "62b46bda376f67a4f6c1f2a478f9430b8b373a9b78372973b4943f1ef41baa5602fc0774a3a2162aa01a85f6260ea2fb",
        "fe3fe4fd99c10912eefa587e5cf65f439b86a4dce076378de1570c06d0dad50cc2add51d1ef9cf7bcece21297dd46c04",
        "87e782df52cd0b60fe161df5522caf6d54eeb700c3d411b3f7709027e0c5330bb41bfacf603cfafb61ec696db175ba28",
        "92012c8b253611970e3773a7ad2e7661cabec2daf5e25f131589cfed1d0b96541d90da7616b655a78d2150201e1587c2",
        "aed8132a424ac955aeba5e2866835dd66104a1a0e1074688f89590c1fe5b2fd9409ec61e626114ee5b09190100",
    ),
];
const OBJECT_MERKLE_ROOT: &str = "d24ab0c7e767561b6ad84d42b3aaa44bac89b6ebe397e6929bb9ac6aceec90bf";

fn object_vector() -> (Secret32, BlockContext, SealedObject) {
    let (k, repo, branch, object, _admin) = common();
    let ctx = BlockContext {
        repo,
        branch,
        epoch: Epoch(7),
        kind: ObjectKind::Operation,
    };
    let mut indices = Vec::new();
    let sealed = seal_object_deterministic(
        &k,
        &ctx,
        &object,
        b"some quads",
        ObjectLayout {
            chunk_size: 4,
            arity: 2,
        },
        |index| {
            indices.push(index);
            let index = u8::try_from(index).unwrap();
            (
                BlockId::from_bytes([0x10 + index; 32]),
                [0x20 + index; AEAD_NONCE_LEN],
            )
        },
    )
    .unwrap();
    assert_eq!(indices, [1, 2, 3, 4, 5, 0]);
    (k, ctx, sealed)
}

#[test]
fn vector_object_node_plaintext() {
    let (k, ctx, sealed) = object_vector();
    // Pin both levels, including the short final subtree.
    for (index, expected) in OBJECT_NODE_PLAINTEXT {
        let node = sealed
            .blocks
            .iter()
            .find(|b| b.chunk_index == index)
            .unwrap();
        assert_eq!(hex::encode(open_block(&k, &ctx, node).unwrap()), expected);
    }
}

#[test]
fn vector_object_multiblock() {
    let (k, ctx, sealed) = object_vector();
    assert_eq!(sealed, object_vector().2);
    assert_eq!(sealed.block_count(), 6);
    for (block, expected) in sealed.blocks.iter().zip(OBJECT_BLOCKS) {
        assert_eq!(hex::encode(block.encode()), expected);
        let decoded =
            BlockEnvelope::decode(&hex::decode(expected).unwrap(), Limits::default()).unwrap();
        assert_eq!(decoded, *block);
    }
    assert_eq!(
        hex::encode(sealed.merkle_root().unwrap()),
        OBJECT_MERKLE_ROOT
    );
    assert_eq!(
        sealed.open(&k, &ctx, ObjectLimits::default()).unwrap(),
        b"some quads"
    );
}

#[test]
fn vector_key_schedule() {
    let (k, repo, branch, object, _admin) = common();
    let ok = object_key(&k, &repo, &branch, Epoch(7), &object);
    assert_eq!(hex::encode(ok), OBJECT_KEY);
    let bk = block_key(&ok, &BlockId::from_bytes([6u8; 32]), 0);
    assert_eq!(hex::encode(bk), BLOCK_KEY);
}

#[test]
fn vector_capability_public_grant() {
    let (_k, repo, branch, _object, admin) = common();
    let mut grant = base_grant(
        repo,
        branch,
        Epoch(7),
        TopicId::from_bytes([3u8; 32]),
        Validity { not_before: 100, not_after: 200 },
        vec!["wss://broker.example".to_string()],
    );
    grant.authority = vec![Authority::Read];
    grant.cap_nonce = [0x11u8; 32]; // pin the otherwise-random nonce for a stable vector
    grant.sign(&admin);
    // cap_id is stable and matches the golden value.
    assert_eq!(hex::encode(grant.cap_id().as_bytes()), CAP_ID);
    // The signed grant verifies and round-trips through decode byte-for-byte.
    grant.verify(&admin.public()).unwrap();
    let bytes = grant.encode();
    let decoded =
        sparq_e2ee_ng::capability::PublicGrant::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(decoded.encode(), bytes);
    assert_eq!(decoded.cap_id(), grant.cap_id());
}

#[test]
fn vector_block_envelope_commit_id() {
    let (k, repo, branch, object, _admin) = common();
    let ctx = BlockContext { repo, branch, epoch: Epoch(7), kind: ObjectKind::Operation };
    let env = seal_block(
        &k,
        &ctx,
        &object,
        &BlockId::from_bytes([6u8; 32]),
        0,
        1,
        [7u8; AEAD_NONCE_LEN],
        b"some quads",
    )
    .unwrap();
    // Deterministic ciphertext => stable CommitId over the encrypted envelope.
    assert_eq!(hex::encode(env.commit_id().as_bytes()), COMMIT_ID_OF_BLOCK);
    // And it opens back to the plaintext.
    assert_eq!(
        sparq_e2ee_ng::envelope::open_block(&k, &ctx, &env).unwrap(),
        b"some quads"
    );
}

#[test]
fn vector_commit_plaintext_verifies() {
    let (_k, repo, branch, object, _admin) = common();
    let author = SecretSigningKey::from_seed([3u8; 32]);
    let mut commit = Commit {
        repo,
        branch,
        epoch: Epoch(7),
        parents: vec![CommitId::from_bytes([8u8; 32])],
        author: AuthorKeyId::from_bytes([0u8; 32]),
        clock: 42,
        crdt_kind: "or-set-quads-v0".to_string(),
        operations: vec![object],
        snapshot: None,
        author_sig: None,
    };
    commit.sign(&author);
    let bytes = commit.encode();
    // Round-trips byte-for-byte and re-verifies.
    let decoded = Commit::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(decoded.encode(), bytes);
    decoded.verify().unwrap();
}

#[test]
fn vector_epoch_transition_verifies() {
    let (_k, repo, branch, _object, admin) = common();
    let mut t = EpochTransition {
        repo,
        branch,
        old_epoch: Epoch(7),
        new_epoch: Epoch(8),
        old_topic: TopicId::from_bytes([3u8; 32]),
        new_topic: TopicId::from_bytes([4u8; 32]),
        new_publishers: vec![SecretSigningKey::from_seed([2u8; 32]).public().to_bytes()],
        history_policy: HistoryPolicy::ForwardOnly,
        admin_sig: None,
    };
    t.sign(&admin).unwrap();
    let bytes = t.encode();
    let decoded = EpochTransition::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(decoded.encode(), bytes);
    decoded.verify(&admin.public()).unwrap();
}

#[test]
fn vector_wrapped_secret_unwraps() {
    let recipient = RecipientSecretKey::from_bytes([5u8; 32]);
    let w = wrap_with(
        [11u8; 32],
        [12u8; AEAD_NONCE_LEN],
        &recipient.public(),
        b"K_read",
        b"purpose:cap",
    )
    .unwrap();
    let bytes = w.encode();
    let decoded = WrappedSecret::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(decoded.encode(), bytes);
    assert_eq!(
        sparq_e2ee_ng::wrap::unwrap(&recipient, &decoded, b"purpose:cap").unwrap(),
        b"K_read"
    );
}

#[test]
fn vector_secret_capability_roundtrips() {
    let (_k, repo, branch, _object, admin) = common();
    let publisher = SecretSigningKey::from_seed([2u8; 32]);
    let mut write = Capability::new_write(
        base_grant(
            repo,
            branch,
            Epoch(7),
            TopicId::from_bytes([3u8; 32]),
            Validity { not_before: 100, not_after: 200 },
            vec!["wss://broker.example".to_string()],
        ),
        Secret32([9u8; 32]),
        &publisher,
    )
    .unwrap();
    write.grant.cap_nonce = [0x42; 32];
    write.grant.sign(&admin);

    let bytes = write.encode_secret();
    let decoded = Capability::decode_secret(&bytes, Limits::default()).unwrap();
    assert_eq!(decoded.encode_secret(), bytes);
    decoded.grant.verify(&admin.public()).unwrap();
    assert_eq!(decoded.read_secret.as_ref().unwrap().expose(), &[9u8; 32]);
}
