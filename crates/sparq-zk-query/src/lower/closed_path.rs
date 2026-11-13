//! Path admission, including reconstruction of parser-expanded anonymous sequences.

use super::Builder;
use crate::{
    plan::*,
    plan_closed::ClosedOperator,
    plan_closed_path::{Path, PathOperator},
    Error,
};
use spargebra::{
    algebra::{GraphPattern, PropertyPathExpression as P},
    term::{NamedNodePattern, TermPattern},
};

type Pattern = (TermPattern, P, TermPattern);

// The parser expands top-level sequences into paths/triples with fresh blank
// endpoints. Reassemble only a chain whose internal endpoints are anonymous.
fn composed(g: &GraphPattern) -> Option<Pattern> {
    match g {
        GraphPattern::Path {
            subject,
            path,
            object,
        } => Some((subject.clone(), path.clone(), object.clone())),
        GraphPattern::Bgp { patterns } if !patterns.is_empty() => {
            let mut parts = vec![];
            for t in patterns {
                let NamedNodePattern::NamedNode(p) = &t.predicate else {
                    return None;
                };
                parts.push((t.subject.clone(), P::NamedNode(p.clone()), t.object.clone()));
            }
            chain(parts)
        }
        GraphPattern::Join { left, right } => chain(vec![composed(left)?, composed(right)?]),
        _ => None,
    }
}
fn chain(mut parts: Vec<Pattern>) -> Option<Pattern> {
    let (s, mut p, mut o) = parts.remove(0);
    for (next, q, end) in parts {
        if !matches!(o, TermPattern::BlankNode(_)) || o != next {
            return None;
        }
        p = P::Sequence(Box::new(p), Box::new(q));
        o = end;
    }
    Some((s, p, o))
}
pub(super) fn handles(g: &GraphPattern) -> bool {
    matches!(g, GraphPattern::Path { .. })
        || matches!(g,GraphPattern::Bgp{patterns} if patterns.iter().any(|p| matches!(p.subject,TermPattern::BlankNode(_)) || matches!(p.object,TermPattern::BlankNode(_))))
        || matches!(
            g,
            GraphPattern::Join { .. } | GraphPattern::Bgp { patterns: _ }
        ) && composed(g).is_some_and(|(_, p, _)| matches!(p, P::Sequence(..)))
}
impl Builder {
    pub(super) fn closed_path(
        &mut self,
        g: &GraphPattern,
        context: Option<NodeId>,
        credential: Option<u32>,
    ) -> Result<NodeId, Error> {
        let Some((s, p, o)) = composed(g) else {
            let GraphPattern::Bgp { patterns } = g else {
                return Err(Error::Unsupported("property path sequence shape".into()));
            };
            let mut input = None;
            for p in patterns {
                let slots = [
                    self.path_slot(&p.subject)?,
                    self.named_slot(&p.predicate),
                    self.path_slot(&p.object)?,
                ];
                let mut vars: Vec<_> = slots
                    .iter()
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
                let op = if let Some(context) = context {
                    Operator::Closed(ClosedOperator::Scan {
                        pattern: slots,
                        context,
                        credential,
                    })
                } else {
                    Operator::Scan(slots)
                };
                let scan = self.push(op, vars)?;
                input = Some(if let Some(a) = input {
                    self.join(a, scan)?
                } else {
                    scan
                });
            }
            return Ok(input.expect("nonempty anonymous BGP"));
        };
        let endpoints = [self.path_slot(&s)?, self.path_slot(&o)?];
        let path = self.path_expression(&p, 0)?;
        let depth = 2.min(self.plan.capacity.rows_per_table);
        let depth_parameter = self.parameter(Parameter::Index(depth));
        let mut vars: Vec<_> = endpoints
            .iter()
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
        self.push(
            Operator::Closed(ClosedOperator::Path(PathOperator {
                path,
                endpoints,
                context,
                credential,
                depth,
                depth_parameter,
            })),
            vars,
        )
    }
    fn path_slot(&mut self, p: &TermPattern) -> Result<Slot, Error> {
        if let TermPattern::BlankNode(b) = p {
            // The prefix cannot be a SPARQL variable; parser labels stay out of the shape.
            Ok(Slot::Variable(
                self.variable(&format!("@path-blank:{}", b.as_str())),
            ))
        } else {
            self.slot(p)
        }
    }
    fn path_expression(&mut self, p: &P, nesting: u32) -> Result<Path, Error> {
        if nesting >= 32 {
            return Err(Error::Capacity("path expression nesting".into()));
        }
        let mut lower = |p: &P| self.path_expression(p, nesting + 1).map(Box::new);
        Ok(match p {
            P::NamedNode(n) => Path::Link(self.parameter(Parameter::Term(n.clone().into()))),
            P::NegatedPropertySet(ns) => Path::Negated(
                ns.iter()
                    .map(|n| self.parameter(Parameter::Term(n.clone().into())))
                    .collect(),
            ),
            P::Reverse(a) => Path::Inverse(lower(a)?),
            P::Sequence(a, b) => Path::Sequence(lower(a)?, lower(b)?),
            P::Alternative(a, b) => Path::Alternative(lower(a)?, lower(b)?),
            P::ZeroOrOne(a) => Path::ZeroOrOne(lower(a)?),
            P::ZeroOrMore(a) => Path::ZeroOrMore(lower(a)?),
            P::OneOrMore(a) => Path::OneOrMore(lower(a)?),
        })
    }
}
