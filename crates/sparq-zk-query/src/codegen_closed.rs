//! Wiring for closed relations. All constraints live in library gadgets.
use super::*;
use crate::plan_closed::{ClosedOperator as C, Predicate};

fn columns(from: &[Var], to: &[Var]) -> String {
    if to.is_empty() {
        return array([from.len().max(1).to_string()]);
    }
    array(to.iter().map(|v| {
        from.iter()
            .position(|x| x == v)
            .unwrap_or(from.len().max(1))
            .to_string()
    }))
}
fn term(p: u32, ids: &[u32]) -> String {
    format!(
        "parameters[{}]",
        ids.iter().position(|x| *x == p).expect("term parameter")
    )
}
fn steps(e: &Expr, vars: &[Var], ids: &[u32], plan: &Plan) -> String {
    let mut out = vec![];
    let indices = plan
        .parameters
        .iter()
        .enumerate()
        .filter_map(|(i, p)| matches!(p, Parameter::Index(_)).then_some(i as u32))
        .collect::<Vec<_>>();
    expression(e, vars, ids, &indices, &mut out);
    array(out.iter().map(Step::source))
}
pub(super) fn generate(
    op: &C,
    i: usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) -> Result<(), Error> {
    if !out.contains("use core::ops_core_expr;") {
        *out = out.replacen("\nfn main(", "\nuse core::ops_core_expr;\nfn main(", 1);
    }
    emit_reads(i, plan, out);
    let schema = |id: NodeId| plan.nodes[id as usize].output.variables.as_slice();
    let vars = &plan.nodes[i].output.variables;
    let cols = |id: NodeId| columns(schema(id), vars);
    match op {
        C::Join { left, right } => {
            let declaration = format!(
                "    closed_pairs_{i}: [[u32; 2]; {}],\n",
                plan.nodes[i].output.capacity
            );
            *out = out.replacen(") {", &format!("{declaration}) {{"), 1);
            writeln!(out,"    core::ops_closed_join::join(t{left}, t{right}, {}, {}, closed_pairs_{i}, t{i});",cols(*left),cols(*right)).unwrap();
        }
        C::CredentialProfile {
            lexical_capacity,
            literal_capacity,
            literal_types,
            issuers,
        } => {
            if *lexical_capacity != 64
                || !matches!(
                    (*literal_types, *literal_capacity),
                    (1, 16) | (6, 16) | (7, 64) | (31, 64)
                )
            {
                return Err(Error::Unsupported(
                    "closed credential literal profile".into(),
                ));
            }
            if out.contains("credential_0:") {
                if issuers.is_empty() {
                    return Err(Error::Unsupported(
                        "credential graph issuer registry (lane E)".into(),
                    ));
                }
                let registry = array(
                    issuers
                        .iter()
                        .map(|(a, b)| format!("[{},{}]", term(*a, ids), term(*b, ids))),
                );
                for c in 0..plan.capacity().credentials {
                    writeln!(out,"    core::ops_closed_opening::profile_literal_capacity(credential_{c}, counts[{c}], {literal_capacity});").unwrap();
                }
                let capacities = plan.credential_quad_capacities();
                let mut offset = 0;
                for (c, n) in capacities.iter().enumerate() {
                    writeln!(out,"    core::ops_closed_issuer_standin::issuer_identity(dataset_{}, {offset}, {n}, credential_proof_{c}.pk, credential_flags[{c}], {registry});",capacities.len()-1).unwrap();
                    offset += n;
                }
                let messages =
                    array((0..plan.capacity().credentials).map(|c| format!("opened_{c}.message")));
                writeln!(out,"    core::ops_closed_dataset::canonical_credentials({messages}, credential_count);").unwrap();
            }
            let source = if out.contains("credential_0:") {
                format!("dataset_{}", plan.credential_quad_capacities().len() - 1)
            } else if out.contains("let dataset =") {
                "dataset".into()
            } else {
                "_dataset".into()
            };
            writeln!(
                out,
                "    core::ops_closed_opening::literal_types({source}, {literal_types});"
            )
            .unwrap();
            writeln!(out, "    ops_core::unit(t{i});").unwrap();
            *out = out.replace(
                "ops_core_expr::evaluate_indexed(",
                "core::ops_closed_expr::evaluate_indexed(",
            );
        }
        C::GraphScan {
            pattern,
            selector,
            context,
        } => graph_scan_source(pattern, selector, *context, i, plan, ids, out),
        C::DatasetScan { pattern } => scan_source(pattern, None, None, i, plan, ids, out),
        C::Group {
            input,
            keys,
            aggregates,
        } => group_source(*input, keys, aggregates, i, plan, out),
        C::Union { left, right } => writeln!(
            out,
            "    core::ops_closed_union::union(t{left}, t{right}, {}, {}, t{i});",
            cols(*left),
            cols(*right)
        )
        .unwrap(),
        C::Minus { left, right } => writeln!(
            out,
            "    core::ops_closed_negation::minus(t{left}, t{right}, {}, t{i});",
            columns(schema(*right), schema(*left))
        )
        .unwrap(),
        C::Extend {
            input,
            variable,
            value,
            context,
        } => {
            let target = vars
                .iter()
                .position(|v| v == variable)
                .expect("BIND target");
            let (src, c, is_constant) = match value {
                Slot::Variable(v) => (
                    schema(*input)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*input).len().max(1)),
                    "unbound()".into(),
                    false,
                ),
                Slot::Parameter(p) => (0, term(*p, ids), true),
            };
            let (ctx, capture) = match context {
                Some(id) => (
                    format!("t{id}"),
                    match value {
                        Slot::Variable(v) => schema(*id)
                            .iter()
                            .position(|x| x == v)
                            .unwrap_or(schema(*id).len().max(1)),
                        _ => schema(*id).len().max(1),
                    },
                ),
                None => ("core::table::empty::<1,1>()".into(), 1),
            };
            writeln!(out,"    core::ops_closed_extend::extend_context(t{input}, {}, {target}, {src}, {c}, {is_constant}, {ctx}, {capture}, t{i});",cols(*input)).unwrap();
        }
        C::Substitute { input, context } => writeln!(
            out,
            "    core::ops_closed_substitute::values(t{input}, t{context}, {}, t{i});",
            columns(schema(*context), schema(*input))
        )
        .unwrap(),
        C::Capture { input, row, parent } => {
            let (ctx, pc, has) = match parent {
                Some(p) => (format!("t{p}"), cols(*p), true),
                None => (
                    "core::table::empty::<1, 1>()".into(),
                    array(std::iter::repeat_n("1".into(), vars.len().max(1))),
                    false,
                ),
            };
            writeln!(out,"    core::ops_closed_substitute::capture(t{input}, {row}, {ctx}, {has}, {}, {pc}, t{i});",cols(*input)).unwrap();
        }
        C::Unit { context } => writeln!(
            out,
            "    core::ops_closed_substitute::unit(t{context}, t{i});"
        )
        .unwrap(),
        C::GraphSeed {
            context,
            credential,
            name,
            selector,
        } => {
            let (column, constant) = match selector {
                Slot::Variable(v) => (
                    schema(*context)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*context).len().max(1)),
                    "unbound()".into(),
                ),
                Slot::Parameter(p) => (schema(*context).len().max(1), term(*p, ids)),
            };
            writeln!(out,"    core::ops_closed_graph::seed(t{context}, credential_count, {credential}, {}, {column}, {constant}, t{i});",format_args!("core::ops_closed_dataset::graph_name_request({name}, request_digest)")).unwrap();
        }
        C::Graph {
            input,
            context,
            name,
            selector,
        } => {
            let (target, capture) = match selector {
                Slot::Variable(v) => (
                    vars.iter().position(|x| x == v).expect("graph target"),
                    schema(*context)
                        .iter()
                        .position(|x| x == v)
                        .unwrap_or(schema(*context).len().max(1)),
                ),
                _ => (vars.len().max(1), schema(*context).len().max(1)),
            };
            writeln!(out,"    core::ops_closed_graph::graph(t{input}, t{context}, {}, {target}, {capture}, {}, t{i});",cols(*input),format_args!("core::ops_closed_dataset::graph_name_request({name}, request_digest)")).unwrap();
        }
        C::Scan {
            pattern,
            context,
            credential,
        } => {
            scan_source(pattern, Some(*context), *credential, i, plan, ids, out);
        }
        C::LeftJoin {
            left,
            right,
            condition,
            context,
        } => {
            let mut ev = vars.clone();
            if let Some(c) = context {
                ev.extend(schema(*c));
                ev.sort_unstable();
                ev.dedup();
            }
            let (ctx, cc) = match context {
                Some(c) => (format!("t{c}"), columns(schema(*c), &ev)),
                None => (
                    "core::table::empty::<1, 1>()".into(),
                    array(std::iter::repeat_n("1".into(), ev.len().max(1))),
                ),
            };
            let program = condition
                .as_ref()
                .map(|e| steps(e, &ev, ids, plan))
                .unwrap_or_else(|| {
                    array([Step {
                        op: 7,
                        ..Step::default()
                    }
                    .source()])
                });
            writeln!(out,"    core::ops_closed_negation::left_join_indexed(t{left}, t{right}, {}, {}, {ctx}, {}, {cc}, parameters, indices, {program}, {}, t{i});",cols(*left),cols(*right),columns(vars,&ev),condition.is_some()).unwrap();
        }
        C::BindBoolean {
            input,
            variable,
            predicate,
            context,
        } => {
            let value = predicate_source(predicate, *input, *context, i, &mut 0, plan, ids, out);
            let target = vars
                .iter()
                .position(|v| v == variable)
                .expect("BIND column");
            writeln!(
                out,
                "    core::ops_closed_extend::boolean(t{input}, {}, {target}, {value}, t{i});",
                cols(*input)
            )
            .unwrap();
        }
        C::Filter {
            input,
            predicate,
            context,
        } => {
            let mut next = 0;
            let value = predicate_source(predicate, *input, *context, i, &mut next, plan, ids, out);
            writeln!(out, "    ops_core::filter(t{input}, {value}, t{i});").unwrap();
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn predicate_source(
    p: &Predicate,
    input: NodeId,
    context: Option<NodeId>,
    node: usize,
    next: &mut usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) -> String {
    let name = format!("closed_predicate_{node}_{}", *next);
    *next += 1;
    match p {
        Predicate::Exists(rights) => writeln!(
            out,
            "    let {name} = core::ops_closed_negation::existence({});",
            array(rights.iter().map(|id| format!("t{id}")))
        )
        .unwrap(),
        Predicate::Arithmetic(e) => {
            let input_vars = &plan.nodes[input as usize].output.variables;
            let mut vars = input_vars.clone();
            let source = if let Some(c) = context {
                let cv = &plan.nodes[c as usize].output.variables;
                vars.extend(cv);
                vars.sort_unstable();
                vars.dedup();
                writeln!(out,"    let overlay_{name} = core::ops_closed_common::overlay(t{input}, t{c}, {}, {});",columns(input_vars,&vars),columns(cv,&vars)).unwrap();
                format!("overlay_{name}")
            } else {
                format!("t{input}")
            };
            let (program, left, right) = arithmetic_steps(e, &vars, ids);
            writeln!(out,"    let {name} = core::ops_closed_arith::evaluate({source}, parameters, {program}, {}, {left}, {right});",e.op as u8).unwrap();
        }
        Predicate::Expression(e) => {
            let input_vars = &plan.nodes[input as usize].output.variables;
            let mut vars = input_vars.clone();
            let source = if let Some(c) = context {
                let cv = &plan.nodes[c as usize].output.variables;
                vars.extend(cv);
                vars.sort_unstable();
                vars.dedup();
                writeln!(out,"    let overlay_{name} = core::ops_closed_common::overlay(t{input}, t{c}, {}, {});",columns(input_vars,&vars),columns(cv,&vars)).unwrap();
                format!("overlay_{name}")
            } else {
                format!("t{input}")
            };
            writeln!(
                out,
                "    let {name} = core::ops_closed_expr::evaluate_indexed({source}, parameters, indices, {});",
                steps(e, &vars, ids, plan)
            )
            .unwrap();
        }
        Predicate::Not(a) => {
            let a = predicate_source(a, input, context, node, next, plan, ids, out);
            writeln!(
                out,
                "    let {name} = core::ops_closed_common::combine({a}, {a}, 0);"
            )
            .unwrap();
        }
        Predicate::And(a, b) | Predicate::Or(a, b) => {
            let a = predicate_source(a, input, context, node, next, plan, ids, out);
            let b = predicate_source(b, input, context, node, next, plan, ids, out);
            writeln!(
                out,
                "    let {name} = core::ops_closed_common::combine({a}, {b}, {});",
                if matches!(p, Predicate::And(..)) {
                    1
                } else {
                    2
                }
            )
            .unwrap();
        }
    }
    name
}

fn group_source(
    input: NodeId,
    keys: &[Var],
    aggregates: &[crate::plan_closed::GroupAggregate],
    i: usize,
    plan: &Plan,
    out: &mut String,
) {
    let vars = &plan.nodes[input as usize].output.variables;
    let column = |v: Var| {
        vars.iter()
            .position(|x| *x == v)
            .unwrap_or(vars.len().max(1))
    };
    let columns = array(keys.iter().map(|v| column(*v).to_string()));
    let specs = array(aggregates.iter().map(|a| {
        format!(
            "core::ops_closed_group::Aggregate {{ kind: {}, column: {}, distinct: {} }}",
            a.kind as u8,
            a.operand.map(column).unwrap_or(vars.len().max(1)),
            a.distinct
        )
    }));
    writeln!(
        out,
        "    core::ops_closed_group_plan::group(t{input}, t{i}, {columns}, {specs});"
    )
    .unwrap();
}

fn scan_source(
    pattern: &[Slot; 3],
    context: Option<NodeId>,
    credential: Option<u32>,
    i: usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) {
    let vars = &plan.nodes[i].output.variables;
    let ctxvars = context
        .map(|c| plan.nodes[c as usize].output.variables.as_slice())
        .unwrap_or(&[]);
    let mapping = array(pattern.iter().map(|s| {
        match s {
            Slot::Variable(v) => vars
                .iter()
                .position(|a| a == v)
                .expect("scan column")
                .to_string(),
            _ => vars.len().max(1).to_string(),
        }
    }));
    let captures = array(pattern.iter().map(|s| {
        match s {
            Slot::Variable(v) => ctxvars
                .iter()
                .position(|a| a == v)
                .unwrap_or(ctxvars.len().max(1))
                .to_string(),
            _ => ctxvars.len().max(1).to_string(),
        }
    }));
    let constants = array(pattern.iter().map(|s| match s {
        Slot::Parameter(p) => term(*p, ids),
        _ => "core::table::unbound()".into(),
    }));
    let authenticated = out.contains("credential_0:");
    let capacities = if authenticated {
        plan.credential_quad_capacities()
    } else {
        vec![plan.capacity().quads_per_credential; plan.capacity().credentials as usize]
    };
    let total: u32 = capacities.iter().sum();
    let source = if authenticated {
        format!("dataset_{}", capacities.len() - 1)
    } else if out.contains("let dataset =") {
        "dataset".into()
    } else {
        "_dataset".into()
    };
    let mut offset = 0;
    let offsets = array(capacities.iter().map(|n| {
        let o = offset;
        offset += n;
        o.to_string()
    }));
    let lens = array(capacities.iter().map(u32::to_string));
    let selected = if let Some(c) = credential {
        format!(
            "core::ops_closed_dataset::named({source}, {}, {})",
            capacities.iter().take(c as usize).sum::<u32>(),
            capacities[c as usize]
        )
    } else if crate::credential_graphs::is_profile(plan) {
        ensure_graphs(plan, out);
        if !out.contains("let closed_default_dataset =") {
            writeln!(out,"    let closed_default_dataset = core::ops_closed_dataset::default_graph::<{total}, {}, {}>(closed_named_dataset, {offsets}, {lens}, credential_count);",capacities.len(),total+capacities.len() as u32).unwrap();
        }
        "closed_default_dataset".into()
    } else {
        source
    };
    let ctx = context
        .map(|c| format!("t{c}"))
        .unwrap_or_else(|| "core::ops_closed_dataset::unit_context()".into());
    writeln!(out,"    core::ops_closed_dataset::scan({selected}, {ctx}, {mapping}, {captures}, {constants}, t{i});").unwrap();
}

fn arithmetic_steps(
    e: &crate::closed_arith::Comparison,
    schema: &[Var],
    ids: &[u32],
) -> (String, usize, usize) {
    fn emit(
        e: &crate::closed_arith::Integer,
        schema: &[Var],
        ids: &[u32],
        steps: &mut Vec<String>,
    ) -> usize {
        use crate::closed_arith::Integer as I;
        let (op, a, b, parameter) = match e {
            I::Term(Slot::Variable(v)) => (
                0,
                0,
                0,
                schema
                    .iter()
                    .position(|x| x == v)
                    .unwrap_or(schema.len().max(1)),
            ),
            I::Term(Slot::Parameter(p)) => {
                (1, 0, 0, ids.iter().position(|x| x == p).expect("term"))
            }
            I::Add(a, b) | I::Subtract(a, b) | I::Multiply(a, b) => {
                let av = emit(a, schema, ids, steps);
                let bv = emit(b, schema, ids, steps);
                (
                    match e {
                        I::Add(..) => 2,
                        I::Subtract(..) => 3,
                        _ => 4,
                    },
                    av,
                    bv,
                    0,
                )
            }
        };
        let i = steps.len();
        steps.push(format!(
            "core::ops_closed_arith::Step {{op:{op},a:{a},b:{b},parameter:{parameter}}}"
        ));
        i
    }
    let mut steps = vec![];
    let a = emit(&e.left, schema, ids, &mut steps);
    let b = emit(&e.right, schema, ids, &mut steps);
    (array(steps), a, b)
}

fn graph_scan_source(
    pattern: &[Slot; 3],
    selector: &Slot,
    context: Option<NodeId>,
    i: usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) {
    let vars = &plan.nodes[i].output.variables;
    let ctxvars = context
        .map(|id| plan.nodes[id as usize].output.variables.as_slice())
        .unwrap_or(&[]);
    let mapping = array(pattern.iter().chain(std::iter::once(selector)).map(|s| {
        match s {
            Slot::Variable(v) => vars
                .iter()
                .position(|a| a == v)
                .expect("graph scan schema")
                .to_string(),
            _ => vars.len().max(1).to_string(),
        }
    }));
    let captures = array(pattern.iter().chain(std::iter::once(selector)).map(|s| {
        match s {
            Slot::Variable(v) => ctxvars
                .iter()
                .position(|a| a == v)
                .unwrap_or(ctxvars.len().max(1))
                .to_string(),
            _ => ctxvars.len().max(1).to_string(),
        }
    }));
    let constants = array(
        pattern
            .iter()
            .chain(std::iter::once(selector))
            .map(|s| match s {
                Slot::Parameter(p) => term(*p, ids),
                _ => "core::table::unbound()".into(),
            }),
    );
    ensure_graphs(plan, out);
    let ctx = context
        .map(|id| format!("t{id}"))
        .unwrap_or_else(|| "core::ops_closed_dataset::unit_context()".into());
    writeln!(out,"    core::ops_closed_dataset::graph_scan_bound(closed_named_dataset, {ctx}, {mapping}, {captures}, {constants}, t{i});").unwrap();
}

fn ensure_graphs(plan: &Plan, out: &mut String) {
    if out.contains("let closed_named_dataset =") {
        return;
    }
    let auth = out.contains("credential_0:");
    let capacities = if auth {
        plan.credential_quad_capacities()
    } else {
        vec![plan.capacity().quads_per_credential; plan.capacity().credentials as usize]
    };
    let source = if auth {
        format!("dataset_{}", capacities.len() - 1)
    } else if out.contains("let dataset =") {
        "dataset".into()
    } else {
        "_dataset".into()
    };
    let mut offset = 0;
    let offsets = array(capacities.iter().map(|n| {
        let o = offset;
        offset += n;
        o.to_string()
    }));
    let lengths = array(capacities.iter().map(u32::to_string));
    writeln!(out,"    let closed_named_dataset = core::ops_closed_dataset::request_graphs({source}, {offsets}, {lengths}, request_digest);").unwrap();
}

fn emit_reads(i: usize, plan: &Plan, out: &mut String) {
    if !out.contains("credential_0:") {
        return;
    }
    for site in crate::reads_closed::sites(plan, i) {
        let name = site.name(i);
        let lexical = plan.opening_profile().limits().object;
        let declaration = format!(
            "    {name}: [[core::suite_entry::BoundedTermOpening<{lexical}, 41, 1>; {}]; {}],\n",
            site.columns.len(),
            plan.nodes[site.input as usize].output.capacity
        );
        *out = out.replacen(") {", &format!("{declaration}) {{"), 1);
        writeln!(
            out,
            "    core::ops_core_reads::check_with_parameters(t{}, {name}, {}, parameters);",
            site.input,
            array(site.columns.iter().map(usize::to_string))
        )
        .unwrap();
    }
}

/// The shared generator's bound is a per-table limit. Closed profiles can use
/// that public value as a ceiling above independently compiled table buckets.
pub(super) fn finish(plan: &Plan, out: &mut String) {
    if crate::credential_graphs::is_profile(plan) {
        *out = out.replace(
            "ops_core::check_bound(",
            "core::ops_closed_common::check_bound(",
        );
    }
}
