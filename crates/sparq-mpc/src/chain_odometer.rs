// Shared lazy enumerator for the fixed predicate sequences a bounded
// property-path unroll distributes an alternation over. Replaces the two
// materialized `Vec<Vec<_>>` chain sets in `bounded_path` and `hidden_path`, which
// held every chain — and a clone of every predicate IRI in every chain — in memory
// at once.
//! Lazy enumeration of the `a^length` fixed predicate sequences of a bounded
//! property-path unroll.
//!
//! ## Why lazy
//!
//! `(step){m,k}` over an `a`-way alternation unrolls to the `a^ℓ` fixed predicate
//! sequences of each length `ℓ`. Building them as a `Vec<Vec<P>>` holds all of them
//! at once and CLONES each predicate IRI once per chain it appears in, so the peak
//! memory is `a^ℓ · ℓ · |IRI|` even though the consumer evaluates one chain at a
//! time and never looks back. [`ChainOdometer`] yields them one at a time from a
//! single reusable buffer instead: the peak is `ℓ · |IRI|` plus the `ℓ` odometer
//! digits, whatever `a^ℓ` is, so the memory no longer scales with the chain COUNT
//! and no product of chain count and IRI length has to be bounded ahead of it.
//!
//! The chain COUNT is still bounded by the caller before the enumeration starts
//! (`bounded_path::MAX_UNROLL_CHAINS` / `hidden_path::MAX_CHAIN_TUPLES`) — the
//! odometer bounds the memory a given count costs, not the count.
//!
//! ## The order is the materialized order
//!
//! The digits advance least-significant-LAST (`digits[length - 1]` first), which is
//! exactly the order the nested `for prefix in &chains { for alt in alternatives }`
//! build produced. That matters: both callers draw randomness per chain, so a
//! different order would move every transcript pin downstream of them.

/// A lazy odometer over the `alternatives.len()^length` fixed predicate sequences
/// of one unroll length.
///
/// Not an [`Iterator`]: each chain is handed out as a borrow of the odometer's own
/// reusable buffer, which `Iterator::next` cannot express. Drive it with
/// [`Self::next_chain`] in a `while let` loop.
pub(crate) struct ChainOdometer<'a, P> {
    alternatives: &'a [P],
    /// One digit per hop position, each in `0..alternatives.len()`.
    digits: Vec<usize>,
    /// The chain the digits currently select — rebuilt in place, never reallocated.
    buf: Vec<P>,
    /// Whether [`Self::next_chain`] has handed out the first chain yet.
    started: bool,
    /// Whether the odometer has rolled over past its last chain.
    done: bool,
}

impl<'a, P: Clone> ChainOdometer<'a, P> {
    /// An odometer over the `length`-hop sequences chosen from `alternatives`.
    ///
    /// Two edge cases, both matching what the materialized build produced:
    ///
    /// - `length == 0` yields exactly ONE chain, the empty one (the old
    ///   `vec![Vec::new()]` seed, never extended).
    /// - an empty `alternatives` with `length > 0` yields NOTHING (the old build's
    ///   inner loop never ran, so the chain set collapsed to empty).
    pub(crate) fn new(alternatives: &'a [P], length: usize) -> Self {
        let done = length > 0 && alternatives.is_empty();
        ChainOdometer {
            alternatives,
            digits: vec![0; length],
            buf: Vec::with_capacity(length),
            started: false,
            done,
        }
    }

    /// The next chain, or `None` once every chain has been handed out.
    ///
    /// The returned slice is the odometer's own buffer and is valid until the next
    /// call.
    pub(crate) fn next_chain(&mut self) -> Option<&[P]> {
        if self.done {
            return None;
        }
        if self.started {
            // Advance the base-`a` odometer, least-significant digit LAST.
            let mut pos = self.digits.len();
            loop {
                if pos == 0 {
                    self.done = true;
                    return None;
                }
                pos -= 1;
                self.digits[pos] += 1;
                if self.digits[pos] < self.alternatives.len() {
                    break;
                }
                self.digits[pos] = 0;
            }
        } else {
            self.started = true;
        }
        self.buf.clear();
        for &d in &self.digits {
            self.buf.push(self.alternatives[d].clone());
        }
        Some(&self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The materialized build this odometer replaces, kept verbatim as the
    /// differential oracle: every chain, in build order, with a clone of each
    /// alternative per position.
    fn materialized<P: Clone>(alternatives: &[P], length: usize) -> Vec<Vec<P>> {
        let mut chains: Vec<Vec<P>> = vec![Vec::new()];
        for _ in 0..length {
            let mut next: Vec<Vec<P>> = Vec::new();
            for prefix in &chains {
                for alt in alternatives {
                    let mut extended = prefix.clone();
                    extended.push(alt.clone());
                    next.push(extended);
                }
            }
            chains = next;
        }
        chains
    }

    fn drained<P: Clone>(alternatives: &[P], length: usize) -> Vec<Vec<P>> {
        let mut odo = ChainOdometer::new(alternatives, length);
        let mut out = Vec::new();
        while let Some(chain) = odo.next_chain() {
            out.push(chain.to_vec());
        }
        out
    }

    /// **THE odometer guard.** The lazy enumeration equals the materialized one
    /// element for element and in the same ORDER, across alternation widths and
    /// unroll lengths — including the two edge cases (`length == 0`, which yields
    /// one empty chain, and an empty alternation, which yields none).
    ///
    /// Order is load-bearing, not cosmetic: both callers draw randomness per chain,
    /// so a reordering would move every transcript pin downstream.
    #[test]
    fn the_odometer_matches_the_materialized_chains_exactly() {
        let alts: Vec<String> = ["p", "q", "r"].iter().map(|s| s.to_string()).collect();
        for a in 0..=alts.len() {
            for length in 0..=4 {
                let expected = materialized(&alts[..a], length);
                let got = drained(&alts[..a], length);
                assert_eq!(
                    got, expected,
                    "a={}, length={}: lazy enumeration must equal the materialized one",
                    a, length
                );
            }
        }

        // The edge cases, called out explicitly so a change to either is visible.
        assert_eq!(drained(&alts, 0), vec![Vec::<String>::new()]);
        assert_eq!(drained(&alts[..0], 3), Vec::<Vec<String>>::new());
    }

    /// Exhaustion is terminal: once the odometer is done it stays done, so a
    /// consumer's `while let` loop cannot be restarted by accident.
    #[test]
    fn an_exhausted_odometer_stays_exhausted() {
        let alts = ["p".to_string()];
        let mut odo = ChainOdometer::new(&alts, 1);
        assert!(odo.next_chain().is_some());
        assert!(odo.next_chain().is_none());
        assert!(odo.next_chain().is_none());
    }
}
