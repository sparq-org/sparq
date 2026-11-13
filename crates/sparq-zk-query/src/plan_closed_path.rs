//! Property-path shape and compiled closure depth for the unaudited query method.

use crate::{
    plan::{NodeId, Parameter, ParameterId, Plan, Slot},
    plan_closed::ClosedOperator,
    Error,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Path {
    Link(ParameterId),
    Negated(Vec<ParameterId>),
    Inverse(Box<Path>),
    Sequence(Box<Path>, Box<Path>),
    Alternative(Box<Path>, Box<Path>),
    ZeroOrOne(Box<Path>),
    ZeroOrMore(Box<Path>),
    OneOrMore(Box<Path>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathOperator {
    pub path: Path,
    pub endpoints: [Slot; 2],
    pub context: Option<NodeId>,
    pub credential: Option<u32>,
    pub depth: u32,
    pub depth_parameter: ParameterId,
}

impl PathOperator {
    pub(crate) fn inputs(&self) -> Vec<NodeId> {
        self.context.into_iter().collect()
    }
    pub(crate) fn canonical_bytes(&self) -> Vec<u8> {
        fn id(w: &mut Vec<u8>, n: u32) {
            w.extend(n.to_le_bytes());
        }
        fn path(w: &mut Vec<u8>, p: &Path) {
            match p {
                Path::Link(p) => {
                    w.push(0);
                    id(w, *p);
                }
                Path::Negated(ps) => {
                    w.push(1);
                    id(w, ps.len() as u32);
                    for p in ps {
                        id(w, *p);
                    }
                }
                Path::Inverse(a)
                | Path::ZeroOrOne(a)
                | Path::ZeroOrMore(a)
                | Path::OneOrMore(a) => {
                    w.push(match p {
                        Path::Inverse(_) => 2,
                        Path::ZeroOrOne(_) => 5,
                        Path::ZeroOrMore(_) => 6,
                        _ => 7,
                    });
                    path(w, a);
                }
                Path::Sequence(a, b) | Path::Alternative(a, b) => {
                    w.push(if matches!(p, Path::Sequence(..)) {
                        3
                    } else {
                        4
                    });
                    path(w, a);
                    path(w, b);
                }
            }
        }
        let mut w = vec![];
        path(&mut w, &self.path);
        for s in &self.endpoints {
            match s {
                Slot::Variable(v) => {
                    w.push(0);
                    id(&mut w, *v);
                }
                Slot::Parameter(p) => {
                    w.push(1);
                    id(&mut w, *p);
                }
            }
        }
        for n in [self.context, self.credential] {
            w.push(u8::from(n.is_some()));
            if let Some(n) = n {
                id(&mut w, n);
            }
        }
        id(&mut w, self.depth);
        id(&mut w, self.depth_parameter);
        w
    }
}

impl Plan {
    /// Select a verifier-agreed compiled depth, also carried by public `indices`.
    /// Every closure must stabilize by this depth; otherwise evaluation refuses.
    pub fn with_path_depth(mut self, depth: u32) -> Result<Self, Error> {
        if depth == 0 || depth > self.capacity.rows_per_table {
            return Err(Error::Capacity("compiled path depth".into()));
        }
        for n in &mut self.nodes {
            if let crate::plan::Operator::Closed(ClosedOperator::Path(p)) = &mut n.operator {
                p.depth = depth;
                self.parameters[p.depth_parameter as usize] = Parameter::Index(depth);
            }
        }
        Ok(self)
    }
}
