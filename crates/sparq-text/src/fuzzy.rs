//! Bounded typo-tolerant lookup over a [`TextIndex`](crate::TextIndex).
//!
//! The `fuzzy` feature is default-OFF. When enabled, the ordinary token index
//! additionally records deletion signatures through distance two. A query
//! probes only its own bounded deletion neighbourhood, then verifies candidates
//! with exact Levenshtein distance; it never walks the full token dictionary.
//! sq-lsp7k.14

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use sparq_core::dict::Id;

/// Default edit-distance budget for fuzzy lookup.
pub const DEFAULT_MAX_DISTANCE: u8 = 1;
/// Largest accepted edit-distance budget.
pub const MAX_DISTANCE: u8 = 2;

/// One typo-tolerant text hit, ordered by distance ascending, BM25 descending,
/// matched token ascending, then dictionary id ascending.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyHit {
    /// Matching literal dictionary id.
    pub id: Id,
    /// Levenshtein distance between the analyzed query and `term`.
    pub distance: u8,
    /// BM25 score for `term` in this literal.
    pub score: f32,
    /// The indexed token that matched the query.
    pub term: Box<str>,
}

/// Invalid fuzzy-query configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FuzzyError {
    /// Fuzzy lookup accepts exactly one analyzed token.
    ExpectedSingleToken { found: usize },
    /// Prefix syntax belongs to exact `search`, not fuzzy lookup.
    PrefixNotSupported,
    /// The requested edit-distance budget exceeds [`MAX_DISTANCE`].
    DistanceTooLarge { requested: u8 },
}

impl fmt::Display for FuzzyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedSingleToken { found } => {
                write!(
                    f,
                    "fuzzy query must contain exactly one token, found {found}"
                )
            }
            Self::PrefixNotSupported => {
                f.write_str("fuzzy query does not accept the `*` prefix marker")
            }
            Self::DistanceTooLarge { requested } => write!(
                f,
                "fuzzy max_distance must be at most {MAX_DISTANCE}, got {requested}"
            ),
        }
    }
}

impl std::error::Error for FuzzyError {}

/// Deletion signature -> sorted distinct indexed tokens. Signatures through
/// the global cap are built once when a token first enters the inverted index.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct FuzzyTerms {
    signatures: BTreeMap<Box<str>, Vec<Box<str>>>,
}

impl FuzzyTerms {
    pub(crate) fn insert(&mut self, term: &str) {
        for signature in deletion_signatures(term, MAX_DISTANCE) {
            let terms = self
                .signatures
                .entry(signature.into_boxed_str())
                .or_default();
            match terms.binary_search_by(|known| known.as_ref().cmp(term)) {
                Ok(_) => {}
                Err(at) => terms.insert(at, term.into()),
            }
        }
    }

    pub(crate) fn candidates<'a>(&'a self, query: &str, max_distance: u8) -> Vec<(&'a str, u8)> {
        let mut candidates: BTreeSet<&str> = BTreeSet::new();
        for signature in deletion_signatures(query, max_distance) {
            if let Some(terms) = self.signatures.get(signature.as_str()) {
                candidates.extend(terms.iter().map(Box::as_ref));
            }
        }
        candidates
            .into_iter()
            .filter_map(|term| {
                bounded_levenshtein(query, term, max_distance).map(|distance| (term, distance))
            })
            .collect()
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.signatures
            .iter()
            .map(|(signature, terms)| {
                signature.len()
                    + terms.iter().map(|term| term.len()).sum::<usize>()
                    + terms.capacity() * std::mem::size_of::<Box<str>>()
            })
            .sum()
    }
}

fn deletion_signatures(term: &str, distance: u8) -> BTreeSet<String> {
    let mut all = BTreeSet::from([term.to_owned()]);
    let mut frontier = vec![term.to_owned()];
    for _ in 0..distance {
        let mut next = Vec::new();
        for value in frontier {
            let chars: Vec<char> = value.chars().collect();
            for removed in 0..chars.len() {
                let deletion: String = chars[..removed]
                    .iter()
                    .chain(&chars[removed + 1..])
                    .collect();
                if all.insert(deletion.clone()) {
                    next.push(deletion);
                }
            }
        }
        frontier = next;
    }
    all
}

/// Exact Unicode-scalar Levenshtein distance with a small upper bound.
fn bounded_levenshtein(a: &str, b: &str, cap: u8) -> Option<u8> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > usize::from(cap) {
        return None;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, left) in a.iter().enumerate() {
        current[0] = i + 1;
        let mut row_min = current[0];
        for (j, right) in b.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(left != right));
            row_min = row_min.min(current[j + 1]);
        }
        if row_min > usize::from(cap) {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    (previous[b.len()] <= usize::from(cap)).then_some(previous[b.len()] as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_index_has_no_false_negatives_for_reference_pairs() {
        let mut terms = FuzzyTerms::default();
        for term in ["quickly", "quote", "quick", "quaxly", "café"] {
            terms.insert(term);
        }
        assert_eq!(terms.candidates("quikly", 1), vec![("quickly", 1)]);
        assert_eq!(
            terms.candidates("quikly", 2),
            vec![("quaxly", 2), ("quickly", 1)]
        );
        assert!(terms.candidates("quikly", 0).is_empty());
    }
}
