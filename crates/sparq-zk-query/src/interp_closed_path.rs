//! Complete bounded path evaluation. No traversal samples or truncated closures.

use super::*;
use crate::plan_closed_path::{Path, PathOperator};

type Pair = [Cell; 2];
fn same(a: &Pair, b: &Pair) -> bool {
    a[0].same_term(b[0]) && a[1].same_term(b[1])
}
fn add(rows: &mut Vec<Pair>, row: Pair, set: bool, capacity: usize) -> Result<(), Error> {
    if !set || !rows.iter().any(|r| same(r, &row)) {
        if rows.len() >= capacity {
            return Err(Error::Capacity("path intermediate relation".into()));
        }
        rows.push(row);
    }
    Ok(())
}
fn relation(
    p: &Path,
    data: &[[Cell; 4]],
    anchors: [Cell; 2],
    params: &[Option<Cell>],
    depth: u32,
    capacity: usize,
    restrict: bool,
) -> Result<Vec<Pair>, Error> {
    let mut out = vec![];
    match p {
        Path::Link(_) | Path::Negated(_) => {
            let negated = matches!(p, Path::Negated(_));
            for q in data {
                let selected = match p {
                    Path::Link(id) => q[1].same_term(params[*id as usize].expect("path predicate")),
                    Path::Negated(ids) => !ids
                        .iter()
                        .any(|id| q[1].same_term(params[*id as usize].expect("path exclusion"))),
                    _ => unreachable!(),
                };
                if selected {
                    add(&mut out, [q[0], q[2]], negated, capacity)?;
                }
            }
        }
        Path::Inverse(a) => {
            for r in relation(
                a,
                data,
                [anchors[1], anchors[0]],
                params,
                depth,
                capacity,
                restrict,
            )? {
                add(&mut out, [r[1], r[0]], false, capacity)?;
            }
        }
        Path::Sequence(a, b) | Path::Alternative(a, b) => {
            let sequence = matches!(p, Path::Sequence(..));
            let a = relation(
                a,
                data,
                if sequence {
                    [anchors[0], Cell::default()]
                } else {
                    anchors
                },
                params,
                depth,
                capacity,
                restrict,
            )?;
            let b = relation(
                b,
                data,
                if sequence {
                    [Cell::default(), anchors[1]]
                } else {
                    anchors
                },
                params,
                depth,
                capacity,
                restrict,
            )?;
            if matches!(p, Path::Alternative(..)) {
                for r in a.into_iter().chain(b) {
                    add(&mut out, r, false, capacity)?;
                }
            } else {
                for l in &a {
                    for r in &b {
                        if l[1].same_term(r[0]) {
                            add(&mut out, [l[0], r[1]], false, capacity)?;
                        }
                    }
                }
            }
        }
        Path::ZeroOrOne(a) | Path::ZeroOrMore(a) | Path::OneOrMore(a) => {
            let input = relation(a, data, anchors, params, depth, capacity, false)?;
            for r in &input {
                add(&mut out, *r, true, capacity)?;
            }
            if !matches!(p, Path::ZeroOrOne(_)) {
                for _ in 1..depth {
                    let prior = out.clone();
                    for l in &prior {
                        for r in &input {
                            if l[1].same_term(r[0]) {
                                add(&mut out, [l[0], r[1]], true, capacity)?;
                            }
                        }
                    }
                }
                // One full extension over every edge must add no pair.
                for l in &out {
                    for r in &input {
                        if l[1].same_term(r[0]) && !out.iter().any(|x| same(x, &[l[0], r[1]])) {
                            return Err(Error::Capacity(
                                "path closure exceeds compiled depth".into(),
                            ));
                        }
                    }
                }
            }
            if !matches!(p, Path::OneOrMore(_)) {
                for q in data {
                    for c in [q[0], q[2]] {
                        add(&mut out, [c, c], true, capacity)?;
                    }
                }
                for c in anchors {
                    if c.bound {
                        add(&mut out, [c, c], true, capacity)?;
                    }
                }
            }
        }
    }
    if restrict {
        out.retain(|r| (0..2).all(|i| !anchors[i].bound || anchors[i].same_term(r[i])));
    }
    out.sort_by_key(|r| row_key(r));
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate(
    p: &PathOperator,
    node: &Node,
    plan: &Plan,
    tables: &[Table],
    dataset: &[[Cell; 4]],
    credentials: &[CredentialCells],
    params: &[Option<Cell>],
) -> Result<Vec<Row>, Error> {
    if !matches!(plan.parameters[p.depth_parameter as usize], Parameter::Index(d) if d == p.depth) {
        return Err(Error::Invalid("public path depth".into()));
    }
    let context = p.context.and_then(|id| {
        tables[id as usize].rows.first().map(|r| {
            (
                plan.nodes[id as usize].output.variables.as_slice(),
                r.as_slice(),
            )
        })
    });
    let mut captured = [false; 2];
    let anchors = std::array::from_fn(|i| match &p.endpoints[i] {
        Slot::Parameter(id) => params[*id as usize].expect("path endpoint"),
        Slot::Variable(v) => {
            let c = context.map(|(s, r)| lookup(s, r, *v)).unwrap_or_default();
            captured[i] = c.bound;
            c
        }
    });
    let mut named = vec![];
    if let Some(c) = p.credential.and_then(|c| credentials.get(c as usize)) {
        let mut seen = BTreeSet::new();
        named.extend(
            c.quads
                .iter()
                .filter(|q| seen.insert(std::array::from_fn::<_, 3, _>(|i| q[i].identity())))
                .copied(),
        );
    }
    let data = if p.credential.is_some() {
        named.as_slice()
    } else {
        dataset
    };
    let pairs = relation(
        &p.path,
        data,
        anchors,
        params,
        p.depth,
        plan.capacity.rows_per_table as usize,
        true,
    )?;
    let mut out = vec![];
    for pair in pairs {
        let mut matches = p.context.is_none() || context.is_some();
        let mut row = vec![Cell::default(); node.output.variables.len()];
        for i in 0..2 {
            matches &= !anchors[i].bound || anchors[i].same_term(pair[i]);
            if let Slot::Variable(v) = p.endpoints[i] {
                if !captured[i] {
                    let col = node
                        .output
                        .variables
                        .iter()
                        .position(|x| *x == v)
                        .expect("endpoint column");
                    matches &= !row[col].bound || row[col].same_term(pair[i]);
                    row[col] = pair[i];
                }
            }
        }
        if matches {
            out.push(row);
        }
    }
    Ok(out)
}
