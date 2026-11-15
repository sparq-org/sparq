//! Standalone research ORDER BY over supplied term cells. Not externally audited.
//!
//! Defined SPARQL comparisons form a partial order. At each output position we
//! choose the smallest canonical row with no remaining predecessor. This avoids
//! cyclic comparators when an undefined pair is mixed with numeric comparisons.
//! The suite opening must authenticate lexical keys for IRIs and literals.

use crate::interp::{row_key, Cell, Table};
use crate::Error;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderKey {
    pub column: usize,
    pub descending: bool,
}

fn kind_rank(c: Cell) -> u8 {
    match (c.bound, c.kind) {
        (false, _) => 0,
        (_, 2) => 1,
        (_, 1) => 2,
        _ => 3,
    }
}

pub(crate) fn order_cell(c: Cell) -> Result<(), Error> {
    c.validate()?;
    if c.bound && c.kind == 1 && c.key_class != 6 {
        return Err(Error::Unsupported(
            "ORDER BY IRI lexical key opening required".into(),
        ));
    }
    if c.bound && c.kind == 3 {
        match c.key_class {
            2 => {
                return Err(Error::Unsupported(
                    "ORDER BY double arithmetic pending".into(),
                ))
            }
            7 => {
                return Err(Error::Unsupported(
                    "ORDER BY lexical opening required".into(),
                ))
            }
            127 => return Err(Error::Capacity("ORDER BY comparison capacity".into())),
            0..=6 => {}
            _ => return Err(Error::Invalid("ORDER BY key class".into())),
        }
    }
    if c.bound && (c.kind == 1 || c.kind == 3 && c.key_class > 0) && c.key[..2] != [0; 2] {
        return Err(Error::Capacity("ORDER BY comparison capacity".into()));
    }
    Ok(())
}

/// None means the Recommendation leaves this pair unordered.
pub(crate) fn term_relation(a: Cell, b: Cell) -> Option<Ordering> {
    let rank = kind_rank(a).cmp(&kind_rank(b));
    if rank != Ordering::Equal {
        return Some(rank);
    }
    let comparable = a.bound
        && b.bound
        && (a.kind == 1
            || a.kind == 3
                && a.key_class == b.key_class
                && match a.key_class {
                    1 | 5 => true,
                    3 | 4 => a.dt == b.dt,
                    6 => a.lang == [0; 32] && b.lang == [0; 32] && a.dt == b.dt,
                    _ => false,
                });
    if comparable {
        Some(a.key.cmp(&b.key))
    } else if a.same_term(b) {
        Some(Ordering::Equal)
    } else {
        None
    }
}

pub(crate) fn order_pair(a: Cell, b: Cell) -> Result<(), Error> {
    if a.bound
        && b.bound
        && a.kind == 3
        && b.kind == 3
        && a.dt == b.dt
        && matches!((a.key_class, b.key_class), (3, 4) | (4, 3))
    {
        return Err(Error::Unsupported(
            "ORDER BY mixed timezone comparison pending".into(),
        ));
    }
    let lexical = a.bound
        && b.bound
        && (a.kind == 1 && b.kind == 1
            || a.kind == 3
                && b.kind == 3
                && a.key_class == 6
                && b.key_class == 6
                && a.dt == b.dt
                && a.lang == [0; 32]
                && b.lang == [0; 32]);
    if lexical && a.key == b.key && !a.same_term(b) {
        return Err(Error::Unsupported(
            "ORDER BY lexical key collision opening required".into(),
        ));
    }
    Ok(())
}

pub(crate) fn table_shape(input: &Table, width: usize) -> Result<(), Error> {
    for row in &input.rows {
        if row.len() != width {
            return Err(Error::Invalid("closed operator row width".into()));
        }
        for c in row {
            c.validate()?;
        }
    }
    if input
        .rows
        .windows(2)
        .any(|r| row_key(&r[1]) < row_key(&r[0]))
    {
        return Err(Error::Invalid("closed operator input order".into()));
    }
    Ok(())
}

/// Width comes from the compiled schema, including when the input has no rows.
pub fn order_by(
    input: &Table,
    width: usize,
    keys: &[OrderKey],
    capacity: usize,
) -> Result<Table, Error> {
    table_shape(input, width)?;
    for key in keys {
        if key.column >= width {
            return Err(Error::Invalid("ORDER BY column".into()));
        }
        for row in &input.rows {
            order_cell(row[key.column])?;
            for other in &input.rows {
                order_pair(row[key.column], other[key.column])?;
            }
        }
    }
    if input.rows.len() > capacity {
        return Err(Error::Capacity("ORDER BY output".into()));
    }
    let precedes = |a: &[Cell], b: &[Cell]| {
        for key in keys {
            match term_relation(a[key.column], b[key.column]) {
                Some(Ordering::Equal) => {}
                Some(cmp) => return (cmp == Ordering::Less) != key.descending,
                None => return false,
            }
        }
        false
    };
    let n = input.rows.len();
    let before: Vec<Vec<bool>> = input
        .rows
        .iter()
        .map(|a| input.rows.iter().map(|b| precedes(a, b)).collect())
        .collect();
    let mut used = vec![false; n];
    let mut rows = Vec::with_capacity(n);
    for _ in 0..n {
        let selected = (0..n)
            .filter(|&i| !used[i] && (0..n).all(|j| used[j] || !before[j][i]))
            .min_by_key(|&i| row_key(&input.rows[i]))
            .ok_or_else(|| Error::Invalid("ORDER BY comparison cycle".into()))?;
        used[selected] = true;
        rows.push(input.rows[selected].clone());
    }
    Ok(Table { rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_malformed_tables() {
        assert_eq!(
            order_by(&Table { rows: vec![] }, 0, &[], 0).unwrap().rows,
            Vec::<Vec<Cell>>::new()
        );
        assert!(matches!(
            order_by(
                &Table {
                    rows: vec![vec![], vec![Cell::default()]]
                },
                0,
                &[],
                2
            ),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            order_by(
                &Table { rows: vec![] },
                0,
                &[OrderKey {
                    column: 0,
                    descending: false
                }],
                1
            ),
            Err(Error::Invalid(_))
        ));
    }
}
