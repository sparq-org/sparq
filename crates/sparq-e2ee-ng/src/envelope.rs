//! **Randomized block & commit envelopes** (§8.3). A block is padded, then
//! AEAD-sealed under a domain-separated per-block key drawn from the key
//! schedule, with a **randomized** nonce and a **random** (never plaintext- or
//! ciphertext-derived) block id. The associated data binds every routing/scoping
//! field — including repo/branch/epoch, which live only in the *opaque header*
//! and are NOT serialized in the envelope (§5) — so a block cannot be replayed
//! into a different object, chunk position, epoch, or branch.
//!
//! `CommitId = SHA-256(canonical encrypted root envelope)` (§8.3): the identity
//! of a commit is the hash of its *encrypted* root block, not of any plaintext.

use crate::cbor::{
    enc_array, enc_bytes, enc_map, enc_text, enc_uint, read_struct_map, Limits, Reader,
};
use crate::error::{Error, Result};
use crate::ids::{
    AuthorKeyId, BlockId, BranchId, CommitId, Epoch, ObjectId, RepoId, Secret32,
};
use crate::keyschedule::{block_key, object_key};
use crate::sign::{PublicVerifyingKey, SecretSigningKey, PUBLIC_KEY_LEN, SIGNATURE_LEN};
use crate::suite::{aead_open, aead_seal, check_suite, AEAD_NONCE_LEN, SUITE_V0};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

/// The kind of plaintext a block carries (bound into the AEAD associated data so
/// a block cannot be reinterpreted as another kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A commit root object (§8.3 decrypted root).
    Commit = 1,
    /// A CRDT operation object.
    Operation = 2,
    /// A materialized snapshot object.
    Snapshot = 3,
}

/// The scoping context bound into a block's AEAD associated data but **not**
/// serialized into the envelope (it is the "opaque header" of §5). The recipient
/// reconstructs it from the capability/topic it already holds.
#[derive(Debug, Clone, Copy)]
pub struct BlockContext {
    /// Repository id (AD-bound, not on the wire).
    pub repo: RepoId,
    /// Branch id (AD-bound, not on the wire).
    pub branch: BranchId,
    /// Epoch (AD-bound, not on the wire).
    pub epoch: Epoch,
    /// The plaintext kind carried by the block.
    pub kind: ObjectKind,
}

/// Padding length classes (§8.3 field 9). A block's padded plaintext is grown to
/// the smallest class that fits `4 + plaintext_len` bytes, so on-wire ciphertext
/// size reveals only a coarse bucket, not the exact plaintext length.
pub const PAD_CLASSES: [usize; 8] = [
    256, 1024, 4096, 16384, 65536, 262144, 1_048_576, 4_194_304,
];

/// Pad `plaintext` to the smallest fitting class. Layout: 4-byte big-endian real
/// length, then the plaintext, then zero padding to the class size. Returns the
/// padded buffer (whose length equals the class size).
fn pad(plaintext: &[u8]) -> Result<Vec<u8>> {
    let need = 4usize
        .checked_add(plaintext.len())
        .ok_or(Error::LimitExceeded("plaintext length overflow"))?;
    let class = *PAD_CLASSES
        .iter()
        .find(|&&c| c >= need)
        .ok_or(Error::LimitExceeded("plaintext exceeds largest pad class"))?;
    let mut out = vec![0u8; class];
    out[..4].copy_from_slice(&(plaintext.len() as u32).to_be_bytes());
    out[4..4 + plaintext.len()].copy_from_slice(plaintext);
    Ok(out)
}

/// Reverse [`pad`]: validate the padded length matches a class and extract the
/// real plaintext.
fn unpad(padded: &[u8]) -> Result<Vec<u8>> {
    if !PAD_CLASSES.contains(&padded.len()) {
        return Err(Error::Malformed("padded length is not a pad class"));
    }
    if padded.len() < 4 {
        return Err(Error::Malformed("padded block too short"));
    }
    let real = u32::from_be_bytes([padded[0], padded[1], padded[2], padded[3]]) as usize;
    let end = 4usize
        .checked_add(real)
        .ok_or(Error::Malformed("pad length overflow"))?;
    if end > padded.len() {
        return Err(Error::Malformed("declared length exceeds pad class"));
    }
    Ok(padded[4..end].to_vec())
}

/// Build the AEAD associated data binding all scoping + position fields (§8.3).
#[allow(clippy::too_many_arguments)]
fn associated_data(
    ctx: &BlockContext,
    object: &ObjectId,
    block: &BlockId,
    chunk_index: u64,
    chunk_count: u64,
    pad_class: u64,
) -> Vec<u8> {
    enc_map(vec![
        (1, enc_uint(0)), // version
        (2, enc_text(SUITE_V0)),
        (3, enc_bytes(ctx.repo.as_bytes())),
        (4, enc_bytes(ctx.branch.as_bytes())),
        (5, enc_uint(ctx.epoch.0)),
        (6, enc_bytes(object.as_bytes())),
        (7, enc_bytes(block.as_bytes())),
        (8, enc_uint(chunk_index)),
        (9, enc_uint(chunk_count)),
        (10, enc_uint(ctx.kind as u64)),
        (11, enc_uint(pad_class)),
    ])
}

/// A serialized encrypted block (§8.3 `BlockEnvelopeV0`). The whole canonical
/// byte string is authenticated as a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEnvelope {
    /// Random block id.
    pub block_id: BlockId,
    /// Object id this block belongs to.
    pub object_id: ObjectId,
    /// Chunk index within the object.
    pub chunk_index: u64,
    /// Total chunk count for the object.
    pub chunk_count: u64,
    /// Bound suite id.
    pub suite: String,
    /// AEAD nonce.
    pub nonce: [u8; AEAD_NONCE_LEN],
    /// AEAD ciphertext-and-tag.
    pub ciphertext: Vec<u8>,
    /// Padded plaintext length class (the padded length in bytes).
    pub pad_class: u64,
}

// wire keys (§8.3)
const B_VERSION: u64 = 1;
const B_BLOCK: u64 = 2;
const B_OBJECT: u64 = 3;
const B_CHUNK_IDX: u64 = 4;
const B_CHUNK_CNT: u64 = 5;
const B_SUITE: u64 = 6;
const B_NONCE: u64 = 7;
const B_CT: u64 = 8;
const B_PAD: u64 = 9;
const B_VERSION_VAL: u64 = 0;

impl BlockEnvelope {
    /// Canonical CBOR of the envelope.
    pub fn encode(&self) -> Vec<u8> {
        enc_map(vec![
            (B_VERSION, enc_uint(B_VERSION_VAL)),
            (B_BLOCK, enc_bytes(self.block_id.as_bytes())),
            (B_OBJECT, enc_bytes(self.object_id.as_bytes())),
            (B_CHUNK_IDX, enc_uint(self.chunk_index)),
            (B_CHUNK_CNT, enc_uint(self.chunk_count)),
            (B_SUITE, enc_text(&self.suite)),
            (B_NONCE, enc_bytes(&self.nonce)),
            (B_CT, enc_bytes(&self.ciphertext)),
            (B_PAD, enc_uint(self.pad_class)),
        ])
    }

    /// Decode + validate an envelope (fail-closed; rejects unknown suite).
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut r = Reader::new(bytes, limits);
        let mut version = None;
        let mut block = None;
        let mut object = None;
        let mut ci = None;
        let mut cc = None;
        let mut suite: Option<String> = None;
        let mut nonce = None;
        let mut ct = None;
        let mut pad_class = None;
        read_struct_map(&mut r, |r, key| match key {
            B_VERSION => {
                version = Some(r.uint()?);
                Ok(true)
            }
            B_BLOCK => {
                block = Some(BlockId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            B_OBJECT => {
                object = Some(ObjectId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            B_CHUNK_IDX => {
                ci = Some(r.uint()?);
                Ok(true)
            }
            B_CHUNK_CNT => {
                cc = Some(r.uint()?);
                Ok(true)
            }
            B_SUITE => {
                suite = Some(r.text()?.to_owned());
                Ok(true)
            }
            B_NONCE => {
                nonce = Some(r.bytes_fixed::<AEAD_NONCE_LEN>()?);
                Ok(true)
            }
            B_CT => {
                ct = Some(r.bytes()?.to_vec());
                Ok(true)
            }
            B_PAD => {
                pad_class = Some(r.uint()?);
                Ok(true)
            }
            _ => Ok(false),
        })?;
        r.finish()?;
        if version != Some(B_VERSION_VAL) {
            return Err(Error::Schema("block version"));
        }
        let suite = suite.ok_or(Error::Schema("missing suite"))?;
        check_suite(&suite)?;
        Ok(BlockEnvelope {
            block_id: block.ok_or(Error::Schema("missing block id"))?,
            object_id: object.ok_or(Error::Schema("missing object id"))?,
            chunk_index: ci.ok_or(Error::Schema("missing chunk index"))?,
            chunk_count: cc.ok_or(Error::Schema("missing chunk count"))?,
            suite,
            nonce: nonce.ok_or(Error::Schema("missing nonce"))?,
            ciphertext: ct.ok_or(Error::Schema("missing ciphertext"))?,
            pad_class: pad_class.ok_or(Error::Schema("missing pad class"))?,
        })
    }

    /// `SHA-256(canonical envelope)` — the digest of the **encrypted** block.
    ///
    /// This is the value a Merkle parent node authenticates for each of its
    /// children (see [`crate::object`]), and it is exactly the byte string
    /// [`Self::commit_id`] wraps for a root block.
    pub fn digest(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.encode());
        h.finalize().into()
    }

    /// The `CommitId` of a root block = `SHA-256(canonical envelope)` (§8.3).
    pub fn commit_id(&self) -> CommitId {
        CommitId::from_bytes(self.digest())
    }
}

/// Seal a single block. `nonce` is supplied explicitly so the deterministic test
/// vectors can pin it; production callers use [`seal_block_random`].
#[allow(clippy::too_many_arguments)]
pub fn seal_block(
    k_read: &Secret32,
    ctx: &BlockContext,
    object: &ObjectId,
    block: &BlockId,
    chunk_index: u64,
    chunk_count: u64,
    nonce: [u8; AEAD_NONCE_LEN],
    plaintext: &[u8],
) -> Result<BlockEnvelope> {
    let padded = pad(plaintext)?;
    let pad_class = padded.len() as u64;
    let ok = object_key(k_read, &ctx.repo, &ctx.branch, ctx.epoch, object);
    let bk = block_key(&ok, block, chunk_index);
    let aad = associated_data(ctx, object, block, chunk_index, chunk_count, pad_class);
    let ct = aead_seal(&bk, &nonce, &aad, &padded);
    Ok(BlockEnvelope {
        block_id: *block,
        object_id: *object,
        chunk_index,
        chunk_count,
        suite: SUITE_V0.to_string(),
        nonce,
        ciphertext: ct,
        pad_class,
    })
}

/// Seal a block with a fresh random nonce and a fresh random block id.
pub fn seal_block_random(
    k_read: &Secret32,
    ctx: &BlockContext,
    object: &ObjectId,
    chunk_index: u64,
    chunk_count: u64,
    plaintext: &[u8],
) -> Result<BlockEnvelope> {
    let block = BlockId::random();
    let mut nonce = [0u8; AEAD_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    seal_block(k_read, ctx, object, &block, chunk_index, chunk_count, nonce, plaintext)
}

/// Open a block, rebuilding the associated data from `ctx`. Fails closed if the
/// key, tag, or any bound field (including the opaque-header repo/branch/epoch/
/// kind and chunk position) does not match.
pub fn open_block(
    k_read: &Secret32,
    ctx: &BlockContext,
    env: &BlockEnvelope,
) -> Result<Vec<u8>> {
    check_suite(&env.suite)?;
    let ok = object_key(k_read, &ctx.repo, &ctx.branch, ctx.epoch, &env.object_id);
    let bk = block_key(&ok, &env.block_id, env.chunk_index);
    let aad = associated_data(
        ctx,
        &env.object_id,
        &env.block_id,
        env.chunk_index,
        env.chunk_count,
        env.pad_class,
    );
    let padded = aead_open(&bk, &env.nonce, &aad, &env.ciphertext)?;
    unpad(&padded)
}

// ===========================================================================
// Commit root object (§8.3 decrypted root)
// ===========================================================================

/// A decrypted commit root object (§8.3). Its plaintext is the CBOR of fields
/// 1..11; the author signature (field 11) binds fields 1..10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Repository id.
    pub repo: RepoId,
    /// Branch id.
    pub branch: BranchId,
    /// Epoch.
    pub epoch: Epoch,
    /// Causal parents (a frontier).
    pub parents: Vec<CommitId>,
    /// Author public-key id (raw Ed25519 public key).
    pub author: AuthorKeyId,
    /// Author logical clock.
    pub clock: u64,
    /// CRDT kind (`"or-set-quads-v0"` in the initial profile).
    pub crdt_kind: String,
    /// Operation-object roots.
    pub operations: Vec<ObjectId>,
    /// Optional snapshot root.
    pub snapshot: Option<ObjectId>,
    /// Author signature over fields 1..10 (set by [`Commit::sign`]).
    pub author_sig: Option<[u8; SIGNATURE_LEN]>,
}

// wire keys (§8.3 decrypted root)
const C_VERSION: u64 = 1;
const C_REPO: u64 = 2;
const C_BRANCH: u64 = 3;
const C_EPOCH: u64 = 4;
const C_PARENTS: u64 = 5;
const C_AUTHOR: u64 = 6;
const C_CLOCK: u64 = 7;
const C_CRDT: u64 = 8;
const C_OPS: u64 = 9;
const C_SNAPSHOT: u64 = 10;
const C_SIG: u64 = 11;
const C_VERSION_VAL: u64 = 0;

impl Commit {
    fn signing_entries(&self) -> Vec<(u64, Vec<u8>)> {
        let mut e = vec![
            (C_VERSION, enc_uint(C_VERSION_VAL)),
            (C_REPO, enc_bytes(self.repo.as_bytes())),
            (C_BRANCH, enc_bytes(self.branch.as_bytes())),
            (C_EPOCH, enc_uint(self.epoch.0)),
            (
                C_PARENTS,
                enc_array(
                    &self
                        .parents
                        .iter()
                        .map(|p| enc_bytes(p.as_bytes()))
                        .collect::<Vec<_>>(),
                ),
            ),
            (C_AUTHOR, enc_bytes(self.author.as_bytes())),
            (C_CLOCK, enc_uint(self.clock)),
            (C_CRDT, enc_text(&self.crdt_kind)),
            (
                C_OPS,
                enc_array(
                    &self
                        .operations
                        .iter()
                        .map(|o| enc_bytes(o.as_bytes()))
                        .collect::<Vec<_>>(),
                ),
            ),
        ];
        if let Some(s) = &self.snapshot {
            e.push((C_SNAPSHOT, enc_bytes(s.as_bytes())));
        }
        e
    }

    /// The exact bytes the author signs (fields 1..10).
    pub fn signing_bytes(&self) -> Vec<u8> {
        enc_map(self.signing_entries())
    }

    /// Sign the commit with the author key (field 6 must match the author's
    /// public key). Sets `author_sig`.
    pub fn sign(&mut self, author: &SecretSigningKey) {
        self.author = AuthorKeyId::from_bytes(author.public().to_bytes());
        let sig = author.sign(&self.signing_bytes());
        self.author_sig = Some(sig);
    }

    /// Verify the author signature against the commit's own author key id.
    pub fn verify(&self) -> Result<()> {
        let sig = self.author_sig.ok_or(Error::Schema("missing author signature"))?;
        let pk = PublicVerifyingKey::from_bytes(self.author.as_bytes())?;
        pk.verify(&self.signing_bytes(), &sig)
    }

    /// Canonical plaintext of the full commit (fields 1..11).
    pub fn encode(&self) -> Vec<u8> {
        let mut e = self.signing_entries();
        if let Some(sig) = &self.author_sig {
            e.push((C_SIG, enc_bytes(sig)));
        }
        enc_map(e)
    }

    /// Decode + validate a commit plaintext (fail-closed).
    pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut r = Reader::new(bytes, limits);
        let mut version = None;
        let mut repo = None;
        let mut branch = None;
        let mut epoch = None;
        let mut parents: Option<Vec<CommitId>> = None;
        let mut author = None;
        let mut clock = None;
        let mut crdt: Option<String> = None;
        let mut ops: Option<Vec<ObjectId>> = None;
        let mut snapshot = None;
        let mut sig = None;
        read_struct_map(&mut r, |r, key| match key {
            C_VERSION => {
                version = Some(r.uint()?);
                Ok(true)
            }
            C_REPO => {
                repo = Some(RepoId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            C_BRANCH => {
                branch = Some(BranchId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            C_EPOCH => {
                epoch = Some(Epoch(r.uint()?));
                Ok(true)
            }
            C_PARENTS => {
                let n = r.array_header()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(CommitId::from_bytes(r.bytes_fixed::<32>()?));
                }
                parents = Some(v);
                Ok(true)
            }
            C_AUTHOR => {
                author = Some(AuthorKeyId::from_bytes(r.bytes_fixed::<PUBLIC_KEY_LEN>()?));
                Ok(true)
            }
            C_CLOCK => {
                clock = Some(r.uint()?);
                Ok(true)
            }
            C_CRDT => {
                crdt = Some(r.text()?.to_owned());
                Ok(true)
            }
            C_OPS => {
                let n = r.array_header()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(ObjectId::from_bytes(r.bytes_fixed::<32>()?));
                }
                ops = Some(v);
                Ok(true)
            }
            C_SNAPSHOT => {
                snapshot = Some(ObjectId::from_bytes(r.bytes_fixed::<32>()?));
                Ok(true)
            }
            C_SIG => {
                sig = Some(r.bytes_fixed::<SIGNATURE_LEN>()?);
                Ok(true)
            }
            _ => Ok(false),
        })?;
        r.finish()?;
        if version != Some(C_VERSION_VAL) {
            return Err(Error::Schema("commit version"));
        }
        Ok(Commit {
            repo: repo.ok_or(Error::Schema("missing repo"))?,
            branch: branch.ok_or(Error::Schema("missing branch"))?,
            epoch: epoch.ok_or(Error::Schema("missing epoch"))?,
            parents: parents.ok_or(Error::Schema("missing parents"))?,
            author: author.ok_or(Error::Schema("missing author"))?,
            clock: clock.ok_or(Error::Schema("missing clock"))?,
            crdt_kind: crdt.ok_or(Error::Schema("missing crdt kind"))?,
            operations: ops.ok_or(Error::Schema("missing operations"))?,
            snapshot,
            author_sig: sig,
        })
    }
}
