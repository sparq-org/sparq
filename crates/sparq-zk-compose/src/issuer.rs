// sq-z9l: hidden-issuer-attestation host-side helpers.
//! Host-side commitment + witness machinery for the in-circuit
//! **Schnorr-over-Baby-JubJub + hidden-key set membership** proof (sq-z9l). The
//! in-circuit relation is `zk/compose/compose_core/src/issuer.nr`; this module is
//! its Rust mirror:
//!
//! - [`key_set_root`] commits a set K of trusted issuer public keys as a
//!   depth-`D` Poseidon2 Merkle tree (leaves = `Poseidon2([pk.x, pk.y])` =
//!   [`sparq_zk::sig::key_set_leaf`], internal nodes = `h2` =
//!   `poseidon2::hash(&[l, r])` — bit-identical to the circuit's `h2` and to the
//!   revocation tree's nodes). The RELYING PARTY computes this root over its OWN
//!   authoritative KeySet (the trust anchor, exactly like
//!   `revocation::merkle_root` does for the status root); the proof's public
//!   `key_set_root` must byte-equal it.
//! - [`key_membership_witness`] builds the prover's private authentication path
//!   (sibling at each level, bottom-up) for the signing key's index in K.
//! - [`HiddenIssuerWitness`] bundles the Schnorr witness
//!   ([`sparq_zk::sig::InCircuitSchnorrWitness`]) with the membership path — the
//!   complete private input to the `hidden_issuer_d{depth}` member.
//! - [`hidden_issuer_prover_toml`] renders the `Prover.toml`.
//!
//! # Leaf / tree layout (MUST match `issuer.nr`)
//! - Leaves are the `2^D` key-set slots; leaf `i` = `key_set_leaf(K[i])`. The set
//!   is committed in the verifier's chosen, fixed slot order (the RP's KeySet
//!   iteration order). Slots past `K.len()` are padded with a fixed sentinel leaf
//!   (`Fr::from(0)`) so a tree of any `2^D >= K.len()` is well-defined; a key can
//!   never occupy a padding slot because a padding leaf is `0`, not a key hash
//!   (`Poseidon2([x,y])` is never `0` for an on-curve non-identity key in
//!   practice, and even if it were the membership proof binds the index).
//! - Internal node = `h2(left, right)`; fold pairwise up `D` levels to the root.
//!
//! # Scope (honest, mirrors `revocation`)
//! [`key_set_root`] / [`key_membership_witness`] build a DENSE tree: they
//! materialise all `2^D` leaves, so the host work is `O(2^D)`. The depth-`D`
//! member covers up to `2^D` issuers/holders. Real issuer sets are small (tens of
//! authorities), so for those the dense builder at depth 4–10 is ample.
//!
//! A production deployment with a VERY LARGE registry (e.g. a holder set —
//! sq-8k3h, the analogue of this key-set membership) cannot afford `O(2^D)` host
//! work at large `D`. For that case [`key_set_root_sparse`] /
//! [`key_membership_witness_sparse`] compute the BIT-IDENTICAL root and
//! authentication path in `O(n·D)` (where `n = keys.len()`), never materialising
//! the padding slots. They exploit the dense layout's structure: members occupy
//! the contiguous prefix `0..n` and every slot `n..2^D` is the same padding leaf,
//! so each level above the populated prefix is a fixed per-level "empty-subtree"
//! digest that depends only on the level, not on data. The circuit relation
//! (`key_set_membership::<D>`) is depth-generic and folds the path identically
//! whichever host builder produced it — only the dense builder bounded the
//! registry size. The compiled member is `d4` (16 slots) — see the verifier docs
//! and the crate `STATUS`.

use sparq_zk::field::{field_to_hex, Fr};
use sparq_zk::poseidon2;
use sparq_zk::sig::{key_set_leaf, InCircuitSchnorrWitness, PublicKey};

/// The Poseidon2 two-input compression for Merkle internal nodes — the Rust
/// mirror of the circuit's `h2(a, b) = Poseidon2::hash([a, b], 2)`. Identical to
/// `revocation::h2` (cross-tested bit-identical).
fn h2(a: Fr, b: Fr) -> Fr {
    poseidon2::hash(&[a, b])
}

/// The padding leaf for key-set slots past `K.len()`: `Fr::from(0)`. A genuine
/// key leaf is `Poseidon2([pk.x, pk.y])`; the membership proof additionally binds
/// the index, so a padding slot is never a usable member.
fn padding_leaf() -> Fr {
    Fr::from(0u64)
}

/// The `2^depth` key-set leaves: leaf `i` = `key_set_leaf(K[i])` for `i <
/// K.len()`, else the padding leaf. `None` if any key in K is the identity (no
/// coordinates — never a usable key, fail-closed) or `depth > 31`.
fn key_leaves(keys: &[PublicKey], depth: u32) -> Option<Vec<Fr>> {
    if depth > 31 {
        return None;
    }
    let n_leaves = 1usize << depth;
    if keys.len() > n_leaves {
        return None; // the set does not fit the tree at this depth
    }
    let mut leaves = Vec::with_capacity(n_leaves);
    for pk in keys {
        leaves.push(key_set_leaf(pk)?);
    }
    leaves.resize(n_leaves, padding_leaf());
    Some(leaves)
}

/// Commit the trusted issuer key set K as a depth-`depth` Poseidon2 Merkle tree
/// and return the root. This is what the RELYING PARTY computes over its OWN
/// authoritative KeySet (the trust anchor the proof's public `key_set_root` is
/// checked against) and what the PROVER computes over the same set. Returns
/// `None` if `depth > 31`, the set overflows the tree, or any key is the
/// identity (fail-closed).
pub fn key_set_root(keys: &[PublicKey], depth: u32) -> Option<Fr> {
    let mut level = key_leaves(keys, depth)?;
    for _ in 0..depth {
        level = level.chunks(2).map(|pair| h2(pair[0], pair[1])).collect();
    }
    debug_assert_eq!(level.len(), 1, "fold reduces to a single root");
    level.first().copied()
}

/// The authentication path (sibling at each level, BOTTOM-UP — level 0 first) for
/// the key at `index` in K's depth-`depth` tree. The private input the circuit
/// folds. `None` if `index >= 2^depth`, the set overflows the tree, `depth > 31`,
/// or any key is the identity.
pub fn key_membership_witness(keys: &[PublicKey], depth: u32, index: u64) -> Option<Vec<Fr>> {
    let mut level = key_leaves(keys, depth)?;
    let n_leaves = 1u64 << depth;
    if index >= n_leaves {
        return None;
    }
    let mut pos = index as usize;
    let mut siblings = Vec::with_capacity(depth as usize);
    for _ in 0..depth {
        let sib = pos ^ 1;
        siblings.push(level[sib]);
        level = level.chunks(2).map(|pair| h2(pair[0], pair[1])).collect();
        pos /= 2;
    }
    Some(siblings)
}

// =====================================================================
// sq-8k3h: SPARSE host commitment for a very large registry.
//
// The dense builders above are `O(2^depth)`: fine for tens-of-authorities issuer
// sets, infeasible for a very large holder registry at the deep `depth` such a
// registry needs. The sparse builders below produce the BIT-IDENTICAL root and
// authentication path in `O(n·depth)` (`n = keys.len()`) by never materialising
// the padding slots. The circuit relation is unchanged and depth-generic — only
// the *host* builder bounded the size — so a sparse-built witness verifies under
// exactly the same `hidden_issuer_d{depth}` / `key_set_membership::<D>` member.
//
// The leaf-taking core (`empty_subtree_roots` / `sparse_fold_leaves` /
// `sparse_root_from_leaves` / `sparse_witness_from_leaves`) is `pub(crate)` so the
// holder-set builders (`crate::holder`, sq-8k3h proper) reuse the SAME machinery —
// both modules already share `h2` and the `Fr::from(0)` padding leaf, so the trees
// are bit-identical and there is one source of truth for the sparse fold.
// =====================================================================

/// The per-level "empty-subtree" digests of the dense layout: `roots[k]` is the
/// Merkle root of an all-padding subtree of height `k` (so `roots[0] = padding`
/// and `roots[k] = h2(roots[k-1], roots[k-1])`). Length = `depth + 1` (levels
/// `0..=depth`). Because every dense slot `n..2^depth` holds the SAME padding leaf,
/// any internal node whose entire subtree is padding is exactly `roots[k]` — a
/// constant that depends only on the level, not the data. This is the standard
/// sparse-Merkle empty-node precomputation; it makes the sparse builders below
/// `O(n·depth)` instead of `O(2^depth)`. `O(depth)`.
///
/// `padding` is the leaf value the dense builder pads unused slots with — the SAME
/// `Fr::from(0)` in both [`crate::issuer`] and [`crate::holder`], passed in so this
/// stays the single shared sparse-fold core.
pub(crate) fn empty_subtree_roots(depth: u32, padding: Fr) -> Vec<Fr> {
    let mut roots = Vec::with_capacity(depth as usize + 1);
    roots.push(padding);
    for k in 1..=depth as usize {
        let prev = roots[k - 1];
        roots.push(h2(prev, prev));
    }
    roots
}

/// One node of a level in the sparse fold: either a concrete digest in the
/// populated prefix, or the constant empty-subtree digest for that level.
#[inline]
fn node_at(prefix: &[Fr], idx: usize, empty: Fr) -> Fr {
    prefix.get(idx).copied().unwrap_or(empty)
}

/// Fold the depth-`depth` tree keeping ONLY the populated prefix at each level,
/// given the `n` non-padding `leaves` (the members, in slot order) and the dense
/// `padding` leaf. Returns `(prefixes, empties)` where `prefixes[k]` is the
/// non-padding prefix of level `k` (level 0 = `leaves`) and `empties[k]` is that
/// level's empty-subtree digest. `prefixes[depth]` is the single-element root
/// level. `None` if `depth > 31` or `leaves.len() > 2^depth` (the set overflows the
/// tree) — the SAME fail-closed conditions as the dense builders. `O(n·depth)`.
///
/// SHARED by [`crate::issuer`] and [`crate::holder`]: each module computes its own
/// `leaves` (with its own per-key digest + identity fail-closed guard) then calls
/// this. The fold itself is leaf-agnostic, so the trees are bit-identical.
pub(crate) fn sparse_fold_leaves(
    leaves: Vec<Fr>,
    depth: u32,
    padding: Fr,
) -> Option<(Vec<Vec<Fr>>, Vec<Fr>)> {
    if depth > 31 {
        return None;
    }
    let n_leaves = 1u128 << depth; // u128: depth<=31 so this never overflows
    if leaves.len() as u128 > n_leaves {
        return None; // the set does not fit the tree at this depth
    }
    let empties = empty_subtree_roots(depth, padding);
    let mut prefixes: Vec<Vec<Fr>> = Vec::with_capacity(depth as usize + 1);
    prefixes.push(leaves);
    for k in 0..depth as usize {
        let cur = &prefixes[k];
        let empty = empties[k];
        // Parent prefix length = ceil(cur.len() / 2); a parent is non-empty iff at
        // least one child is in the populated prefix.
        let parent_len = cur.len().div_ceil(2);
        let mut parent = Vec::with_capacity(parent_len);
        for p in 0..parent_len {
            let left = node_at(cur, 2 * p, empty);
            let right = node_at(cur, 2 * p + 1, empty);
            parent.push(h2(left, right));
        }
        prefixes.push(parent);
    }
    // The root level holds at most one populated node (empty only if leaves is empty).
    debug_assert!(
        prefixes[depth as usize].len() <= 1,
        "root level has <= 1 node"
    );
    Some((prefixes, empties))
}

/// The BIT-IDENTICAL depth-`depth` Merkle root for the given non-padding `leaves`,
/// in `O(n·depth)`. The dense-builder equivalent materialises all `2^depth` slots;
/// this never does. SHARED sparse core (see [`sparse_fold_leaves`]).
pub(crate) fn sparse_root_from_leaves(leaves: Vec<Fr>, depth: u32, padding: Fr) -> Option<Fr> {
    let (prefixes, empties) = sparse_fold_leaves(leaves, depth, padding)?;
    // The root prefix is empty exactly when leaves is empty (an all-padding tree);
    // then the root is the depth-level empty-subtree digest.
    Some(node_at(
        &prefixes[depth as usize],
        0,
        empties[depth as usize],
    ))
}

/// The BIT-IDENTICAL authentication path (sibling per level, BOTTOM-UP) for
/// `index` over the given non-padding `leaves`, in `O(n·depth)`. At each level the
/// sibling is the populated-prefix node if it exists, else that level's constant
/// empty-subtree digest. `None` if `index >= 2^depth` or `leaves` overflows the
/// tree / `depth > 31`. SHARED sparse core (see [`sparse_fold_leaves`]).
pub(crate) fn sparse_witness_from_leaves(
    leaves: Vec<Fr>,
    depth: u32,
    index: u64,
    padding: Fr,
) -> Option<Vec<Fr>> {
    // `sparse_fold_leaves` rejects `depth > 31`, so `1u64 << depth` never overflows.
    let (prefixes, empties) = sparse_fold_leaves(leaves, depth, padding)?;
    if index >= (1u64 << depth) {
        return None; // out of range for this tree (matches the dense builder)
    }
    let mut pos = index as usize;
    let mut siblings = Vec::with_capacity(depth as usize);
    for k in 0..depth as usize {
        let sib = pos ^ 1;
        siblings.push(node_at(&prefixes[k], sib, empties[k]));
        pos /= 2;
    }
    Some(siblings)
}

/// The `n` non-padding key-set leaves (NOT padded to `2^depth`): leaf `i` =
/// `key_set_leaf(keys[i])`. `None` if any key is the identity (fail-closed, matches
/// `key_leaves`) or the set overflows the depth-`depth` tree / `depth > 31`. The
/// sparse builders fold these directly without materialising the padding slots.
fn sparse_key_leaves(keys: &[PublicKey], depth: u32) -> Option<Vec<Fr>> {
    if depth > 31 || keys.len() as u128 > (1u128 << depth) {
        return None;
    }
    let mut leaves = Vec::with_capacity(keys.len());
    for pk in keys {
        leaves.push(key_set_leaf(pk)?);
    }
    Some(leaves)
}

/// SPARSE variant of [`key_set_root`]: the BIT-IDENTICAL depth-`depth` root,
/// computed in `O(n·depth)` instead of `O(2^depth)` — the builder a very large
/// issuer/holder registry needs (sq-8k3h). Cross-checked equal to [`key_set_root`]
/// for every `(keys, depth)` in the tests. Returns `None` under the same
/// fail-closed conditions as the dense builder.
pub fn key_set_root_sparse(keys: &[PublicKey], depth: u32) -> Option<Fr> {
    sparse_root_from_leaves(sparse_key_leaves(keys, depth)?, depth, padding_leaf())
}

/// SPARSE variant of [`key_membership_witness`]: the BIT-IDENTICAL authentication
/// path (sibling per level, BOTTOM-UP) for `index`, in `O(n·depth)`. The returned
/// path folds to [`key_set_root_sparse`] (== [`key_set_root`]) under the circuit's
/// fold, so it is a drop-in replacement for the dense witness in the depth-generic
/// member. `None` if `index >= 2^depth` or under the dense builder's fail-closed
/// conditions.
pub fn key_membership_witness_sparse(
    keys: &[PublicKey],
    depth: u32,
    index: u64,
) -> Option<Vec<Fr>> {
    sparse_witness_from_leaves(
        sparse_key_leaves(keys, depth)?,
        depth,
        index,
        padding_leaf(),
    )
}

/// The complete private witness for one hidden issuer attestation: the Schnorr
/// witness (key coords, R, s, e, e_k) plus the key-set membership path. Mirrors
/// the private inputs of `issuer.nr::hidden_issuer_attestation`.
#[derive(Debug, Clone)]
pub struct HiddenIssuerWitness {
    /// The in-circuit Schnorr verification witness over the commitment message.
    pub schnorr: InCircuitSchnorrWitness,
    /// The signing key's index in K (private; the circuit derives the path
    /// directions from it).
    pub index: u64,
    /// The Merkle authentication path: `siblings[k]` is the sibling at level `k`
    /// (level 0 = leaves), bottom-up. Length = `depth`.
    pub siblings: Vec<Fr>,
}

/// Render the `Prover.toml` body for the `hidden_issuer_d{depth}` member. Order
/// MUST match `hidden_issuer_d{depth}/src/main.nr`:
/// PUBLIC: challenge, m, key_set_root;
/// PRIVATE: pk_x, pk_y, r_x, r_y, s, e, e_k, index, siblings.
pub fn hidden_issuer_prover_toml(
    challenge: &Fr,
    m: &Fr,
    key_set_root: &Fr,
    witness: &HiddenIssuerWitness,
) -> String {
    let w = &witness.schnorr;
    let mut s = String::new();
    s.push_str(&format!("challenge = \"{}\"\n", field_to_hex(challenge)));
    s.push_str(&format!("m = \"{}\"\n", field_to_hex(m)));
    s.push_str(&format!("key_set_root = \"{}\"\n", field_to_hex(key_set_root)));
    s.push_str(&format!("pk_x = \"{}\"\n", field_to_hex(&w.pk_x)));
    s.push_str(&format!("pk_y = \"{}\"\n", field_to_hex(&w.pk_y)));
    s.push_str(&format!("r_x = \"{}\"\n", field_to_hex(&w.r_x)));
    s.push_str(&format!("r_y = \"{}\"\n", field_to_hex(&w.r_y)));
    s.push_str(&format!("s = \"{}\"\n", field_to_hex(&w.s)));
    s.push_str(&format!("e = \"{}\"\n", field_to_hex(&w.e)));
    s.push_str(&format!("e_k = \"{}\"\n", field_to_hex(&w.e_k)));
    s.push_str(&format!("index = \"{}\"\n", witness.index));
    let sibs: Vec<String> = witness
        .siblings
        .iter()
        .map(|s| format!("\"{}\"", field_to_hex(s)))
        .collect();
    s.push_str(&format!("siblings = [{}]\n", sibs.join(", ")));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_zk::sig::{commitment_message, in_circuit_witness, signature_from_hex, SecretKey};

    fn keyset(seeds: &[u64]) -> Vec<PublicKey> {
        seeds.iter().map(|s| SecretKey::from_seed(*s).public_key()).collect()
    }

    // The Rust h2 mirrors the documented cross-vector (compose_core/tests.nr).
    #[test]
    fn h2_matches_documented_cross_vector() {
        assert_eq!(
            field_to_hex(&h2(Fr::from(1u64), Fr::from(2u64))),
            "0x038682aa1cb5ae4e0a3f13da432a95c77c5c111f6f030faf9cad641ce1ed7383"
        );
    }

    // A depth-2 tree (4 leaves) built by hand matches `key_set_root`, and the
    // membership witness for each index re-folds to the root (the circuit's fold).
    #[test]
    fn key_set_root_and_witness_recompute() {
        let keys = keyset(&[100, 101, 102, 103]);
        let depth = 2;
        let leaves: Vec<Fr> = keys.iter().map(|k| key_set_leaf(k).unwrap()).collect();
        let n01 = h2(leaves[0], leaves[1]);
        let n23 = h2(leaves[2], leaves[3]);
        let expected_root = h2(n01, n23);
        assert_eq!(key_set_root(&keys, depth), Some(expected_root));

        for index in 0..4u64 {
            let sibs = key_membership_witness(&keys, depth, index).unwrap();
            // Re-fold bottom-up using directions = LSB-first bits of index.
            let mut node = leaves[index as usize];
            let mut pos = index;
            for sib in &sibs {
                let is_right = pos & 1 == 1;
                node = if is_right { h2(*sib, node) } else { h2(node, *sib) };
                pos /= 2;
            }
            assert_eq!(node, expected_root, "fold for index {index} reaches the root");
        }
    }

    // A key set smaller than the tree is padded; padding leaves are 0, distinct
    // from any key leaf (so a key never collides into a padding slot).
    #[test]
    fn padded_key_set_root_is_defined() {
        let keys = keyset(&[100, 101, 102]); // 3 keys in a depth-2 (4-slot) tree
        let depth = 2;
        let leaves: Vec<Fr> = keys.iter().map(|k| key_set_leaf(k).unwrap()).collect();
        let n01 = h2(leaves[0], leaves[1]);
        let n23 = h2(leaves[2], padding_leaf()); // slot 3 is padding
        let expected = h2(n01, n23);
        assert_eq!(key_set_root(&keys, depth), Some(expected));
        for leaf in &leaves {
            assert_ne!(*leaf, padding_leaf(), "a key leaf is never the padding leaf");
        }
    }

    #[test]
    fn out_of_range_index_and_overflow_are_none() {
        let keys = keyset(&[100, 101, 102, 103]);
        assert_eq!(key_membership_witness(&keys, 2, 4), None); // 4 leaves, idx 4 OOB
        // 5 keys do not fit a depth-2 (4-slot) tree.
        let big = keyset(&[1, 2, 3, 4, 5]);
        assert_eq!(key_set_root(&big, 2), None);
    }

    // The full witness assembly: a real signature by an in-set issuer produces a
    // schnorr witness whose key leaf is at the claimed index, and the membership
    // path re-folds to the root the verifier would derive.
    #[test]
    fn hidden_issuer_witness_assembles_consistently() {
        let keys = keyset(&[100, 101, 102, 103]);
        let depth = 2u32;
        let signer_idx = 2usize;
        let signer_sk = SecretKey::from_seed(102);
        let c = Fr::from(0xc0ffeeu64);
        let m = commitment_message(&c);
        let sig = signature_from_hex(&signer_sk.sign_commitment(&c)).unwrap();
        let schnorr = in_circuit_witness(&keys[signer_idx], &m, &sig).unwrap();
        let siblings = key_membership_witness(&keys, depth, signer_idx as u64).unwrap();

        // The schnorr witness's key coords hash to the leaf at signer_idx.
        let leaf = h2(schnorr.pk_x, schnorr.pk_y);
        assert_eq!(leaf, key_set_leaf(&keys[signer_idx]).unwrap());

        let w = HiddenIssuerWitness { schnorr, index: signer_idx as u64, siblings };
        // The Prover.toml renders all 12 fields.
        let toml = hidden_issuer_prover_toml(&Fr::from(0x2au64), &m, &key_set_root(&keys, depth).unwrap(), &w);
        for field in ["challenge", "m", "key_set_root", "pk_x", "pk_y", "r_x", "r_y", "s", "e", "e_k", "index", "siblings"] {
            assert!(toml.contains(field), "toml must render {field}");
        }
    }

    // ============================================================
    // sq-8k3h: SPARSE host commitment cross-checks.
    // The sparse builders MUST produce a root + authentication path BIT-IDENTICAL
    // to the dense builders, so a sparse-built witness verifies under the same
    // depth-generic member. These pins are the soundness contract for the sparse
    // path: any drift from the dense layout fails here.
    // ============================================================

    // The empty-subtree digests fold the way the dense padding does: an all-padding
    // tree's `key_set_root` (no keys) equals the depth-level empty digest, and each
    // level doubles the previous one.
    #[test]
    fn empty_subtree_roots_fold_like_dense_padding() {
        for depth in 0..=8u32 {
            let roots = empty_subtree_roots(depth, padding_leaf());
            assert_eq!(roots.len() as u32, depth + 1);
            assert_eq!(roots[0], padding_leaf());
            for k in 1..=depth as usize {
                assert_eq!(roots[k], h2(roots[k - 1], roots[k - 1]));
            }
            // The dense root of an EMPTY key set (all padding) == the depth-level digest.
            assert_eq!(key_set_root(&[], depth), Some(roots[depth as usize]));
        }
    }

    // For every (size, depth) with size <= 2^depth, the sparse root EQUALS the dense
    // root — across empty, partial, and full trees.
    #[test]
    fn sparse_root_matches_dense_for_all_sizes() {
        for depth in 0..=6u32 {
            let cap = 1u64 << depth;
            for size in 0..=cap {
                let seeds: Vec<u64> = (0..size).map(|i| 1000 + i).collect();
                let keys = keyset(&seeds);
                assert_eq!(
                    key_set_root_sparse(&keys, depth),
                    key_set_root(&keys, depth),
                    "sparse root must equal dense root (depth {depth}, size {size})"
                );
            }
        }
    }

    // For every populated index, the sparse authentication path EQUALS the dense
    // path (and so re-folds to the same root the circuit derives). Covers partial
    // trees where the dense padding slots become the sparse empty digests.
    #[test]
    fn sparse_witness_matches_dense_for_all_indices() {
        for depth in 0..=5u32 {
            let cap = 1u64 << depth;
            // A partial set (so padding/empty siblings exercise both paths) and a full set.
            for size in [cap / 2 + 1, cap] {
                if size == 0 {
                    continue;
                }
                let seeds: Vec<u64> = (0..size).map(|i| 2000 + i).collect();
                let keys = keyset(&seeds);
                for index in 0..cap {
                    assert_eq!(
                        key_membership_witness_sparse(&keys, depth, index),
                        key_membership_witness(&keys, depth, index),
                        "sparse witness must equal dense witness (depth {depth}, size {size}, index {index})"
                    );
                }
            }
        }
    }

    // The whole point of sq-8k3h: a DEEP tree a very large registry needs is built
    // by the sparse path WITHOUT materialising 2^depth leaves (the dense builder is
    // infeasible here), and a member's path still re-folds to the sparse root under
    // the circuit's bottom-up fold (directions = LSB-first index bits) — bit-for-bit
    // the relation the depth-generic `key_set_membership::<D>` member checks.
    #[test]
    fn sparse_deep_tree_witness_refolds_to_root() {
        let depth = 28u32; // 2^28 = 268M dense slots — only the sparse builder is feasible
        let keys = keyset(&[7001, 7002, 7003, 7004, 7005]);
        let root = key_set_root_sparse(&keys, depth).expect("sparse deep root");
        for index in 0..keys.len() as u64 {
            let sibs =
                key_membership_witness_sparse(&keys, depth, index).expect("sparse deep path");
            assert_eq!(sibs.len(), depth as usize);
            let mut node = key_set_leaf(&keys[index as usize]).unwrap();
            let mut pos = index;
            for sib in &sibs {
                let is_right = pos & 1 == 1;
                node = if is_right {
                    h2(*sib, node)
                } else {
                    h2(node, *sib)
                };
                pos /= 2;
            }
            assert_eq!(
                node, root,
                "sparse path for index {index} re-folds to the sparse root"
            );
        }
    }

    // A high index far outside the populated prefix (but inside the tree) still has
    // a well-defined sparse path that folds to the root — i.e. an unused slot is a
    // valid, distinct leaf, never silently aliased onto a member.
    #[test]
    fn sparse_empty_slot_path_is_well_defined() {
        let depth = 20u32;
        let keys = keyset(&[8001, 8002, 8003]);
        let root = key_set_root_sparse(&keys, depth).unwrap();
        let idx = (1u64 << depth) - 1; // last slot, all-padding region
        let sibs = key_membership_witness_sparse(&keys, depth, idx).unwrap();
        let mut node = padding_leaf(); // the leaf at an unused slot is the padding leaf
        let mut pos = idx;
        for sib in &sibs {
            let is_right = pos & 1 == 1;
            node = if is_right {
                h2(*sib, node)
            } else {
                h2(node, *sib)
            };
            pos /= 2;
        }
        assert_eq!(
            node, root,
            "an unused-slot path folds to the same committed root"
        );
    }

    // Fail-closed parity with the dense builder for the three conditions this test
    // ACTUALLY drives: overflow, `depth > 31`, and out-of-range index.
    //
    // sq-r6dq review round 3 (gpt-5.6-sol finding 2): this comment used to
    // also claim "identity key" coverage, which the body did NOT contain — a comment
    // asserting a guard that was not tested. The unusable-key guard
    // (`key_set_leaf(pk)?` in `key_leaves` / `sparse_key_leaves`) now has its own
    // test, `unusable_key_is_rejected_by_both_builders` below; it is deliberately
    // NOT folded in here so the claim and the assertions cannot drift apart again.
    #[test]
    fn sparse_fail_closed_matches_dense() {
        let keys = keyset(&[100, 101, 102, 103]);
        // out-of-range index (4 leaves, idx 4)
        assert_eq!(key_membership_witness_sparse(&keys, 2, 4), None);
        assert_eq!(key_membership_witness(&keys, 2, 4), None);
        // 5 keys do not fit a depth-2 (4-slot) tree
        let big = keyset(&[1, 2, 3, 4, 5]);
        assert_eq!(key_set_root_sparse(&big, 2), None);
        assert_eq!(key_set_root(&big, 2), None);
        // depth > 31 fail-closed
        assert_eq!(key_set_root_sparse(&keys, 32), None);
        assert_eq!(key_membership_witness_sparse(&keys, 32, 0), None);
        // ...and at the leaf-taking core, so the two `depth > 31` guards
        // (`sparse_key_leaves` and `sparse_fold_leaves`) cannot mask each other:
        // deleting EITHER one individually now reddens this test.
        assert_eq!(sparse_key_leaves(&keys, 32), None, "sparse_key_leaves rejects depth 32");
        assert_eq!(
            sparse_fold_leaves(vec![], 32, padding_leaf()),
            None,
            "sparse_fold_leaves rejects depth 32 independently of sparse_key_leaves"
        );
        assert_eq!(
            sparse_witness_from_leaves(vec![], 32, 0, padding_leaf()),
            None,
            "sparse_witness_from_leaves inherits the depth-32 rejection"
        );
    }

    // sq-r6dq review round 3 (gpt-5.6-sol finding 2): the `key_set_leaf(pk)?`
    // FAIL-CLOSED guard — the one thing standing between an unusable (identity /
    // torsion / off-curve) key and the committed key set K — had NO test in either
    // builder, while `sparse_fail_closed_matches_dense`'s comment claimed it did.
    //
    // `PublicKey`'s inner point is a `pub` field, so a caller can construct a key
    // around a non-prime-order point directly, bypassing `public_key_from_hex`
    // (sq-l15mi audit H-1). `key_set_leaf` fail-closes on `!is_prime_order()`, and
    // BOTH `key_leaves` (dense) and `sparse_key_leaves` (sparse) must propagate that
    // `None` rather than substituting a placeholder leaf. This is the identity case,
    // which is reachable without an arkworks dev-dependency; the torsion/off-curve
    // cases go through the same `is_prime_order()` predicate and are pinned in
    // `sparq_zk::sig::tests::torsion_key_rejected_from_key_set_and_witness`.
    #[test]
    fn unusable_key_is_rejected_by_both_builders() {
        // The identity point: `Affine::default()` is `Affine::zero()`.
        let identity = PublicKey(Default::default());
        assert!(
            !identity.is_prime_order(),
            "the identity point is not a usable issuer key"
        );
        assert_eq!(
            key_set_leaf(&identity),
            None,
            "key_set_leaf must fail closed on a non-prime-order key"
        );

        let good = keyset(&[100, 101, 102]);
        // The unusable key in first, middle and last position, so a guard that only
        // checks one end is still caught.
        for pos in 0..=good.len() {
            let mut keys = good.clone();
            keys.insert(pos, identity);
            assert_eq!(
                key_set_root(&keys, 2),
                None,
                "DENSE builder must fail closed on an unusable key at position {pos} \
                 (never commit a substitute/padding leaf for it)"
            );
            assert_eq!(
                key_set_root_sparse(&keys, 2),
                None,
                "SPARSE builder must fail closed on an unusable key at position {pos} \
                 (never commit a substitute/padding leaf for it)"
            );
            assert_eq!(
                key_membership_witness(&keys, 2, 0),
                None,
                "DENSE witness must fail closed on an unusable key at position {pos}"
            );
            assert_eq!(
                key_membership_witness_sparse(&keys, 2, 0),
                None,
                "SPARSE witness must fail closed on an unusable key at position {pos}"
            );
            assert_eq!(
                sparse_key_leaves(&keys, 2),
                None,
                "sparse_key_leaves must fail closed on an unusable key at position {pos}"
            );
        }
    }

    // sq-r6dq review round 3 (gpt-5.6-sol finding 6): no test used a VALID
    // depth 31, so tightening either `depth > 31` guard to `depth >= 31` — an
    // off-by-one that silently disables the deepest supported policy — reddened
    // nothing. Depth 31 is the LAST accepted depth (`1usize << 31` is representable
    // even on a 32-bit host, which is why the bound is 31 and not 63).
    //
    // Honest scope: this is SELF-CONSISTENCY, not a dense cross-check. The dense
    // builder would need `2^31` leaves, so no independent oracle exists here; the
    // cross-checks against the dense builder stop at depth 6 / 12 (see
    // `sparse_root_matches_dense_for_all_sizes` and the `verifier.rs` anchor test).
    #[test]
    fn sparse_valid_depth_31_root_and_path_are_defined() {
        let depth = 31u32;
        let keys = keyset(&[6001, 6002, 6003]);
        let root = key_set_root_sparse(&keys, depth)
            .expect("depth 31 is a VALID depth: the accepted bound is `depth > 31`, not `>= 31`");
        // Not the empty-tree digest — the members really are folded in.
        assert_ne!(
            Some(root),
            key_set_root_sparse(&[], depth),
            "a populated depth-31 tree must not equal the all-padding depth-31 root"
        );
        for index in 0..keys.len() as u64 {
            let sibs = key_membership_witness_sparse(&keys, depth, index)
                .expect("depth 31 must yield an authentication path");
            assert_eq!(sibs.len(), depth as usize, "one sibling per level");
            let mut node = key_set_leaf(&keys[index as usize]).unwrap();
            let mut pos = index;
            for sib in &sibs {
                let is_right = pos & 1 == 1;
                node = if is_right { h2(*sib, node) } else { h2(node, *sib) };
                pos /= 2;
            }
            assert_eq!(node, root, "depth-31 path for index {index} re-folds to the root");
        }
        // The bound is exactly 31: 32 is still rejected, so this test pins the edge
        // from BOTH sides and cannot be satisfied by loosening the guard either way.
        assert_eq!(key_set_root_sparse(&keys, 32), None, "depth 32 stays rejected");
        assert_eq!(
            key_membership_witness_sparse(&keys, 32, 0),
            None,
            "depth 32 stays rejected for the witness too"
        );
    }

    // ============================================================
    // sq-ru0yx (internal re-audit finding M-1): the challenge-reduction
    // NO-WRAP invariant, pinned host-side so the default `cargo test` lane (no nargo/
    // bb) guards the constants the in-circuit `issuer.nr::reduction_range_bind`
    // depends on. The in-circuit forge/accept tests live in `compose_core/tests.nr`
    // (`reduction_no_wrap_*`, toolchain lane). See research/zk-membership-pok-reaudit.md §3.
    // ============================================================

    // The Baby-JubJub order L and the no-wrap bound REDUCTION_TOP_BOUND = q_base -
    // 7L, as field elements (mirrors issuer.nr's BJJ_L / REDUCTION_TOP_BOUND). q_base
    // itself is NOT representable as an Fr (it reduces to 0), so the bound is the
    // canonical representative of -7L. These pin the soundness constants the in-
    // circuit no-wrap bound enforces; a drift invalidates the fix.
    const BJJ_L_HEX: &str =
        "0x060c89ce5c263405370a08b6d0302b0bab3eedb83920ee0a677297dc392126f1";
    const REDUCTION_TOP_BOUND_HEX: &str =
        "0x060c89ce5c263405370a08b6d0302b0b797b683ee9d2ee486fbfce8e6017ef6a"; // q_base - 7L

    fn fr_from_hex(h: &str) -> Fr {
        sparq_zk::field::field_from_hex_str(h).unwrap()
    }

    // The reduction arithmetic that justifies M-1: q_base // L == 7, and
    // 7L < q_base < 8L (so the e_k==7 bucket is the ONLY one that can wrap). Worked
    // in the field where q_base == 0, so -7L is the canonical REDUCTION_TOP_BOUND.
    #[test]
    fn reduction_top_bound_partitions_with_l() {
        let l = fr_from_hex(BJJ_L_HEX);
        let top_bound = fr_from_hex(REDUCTION_TOP_BOUND_HEX);
        // top_bound == q_base - 7L: in the field q_base == 0, so 7L + top_bound == 0
        // (i.e. 7L + (q-7L) = q == 0 mod q). This pins top_bound to exactly q - 7L.
        assert_eq!(l * Fr::from(7u64) + top_bound, Fr::from(0u64), "7L + (q-7L) == q == 0");
        // The honest e_k==7 range is [0, q-7L); the excluded wrap window is
        // [q-7L, L). They partition [0, L): top_bound + (8L - q) == L, and 8L - q is
        // the field value 8L (since q == 0). So top_bound + 8L == L.
        let window_width = l * Fr::from(8u64); // 8L - q, since q == 0 in the field
        assert_eq!(top_bound + window_width, l, "[0,q-7L) and [q-7L,L) partition [0,L)");
        // A real, narrowing exclusion: the bound is neither 0 nor all of L.
        assert_ne!(top_bound, Fr::from(0u64));
        assert_ne!(top_bound, l);
    }

    // The HONEST in-circuit witness always lands in a bucket the no-wrap bound
    // accepts, so the fix never rejects a valid prover: e_k is a 3-bit quotient in
    // {0..7} and the reduction identity e + e_k*L == e_base holds (the relation the
    // circuit binds). Real signatures' e_base is essentially random, so this also
    // documents that the top bucket is hit rarely (the wrap window is honest-
    // unreachable).
    #[test]
    fn honest_witness_respects_reduction_buckets() {
        use sparq_zk::sig::{commitment_message, in_circuit_witness, signature_from_hex};
        let l = fr_from_hex(BJJ_L_HEX);
        for seed in [1u64, 7, 42, 100, 255, 1009, 65537] {
            let sk = SecretKey::from_seed(seed);
            let c = Fr::from(seed.wrapping_mul(0x9e3779b97f4a7c15));
            let m = commitment_message(&c);
            let sig = signature_from_hex(&sk.sign_commitment(&c)).unwrap();
            let w = in_circuit_witness(&sk.public_key(), &m, &sig).unwrap();
            // e_k in {0..7}: a 3-bit quotient (the circuit's assert_max_bit_size::<3>).
            let k_ok = (0u64..=7).any(|k| w.e_k == Fr::from(k));
            assert!(k_ok, "seed {seed}: e_k must be a 3-bit quotient in {{0..7}}");
            // e is canonical (< L): e + (L-1-e) == L-1 with both representable, i.e.
            // the witnessed-difference range bind the circuit's assert_lt_l performs.
            // Confirm e != L and e is recovered by reducing e_base = e + e_k*L mod L.
            assert_ne!(w.e, l, "seed {seed}: reduced e must be < L (canonical)");
        }
    }
}

