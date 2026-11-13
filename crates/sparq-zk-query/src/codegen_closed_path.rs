//! Deterministic path wiring; relation constraints stay in the Noir library.

use super::*;
use crate::plan_closed_path::{Path, PathOperator};

pub(super) fn generate(
    p: &PathOperator,
    i: usize,
    plan: &Plan,
    ids: &[u32],
    out: &mut String,
) -> Result<(), Error> {
    if !out.contains("use core::table::unbound;") {
        *out = out.replacen(
            "use core::table::{TermCell, Table};",
            "use core::table::{TermCell, Table, unbound};",
            1,
        );
    }
    if !out.contains("use core::ops_core_expr;") {
        *out = out.replacen(
            "use core::{ops_core, ops_core_statement};",
            "use core::{ops_core, ops_core_statement, ops_core_expr};",
            1,
        );
    }
    let dataset = if plan
        .nodes
        .iter()
        .any(|n| matches!(n.operator, Operator::Scan(_)))
    {
        "dataset"
    } else {
        "_dataset"
    };
    let prefix = format!("path_{i}");
    let vars = &plan.nodes[i].output.variables;
    let endpoint = |s: &Slot| match s {
        Slot::Parameter(p) => super::term(*p, ids),
        Slot::Variable(v) => match p.context {
            Some(id) => {
                let sv = &plan.nodes[id as usize].output.variables;
                let col = sv.iter().position(|x| x == v).unwrap_or(sv.len().max(1));
                format!("core::ops_closed_path::endpoint(t{id}, {col}, core::table::unbound())")
            }
            None => "core::table::unbound()".into(),
        },
    };
    writeln!(
        out,
        "    let {prefix}_anchors = {};",
        array(p.endpoints.iter().map(endpoint))
    )
    .unwrap();
    writeln!(out,"    let {prefix}_active = core::ops_closed_path::graph_active({dataset}.active, {}, {}, {}, {});", array(plan.credential_quad_capacities().iter().map(u32::to_string)),plan.capacity.quads_per_credential,p.credential.is_some(),p.credential.unwrap_or(0)).unwrap();
    let index = plan
        .parameters
        .iter()
        .take(p.depth_parameter as usize)
        .filter(|p| matches!(p, Parameter::Index(_)))
        .count();
    // Even paths without repetition carry and constrain their compiled depth.
    writeln!(
        out,
        "    core::ops_closed_path::check_depth::<{}>(indices[{index}]);",
        p.depth
    )
    .unwrap();
    let mut wiring = Wiring {
        out,
        prefix: &prefix,
        dataset,
        rows: plan.capacity.rows_per_table,
        depth: p.depth,
        index,
        ids,
        next: 0,
    };
    let relation = wiring.path(&p.path, &format!("{prefix}_anchors"), true);
    let columns = array(p.endpoints.iter().map(|s| {
        match s {
            Slot::Variable(v) => vars
                .iter()
                .position(|x| x == v)
                .expect("path column")
                .to_string(),
            _ => vars.len().max(1).to_string(),
        }
    }));
    let captured = array(p.endpoints.iter().enumerate().map(|(j, s)| {
        if matches!(s, Slot::Variable(_)) && p.context.is_some() {
            format!("{prefix}_anchors[{j}].bound")
        } else {
            "false".into()
        }
    }));
    let enabled = p
        .context
        .map(|id| format!("core::ops_closed_path::enabled(t{id})"))
        .unwrap_or_else(|| "true".into());
    writeln!(out,"    core::ops_closed_path::bind({relation}, {prefix}_anchors, {columns}, {captured}, {enabled}, t{i});").unwrap();
    Ok(())
}
struct Wiring<'a> {
    out: &'a mut String,
    prefix: &'a str,
    dataset: &'a str,
    rows: u32,
    depth: u32,
    index: usize,
    ids: &'a [u32],
    next: usize,
}
impl Wiring<'_> {
    fn emit(&mut self, expression: String) -> String {
        let name = format!("{}_r{}", self.prefix, self.next);
        self.next += 1;
        writeln!(
            self.out,
            "    let {name}: Table<{}, 2> = {expression};",
            self.rows
        )
        .unwrap();
        name
    }
    fn anchors(&mut self, expression: String) -> String {
        let name = format!("{}_a{}", self.prefix, self.next);
        self.next += 1;
        writeln!(self.out, "    let {name} = {expression};").unwrap();
        name
    }
    fn path(&mut self, p: &Path, anchors: &str, restrict: bool) -> String {
        let module = "core::ops_closed_path";
        let result = match p {
            Path::Link(p) => self.emit(format!(
                "{module}::edges({}.rows, {}_active, [{}], 1, false)",
                self.dataset,
                self.prefix,
                super::term(*p, self.ids)
            )),
            Path::Negated(ps) => self.emit(format!(
                "{module}::edges({}.rows, {}_active, {}, {}, true)",
                self.dataset,
                self.prefix,
                if ps.is_empty() {
                    "[core::table::unbound()]".into()
                } else {
                    array(ps.iter().map(|p| super::term(*p, self.ids)))
                },
                ps.len()
            )),
            Path::Inverse(a) => {
                let swapped = self.anchors(format!("[{anchors}[1], {anchors}[0]]"));
                let a = self.path(a, &swapped, restrict);
                self.emit(format!("{module}::inverse({a})"))
            }
            Path::Sequence(a, b) | Path::Alternative(a, b) => {
                let (left, right) = if matches!(p, Path::Sequence(..)) {
                    (
                        self.anchors(format!("[{anchors}[0], core::table::unbound()]")),
                        self.anchors(format!("[core::table::unbound(), {anchors}[1]]")),
                    )
                } else {
                    (anchors.to_owned(), anchors.to_owned())
                };
                let a = self.path(a, &left, restrict);
                let b = self.path(b, &right, restrict);
                self.emit(if matches!(p, Path::Sequence(..)) {
                    format!("{module}::sequence({a}, {b})")
                } else {
                    format!("{module}::alternative({a}, {b}, false)")
                })
            }
            Path::ZeroOrOne(a) | Path::ZeroOrMore(a) | Path::OneOrMore(a) => {
                let a = self.path(a, anchors, false);
                let zero = if matches!(p, Path::OneOrMore(_)) {
                    format!("core::table::empty::<{}, 2>()", self.rows)
                } else {
                    self.emit(format!(
                        "{module}::identity({}.rows, {}_active, {anchors})",
                        self.dataset, self.prefix
                    ))
                };
                if matches!(p, Path::ZeroOrOne(_)) {
                    self.emit(format!("{module}::optional({a}, {zero})"))
                } else {
                    self.emit(format!(
                        "{module}::closure::<{}, {}>({a}, {zero}, {}, indices[{}])",
                        self.depth,
                        self.rows,
                        matches!(p, Path::ZeroOrMore(_)),
                        self.index
                    ))
                }
            }
        };
        if restrict {
            self.emit(format!("{module}::restrict({result}, {anchors})"))
        } else {
            result
        }
    }
}
