//! # `secprop` — the sparq `sec-prop:` extension vocabulary (re-export)
//!
//! The IRIs of the sparq **security-properties ontology** as Rust constants. Since
//! issue #3705 the constants, the canonical machine-readable
//! `secprop-ext.ttl`, and the single TTL↔constant drift test all live in the
//! dependency-free leaf crate [`sparq_secprop_vocab`]; this module **re-exports**
//! them so `sparq_trust::secprop::SECX_…` keeps working unchanged.
//!
//! ## Why the vocabulary moved out
//!
//! `sparq-trust` is the *declared owner* of this vocabulary but it **depends on**
//! `sparq-zk`, so `sparq-zk` could not import the constants (a cycle) and
//! `sparq-policy` could not either without dragging `sparq-zk` + `sparq-canon` +
//! `sparq-shacl` + `sparq-reason` into its lean graph. All three kept their own
//! copy, pinned to this crate's Turtle by a cross-package `include_str!` — which
//! also broke `cargo package` file inclusion for `sparq-zk`. The leaf crate is the
//! home below all three, so there is now exactly one copy of every IRI.
//!
//! ## What this vocabulary is — and is NOT
//!
//! It **records** a (method, property) → (level, assurance, audit-status, assumption)
//! claim and its **epistemic basis**; it is **NOT** a proof of any property. The
//! **assurance axis** ([`SECX_PROVEN`] ⊐ [`SECX_CLAIMED`] ⊐ [`SECX_CONJECTURED`]) is
//! the honesty mechanism: it is **one** axis, orthogonal to every property (stated
//! once, not multiplied across dimensions). The **default** assurance for a
//! sparq-asserted ZK property is [`SECX_CLAIMED`] (issue #1001 Option A) and the live
//! audit status is [`SECX_EXTERNAL_SIGN_OFF_PENDING`] — sparq's ZK estate is
//! research-grade and **externally UNAUDITED** (`sq-qhy4`, pending an external
//! accredited-cryptographer sign-off). **No sparq ZK method may be labelled
//! [`SECX_PROVEN`] while `sq-qhy4` is open.**
//!
//! ## Opt-in by construction
//!
//! This module is behind the **default-OFF `secprop-vocab`** cargo feature, which is
//! what pulls the (zero-dependency) leaf crate in, so the lean default build is
//! byte-unchanged — plain `const &str` data, no new dependency, strictly additive.
//!
//! [OPUS-4.8] sq-5oru9 (epic sq-0dksu; design PR #972); extracted to
//! [`sparq_secprop_vocab`] by [OPUS-5] sq-3705. 🤖 SPARQ agent —
//! security-properties ontology.

pub use sparq_secprop_vocab::*;

#[cfg(test)]
mod tests {
    /// The re-export is the SAME data, not a second copy — the whole point of
    /// #3705. Spot-check that the namespace base and the registry this crate's
    /// `admit`/`admissibility` modules read really do come from the leaf crate.
    #[test]
    fn secprop_re_exports_the_leaf_crate_vocabulary() {
        assert!(std::ptr::eq(
            super::ALL_SECPROP_IRIS,
            sparq_secprop_vocab::ALL_SECPROP_IRIS,
        ));
        assert_eq!(super::SEC_PROP_NS, sparq_secprop_vocab::SEC_PROP_NS);
    }
}
