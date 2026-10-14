//! sq-lsp7k.27: deterministic scalar aggregates over closed windows.

use oxrdf::Term;

use crate::WindowResult;

/// A scalar aggregate computed over one closed window's emitted rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agg {
    /// Number of rows, including rows where the selected variable is unbound.
    Count,
    /// Sum of the selected variable's numeric literal bindings.
    Sum,
    /// Arithmetic mean of the selected variable's numeric literal bindings.
    // sq-xf3b8: keep AVG on the same deterministic numeric fold as SUM.
    Avg,
    /// Minimum of the selected variable's numeric literal bindings.
    Min,
    /// Median of the selected variable's numeric literal bindings.
    // sq-sfle1: fold the sorted numeric input at its middle value(s).
    Median,
    /// Maximum of the selected variable's numeric literal bindings.
    Max,
}

/// Computes one scalar aggregate over a closed SELECT window result.
///
/// `var` is the projected variable name without a leading `?`. If it is not in
/// `window.vars`, this returns `None`. [`Agg::Count`] counts every emitted row,
/// including unbound and non-numeric bindings. The other aggregates skip
/// unbound, non-literal, non-numeric, and ill-formed numeric bindings. An empty
/// numeric input has sum `0.0` and no average, minimum, median, or maximum.
///
/// This helper only folds `window.rows`: it does not read the clock, re-query
/// the graph, or mutate the continuous query.
pub fn window_aggregate(window: &WindowResult, var: &str, aggregate: Agg) -> Option<f64> {
    let column = window
        .vars
        .iter()
        .position(|candidate| candidate.as_str() == var)?;

    if aggregate == Agg::Count {
        return Some(window.rows.len() as f64);
    }

    let mut numbers: Vec<f64> = window
        .rows
        .iter()
        .filter_map(|row| row.get(column))
        .filter_map(Option::as_ref)
        .filter_map(|value| match value {
            Term::Literal(literal) => {
                sparq_core::numeric_cache_value(literal.value(), literal.datatype().as_str())
            }
            _ => None,
        })
        .collect();

    // A canonical numeric order makes SUM independent of the engine's emitted
    // row order despite floating-point addition not being associative.
    numbers.sort_by(f64::total_cmp);

    match aggregate {
        Agg::Sum => Some(numbers.into_iter().sum()),
        Agg::Avg => {
            let count = numbers.len();
            (count != 0).then(|| numbers.into_iter().sum::<f64>() / count as f64)
        }
        Agg::Min => numbers.first().copied(),
        Agg::Median => {
            let middle = numbers.len() / 2;
            if numbers.len() % 2 == 1 {
                numbers.get(middle).copied()
            } else if middle == 0 {
                None
            } else {
                Some((numbers[middle - 1] + numbers[middle]) / 2.0)
            }
        }
        Agg::Max => numbers.last().copied(),
        Agg::Count => unreachable!("count returns before the numeric fold"),
    }
}
