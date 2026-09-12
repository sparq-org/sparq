// [OPUS-4.8] sq-3e5 + sq-h2v: hidden-index revocation host-side helpers.
//! Host-side commitment + witness machinery for the hidden-index revocation
//! proof (sq-3e5 / sq-h2v). The in-circuit relation is
//! `zk/compose/compose_core/src/revoke.nr`; this module is its Rust mirror:
//!
//! - [`merkle_root`] commits a [`StatusListSnapshot`] as a depth-`D` Poseidon2
//!   Merkle tree (leaves = status bits, internal nodes = `h2` =
//!   `poseidon2::hash(&[l, r])` — bit-identical to the circuit's `h2`). The
//!   RELYING PARTY computes this over its OWN authoritative snapshot; the proof's
//!   public `root` must byte-equal it. The PROVER computes the same root over the
//!   list it holds.
//! - [`merkle_witness`] builds the prover's private witness (the leaf bit + the
//!   authentication path of siblings) for a given index.
//! - [`revoke_prover_toml`] renders the `Prover.toml` for the
//!   `revoke_unset_d{depth}` member.
//!
//! # Leaf / tree layout (MUST match `revoke.nr`)
//! - A status list of `2^D` slots. Bit `i` (LSB-first within each byte, the
//!   StatusList2021 convention — see [`StatusListSnapshot::bit`]) is the leaf at
//!   index `i`. Leaf field value = `Fr::from(bit as u64)` (0 or 1), exactly the
//!   circuit's `leaf = bit`.
//! - Internal node = `h2(left, right)`. Level 0 is the leaves; we fold pairwise
//!   up `D` levels to the single root.
//! - Indices past the snapshot's `bits` length read as SET (revoked) via
//!   [`StatusListSnapshot::bit`] — fail-closed, mirroring the verifier.
//!
//! # Scaling (sq-hwe): sparse builder, dense-identical roots
//! [OPUS-4.8] sq-hwe: [`merkle_root`] / [`merkle_witness`] are SPARSE — they do
//! NOT materialise `2^depth` leaves. A StatusList2021 list is overwhelmingly
//! uniform (a credential is live = bit unset; the revoked set `S` is tiny), so
//! the tree is dominated by all-ZERO subtrees in the covered region and all-ONE
//! subtrees in the fail-closed padding region past the bitstring. We cache the
//! "all-zeros subtree root" and "all-ones subtree root" at every height
//! (`SubtreeDefaults`) and recurse only into the `O(S)` subtrees that actually
//! contain a set bit or straddle the covered/padding boundary. Cost is
//! `O(S * depth + depth)` host work and `O(S * depth)` memory — INDEPENDENT of
//! `2^depth` — so production depths (StatusList2021 lists are commonly 2^17+) are
//! supported WITHOUT the old dense `O(2^depth)` full-size builder (`dense_merkle_root`,
//! retained only as the cross-check oracle).
//!
//! The root is BIT-IDENTICAL to the dense fold (same Poseidon2 `h2`, same leaf
//! layout, same fail-closed padding — indices past `bits` read as SET/1 via
//! [`StatusListSnapshot::bit`]), so the circuit relation (`revoke.nr`,
//! `revoke_unset_check_committed`) and the verifier's root-equality binding are
//! unchanged — only the host-side construction is cheaper. Cross-checked against
//! the dense oracle in the unit tests.
//!
//! # The sq-6qe / sq-kndw ACCEPTED-SET anchor + FULLY-HIDDEN witness
//! [OPUS-5] [`AcceptedStatusEntry`] / [`accepted_set_leaf`] / [`accepted_set_root`]
//! / [`accepted_set_witness`] are the host mirror of the sub-option-A accepted-set
//! commitment in `research/zk-statuslist-hide-iri-version.md` §3 — the relying
//! party commits every `(list, version, status_list_root)` triple it currently
//! trusts to ONE Merkle root, so the fully-hidden circuit can prove "some accepted
//! `(list, version)` has my hidden index unset" without the verifier learning WHICH
//! list/version. [`HiddenRefWitness`] / [`hidden_ref_witness`] assemble the
//! prover's private witness for that circuit (the accepted-set slot + path + the
//! privately-bound `status_list_root`, plus the status-list half), and
//! [`revoke_hidden_ref_prover_toml`] renders its `Prover.toml`.
//!
//! sq-kndw closed the loop: the member `revoke_hidden_ref_d10_a4` is COMPILED, the
//! manifest carries a fully-hidden [`crate::manifest::RevocationStatus`] mode, and
//! `crate::verifier::bind_fully_hidden_revocation` consumes the anchor — so on that
//! mode the status-list IRI and version are NOT disclosed. The committed-index path
//! below is unchanged and still discloses both. Nothing here is externally audited
//! (sq-qhy4).
//!
//! # Residual scope (honest)
//! Only the `revoke_unset_d10` circuit member is currently COMPILED, so although
//! the host builder now scales, an end-to-end hidden-revocation PROOF is still
//! bounded by the compiled member's depth (compiling a larger member —
//! `revoke_unset_d17` etc. — is mechanical: the relation is depth-generic; tracked
//! as follow-up). The same holds for the fully-hidden member, of which only the
//! `(D=10, A=4)` bucket is compiled. And the status-list IRI + version are still
//! disclosed in the clear on the COMMITTED-INDEX path (use the fully-hidden mode to
//! hide them). See the crate verifier docs (`bind_hidden_revocation` /
//! `bind_fully_hidden_revocation`).

use crate::manifest::StatusListSnapshot;
use sparq_zk::field::{field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk::sig;

/// The Poseidon2 two-input compression used for Merkle internal nodes — the Rust
/// mirror of the circuit's `h2(a, b) = Poseidon2::hash([a, b], 2)`. Cross-tested
/// bit-identical (`crates/sparq-zk/tests/poseidon2_noir_cross.rs`).
fn h2(a: Fr, b: Fr) -> Fr {
    poseidon2::hash(&[a, b])
}

/// The field value of the leaf at `index`: the status bit (0 = active, 1 =
/// revoked) as a field element. Out-of-range reads as 1 (revoked), fail-closed —
/// identical to [`StatusListSnapshot::bit`].
fn leaf(snapshot: &StatusListSnapshot, index: u64) -> Fr {
    Fr::from(u64::from(snapshot.bit(index)))
}

/// [OPUS-4.8] sq-hwe: precomputed roots of UNIFORM subtrees, one per height. The
/// sparse builder folds only the subtrees that contain a non-default leaf; every
/// other subtree's root is read from here in `O(1)`.
///
/// `zeros[h]` is the root of a height-`h` subtree whose `2^h` leaves are ALL the
/// leaf value `0` (active); `ones[h]` is the root of a height-`h` subtree of ALL
/// `1` (revoked / fail-closed padding). Height 0 is a single leaf, so
/// `zeros[0] = Fr(0)` and `ones[0] = Fr(1)` (the circuit's `leaf = bit`); each
/// higher level is `h2(child, child)` of the level below. Computed once in
/// `O(depth)` — exactly the two "merkle-path-of-a-constant-subtree" defaults a
/// sparse Merkle tree caches.
struct SubtreeDefaults {
    /// `zeros[h]` = root of an all-zero (all-active) height-`h` subtree.
    zeros: Vec<Fr>,
    /// `ones[h]` = root of an all-one (all-revoked / padding) height-`h` subtree.
    ones: Vec<Fr>,
}

impl SubtreeDefaults {
    /// Precompute the uniform-subtree roots for heights `0..=depth` in `O(depth)`.
    fn new(depth: u32) -> Self {
        let mut zeros = Vec::with_capacity(depth as usize + 1);
        let mut ones = Vec::with_capacity(depth as usize + 1);
        zeros.push(Fr::from(0u64)); // a single 0 leaf
        ones.push(Fr::from(1u64)); // a single 1 leaf
        for h in 1..=depth as usize {
            zeros.push(h2(zeros[h - 1], zeros[h - 1]));
            ones.push(h2(ones[h - 1], ones[h - 1]));
        }
        SubtreeDefaults { zeros, ones }
    }
}

/// [OPUS-4.8] sq-hwe: sparse root of the height-`h` subtree covering leaf indices
/// `[lo, lo + 2^h)`, given the snapshot and the precomputed uniform-subtree roots.
///
/// `set_after` is the first leaf index past the snapshot's covered bits
/// (`8 * bits.len()`): every leaf at `idx >= set_after` reads as `1` (the
/// fail-closed padding), and every leaf below it reads the snapshot bit. The three
/// fast paths that avoid recursion (the whole point of the sparse builder):
///   1. the subtree lies ENTIRELY in the padding (`lo >= set_after`) → all `1` →
///      `defaults.ones[h]`;
///   2. the subtree lies entirely in the covered region AND holds NO set bit →
///      all `0` → `defaults.zeros[h]`;
///   3. otherwise recurse into the two half-subtrees and combine with `h2`.
///
/// A height-0 subtree is a single leaf and just reads its bit. Recursion depth is
/// `depth`, and a branch only splits when it contains a set bit or straddles the
/// covered/padding boundary, so the work is `O(S * depth + depth)`.
fn sparse_subtree_root(
    snapshot: &StatusListSnapshot,
    defaults: &SubtreeDefaults,
    set_after: u64,
    lo: u64,
    h: u32,
) -> Fr {
    // Fast path 1: the whole subtree is in the fail-closed padding (all ones).
    if lo >= set_after {
        return defaults.ones[h as usize];
    }
    let span = 1u64 << h;
    // Fast path 2: the whole subtree is in the covered region and contains no set
    // bit. `hi` is exclusive; `lo < set_after` already, so if `hi <= set_after`
    // the subtree is fully covered. `any_bit_set` short-circuits on the first set
    // bit, so a genuinely all-zero subtree costs only the byte-range scan.
    let hi = lo + span;
    if hi <= set_after && !any_bit_set(snapshot, lo, hi) {
        return defaults.zeros[h as usize];
    }
    if h == 0 {
        // Single leaf: read the (boundary-straddling or set) bit directly.
        return leaf(snapshot, lo);
    }
    // Recurse into the two halves; one of them is frequently a cached uniform
    // subtree (fast path 1 or 2 above), so the recursion fans out only along the
    // O(S) branches that hold a set bit or the covered/padding boundary.
    let left = sparse_subtree_root(snapshot, defaults, set_after, lo, h - 1);
    let right = sparse_subtree_root(snapshot, defaults, set_after, lo + (span >> 1), h - 1);
    h2(left, right)
}

/// [OPUS-4.8] sq-hwe: whether ANY status bit in `[lo, hi)` is set, scanning the
/// snapshot's bytes (not `bit()`-per-index) and short-circuiting on the first set
/// bit. Caller guarantees `hi <= 8 * bits.len()` (the covered region), so every
/// byte touched exists. Used by the sparse builder's all-zero fast path.
fn any_bit_set(snapshot: &StatusListSnapshot, lo: u64, hi: u64) -> bool {
    debug_assert!(lo <= hi);
    let mut idx = lo;
    while idx < hi {
        let byte = (idx / 8) as usize;
        let off = (idx % 8) as u32;
        // How many bits remain in this byte from `off`, capped by `hi`.
        let bits_left_in_byte = 8 - off;
        let bits_to_hi = (hi - idx) as u32;
        let take = bits_left_in_byte.min(bits_to_hi);
        // Mask the `take` bits starting at `off` and test them in one shot.
        let mask: u8 = (((1u16 << take) - 1) as u8) << off;
        // The covered-region guarantee means the byte exists, so this fallback is
        // unreachable on the real call site. It is FAIL-CLOSED nonetheless: a
        // missing byte reads as all-ones (`0xFF`), matching `StatusListSnapshot::bit`
        // / `leaf`, which define out-of-range bits as `1` (revoked). So if this
        // helper is ever called on a range that touches a missing byte, it reports
        // the region as set (returns `true`) rather than letting `sparse_subtree_root`
        // misclassify an unknown/padding region as an all-zero subtree.
        let b = snapshot.bits.get(byte).copied().unwrap_or(0xFF);
        if b & mask != 0 {
            return true;
        }
        idx += u64::from(take);
    }
    false
}

/// Commit `snapshot` as a depth-`depth` Poseidon2 Merkle tree and return the
/// root. Leaves are the `2^depth` status bits (indices `0..2^depth`, padded with
/// SET/revoked bits past the snapshot length — fail-closed). Internal nodes are
/// `h2`. This is what the RELYING PARTY computes over its authoritative snapshot
/// (the trust anchor the proof's public root is checked against) and what the
/// PROVER computes over the list it holds.
///
/// [OPUS-4.8] sq-hwe: computed by the SPARSE builder (`sparse_subtree_root`) —
/// `O(S * depth + depth)` host work / `O(S * depth)` memory for `S` set bits,
/// INDEPENDENT of `2^depth`, so production depths (2^17+) are supported without a
/// dense full-size builder. The result is BIT-IDENTICAL to the dense fold (same
/// `h2`, same leaf layout, same fail-closed padding); cross-checked against
/// `dense_merkle_root` in the unit tests.
///
/// Returns `None` if `depth > 31` or the snapshot contains more than `2^depth`
/// bits. Snapshots are byte-sized, so a nonempty snapshot requires `depth >= 3`.
/// Oversized authoritative snapshots are rejected rather than prefix-committed.
pub fn merkle_root(snapshot: &StatusListSnapshot, depth: u32) -> Option<Fr> {
    let set_after = covered_bits(snapshot, depth)?;
    let defaults = SubtreeDefaults::new(depth);
    // The first leaf index that reads as SET (padding past the covered bits).
    Some(sparse_subtree_root(snapshot, &defaults, set_after, 0, depth))
}

// [GPT-6] Both roots and paths must cover the whole byte-sized snapshot. Checking
// before any hashing also protects accepted-policy and hidden-reference callers.
fn covered_bits(snapshot: &StatusListSnapshot, depth: u32) -> Option<u64> {
    if depth > 31 {
        return None;
    }
    let bits = u64::try_from(snapshot.bits.len()).ok()?.checked_mul(8)?;
    (bits <= (1u64 << depth)).then_some(bits)
}

/// [OPUS-4.8] sq-hwe: the ORIGINAL dense `O(2^depth)` Merkle-root builder, retained
/// ONLY as the cross-check ORACLE for [`merkle_root`]'s sparse output in tests. It
/// materialises all `2^depth` leaves and folds them pairwise, so it is unusable at
/// production depths — never call it on the hot path. `merkle_root` MUST return a
/// byte-identical root for the same `(snapshot, depth)`; the unit tests assert this
/// across uniform, sparse, and boundary-straddling snapshots.
#[cfg(test)]
fn dense_merkle_root(snapshot: &StatusListSnapshot, depth: u32) -> Option<Fr> {
    if depth > 31 || snapshot.bits.len() > (1usize << depth) / 8 {
        return None;
    }
    let n_leaves = 1usize << depth;
    // Level 0: the leaves.
    let mut level: Vec<Fr> = (0..n_leaves as u64).map(|i| leaf(snapshot, i)).collect();
    // Fold pairwise up to the root.
    for _ in 0..depth {
        level = level
            .chunks(2)
            .map(|pair| h2(pair[0], pair[1]))
            .collect();
    }
    debug_assert_eq!(level.len(), 1, "fold reduces to a single root");
    level.first().copied()
}

/// The hidden-index revocation witness for `index`: the leaf bit and the
/// authentication path (sibling at each level, BOTTOM-UP — level 0 first), the
/// private inputs the `revoke_unset_d{depth}` circuit folds. Mirrors `revoke.nr`'s
/// `(bit, siblings)` exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MerkleWitness {
    /// The status bit at `index` (the circuit's private `bit`; must be 0 for a
    /// satisfiable, i.e. unrevoked, proof).
    pub bit: Fr,
    /// The authentication path: `siblings[k]` is the sibling hash at level `k`
    /// (level 0 = leaves), bottom-up. Length = `depth`.
    pub siblings: Vec<Fr>,
}

/// Build the [`MerkleWitness`] for `index` in `snapshot`'s depth-`depth` tree.
/// `None` if `index >= 2^depth` (out of range for the tree — the circuit also
/// range-bounds the index), `depth > 31`, or the snapshot exceeds the tree's
/// bit capacity. As for [`merkle_root`], oversized snapshots are never truncated.
///
/// [OPUS-4.8] sq-hwe: SPARSE. The sibling at level `k` is the root of the
/// height-`k` subtree hanging off the OTHER side of the path node at that level;
/// each is computed by `sparse_subtree_root` (which short-circuits uniform
/// subtrees against the precomputed defaults), so the whole authentication path is
/// `O(depth^2)` worst case but `O(depth)` for the common case where the siblings
/// are uniform subtrees — and, crucially, NEVER materialises `2^depth` leaves. The
/// path is byte-identical to the one the old dense builder produced (cross-checked
/// in the unit tests by re-folding to the dense root).
pub fn merkle_witness(snapshot: &StatusListSnapshot, depth: u32, index: u64) -> Option<MerkleWitness> {
    let set_after = covered_bits(snapshot, depth)?;
    let n_leaves = 1u64 << depth;
    if index >= n_leaves {
        return None;
    }
    let bit = leaf(snapshot, index);
    let defaults = SubtreeDefaults::new(depth);
    // At level `k` the path node spans 2^k leaves; its sibling is the height-`k`
    // subtree sharing the same parent. `pos` tracks the path node's index AMONG the
    // 2^(depth-k) nodes at level k (i.e. index >> k); the sibling's node index is
    // `pos ^ 1`, whose leaf range begins at `(pos ^ 1) << k`.
    let mut siblings = Vec::with_capacity(depth as usize);
    let mut pos = index;
    for k in 0..depth {
        let sib_node = pos ^ 1;
        let sib_lo = sib_node << k;
        siblings.push(sparse_subtree_root(snapshot, &defaults, set_after, sib_lo, k));
        pos >>= 1;
    }
    Some(MerkleWitness { bit, siblings })
}

/// Render the `Prover.toml` body for the `revoke_unset_d{depth}` member. Order
/// MUST match `revoke_unset_d{depth}/src/main.nr`: challenge, root,
/// index_commitment (public); index, bit, blinding, siblings (private).
///
/// [OPUS-4.8] sq-ayv: the member now binds a HIDING index commitment. The
/// `index_commitment` PUBLIC input is recomputed in-circuit from `index` +
/// `blinding` and byte-matched by the verifier against the ISSUER-SIGNED
/// commitment, so the index proven unset is provably the one the issuer committed
/// to. `index_commitment` MUST equal `sparq_zk::sig::status_index_commitment(index,
/// blinding)` (the issuer-signed value); the host computes it that way.
pub fn revoke_prover_toml(
    challenge: &Fr,
    root: &Fr,
    index_commitment: &Fr,
    index: u64,
    blinding: &Fr,
    witness: &MerkleWitness,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", field_to_hex(challenge)));
    s.push_str(&format!("root = \"{}\"\n", field_to_hex(root)));
    s.push_str(&format!("index_commitment = \"{}\"\n", field_to_hex(index_commitment)));
    s.push_str(&format!("index = \"{index}\"\n"));
    s.push_str(&format!("bit = \"{}\"\n", field_to_hex(&witness.bit)));
    s.push_str(&format!("blinding = \"{}\"\n", field_to_hex(blinding)));
    let sibs: Vec<String> = witness
        .siblings
        .iter()
        .map(|s| format!("\"{}\"", field_to_hex(s)))
        .collect();
    s.push_str(&format!("siblings = [{}]\n", sibs.join(", ")));
    s
}

// =====================================================================
// [OPUS-5] sq-6qe: the ACCEPTED-SET commitment (host side).
//
// The relying party's set of `(status list IRI, version, status-list Merkle root)`
// triples it currently trusts, committed as ONE Poseidon2 Merkle root. This is the
// trust anchor of `research/zk-statuslist-hide-iri-version.md` §3 sub-option A:
// the FUTURE `revoke_hidden_ref_d{D}_a{A}` member proves, in zero knowledge, that
// the credential's HIDDEN `(list, version)` is a member of this set — which also
// PRIVATELY binds the `status_list_root` the bit-unset fold then runs against — so
// the relying party publishes only the SET root and never learns which list,
// version, or root the presentation pertains to.
//
// SCOPE (honest): anchor derivation + the prover's membership path ONLY. No
// circuit member, manifest mode, or verifier gate consumes this yet, so the
// IRI/version disclosure on the committed-index path is UNCHANGED. Not externally
// audited (sq-qhy4).
// =====================================================================

/// [OPUS-5] sq-6qe: one entry of the relying party's ACCEPTED SET — a
/// `(status list IRI, version, status-list Merkle root)` triple it trusts.
///
/// The relying party builds these from its OWN authoritative, freshness-curated
/// snapshots (`RevocationPolicy::accepted_entries`), so the accepted set moves the
/// audit-#12 trust anchor BEHIND a commitment without introducing a new trust
/// assumption: the roots are still the relying party's own, derived from
/// [`merkle_root`] over bitstrings it resolved and authenticated out of band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedStatusEntry {
    /// The status-list IRI (hashed to a field via
    /// [`sparq_zk::sig::status_list_id_to_field`] for the leaf).
    pub status_list: String,
    /// The status list's publication version (the audit-#12 freshness counter).
    pub version: u64,
    /// The depth-`hidden_index_depth` Poseidon2 Merkle root of that
    /// `(list, version)`'s AUTHORITATIVE bitstring — i.e. [`merkle_root`] of the
    /// relying party's own snapshot. The future circuit folds the bit-unset
    /// inclusion against THIS root, kept private.
    pub status_list_root: Fr,
}

/// [OPUS-5] sq-6qe: the accepted-set Merkle LEAF for one entry —
/// [`sparq_zk::sig::accepted_status_leaf`] over the field-mapped IRI. The host and
/// the future in-circuit leaf recompute the SAME hash from the same three inputs,
/// so there is one source of truth and no host/circuit drift.
///
/// Binding all three in one leaf is load-bearing: because the leaf commits the
/// triple ATOMICALLY, a prover cannot pair list₁'s identity with list₂'s root —
/// the membership fold would not reproduce the public accepted-set root.
pub fn accepted_set_leaf(entry: &AcceptedStatusEntry) -> Fr {
    sig::accepted_status_leaf(
        &sig::status_list_id_to_field(&entry.status_list),
        entry.version,
        &entry.status_list_root,
    )
}

/// The padding leaf for unoccupied accepted-set slots — `Fr::from(0)`, the SAME
/// padding [`crate::issuer`]'s key-set tree uses, so the shared sparse fold
/// (`sparse_root_from_leaves`) produces a tree bit-identical to the one the
/// generalised `key_set_membership` relation folds.
///
/// A padding slot is not a usable membership witness: the circuit recomputes the
/// leaf as [`sparq_zk::sig::accepted_status_leaf`] of the private triple, and a
/// Poseidon2 output equal to `0` is negligible-probability, so a prover cannot
/// "prove membership" at an empty slot.
fn accepted_padding_leaf() -> Fr {
    Fr::from(0u64)
}

/// The non-padding accepted-set leaves in the caller's order. `None` if
/// `set_depth > 31` or the set overflows a depth-`set_depth` tree (fail-closed —
/// a relying party whose accepted set does not fit must widen `set_depth`, never
/// silently truncate its trust anchor).
fn accepted_leaves(entries: &[AcceptedStatusEntry], set_depth: u32) -> Option<Vec<Fr>> {
    if set_depth > 31 || entries.len() as u128 > (1u128 << set_depth) {
        return None;
    }
    Some(entries.iter().map(accepted_set_leaf).collect())
}

/// [OPUS-5] sq-6qe: the ACCEPTED-SET Merkle root over `entries` at depth
/// `set_depth` — the relying party's committed liveness policy, and the public
/// input the future fully-hidden revocation proof is bound to.
///
/// `entries` MUST be in the CANONICAL leaf order both sides agree on: sorted by
/// `(status_list, version)`, which is exactly what
/// `RevocationPolicy::accepted_entries` emits (its `BTreeMap` iteration order).
/// The relying party derives the anchor from its own entries; the prover builds
/// its membership path over the same order, so the roots agree — the same
/// canonical-order discipline `KeySet::hidden_issuer_root` uses.
///
/// Uses the shared SPARSE fold, so the cost is `O(n·set_depth)` in the number of
/// accepted `(list, version)` pairs, never `O(2^set_depth)`. `None` if
/// `set_depth > 31` or the set overflows the tree (fail-closed — a relying party
/// whose accepted set does not fit must widen `set_depth`, never silently
/// truncate its trust anchor).
pub fn accepted_set_root(entries: &[AcceptedStatusEntry], set_depth: u32) -> Option<Fr> {
    crate::issuer::sparse_root_from_leaves(
        accepted_leaves(entries, set_depth)?,
        set_depth,
        accepted_padding_leaf(),
    )
}

/// [OPUS-5] sq-6qe: the PROVER's accepted-set membership path for `index` — the
/// sibling at each level, BOTTOM-UP (level 0 = leaves), length `set_depth`. Folds
/// to [`accepted_set_root`] under the circuit's fold, so it is the private witness
/// the future member consumes alongside the private `(list_id, version,
/// status_list_root)` triple.
///
/// `None` if `index >= 2^set_depth`, `set_depth > 31`, or the set overflows the
/// tree. The verifier never needs this — the index stays private.
pub fn accepted_set_witness(
    entries: &[AcceptedStatusEntry],
    set_depth: u32,
    index: u64,
) -> Option<Vec<Fr>> {
    crate::issuer::sparse_witness_from_leaves(
        accepted_leaves(entries, set_depth)?,
        set_depth,
        index,
        accepted_padding_leaf(),
    )
}

/// [OPUS-5] sq-kndw: the PROVER's complete private witness for the fully-hidden
/// revocation member `revoke_hidden_ref_d{depth}_a{set_depth}` — the accepted-set
/// half (which slot the credential's `(list, version)` occupies, its path, and the
/// `status_list_root` the leaf binds) plus the status-list half (the leaf bit and
/// its path), assembled so the two trees are provably consistent BEFORE proving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenRefWitness {
    /// The depth-`depth` status-list Merkle root for the credential's `(list,
    /// version)`. PRIVATE in-circuit: it is bound by the accepted-set leaf, not
    /// disclosed, which is exactly what lets the IRI + version stay hidden.
    pub status_list_root: Fr,
    /// The credential's 0-based slot in the CANONICAL accepted-set leaf order.
    /// PRIVATE (the verifier never needs it).
    pub set_index: u64,
    /// The accepted-set authentication path, bottom-up, length `set_depth`.
    pub set_siblings: Vec<Fr>,
    /// The status-list half: the leaf bit at the holder's index and its
    /// authentication path (see [`MerkleWitness`]).
    pub merkle: MerkleWitness,
}

/// [OPUS-5] sq-kndw: build the [`HiddenRefWitness`] for `index` in `snapshot`,
/// against the relying party's accepted-set `entries` (which MUST be the canonical,
/// freshness-curated order the anchor was derived from —
/// `RevocationPolicy::accepted_entries`).
///
/// Fails closed (`None`) when:
/// - `snapshot`'s `(status_list, version)` is NOT a curated accepted-set member
///   (e.g. its version is outside the relying party's freshness window) — there is
///   then no membership path to prove and no proof should be attempted;
/// - the entry's recorded `status_list_root` disagrees with the root recomputed
///   from `snapshot` at `depth` — the prover and the relying party are looking at
///   different bitstrings, and proving would silently produce an unverifiable
///   proof;
/// - `index` is out of range for the depth-`depth` tree, or either depth overflows.
///
/// The consistency check is the load-bearing one: because the accepted-set leaf
/// binds `(list_id, version, status_list_root)` ATOMICALLY, a witness whose root
/// does not match the entry cannot fold to the anchor, and catching that here
/// turns a confusing `bb verify` failure into an explicit refusal.
pub fn hidden_ref_witness(
    entries: &[AcceptedStatusEntry],
    set_depth: u32,
    snapshot: &StatusListSnapshot,
    depth: u32,
    index: u64,
) -> Option<HiddenRefWitness> {
    let status_list_root = merkle_root(snapshot, depth)?;
    let set_index = entries
        .iter()
        .position(|e| e.status_list == snapshot.status_list && e.version == snapshot.version)?;
    if entries[set_index].status_list_root != status_list_root {
        return None;
    }
    let set_siblings = accepted_set_witness(entries, set_depth, set_index as u64)?;
    let merkle = merkle_witness(snapshot, depth, index)?;
    Some(HiddenRefWitness {
        status_list_root,
        set_index: set_index as u64,
        set_siblings,
        merkle,
    })
}

/// [OPUS-5] sq-kndw: render the `Prover.toml` body for the
/// `revoke_hidden_ref_d{depth}_a{set_depth}` member. Order MUST match
/// `revoke_hidden_ref_d{depth}_a{set_depth}/src/main.nr`:
///
/// PUBLIC:  `challenge`, `ref_commitment`, `index_commitment`,
///          `accepted_set_root`, `min_version`
/// PRIVATE: `list_id`, `version`, `ref_blinding`, `status_list_root`, `set_index`,
///          `set_siblings`, `index`, `bit`, `blinding`, `siblings`
///
/// `ref_commitment` MUST equal [`sparq_zk::sig::status_ref_commitment`]`(list_id,
/// version, ref_blinding)` and `index_commitment`
/// [`sparq_zk::sig::status_index_commitment`]`(index, blinding)` — the two
/// ISSUER-SIGNED values. The circuit recomputes both from the private witness, so a
/// mismatch here is unprovable rather than silently accepted.
///
/// `version` / `min_version` / `index` / `set_index` are integer-typed in the
/// circuit and are rendered as DECIMAL strings, the form `nargo` parses for
/// integer inputs; the field elements are rendered as `0x`-hex.
///
/// ⚠️ `ref_blinding` and `blinding` MUST be freshly sampled PER PRESENTATION (and
/// the issuer must re-sign the resulting commitments). Reusing them makes the two
/// public commitments a stable cross-presentation correlation handle and voids the
/// privacy guarantee — see [`crate::manifest::FullyHiddenRevocation`].
#[allow(clippy::too_many_arguments)]
pub fn revoke_hidden_ref_prover_toml(
    challenge: &Fr,
    ref_commitment: &Fr,
    index_commitment: &Fr,
    accepted_set_root: &Fr,
    min_version: u64,
    list_id: &Fr,
    version: u64,
    ref_blinding: &Fr,
    index: u64,
    blinding: &Fr,
    witness: &HiddenRefWitness,
) -> String {
    let list = |v: &[Fr]| {
        let parts: Vec<String> = v.iter().map(|s| format!("\"{}\"", field_to_hex(s))).collect();
        format!("[{}]", parts.join(", "))
    };
    let mut s = String::new();
    // -- public, in main() declaration order --
    s.push_str(&format!("challenge = \"{}\"\n", field_to_hex(challenge)));
    s.push_str(&format!("ref_commitment = \"{}\"\n", field_to_hex(ref_commitment)));
    s.push_str(&format!("index_commitment = \"{}\"\n", field_to_hex(index_commitment)));
    s.push_str(&format!("accepted_set_root = \"{}\"\n", field_to_hex(accepted_set_root)));
    s.push_str(&format!("min_version = \"{min_version}\"\n"));
    // -- private --
    s.push_str(&format!("list_id = \"{}\"\n", field_to_hex(list_id)));
    s.push_str(&format!("version = \"{version}\"\n"));
    s.push_str(&format!("ref_blinding = \"{}\"\n", field_to_hex(ref_blinding)));
    s.push_str(&format!(
        "status_list_root = \"{}\"\n",
        field_to_hex(&witness.status_list_root)
    ));
    s.push_str(&format!("set_index = \"{}\"\n", witness.set_index));
    s.push_str(&format!("set_siblings = {}\n", list(&witness.set_siblings)));
    s.push_str(&format!("index = \"{index}\"\n"));
    s.push_str(&format!("bit = \"{}\"\n", field_to_hex(&witness.merkle.bit)));
    s.push_str(&format!("blinding = \"{}\"\n", field_to_hex(blinding)));
    s.push_str(&format!("siblings = {}\n", list(&witness.merkle.siblings)));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(bits: Vec<u8>) -> StatusListSnapshot {
        StatusListSnapshot { status_list: "http://ex/s".to_string(), version: 1, bits }
    }

    // The Rust h2 mirrors the circuit's documented vector: h2(1,2) ==
    // 0x038682aa... (compose_core/tests.nr cross-vector).
    #[test]
    fn h2_matches_documented_cross_vector() {
        let got = field_to_hex(&h2(Fr::from(1u64), Fr::from(2u64)));
        assert_eq!(
            got,
            "0x038682aa1cb5ae4e0a3f13da432a95c77c5c111f6f030faf9cad641ce1ed7383"
        );
    }

    // [GPT-6] A depth-3 tree covers the entire byte. Its first four leaves
    // [0,1,0,0] retain the hand-built subtree used by the Noir accept tests.
    #[test]
    fn merkle_root_depth3_matches_hand_built() {
        let s = snap(vec![0b0000_0010]); // bit 1 set, rest unset
        let zero_pair = h2(Fr::from(0u64), Fr::from(0u64));
        let left = h2(h2(Fr::from(0u64), Fr::from(1u64)), zero_pair);
        assert_eq!(merkle_root(&s, 3), Some(h2(left, h2(zero_pair, zero_pair))));
    }

    // [OPUS-4.8] sq-hwe: the SPARSE builder must return a BYTE-IDENTICAL root to
    // the dense oracle across every snapshot shape — uniform-zero, sparse 1s, a
    // snapshot SHORTER than the tree (so the tail leaves read as fail-closed 1s),
    // and a boundary that straddles the covered/padding line mid-byte. This is the
    // load-bearing correctness invariant: the circuit + verifier binding are
    // unchanged precisely BECAUSE the root is identical to the old dense fold.
    #[test]
    fn sparse_root_equals_dense_root_for_every_shape() {
        let cases = vec![
            // (bits, depths to test)
            (vec![0u8], vec![3u32, 6]),                         // full byte + tail padding
            (vec![0b0000_0010u8], vec![3, 6]),                  // one set bit
            (vec![0xFFu8], vec![3, 6]),                          // a fully-revoked byte + padding
            (vec![0u8, 0, 0, 0, 0, 0, 0, 0], vec![6]),          // 64 covered, all zero
            (vec![0b1000_0001u8, 0, 0b0100_0000, 0], vec![5, 6]),// scattered set bits
            (vec![], vec![0, 2, 4]),                             // EMPTY: every leaf is fail-closed 1
            (vec![0u8; 3], vec![6]),                             // 24 covered (boundary mid-tree)
        ];
        for (bits, depths) in cases {
            let s = snap(bits.clone());
            for d in depths {
                assert_eq!(
                    merkle_root(&s, d),
                    dense_merkle_root(&s, d),
                    "sparse != dense for bits={bits:?} depth={d}"
                );
            }
        }
    }

    // [OPUS-4.8] sq-hwe: the sparse builder handles a PRODUCTION-scale depth
    // (2^17 leaves) without materialising them — the whole point of the bead. The
    // dense oracle would allocate 131072 field elements; the sparse builder folds
    // only the cached uniform-subtree defaults plus the single set bit. We assert
    // the witness for an unset index re-folds to this same large-depth root.
    #[test]
    fn sparse_root_and_witness_scale_to_production_depth() {
        // One revoked credential at index 5; the list nominally covers 2^17 slots
        // but the snapshot byte vector is tiny (the rest read as fail-closed 1s
        // past the covered region — exactly the StatusList2021 shape).
        let s = snap(vec![0b0010_0000u8]); // index 5 set; 0..4,6,7 active
        let depth = 17;
        let root = merkle_root(&s, depth).expect("sparse root at depth 17");
        // An ACTIVE index inside the covered region re-folds to the root via the
        // sparse witness.
        let w = merkle_witness(&s, depth, 0).expect("witness");
        assert_eq!(w.bit, Fr::from(0u64), "index 0 is active");
        assert_eq!(w.siblings.len(), depth as usize);
        let mut node = w.bit;
        let mut pos = 0u64;
        for sib in &w.siblings {
            let is_right = pos & 1 == 1;
            node = if is_right { h2(*sib, node) } else { h2(node, *sib) };
            pos >>= 1;
        }
        assert_eq!(node, root, "sparse witness must re-fold to the sparse root");
        // The revoked index 5 carries bit 1 (the circuit's bit-unset assert fails).
        assert_eq!(merkle_witness(&s, depth, 5).unwrap().bit, Fr::from(1u64));
    }

    // [OPUS-4.8] sq-hwe: at a production depth, the sparse witness for an index in
    // the FAIL-CLOSED PADDING region (past the covered bits) still re-folds to the
    // root and carries bit 1 — the padding leaves are genuinely SET, so a holder
    // cannot prove bit-unset for an uncovered slot. Cross-checks the all-ones
    // subtree default path.
    #[test]
    fn sparse_witness_in_padding_region_is_revoked_and_consistent() {
        let s = snap(vec![0u8]); // 8 covered active bits; everything >= 8 is padding
        let depth = 12;
        let root = merkle_root(&s, depth).unwrap();
        let padding_index = 100u64; // >= 8, in the fail-closed region, < 2^12
        let w = merkle_witness(&s, depth, padding_index).unwrap();
        assert_eq!(w.bit, Fr::from(1u64), "padding leaf reads as revoked");
        let mut node = w.bit;
        let mut pos = padding_index;
        for sib in &w.siblings {
            let is_right = pos & 1 == 1;
            node = if is_right { h2(*sib, node) } else { h2(node, *sib) };
            pos >>= 1;
        }
        assert_eq!(node, root, "padding-index witness must still re-fold to the root");
    }

    // [OPUS-4.8] sq-hwe: any_bit_set scans byte ranges and short-circuits; verify
    // it agrees with the per-index `bit()` oracle on awkward sub-byte ranges within
    // the covered region.
    #[test]
    fn any_bit_set_matches_per_index_oracle() {
        let s = snap(vec![0b1010_0101u8, 0b0000_0000, 0b1000_0000]);
        let covered = 24u64;
        for lo in 0..covered {
            for hi in lo..=covered {
                let expected = (lo..hi).any(|i| s.bit(i));
                assert_eq!(
                    any_bit_set(&s, lo, hi),
                    expected,
                    "any_bit_set disagreed on [{lo}, {hi})"
                );
            }
        }
    }

    // [OPUS-4.8] sq-6qe: the real caller never invokes any_bit_set past the covered
    // region, but the missing-byte fallback must be FAIL-CLOSED: a range touching a
    // byte beyond `bits` reads as revoked (`1`), matching `bit()`, so the helper
    // reports the region as set (`true`) and never lets the sparse builder
    // misclassify an unknown/padding region as an all-zero subtree.
    #[test]
    fn any_bit_set_is_fail_closed_on_missing_bytes() {
        let s = snap(vec![0b0000_0000u8]); // one covered byte, all active
        // Range entirely in the missing/padding region (bytes 1..) must read as set.
        assert!(any_bit_set(&s, 8, 24), "padding region must be fail-closed (set)");
        // Range straddling covered (active) + missing also reports set via the tail.
        assert!(any_bit_set(&s, 0, 16), "straddling range must be fail-closed (set)");
        // Sanity: a fully-covered all-active range is correctly NOT set.
        assert!(!any_bit_set(&s, 0, 8), "covered all-active range is not set");
        // And the missing-byte reads agree with the per-index `bit()` oracle.
        for i in 8u64..24 {
            assert!(s.bit(i), "out-of-range bit {i} must be revoked");
        }
    }

    // A witness for an unset index recomputes the root via the same fold the
    // circuit does (bit + siblings + index-derived directions).
    #[test]
    fn witness_recomputes_root() {
        let s = snap(vec![0b0000_0010]); // index 1 revoked; 0,2,3 active
        let depth = 3;
        let root = merkle_root(&s, depth).unwrap();
        for index in [0u64, 2, 3] {
            let w = merkle_witness(&s, depth, index).unwrap();
            assert_eq!(w.bit, Fr::from(0u64), "index {index} should be active");
            // Re-fold bottom-up using directions = LSB-first bits of index.
            let mut node = w.bit;
            let mut pos = index;
            for sib in &w.siblings {
                let is_right = pos & 1 == 1;
                node = if is_right { h2(*sib, node) } else { h2(node, *sib) };
                pos /= 2;
            }
            assert_eq!(node, root, "fold for index {index} must reach the root");
        }
        // The revoked index has bit 1 (the circuit's bit-unset assert would fail).
        assert_eq!(merkle_witness(&s, depth, 1).unwrap().bit, Fr::from(1u64));
    }

    #[test]
    fn out_of_range_index_is_none() {
        let s = snap(vec![0u8]);
        assert_eq!(merkle_witness(&s, 3, 8), None); // full snapshot, index beyond tree
    }

    // [GPT-6] The final representable bit binds the root; any appended byte is
    // rejected, even if the requested witness still lies in the old prefix.
    #[test]
    fn status_snapshot_capacity_binds_roots_witnesses_and_legacy_policy() {
        let mut snapshot = snap(vec![0; 128]);
        let root = merkle_root(&snapshot, 10).unwrap();
        let witness = merkle_witness(&snapshot, 10, 1023).unwrap();
        assert_eq!(witness.bit, Fr::from(0u64));
        snapshot.bits[127] = 0x80;
        assert_ne!(merkle_root(&snapshot, 10), Some(root));
        assert_eq!(merkle_witness(&snapshot, 10, 1023).unwrap().bit, Fr::from(1u64));
        let policy = crate::verifier::RevocationPolicy::accept_version(1)
            .with_hidden_index_depth(10)
            .with_snapshot(snapshot.clone());
        assert!(policy.accepted_entries().is_some());
        for suffix in [0, 0xFF] {
            let mut oversized = snapshot.clone();
            oversized.bits.push(suffix);
            assert_eq!(merkle_root(&oversized, 10), None);
            assert_eq!(merkle_witness(&oversized, 10, 0), None);
            assert!(policy.clone().with_snapshot(oversized).accepted_entries().is_none());
        }
        for depth in 0..3 {
            assert!(merkle_root(&snap(vec![0]), depth).is_none());
            assert!(merkle_witness(&snap(vec![0]), depth, 0).is_none());
            assert!(merkle_root(&snap(vec![]), depth).is_some());
        }
    }

    #[test]
    fn implausible_depth_is_none() {
        let s = snap(vec![0u8]);
        assert_eq!(merkle_root(&s, 32), None);
        assert_eq!(merkle_witness(&s, 32, 0), None);
    }

    // ---------------------------------------------------------------
    // [OPUS-5] sq-6qe: accepted-set commitment (host anchor).
    // ---------------------------------------------------------------

    fn entry(list: &str, version: u64, root: u64) -> AcceptedStatusEntry {
        AcceptedStatusEntry {
            status_list: list.to_string(),
            version,
            status_list_root: Fr::from(root),
        }
    }

    // Canonical three-entry set used by the fold/binding tests.
    fn entries() -> Vec<AcceptedStatusEntry> {
        vec![
            entry("http://ex/a", 7, 0x1111),
            entry("http://ex/a", 8, 0x2222),
            entry("http://ex/b", 7, 0x3333),
        ]
    }

    // [OPUS-5] sq-6qe: the host leaf IS `sig::accepted_status_leaf` over the
    // field-mapped IRI — one source of truth with the future in-circuit leaf. If
    // this drifts, a host-built anchor and a circuit-built path would disagree and
    // every proof would (silently) fail to verify.
    #[test]
    fn accepted_set_leaf_is_the_sig_primitive() {
        let e = entry("http://ex/status", 42, 0xabcd);
        assert_eq!(
            accepted_set_leaf(&e),
            sig::accepted_status_leaf(
                &sig::status_list_id_to_field("http://ex/status"),
                42,
                &Fr::from(0xabcdu64)
            )
        );
    }

    // [OPUS-5] sq-6qe: the LOAD-BEARING invariant — the prover's membership path
    // for every occupied slot re-folds (under the circuit's bottom-up fold, with
    // directions from the index bits) to the published accepted-set root. This is
    // what makes the anchor usable as a public input at all.
    #[test]
    fn accepted_set_witness_refolds_to_root() {
        let es = entries();
        for set_depth in [2u32, 3, 6] {
            let root = accepted_set_root(&es, set_depth).expect("root");
            for (i, e) in es.iter().enumerate() {
                let sibs = accepted_set_witness(&es, set_depth, i as u64).expect("witness");
                assert_eq!(sibs.len(), set_depth as usize);
                let mut node = accepted_set_leaf(e);
                let mut pos = i as u64;
                for sib in &sibs {
                    node = if pos & 1 == 1 { h2(*sib, node) } else { h2(node, *sib) };
                    pos >>= 1;
                }
                assert_eq!(node, root, "fold for entry {i} at depth {set_depth}");
            }
        }
    }

    // [OPUS-5] sq-6qe: the atomic triple binding, observed at the SET level. A
    // prover that pairs list a's identity with list b's root — the exact
    // substitution the single-leaf construction exists to prevent — changes the
    // root, so it cannot reproduce the relying party's public anchor. Also pins
    // that each of the three components independently binds.
    #[test]
    fn accepted_set_root_binds_list_version_and_root_atomically() {
        let depth = 3;
        let base = accepted_set_root(&entries(), depth).expect("root");

        // Swap the two lists' roots (identity a with root of b, and vice versa).
        let mut swapped = entries();
        let a_root = swapped[0].status_list_root;
        swapped[0].status_list_root = swapped[2].status_list_root;
        swapped[2].status_list_root = a_root;
        assert_ne!(
            accepted_set_root(&swapped, depth),
            Some(base),
            "cross-paired (list, root) must not reproduce the anchor"
        );

        // Each component alone binds.
        let mut other_list = entries();
        other_list[0].status_list = "http://ex/c".to_string();
        assert_ne!(accepted_set_root(&other_list, depth), Some(base), "IRI binds");

        let mut other_version = entries();
        other_version[0].version = 9;
        assert_ne!(accepted_set_root(&other_version, depth), Some(base), "version binds");

        let mut other_root = entries();
        other_root[0].status_list_root = Fr::from(0x9999u64);
        assert_ne!(accepted_set_root(&other_root, depth), Some(base), "root binds");
    }

    // [OPUS-5] sq-6qe: the canonical LEAF ORDER is load-bearing — the relying
    // party and the prover must commit the same order or the roots disagree.
    // Reordering the same entries yields a different anchor, which is why
    // `RevocationPolicy::accepted_entries` emits the sorted `BTreeMap` order.
    #[test]
    fn accepted_set_root_depends_on_leaf_order() {
        let depth = 3;
        let base = accepted_set_root(&entries(), depth).expect("root");
        let mut reordered = entries();
        reordered.swap(0, 2);
        assert_ne!(accepted_set_root(&reordered, depth), Some(base));
    }

    // [OPUS-5] sq-6qe: fail-closed sizing. An accepted set that does not fit the
    // depth yields `None` rather than a silently truncated trust anchor, and an
    // implausible depth is refused. An out-of-range witness index is `None` too.
    #[test]
    fn accepted_set_is_fail_closed_on_overflow_and_bad_depth() {
        let es = entries(); // 3 entries
        assert_eq!(accepted_set_root(&es, 1), None, "3 entries do not fit 2^1");
        assert!(accepted_set_root(&es, 2).is_some(), "3 entries fit 2^2");
        assert_eq!(accepted_set_root(&es, 32), None, "implausible depth refused");
        assert_eq!(accepted_set_witness(&es, 32, 0), None);
        assert_eq!(accepted_set_witness(&es, 2, 4), None, "index 4 is out of a 2^2 tree");
    }

    // [OPUS-5] sq-6qe: an EMPTY accepted set still commits (to the all-padding
    // root) — it is a well-formed anchor that no membership proof can satisfy, so
    // a relying party with no curated snapshots is fail-closed rather than
    // undefined.
    #[test]
    fn empty_accepted_set_commits_to_the_padding_root() {
        let depth = 3;
        let empty = accepted_set_root(&[], depth).expect("empty set still commits");
        let mut node = accepted_padding_leaf();
        for _ in 0..depth {
            node = h2(node, node);
        }
        assert_eq!(empty, node);
        assert_ne!(accepted_set_root(&entries(), depth), Some(empty));
    }
}
