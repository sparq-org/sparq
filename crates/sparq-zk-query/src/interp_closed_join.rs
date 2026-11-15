//! Stable canonical bag join and its private inverse map.
use crate::{
    interp::{row_key, Cell, Row, Table},
    plan::Var,
    Error,
};
pub(crate) fn evaluate(
    left: &Table,
    lv: &[Var],
    right: &Table,
    rv: &[Var],
    vars: &[Var],
    capacity: usize,
) -> Result<Vec<(Row, [u32; 2])>, Error> {
    let get = |schema: &[Var], row: &[Cell], v: &Var| {
        schema
            .iter()
            .position(|x| x == v)
            .map_or(Cell::default(), |i| row[i])
    };
    let mut result = vec![];
    for (i, a) in left.rows.iter().enumerate() {
        for (j, b) in right.rows.iter().enumerate() {
            let mut matches = true;
            let row: Row = vars
                .iter()
                .map(|v| {
                    let l = get(lv, a, v);
                    let r = get(rv, b, v);
                    matches &= !l.bound || !r.bound || l.same_term(r);
                    if l.bound {
                        l
                    } else {
                        r
                    }
                })
                .collect();
            if matches {
                result.push((row, [i as u32, j as u32]));
            }
            if result.len() > capacity {
                return Err(Error::Capacity("closed join output".into()));
            }
        }
    }
    result.sort_by_key(|(row, _)| row_key(row));
    Ok(result)
}
