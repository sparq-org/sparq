//! sq-xg6uu — OWL 2 RL differential-vs-external-oracle harness.
//!
//! Read-only differential testing of `sparq_reason::materialize(Profile::OwlRl, ..)`
//! against **pre-captured** golden closure vectors from the Python
//! [`owlrl`](https://github.com/RDFLib/OWL-RL) reference implementation, over the
//! hand-picked TBox/ABox corpus in `bench/reason-diff/rl/`. The goldens are captured
//! OFFLINE by `bench/reason-diff/rl/capture.py` (tool versions pinned in each fixture
//! header) — owlrl is **never** run in CI; CI only re-diffs against the committed
//! vectors.
//!
//! HONESTY SCOPE: a green run claims exactly "sparq-reason's RL closure matches the
//! pinned external oracle on this corpus, under the documented normalization in
//! `rl::normalize`, modulo the explicitly-dispositioned PERMANENT divergences" —
//! nothing more (no completeness claim; the corpus is small and hand-picked). Any
//! UNdispositioned mismatch fails loud, per the sq-pbz04.1 disposition-ledger
//! precedent.
//!
//! Everything is behind the `rl-oracle` feature: a default build of this crate is an
//! empty lib and links neither sparq crate.

#![forbid(unsafe_code)]

#[cfg(feature = "rl-oracle")]
pub mod rl;
