//! **Object chunker — Merkle-linked multi-block objects** (design §2 item 2,
//! §8.3 "objects are chunked into Merkle trees").
//!
//! [`envelope`](crate::envelope) seals **one** block. This module is the layer
//! above it: it splits an object plaintext (a large commit, CRDT operation set,
//! or materialized snapshot) into `N` leaf blocks and links them with internal
//! **Merkle node** blocks, so a single object can span many blocks while still
//! having one root identity.
//!
//! ```text
//!                 root node (chunk 0, level L)
//!                 ├── child link { block id, chunk index, SHA-256(child envelope) }
//!                 …
//!        node (level L-1)            node (level L-1)
//!        ├── leaf   ├── leaf         ├── leaf   ├── leaf
//! ```
//!
//! ## What the structure binds
//!
//! * Every block id is **random** (§4.1/§8.3) — never a hash of plaintext or
//!   ciphertext — so the block set leaks no equality/confirmation information.
//! * A node's plaintext is the **encrypted parent** of its children: it carries,
//!   for each child, the child's random block id, its chunk index, and
//!   `SHA-256` of the child's canonical [`BlockEnvelope`] bytes. Those links are
//!   therefore authenticated *inside* the AEAD, invisible to a broker, and
//!   tamper-evident: changing any byte of any block changes its digest and
//!   breaks the chain up to the root.
//! * Because each digest covers the whole canonical child envelope, the root
//!   digest is the Merkle root of the entire object. For a commit root object
//!   that root digest is exactly `CommitId = SHA-256(canonical encrypted root
//!   envelope)` (§8.3) — see [`SealedObject::commit_id`].
//! * `chunk_index` `0` is **reserved for the root node**, and every block of an
//!   object carries the same `chunk_count` (the total block count). Opening
//!   rejects a duplicate `(object, chunk)` position, a block referenced twice, a
//!   missing child, and a block set whose size disagrees with `chunk_count`
//!   (§8.3 "implementations reject … duplicate `(object, chunk)` positions").
//!
//! ## Honesty / audit boundary
//!
//! As with the rest of this crate, these properties are **designed/intended,
//! not proven**: the construction is research-grade and has had **no external
//! cryptographic review** (gated by `sq-qhy4`). Chunking is deliberately *not*
//! convergent/content-defined — a content-defined chunker would reintroduce the
//! plaintext-derived boundaries the profile rejects, so deduplication across
//! objects is intentionally sacrificed (design §2).
//!
//! ## Example
//!
//! ```
//! use sparq_e2ee_ng::envelope::{BlockContext, ObjectKind};
//! use sparq_e2ee_ng::ids::{BranchId, Epoch, ObjectId, RepoId, Secret32};
//! use sparq_e2ee_ng::object::{seal_object, ObjectLimits};
//!
//! let k_read = Secret32::random();
//! let ctx = BlockContext {
//!     repo: RepoId::random(),
//!     branch: BranchId::random(),
//!     epoch: Epoch(0),
//!     kind: ObjectKind::Snapshot,
//! };
//! let plaintext = vec![7u8; 400_000];           // spans several blocks
//! let sealed = seal_object(&k_read, &ctx, &ObjectId::random(), &plaintext)?;
//! assert!(sealed.block_count() > 1);
//! assert_eq!(sealed.open(&k_read, &ctx, ObjectLimits::default())?, plaintext);
//! # Ok::<(), sparq_e2ee_ng::Error>(())
//! ```

use crate::cbor::{enc_array, enc_bytes, enc_map, enc_uint, read_struct_map, Limits, Reader};
use crate::envelope::{open_block, seal_block_random, BlockContext, BlockEnvelope, PAD_CLASSES};
use crate::error::{Error, Result};
use crate::ids::{BlockId, CommitId, ObjectId, Secret32};
use std::collections::{BTreeMap, BTreeSet};

/// The chunk index reserved for an object's Merkle **root** node block. No other
/// block of the object may occupy it, so a leaf can never be presented as a root.
pub const ROOT_CHUNK_INDEX: u64 = 0;

/// Default leaf chunk size: the largest plaintext that fits the 64 KiB padding
/// class exactly (the 4-byte pad header takes the remainder), so a full leaf
/// wastes no padding and its ciphertext stays well under the default
/// [`Limits::max_str_len`].
pub const DEFAULT_CHUNK_SIZE: usize = 65_536 - 4;

/// Default Merkle fan-out. At the default chunk size one level covers ~2 MiB,
/// two levels ~64 MiB, three ~2 GiB.
pub const DEFAULT_ARITY: usize = 32;

/// Largest fan-out accepted by [`ObjectLayout::validate`]. Chosen so a full
/// node's plaintext (and hence its ciphertext) stays under the default
/// [`Limits::max_str_len`] of 1 MiB, and its child array under the default
/// [`Limits::max_array_len`].
pub const MAX_ARITY: usize = 1024;

/// How [`seal_object_with`] shapes an object's Merkle tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectLayout {
    /// Plaintext bytes per leaf block. It must fit a padding class; a value far
    /// above [`DEFAULT_CHUNK_SIZE`] produces envelopes whose ciphertext field
    /// exceeds the default [`Limits::max_str_len`], so a deployment choosing one
    /// must raise the `Limits` it decodes blocks with to match.
    pub chunk_size: usize,
    /// Maximum children per internal Merkle node.
    pub arity: usize,
}

impl Default for ObjectLayout {
    fn default() -> Self {
        ObjectLayout { chunk_size: DEFAULT_CHUNK_SIZE, arity: DEFAULT_ARITY }
    }
}

impl ObjectLayout {
    /// Reject a layout that cannot produce a well-formed tree: a zero chunk
    /// size, a chunk that no padding class can hold, or a fan-out below 2
    /// (which would never converge to a single root) or above [`MAX_ARITY`].
    pub fn validate(&self) -> Result<()> {
        if self.chunk_size == 0 {
            return Err(Error::Schema("object chunk size must be non-zero"));
        }
        let largest = *PAD_CLASSES.last().expect("PAD_CLASSES is non-empty");
        if self.chunk_size.saturating_add(4) > largest {
            return Err(Error::LimitExceeded("object chunk size exceeds largest pad class"));
        }
        if self.arity < 2 {
            return Err(Error::Schema("object merkle arity must be at least 2"));
        }
        if self.arity > MAX_ARITY {
            return Err(Error::LimitExceeded("object merkle arity too large"));
        }
        Ok(())
    }
}

/// Fail-closed bounds applied when *opening* an object (§8.1 deployment limits).
/// They bound the work an attacker-supplied block set can force before any of it
/// is trusted.
#[derive(Debug, Clone, Copy)]
pub struct ObjectLimits {
    /// Largest accepted `chunk_count` (total blocks in one object).
    pub max_blocks: u64,
    /// Largest accepted reassembled plaintext, in bytes.
    pub max_plaintext_len: usize,
    /// Largest accepted root node level, which bounds traversal depth (each
    /// step strictly decreases the level, so this bounds recursion).
    pub max_level: u64,
    /// Limits applied to every node plaintext decode.
    pub cbor: Limits,
}

impl Default for ObjectLimits {
    fn default() -> Self {
        ObjectLimits {
            max_blocks: 1 << 16,
            max_plaintext_len: 64 << 20,
            max_level: 64,
            cbor: Limits::default(),
        }
    }
}

/// A whole object sealed into a Merkle-linked set of encrypted blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedObject {
    /// The object id every block is bound to.
    pub object_id: ObjectId,
    /// Every block of the object, **root first** (the root is the block whose
    /// `chunk_index` is [`ROOT_CHUNK_INDEX`]); the remaining order is not
    /// significant, since a reader follows the authenticated child links.
    pub blocks: Vec<BlockEnvelope>,
}

impl SealedObject {
    /// The Merkle root block, fail-closed if the set is empty or its first block
    /// is not the reserved root chunk.
    pub fn root(&self) -> Result<&BlockEnvelope> {
        let root = self.blocks.first().ok_or(Error::Schema("sealed object has no blocks"))?;
        if root.chunk_index != ROOT_CHUNK_INDEX {
            return Err(Error::Schema("sealed object root is not chunk 0"));
        }
        Ok(root)
    }

    /// Total number of blocks (leaves plus internal Merkle nodes).
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// The Merkle root of the object: `SHA-256` of the canonical **encrypted**
    /// root envelope. Every block is transitively committed to by this value.
    pub fn merkle_root(&self) -> Result<[u8; 32]> {
        Ok(self.root()?.digest())
    }

    /// The same bytes as [`Self::merkle_root`], typed as a `CommitId` — for an
    /// object sealed with [`ObjectKind::Commit`](crate::envelope::ObjectKind)
    /// this *is* the commit identity of §8.3.
    pub fn commit_id(&self) -> Result<CommitId> {
        Ok(self.root()?.commit_id())
    }

    /// Verify and reassemble the object plaintext from this block set.
    pub fn open(
        &self,
        k_read: &Secret32,
        ctx: &BlockContext,
        limits: ObjectLimits,
    ) -> Result<Vec<u8>> {
        open_object(k_read, ctx, self.root()?, &self.blocks, limits)
    }
}

// ===========================================================================
// Merkle node plaintext (`ObjectNodeV0`)
// ===========================================================================

// wire keys of a node plaintext
const N_VERSION: u64 = 1;
const N_LEVEL: u64 = 2;
const N_CHILDREN: u64 = 3;
const N_LEN: u64 = 4;
const N_VERSION_VAL: u64 = 0;

// wire keys of one child link
const CH_BLOCK: u64 = 1;
const CH_CHUNK: u64 = 2;
const CH_DIGEST: u64 = 3;

/// One authenticated edge from a Merkle node to a child block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChildLink {
    block: BlockId,
    chunk_index: u64,
    digest: [u8; 32],
}

/// The decrypted plaintext of an internal Merkle node.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Node {
    /// `1` = children are leaves; `n > 1` = children are level-`n-1` nodes.
    level: u64,
    children: Vec<ChildLink>,
    /// Total plaintext bytes of the subtree rooted at this node.
    subtree_len: u64,
}

impl Node {
    fn encode(&self) -> Vec<u8> {
        let children: Vec<Vec<u8>> = self
            .children
            .iter()
            .map(|c| {
                enc_map(vec![
                    (CH_BLOCK, enc_bytes(c.block.as_bytes())),
                    (CH_CHUNK, enc_uint(c.chunk_index)),
                    (CH_DIGEST, enc_bytes(&c.digest)),
                ])
            })
            .collect();
        enc_map(vec![
            (N_VERSION, enc_uint(N_VERSION_VAL)),
            (N_LEVEL, enc_uint(self.level)),
            (N_CHILDREN, enc_array(&children)),
            (N_LEN, enc_uint(self.subtree_len)),
        ])
    }

    fn decode(bytes: &[u8], limits: Limits) -> Result<Self> {
        let mut r = Reader::new(bytes, limits);
        let mut version = None;
        let mut level = None;
        let mut children: Option<Vec<ChildLink>> = None;
        let mut subtree_len = None;
        read_struct_map(&mut r, |r, key| match key {
            N_VERSION => {
                version = Some(r.uint()?);
                Ok(true)
            }
            N_LEVEL => {
                level = Some(r.uint()?);
                Ok(true)
            }
            N_CHILDREN => {
                let n = r.array_header()?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(decode_child(r)?);
                }
                children = Some(v);
                Ok(true)
            }
            N_LEN => {
                subtree_len = Some(r.uint()?);
                Ok(true)
            }
            _ => Ok(false),
        })?;
        r.finish()?;
        if version != Some(N_VERSION_VAL) {
            return Err(Error::Schema("object node version"));
        }
        Ok(Node {
            level: level.ok_or(Error::Schema("missing node level"))?,
            children: children.ok_or(Error::Schema("missing node children"))?,
            subtree_len: subtree_len.ok_or(Error::Schema("missing node subtree length"))?,
        })
    }
}

fn decode_child(r: &mut Reader<'_>) -> Result<ChildLink> {
    let mut block = None;
    let mut chunk_index = None;
    let mut digest = None;
    read_struct_map(r, |r, key| match key {
        CH_BLOCK => {
            block = Some(BlockId::from_bytes(r.bytes_fixed::<32>()?));
            Ok(true)
        }
        CH_CHUNK => {
            chunk_index = Some(r.uint()?);
            Ok(true)
        }
        CH_DIGEST => {
            digest = Some(r.bytes_fixed::<32>()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;
    Ok(ChildLink {
        block: block.ok_or(Error::Schema("missing child block id"))?,
        chunk_index: chunk_index.ok_or(Error::Schema("missing child chunk index"))?,
        digest: digest.ok_or(Error::Schema("missing child digest"))?,
    })
}

// ===========================================================================
// Seal
// ===========================================================================

/// Chunk `plaintext` into a Merkle-linked block set with the default
/// [`ObjectLayout`]. All block ids and nonces are freshly random.
pub fn seal_object(
    k_read: &Secret32,
    ctx: &BlockContext,
    object: &ObjectId,
    plaintext: &[u8],
) -> Result<SealedObject> {
    seal_object_with(k_read, ctx, object, plaintext, ObjectLayout::default())
}

/// [`seal_object`] with an explicit layout.
///
/// The tree is built bottom-up: leaves take chunk indices `1..=leaf_count`,
/// internal nodes take the indices after them level by level, and the single
/// top node takes the reserved [`ROOT_CHUNK_INDEX`]. An object always has a node
/// root — even a one-leaf object — so a leaf is never mistakable for a root.
pub fn seal_object_with(
    k_read: &Secret32,
    ctx: &BlockContext,
    object: &ObjectId,
    plaintext: &[u8],
    layout: ObjectLayout,
) -> Result<SealedObject> {
    layout.validate()?;

    // An empty object still has exactly one (empty) leaf, so the tree shape is
    // uniform and `open_object` never has to special-case a childless root.
    let leaf_count = plaintext.len().div_ceil(layout.chunk_size).max(1);

    // Level sizes bottom-up: index 0 is the leaves, the last entry is always 1
    // (the root). A single leaf still gets one node level above it.
    let mut level_sizes = vec![leaf_count];
    loop {
        let n = level_sizes[level_sizes.len() - 1].div_ceil(layout.arity);
        level_sizes.push(n);
        if n == 1 {
            break;
        }
    }
    let total = level_sizes
        .iter()
        .try_fold(0usize, |acc, n| acc.checked_add(*n))
        .ok_or(Error::LimitExceeded("object block count overflow"))?;
    let chunk_count =
        u64::try_from(total).map_err(|_| Error::LimitExceeded("object block count overflow"))?;

    let mut blocks: Vec<BlockEnvelope> = Vec::with_capacity(total);
    let mut next_index: u64 = ROOT_CHUNK_INDEX + 1;

    // Leaves.
    let mut lower: Vec<(ChildLink, u64)> = Vec::with_capacity(leaf_count);
    for i in 0..leaf_count {
        let start = i * layout.chunk_size;
        let end = (start + layout.chunk_size).min(plaintext.len());
        let data = plaintext.get(start..end).unwrap_or(&[]);
        let index = next_index;
        next_index += 1;
        let env = seal_block_random(k_read, ctx, object, index, chunk_count, data)?;
        lower.push((
            ChildLink { block: env.block_id, chunk_index: index, digest: env.digest() },
            data.len() as u64,
        ));
        blocks.push(env);
    }

    // Internal Merkle levels, bottom-up.
    for level in 1..level_sizes.len() {
        let is_root_level = level == level_sizes.len() - 1;
        let mut upper: Vec<(ChildLink, u64)> = Vec::with_capacity(level_sizes[level]);
        for group in lower.chunks(layout.arity) {
            let index = if is_root_level {
                ROOT_CHUNK_INDEX
            } else {
                let i = next_index;
                next_index += 1;
                i
            };
            let subtree_len = group
                .iter()
                .try_fold(0u64, |acc, (_, len)| acc.checked_add(*len))
                .ok_or(Error::LimitExceeded("object plaintext length overflow"))?;
            let node = Node {
                level: level as u64,
                children: group.iter().map(|(link, _)| *link).collect(),
                subtree_len,
            };
            let env = seal_block_random(k_read, ctx, object, index, chunk_count, &node.encode())?;
            upper.push((
                ChildLink { block: env.block_id, chunk_index: index, digest: env.digest() },
                subtree_len,
            ));
            blocks.push(env);
        }
        lower = upper;
    }

    // The root is the last block sealed; move it to the front.
    blocks.rotate_right(1);
    Ok(SealedObject { object_id: *object, blocks })
}

// ===========================================================================
// Open
// ===========================================================================

/// Verify a Merkle-linked block set and reassemble the object plaintext.
///
/// `blocks` is **exactly** the object's block set, in any order, including
/// `root`; a caller holding a larger store filters it by `object_id` first. Any
/// extra, duplicate, or missing block is rejected rather than ignored.
///
/// Every child is checked against the digest, block id, chunk index, object id,
/// and chunk count its **encrypted parent** recorded, so a substituted,
/// reordered, duplicated, or missing block fails closed before its plaintext is
/// used, and every block must be reachable from the root exactly once.
pub fn open_object(
    k_read: &Secret32,
    ctx: &BlockContext,
    root: &BlockEnvelope,
    blocks: &[BlockEnvelope],
    limits: ObjectLimits,
) -> Result<Vec<u8>> {
    if root.chunk_index != ROOT_CHUNK_INDEX {
        return Err(Error::Schema("object root is not chunk 0"));
    }
    if root.chunk_count < 2 {
        return Err(Error::Schema("object chunk count must include a root and a leaf"));
    }
    if root.chunk_count > limits.max_blocks {
        return Err(Error::LimitExceeded("object block count exceeds limit"));
    }

    if blocks.len() as u64 != root.chunk_count {
        return Err(Error::Schema("object block set size does not match chunk count"));
    }

    let mut index: BTreeMap<BlockId, &BlockEnvelope> = BTreeMap::new();
    for b in blocks {
        if b.object_id != root.object_id {
            return Err(Error::Schema("block set contains another object's block"));
        }
        if index.insert(b.block_id, b).is_some() {
            return Err(Error::Schema("duplicate block id in object block set"));
        }
    }

    // The root must itself be a member of `blocks`, and be *the* block that set
    // holds under its id. Otherwise a caller could drop the real root, add any
    // other block to keep `blocks.len()` matching `chunk_count`, and still reach
    // `visited.len() == chunk_count` by traversing an externally supplied root —
    // accepting a set that is not exactly the object's blocks.
    let indexed_root =
        *index.get(&root.block_id).ok_or(Error::Schema("object root is not in the block set"))?;
    if indexed_root != root {
        return Err(Error::Schema("object root differs from the block set entry with its id"));
    }

    let mut opener = Opener {
        k_read,
        ctx,
        object: root.object_id,
        chunk_count: root.chunk_count,
        index,
        visited: BTreeSet::new(),
        positions: BTreeSet::new(),
        limits,
        out: Vec::new(),
    };
    opener.visited.insert(indexed_root.block_id);
    opener.positions.insert(indexed_root.chunk_index);
    opener.visit(indexed_root, None)?;

    // Completeness: exactly the declared number of blocks was reachable.
    if opener.visited.len() as u64 != root.chunk_count {
        return Err(Error::Schema("object block count does not match chunk count"));
    }
    Ok(opener.out)
}

struct Opener<'a> {
    k_read: &'a Secret32,
    ctx: &'a BlockContext,
    object: ObjectId,
    chunk_count: u64,
    index: BTreeMap<BlockId, &'a BlockEnvelope>,
    visited: BTreeSet<BlockId>,
    positions: BTreeSet<u64>,
    limits: ObjectLimits,
    out: Vec<u8>,
}

impl Opener<'_> {
    /// Open one Merkle node and everything under it, appending leaf plaintext to
    /// `out` in object order. Returns the subtree's plaintext length.
    ///
    /// `expected_level` is `None` only for the root (whose level is read from
    /// its own authenticated plaintext and bounded by `limits.max_level`).
    /// Because every recursive step decreases the level by exactly one and a
    /// level of `0` is not a node, recursion depth is bounded by `max_level`.
    fn visit(&mut self, env: &BlockEnvelope, expected_level: Option<u64>) -> Result<u64> {
        let node = Node::decode(&open_block(self.k_read, self.ctx, env)?, self.limits.cbor)?;
        let level = match expected_level {
            Some(expected) => {
                if node.level != expected {
                    return Err(Error::Integrity("merkle node level does not match its parent"));
                }
                expected
            }
            None => {
                if node.level == 0 {
                    return Err(Error::Schema("object root node has level 0"));
                }
                if node.level > self.limits.max_level {
                    return Err(Error::LimitExceeded("object merkle tree too deep"));
                }
                if node.subtree_len > self.limits.max_plaintext_len as u64 {
                    return Err(Error::LimitExceeded("object plaintext exceeds limit"));
                }
                node.level
            }
        };
        if node.children.is_empty() {
            return Err(Error::Schema("merkle node has no children"));
        }

        let mut subtree_len: u64 = 0;
        for link in &node.children {
            if link.chunk_index == ROOT_CHUNK_INDEX {
                return Err(Error::Schema("only the root may occupy chunk 0"));
            }
            let child = *self
                .index
                .get(&link.block)
                .ok_or(Error::Schema("object child block is missing"))?;
            if !self.visited.insert(link.block) {
                return Err(Error::Schema("object block referenced more than once"));
            }
            if self.visited.len() as u64 > self.chunk_count {
                return Err(Error::Schema("object has more blocks than its chunk count"));
            }
            if !self.positions.insert(link.chunk_index) {
                return Err(Error::Schema("duplicate (object, chunk) position"));
            }
            // The digest covers the whole canonical child envelope, so it alone
            // is binding; the field checks make a mismatch diagnosable.
            if child.object_id != self.object
                || child.chunk_count != self.chunk_count
                || child.chunk_index != link.chunk_index
            {
                return Err(Error::Integrity("child block does not match its merkle link"));
            }
            if child.digest() != link.digest {
                return Err(Error::Integrity("merkle child digest mismatch"));
            }

            let len = if level == 1 {
                let data = open_block(self.k_read, self.ctx, child)?;
                if self.out.len().saturating_add(data.len()) > self.limits.max_plaintext_len {
                    return Err(Error::LimitExceeded("object plaintext exceeds limit"));
                }
                self.out.extend_from_slice(&data);
                data.len() as u64
            } else {
                self.visit(child, Some(level - 1))?
            };
            subtree_len = subtree_len
                .checked_add(len)
                .ok_or(Error::Malformed("object length overflow"))?;
        }

        if subtree_len != node.subtree_len {
            return Err(Error::Integrity("merkle node subtree length mismatch"));
        }
        Ok(subtree_len)
    }
}
