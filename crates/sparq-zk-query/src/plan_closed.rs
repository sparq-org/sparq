//! Closed-world algebra extension. Tags in this module are independent of core tags.

use crate::plan::{Expr, NodeId, Slot, Var};

/// Boolean expressions containing correlated existence tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    Expression(Expr),
    Arithmetic(crate::closed_arith::Comparison),
    Exists(Vec<NodeId>),
    Not(Box<Predicate>),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
}

/// Aggregates read a variable column; expression operands belong to lane D.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupAggregate {
    pub variable: Var,
    pub kind: crate::interp_closed_group::AggregateKind,
    pub operand: Option<Var>,
    pub distinct: bool,
}

/// New closed operators extend this enum without changing core dispatch again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClosedOperator {
    /// Exact bag join with a private inverse map over canonical output rows.
    Join {
        left: NodeId,
        right: NodeId,
    },
    CredentialProfile {
        lexical_capacity: u32,
        literal_capacity: u32,
        literal_types: u8,
        issuers: Vec<(u32, u32)>,
    },
    GraphScan {
        pattern: [Slot; 3],
        selector: Slot,
        context: Option<NodeId>,
    },
    DatasetScan {
        pattern: [Slot; 3],
    },
    Group {
        input: NodeId,
        keys: Vec<Var>,
        aggregates: Vec<GroupAggregate>,
    },
    LeftJoin {
        left: NodeId,
        right: NodeId,
        condition: Option<Expr>,
        context: Option<NodeId>,
    },
    Minus {
        left: NodeId,
        right: NodeId,
    },
    Union {
        left: NodeId,
        right: NodeId,
    },
    Extend {
        input: NodeId,
        variable: Var,
        value: Slot,
        context: Option<NodeId>,
    },
    BindBoolean {
        input: NodeId,
        variable: Var,
        predicate: Predicate,
        context: Option<NodeId>,
    },
    Substitute {
        input: NodeId,
        context: NodeId,
    },
    Filter {
        input: NodeId,
        predicate: Predicate,
        context: Option<NodeId>,
    },
    /// One constrained outer row, overlaid on any enclosing substitution.
    Capture {
        input: NodeId,
        row: u32,
        parent: Option<NodeId>,
    },
    /// A substituted BGP scan. Bound captures constrain terms but do not bind columns.
    Scan {
        pattern: [Slot; 3],
        context: NodeId,
        credential: Option<u32>,
    },
    GraphSeed {
        context: NodeId,
        credential: u32,
        name: u32,
        selector: Slot,
    },
    Graph {
        input: NodeId,
        context: NodeId,
        name: u32,
        selector: Slot,
    },
    Unit {
        context: NodeId,
    },
}

impl ClosedOperator {
    pub fn inputs(&self) -> Vec<NodeId> {
        match self {
            Self::GraphScan { context, .. } => context.iter().copied().collect(),
            Self::CredentialProfile { .. } | Self::DatasetScan { .. } => vec![],
            Self::Group { input, .. } => vec![*input],
            Self::LeftJoin {
                left,
                right,
                context,
                ..
            } => {
                let mut ids = vec![*left, *right];
                ids.extend(context);
                ids
            }
            Self::Join { left, right }
            | Self::Minus { left, right }
            | Self::Union { left, right } => vec![*left, *right],
            Self::Extend { input, context, .. } => {
                let mut ids = vec![*input];
                ids.extend(context);
                ids
            }
            Self::Substitute { input, context } => vec![*input, *context],
            Self::BindBoolean {
                input,
                predicate,
                context,
                ..
            }
            | Self::Filter {
                input,
                predicate,
                context,
            } => {
                let mut ids = vec![*input];
                ids.extend(context);
                predicate.inputs(&mut ids);
                ids
            }
            Self::Capture { input, parent, .. } => {
                let mut ids = vec![*input];
                ids.extend(parent);
                ids
            }
            Self::Scan { context, .. }
            | Self::Unit { context }
            | Self::GraphSeed { context, .. } => vec![*context],
            Self::Graph { input, context, .. } => vec![*input, *context],
        }
    }

    /// A tagged, length-prefixed encoding; constants remain public parameter slots.
    pub(crate) fn canonical_bytes(&self) -> Vec<u8> {
        let mut w = Writer(vec![]);
        match self {
            Self::Join { left, right } => {
                w.byte(16);
                w.id(*left);
                w.id(*right);
            }
            Self::CredentialProfile {
                lexical_capacity,
                literal_capacity,
                literal_types,
                issuers,
            } => {
                w.byte(14);
                w.id(*lexical_capacity);
                w.id(*literal_capacity);
                w.byte(*literal_types);
                w.id(issuers.len() as u32);
                for (a, b) in issuers {
                    w.id(*a);
                    w.id(*b);
                }
            }
            Self::GraphScan {
                pattern,
                selector,
                context,
            } => {
                w.byte(13);
                w.optional(*context);
                w.slot(selector);
                for s in pattern {
                    w.slot(s);
                }
            }
            Self::DatasetScan { pattern } => {
                w.byte(12);
                for s in pattern {
                    w.slot(s);
                }
            }
            Self::Group {
                input,
                keys,
                aggregates,
            } => {
                w.byte(11);
                w.id(*input);
                w.id(keys.len() as u32);
                for key in keys {
                    w.id(*key);
                }
                w.id(aggregates.len() as u32);
                for a in aggregates {
                    w.id(a.variable);
                    w.byte(a.kind as u8);
                    w.optional(a.operand);
                    w.byte(u8::from(a.distinct));
                }
            }
            Self::LeftJoin {
                left,
                right,
                condition,
                context,
            } => {
                w.byte(0);
                w.id(*left);
                w.id(*right);
                w.optional(*context);
                w.byte(u8::from(condition.is_some()));
                if let Some(e) = condition {
                    w.expr(e);
                }
            }
            Self::Minus { left, right } => {
                w.byte(1);
                w.id(*left);
                w.id(*right);
            }
            Self::Union { left, right } => {
                w.byte(2);
                w.id(*left);
                w.id(*right);
            }
            Self::Extend {
                input,
                variable,
                value,
                context,
            } => {
                w.byte(3);
                w.id(*input);
                w.id(*variable);
                w.slot(value);
                w.optional(*context);
            }
            Self::BindBoolean {
                input,
                variable,
                predicate,
                context,
            } => {
                w.byte(15);
                w.id(*input);
                w.id(*variable);
                w.optional(*context);
                w.predicate(predicate);
            }
            Self::Substitute { input, context } => {
                w.byte(10);
                w.id(*input);
                w.id(*context);
            }
            Self::Filter {
                input,
                predicate,
                context,
            } => {
                w.byte(4);
                w.id(*input);
                w.optional(*context);
                w.predicate(predicate);
            }
            Self::Capture { input, row, parent } => {
                w.byte(5);
                w.id(*input);
                w.id(*row);
                w.optional(*parent);
            }
            Self::Scan {
                pattern,
                context,
                credential,
            } => {
                w.byte(6);
                w.id(*context);
                w.optional(*credential);
                for s in pattern {
                    w.slot(s);
                }
            }
            Self::GraphSeed {
                context,
                credential,
                name,
                selector,
            } => {
                w.byte(8);
                w.id(*context);
                w.id(*credential);
                w.id(*name);
                w.slot(selector);
            }
            Self::Graph {
                input,
                context,
                name,
                selector,
            } => {
                w.byte(9);
                w.id(*input);
                w.id(*context);
                w.id(*name);
                w.slot(selector);
            }
            Self::Unit { context } => {
                w.byte(7);
                w.id(*context);
            }
        }
        w.0
    }
}
impl Predicate {
    fn inputs(&self, ids: &mut Vec<NodeId>) {
        match self {
            Self::Expression(_) | Self::Arithmetic(_) => (),
            Self::Exists(rights) => ids.extend(rights),
            Self::Not(a) => a.inputs(ids),
            Self::And(a, b) | Self::Or(a, b) => {
                a.inputs(ids);
                b.inputs(ids);
            }
        }
    }
}
struct Writer(Vec<u8>);
impl Writer {
    fn byte(&mut self, n: u8) {
        self.0.push(n);
    }
    fn id(&mut self, n: u32) {
        self.0.extend(n.to_le_bytes());
    }
    fn optional(&mut self, n: Option<u32>) {
        self.byte(u8::from(n.is_some()));
        if let Some(n) = n {
            self.id(n);
        }
    }
    fn slot(&mut self, s: &Slot) {
        match s {
            Slot::Variable(v) => {
                self.byte(0);
                self.id(*v);
            }
            Slot::Parameter(p) => {
                self.byte(1);
                self.id(*p);
            }
        }
    }
    fn expr(&mut self, e: &Expr) {
        use crate::plan::Compare;
        match e {
            Expr::Term(s) => {
                self.byte(0);
                self.slot(s);
            }
            Expr::Compare(op, a, b) => {
                self.byte(1);
                self.byte(match op {
                    Compare::Equal => 0,
                    Compare::Less => 1,
                    Compare::LessOrEqual => 2,
                    Compare::Greater => 3,
                    Compare::GreaterOrEqual => 4,
                });
                self.expr(a);
                self.expr(b);
            }
            Expr::And(a, b) | Expr::Or(a, b) => {
                self.byte(if matches!(e, Expr::And(..)) { 2 } else { 3 });
                self.expr(a);
                self.expr(b);
            }
            Expr::Not(a) => {
                self.byte(4);
                self.expr(a);
            }
            Expr::InList {
                value,
                start,
                count,
            } => {
                self.byte(6);
                self.expr(value);
                self.id(*start);
                self.id(*count);
            }
            Expr::In(a, bs) => {
                self.byte(5);
                self.expr(a);
                self.id(bs.len() as u32);
                for b in bs {
                    self.expr(b);
                }
            }
        }
    }
    fn predicate(&mut self, p: &Predicate) {
        match p {
            Predicate::Arithmetic(e) => {
                self.byte(5);
                self.0.extend(e.canonical_bytes());
            }
            Predicate::Expression(e) => {
                self.byte(0);
                self.expr(e);
            }
            Predicate::Exists(ids) => {
                self.byte(1);
                self.id(ids.len() as u32);
                for id in ids {
                    self.id(*id);
                }
            }
            Predicate::Not(a) => {
                self.byte(2);
                self.predicate(a);
            }
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                self.byte(if matches!(p, Predicate::And(..)) {
                    3
                } else {
                    4
                });
                self.predicate(a);
                self.predicate(b);
            }
        }
    }
}

/// Apply verifier-selected row limits to a closed graph-profile skeleton.
/// Evaluate through credential_graphs so every intermediate row limit is checked.
/// Every node has one limit; private row counts never choose a capacity. The
/// resulting schema capacities already belong to the canonical plan encoding.
pub fn with_row_bounds(
    mut plan: crate::plan::Plan,
    bounds: crate::capacity::Bounds,
    rows: &[u32],
) -> Result<crate::plan::Plan, crate::Error> {
    use crate::{capacity::Bounds, Error};
    if !crate::credential_graphs::is_profile(&plan) {
        return Err(Error::Unsupported(
            "per-table bounds require the closed graph evaluator".into(),
        ));
    }
    if bounds.credentials != plan.bounds.credentials
        || bounds.quads_per_credential != plan.bounds.quads_per_credential
        || rows.len() != plan.nodes.len()
        || rows.iter().any(|&r| r == 0 || r > bounds.rows_per_table)
    {
        return Err(Error::Capacity("per-table row bounds".into()));
    }
    let capacity = bounds.capacity()?;
    for (node, &r) in plan.nodes.iter_mut().zip(rows) {
        let rounded = Bounds {
            rows_per_table: r,
            ..bounds
        }
        .capacity()?
        .rows_per_table;
        // Exact limits avoid a second private/public table-length channel.
        if r != rounded {
            return Err(Error::Capacity("per-table row bucket".into()));
        }
        node.output.capacity = rounded;
    }
    fn predicate(p: &Predicate, bound: u32, nodes: &[crate::plan::Node]) -> Result<(), Error> {
        match p {
            Predicate::Exists(rights) => {
                if rights.len() != bound as usize
                    || rights
                        .windows(2)
                        .any(|ids| nodes[ids[0] as usize].output != nodes[ids[1] as usize].output)
                {
                    return Err(Error::Capacity(
                        "EXISTS row bucket requires matching capture skeleton".into(),
                    ));
                }
            }
            Predicate::Not(a) => predicate(a, bound, nodes)?,
            Predicate::And(a, b) | Predicate::Or(a, b) => {
                predicate(a, bound, nodes)?;
                predicate(b, bound, nodes)?;
            }
            _ => (),
        }
        Ok(())
    }
    for node in &plan.nodes {
        if let crate::plan::Operator::Closed(
            ClosedOperator::Filter {
                input,
                predicate: p,
                ..
            }
            | ClosedOperator::BindBoolean {
                input,
                predicate: p,
                ..
            },
        ) = &node.operator
        {
            predicate(p, plan.nodes[*input as usize].output.capacity, &plan.nodes)?;
        }
    }
    for i in 0..plan.nodes.len() {
        if let crate::plan::Operator::Join(left, right) = plan.nodes[i].operator {
            if [
                plan.nodes[left as usize].output.capacity,
                plan.nodes[right as usize].output.capacity,
                plan.nodes[i].output.capacity,
            ]
            .into_iter()
            .any(|r| r > 4)
            {
                plan.nodes[i].operator =
                    crate::plan::Operator::Closed(ClosedOperator::Join { left, right });
            }
        }
    }
    plan.bounds = bounds;
    // The shared public result ABI uses this bucket. Other tables carry their
    // own capacities above; the request ceiling remains in plan.bounds.
    plan.capacity = crate::capacity::Capacity {
        rows_per_table: plan.result_schema().capacity,
        ..capacity
    };
    Ok(plan)
}

pub(crate) fn check_row_bound(node: &crate::plan::Node, len: usize) -> Result<(), crate::Error> {
    if len > node.output.capacity as usize {
        return Err(crate::Error::Capacity("per-table operator output".into()));
    }
    Ok(())
}
