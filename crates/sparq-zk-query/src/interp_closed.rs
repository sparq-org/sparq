//! Exact closed relations over complete input tables.

use super::*;
use crate::plan_closed::{ClosedOperator as C, Predicate};

#[allow(clippy::too_many_arguments)]
pub(super) fn evaluate(
    op: &C,
    node: &Node,
    plan: &Plan,
    tables: &[Table],
    _dataset: &[[Cell; 4]],
    input_dataset: &Dataset,
    params: &[Option<Cell>],
) -> Result<Vec<Row>, Error> {
    let needs_graphs = crate::credential_graphs::is_profile(plan)
        || matches!(
            op,
            C::GraphScan { .. }
                | C::GraphSeed { .. }
                | C::Graph { .. }
                | C::Scan { credential: Some(_), .. }
        );
    if needs_graphs
        && !input_dataset.counts.is_empty()
        && !input_dataset.quads.iter().any(|q| q[3].bound)
    {
        return Err(Error::Unsupported(
            "complete credential graph view; use evaluate_cells_for_request or evaluate_for_request"
                .into(),
        ));
    }
    let credentials = complete_credentials(input_dataset)?;
    let credentials = credentials.as_slice();
    let names = (0..credentials.len())
        .map(|i| {
            input_dataset
                .quads
                .iter()
                .find(|q| q[3].bound && q[3].scope == i as u32 + 1)
                .map(|q| Ok(q[3]))
                .unwrap_or_else(|| crate::credential_graphs::name(i as u32))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let rows = |id: NodeId| &tables[id as usize].rows;
    let schema = |id: NodeId| plan.nodes[id as usize].output.variables.as_slice();
    let vars = &node.output.variables;
    let context_row = |context: Option<NodeId>, base_vars: &[Var], base: &[Cell]| {
        let mut all = base_vars.to_vec();
        if let Some(c) = context {
            all.extend(schema(c));
            all.sort_unstable();
            all.dedup();
        }
        let row = all
            .iter()
            .map(|v| {
                let outer = context
                    .and_then(|c| rows(c).first().map(|r| lookup(schema(c), r, *v)))
                    .unwrap_or_default();
                if outer.bound {
                    outer
                } else {
                    lookup(base_vars, base, *v)
                }
            })
            .collect::<Row>();
        (all, row)
    };
    Ok(match op {
        C::Join {left,right} => crate::interp_closed_join::evaluate(&tables[*left as usize],schema(*left),&tables[*right as usize],schema(*right),vars,node.output.capacity as usize)?.into_iter().map(|(row,_)|row).collect(),
        C::CredentialProfile { literal_types, .. } => {
            crate::credential_graphs::check_literal_types(*literal_types, credentials)?;
            vec![vec![]]
        }
        C::GraphScan {
            pattern,
            selector,
            context,
        } => {
            if context.is_some_and(|c| rows(c).is_empty()) {
                vec![]
            } else {
                let mut output = vec![];
                for (c, data) in credentials.iter().enumerate() {
                    let name = names[c];
                    let captured = context
                        .and_then(|id| {
                            rows(id).first().map(|r| match selector {
                                Slot::Variable(v) => lookup(schema(id), r, *v),
                                Slot::Parameter(p) => params[*p as usize].expect("graph selector"),
                            })
                        })
                        .unwrap_or_else(|| match selector {
                            Slot::Parameter(p) => params[*p as usize].expect("selector"),
                            _ => Cell::default(),
                        });
                    if captured.bound && !captured.same_term(name) {
                        continue;
                    }
                    let ctxvars = context.map(schema).unwrap_or(&[]);
                    let ctxrow = context.and_then(|id| rows(id).first().map(Vec::as_slice));
                    let mut found = crate::credential_graphs::scan(
                        pattern,
                        vars,
                        ctxrow,
                        ctxvars,
                        (Some(c as u32), false),
                        credentials,
                        params,
                        &names,
                    )?;
                    if let Slot::Variable(v) = selector {
                        if !captured.bound {
                            let col = vars.iter().position(|a| a == v).expect("graph column");
                            found.retain_mut(|r| {
                                let compatible = !r[col].bound || r[col].same_term(name);
                                r[col] = name;
                                compatible
                            });
                        }
                    }
                    let _ = data;
                    output.extend(found);
                }
                output
            }
        }
        C::DatasetScan { pattern } => crate::credential_graphs::scan(
            pattern,
            vars,
            None,
            &[],
            (None, true),
            credentials,
            params,
            &names,
        )?,
        C::Group {
            input,
            keys,
            aggregates,
        } => {
            crate::interp_closed_group_plan::evaluate(
                &tables[*input as usize],
                schema(*input),
                keys,
                aggregates,
                plan.bounds().rows_per_table as usize,
            )?
            .rows
        }
        C::Union { left, right } => [*left, *right]
            .into_iter()
            .flat_map(|id| {
                rows(id)
                    .iter()
                    .map(move |r| vars.iter().map(|v| lookup(schema(id), r, *v)).collect())
            })
            .collect(),
        C::Minus { left, right } => rows(*left)
            .iter()
            .filter(|a| {
                !rows(*right).iter().any(|b| {
                    let mut shared = false;
                    let mut compatible = true;
                    for v in schema(*left) {
                        let l = lookup(schema(*left), a, *v);
                        let r = lookup(schema(*right), b, *v);
                        shared |= l.bound && r.bound;
                        compatible &= !l.bound || !r.bound || l.same_term(r);
                    }
                    shared && compatible
                })
            })
            .cloned()
            .collect(),
        C::LeftJoin {
            left,
            right,
            condition,
            context,
        } => {
            let mut output = vec![];
            for a in rows(*left) {
                let mut matched = false;
                for b in rows(*right) {
                    let mut compatible = true;
                    let merged: Row = vars
                        .iter()
                        .map(|v| {
                            let l = lookup(schema(*left), a, *v);
                            let r = lookup(schema(*right), b, *v);
                            compatible &= !l.bound || !r.bound || l.same_term(r);
                            if l.bound {
                                l
                            } else {
                                r
                            }
                        })
                        .collect();
                    if compatible {
                        let (eval_vars, eval_row) = context_row(*context, vars, &merged);
                        let (pass, error) = condition
                            .as_ref()
                            .map(|e| {
                                eval(e, &eval_vars, &eval_row, params, &plan.parameters)
                                    .and_then(Value::ebv)
                            })
                            .transpose()?
                            .unwrap_or((true, false));
                        if pass && !error {
                            matched = true;
                            output.push(merged);
                        }
                    }
                }
                if !matched {
                    output.push(vars.iter().map(|v| lookup(schema(*left), a, *v)).collect());
                }
            }
            output
        }
        C::Extend {
            input,
            variable,
            value,
            context,
        } => rows(*input)
            .iter()
            .map(|r| {
                let (sv, sr) = context_row(*context, schema(*input), r);
                vars.iter()
                    .map(|v| {
                        if v == variable {
                            match value {
                                Slot::Variable(v) => lookup(&sv, &sr, *v),
                                Slot::Parameter(p) => params[*p as usize].expect("term parameter"),
                            }
                        } else {
                            lookup(schema(*input), r, *v)
                        }
                    })
                    .collect()
            })
            .collect(),
        C::Substitute { input, context } => {
            let mut output = vec![];
            if let Some(c) = rows(*context).first() {
                for r in rows(*input) {
                    let mut matches = true;
                    let row = schema(*input)
                        .iter()
                        .map(|v| {
                            let outer = lookup(schema(*context), c, *v);
                            let value = lookup(schema(*input), r, *v);
                            matches &= !outer.bound || !value.bound || outer.same_term(value);
                            if outer.bound {
                                Cell::default()
                            } else {
                                value
                            }
                        })
                        .collect();
                    if matches {
                        output.push(row);
                    }
                }
            }
            output
        }
        C::Capture { input, row, parent } => {
            if let Some(r) = rows(*input).get(*row as usize) {
                if parent.is_some_and(|p| rows(p).is_empty()) {
                    vec![]
                } else {
                    let (_, r) = context_row(*parent, schema(*input), r);
                    vec![r]
                }
            } else {
                vec![]
            }
        }
        C::Unit { context } => {
            if rows(*context).is_empty() {
                vec![]
            } else {
                vec![vec![]]
            }
        }
        C::GraphSeed {
            context,
            credential,
            name,
            selector,
        } => {
            let graph = names
                .get(*name as usize)
                .copied()
                .unwrap_or(crate::credential_graphs::name(*name)?);
            let selected = match selector {
                Slot::Parameter(p) => params[*p as usize].expect("graph selector"),
                Slot::Variable(v) => rows(*context)
                    .first()
                    .map(|r| lookup(schema(*context), r, *v))
                    .unwrap_or_default(),
            };
            if (*credential as usize) < credentials.len()
                && (!selected.bound || selected.same_term(graph))
            {
                rows(*context).clone()
            } else {
                vec![]
            }
        }
        C::Graph {
            input,
            context,
            name,
            selector,
        } => {
            let graph = names
                .get(*name as usize)
                .copied()
                .unwrap_or(crate::credential_graphs::name(*name)?);
            let mut output = vec![];
            if let Some(c) = rows(*context).first() {
                for row in rows(*input) {
                    let captured = match selector {
                        Slot::Variable(v) => lookup(schema(*context), c, *v).bound,
                        _ => false,
                    };
                    let mut result = vars
                        .iter()
                        .map(|v| lookup(schema(*input), row, *v))
                        .collect::<Row>();
                    let mut compatible = true;
                    if let Slot::Variable(v) = selector {
                        let col = vars.iter().position(|x| x == v).expect("graph variable");
                        compatible = !result[col].bound || result[col].same_term(graph);
                        if !captured {
                            result[col] = graph;
                        }
                    }
                    if compatible {
                        output.push(result);
                    }
                }
            }
            output
        }
        C::Scan {
            pattern,
            context,
            credential,
        } => {
            if rows(*context).is_empty() {
                vec![]
            } else {
                crate::credential_graphs::scan(
                    pattern,
                    vars,
                    rows(*context).first().map(Vec::as_slice),
                    schema(*context),
                    (*credential, crate::credential_graphs::is_profile(plan)),
                    credentials,
                    params,
                    &names,
                )?
            }
        }
        C::BindBoolean {
            input,
            variable,
            predicate,
            context,
        } => {
            let mut output = vec![];
            for (i, r) in rows(*input).iter().enumerate() {
                let (eval_vars, eval_row) = context_row(*context, schema(*input), r);
                let (value, error) = predicate_value(
                    predicate,
                    i,
                    &eval_vars,
                    &eval_row,
                    params,
                    tables,
                    &plan.parameters,
                )?;
                let bound = if error {
                    Cell::default()
                } else {
                    crate::credential::SuiteEncoder.encode(&oxrdf::Literal::from(value).into())?
                };
                output.push(
                    vars.iter()
                        .map(|v| {
                            if v == variable {
                                bound
                            } else {
                                lookup(schema(*input), r, *v)
                            }
                        })
                        .collect(),
                );
            }
            output
        }
        C::Filter {
            input,
            predicate,
            context,
        } => {
            let mut output = vec![];
            for (i, r) in rows(*input).iter().enumerate() {
                let (vars, row) = context_row(*context, schema(*input), r);
                let (v, e) =
                    predicate_value(predicate, i, &vars, &row, params, tables, &plan.parameters)?;
                if v && !e {
                    output.push(r.clone());
                }
            }
            output
        }
    })
}
fn predicate_value(
    p: &Predicate,
    index: usize,
    vars: &[Var],
    row: &[Cell],
    params: &[Option<Cell>],
    tables: &[Table],
    public: &[Parameter],
) -> Result<(bool, bool), Error> {
    Ok(match p {
        Predicate::Arithmetic(e) => e.evaluate(vars, row, params)?,
        Predicate::Expression(e) => eval(e, vars, row, params, public)?.ebv()?,
        Predicate::Exists(ids) => (!tables[ids[index] as usize].rows.is_empty(), false),
        Predicate::Not(a) => {
            let (v, e) = predicate_value(a, index, vars, row, params, tables, public)?;
            (!v, e)
        }
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            let (a, ae) = predicate_value(a, index, vars, row, params, tables, public)?;
            let (b, be) = predicate_value(b, index, vars, row, params, tables, public)?;
            if matches!(p, Predicate::And(..)) {
                (a && b, (ae || be) && !(!a && !ae || !b && !be))
            } else {
                (a || b, (ae || be) && !(a && !ae || b && !be))
            }
        }
    })
}

fn complete_credentials(dataset: &Dataset) -> Result<Vec<CredentialCells>, Error> {
    let mut credentials = vec![CredentialCells { quads: vec![] }; dataset.counts.len()];
    let named = dataset.quads.iter().any(|q| q[3].bound);
    for q in &dataset.quads {
        if q[3].bound {
            if q[3].kind != 2 || q[3].scope == 0 || q[3].scope as usize > credentials.len() {
                return Err(Error::Invalid("credential graph identity".into()));
            }
            credentials[q[3].scope as usize - 1]
                .quads
                .push([q[0], q[1], q[2], Cell::default()]);
        } else if !named && !credentials.is_empty() {
            credentials[0].quads.push(*q);
        }
    }
    Ok(credentials)
}
