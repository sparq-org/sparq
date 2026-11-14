//! A basic GRAPH body scans all credential graphs without duplicating its plan.
use super::Builder;
use crate::{plan::*, plan_closed::ClosedOperator as C, Error};
use spargebra::algebra::GraphPattern as G;
fn has_scan(g: &G) -> bool {
    match g {
        G::Bgp { patterns } => !patterns.is_empty(),
        G::Join { left, right } => has_scan(left) || has_scan(right),
        G::Filter { inner, .. } => has_scan(inner),
        _ => false,
    }
}
fn allowed(g: &G) -> bool {
    match g {
        G::Bgp { patterns } => !patterns.is_empty(),
        G::Values { .. } => true,
        G::Filter { inner, expr } => {
            !super::closed::has_exists(expr) && has_scan(inner) && allowed(inner)
        }
        G::Join { left, right } => allowed(left) && allowed(right),
        _ => false,
    }
}
pub(super) fn supported(g: &G) -> bool {
    has_scan(g) && allowed(g)
}
impl Builder {
    pub(super) fn closed_graph_body(
        &mut self,
        g: &G,
        context: Option<NodeId>,
        selector: &Slot,
    ) -> Result<NodeId, Error> {
        match g {
            G::Values { .. } => self.closed_in(g, context, None),
            G::Bgp { patterns } if !patterns.is_empty() => {
                let mut input = None;
                for p in patterns {
                    let pattern = [
                        self.slot(&p.subject)?,
                        self.named_slot(&p.predicate),
                        self.slot(&p.object)?,
                    ];
                    let mut vars: Vec<_> = pattern
                        .iter()
                        .chain(std::iter::once(selector))
                        .filter_map(|s| {
                            if let Slot::Variable(v) = s {
                                Some(*v)
                            } else {
                                None
                            }
                        })
                        .collect();
                    vars.sort_unstable();
                    vars.dedup();
                    let scan = self.push(
                        Operator::Closed(C::GraphScan {
                            pattern,
                            selector: selector.clone(),
                            context,
                        }),
                        vars,
                    )?;
                    input = Some(match input {
                        None => scan,
                        Some(a) => self.join(a, scan)?,
                    });
                }
                Ok(input.expect("nonempty GRAPH"))
            }
            G::Filter { inner, expr } => {
                let input = self.closed_graph_body(inner, context, selector)?;
                let predicate = self.closed_predicate(expr, input, context, None)?;
                self.push(
                    Operator::Closed(C::Filter {
                        input,
                        predicate,
                        context,
                    }),
                    self.schema(input),
                )
            }
            G::Join { left, right } => {
                let a = self.closed_graph_body(left, context, selector)?;
                let b = self.closed_graph_body(right, context, selector)?;
                self.join(a, b)
            }
            _ => Err(Error::Unsupported("empty optimized GRAPH body".into())),
        }
    }
}
