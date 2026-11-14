//! Typed, bounded algebra. The shape encoding excludes parameter values and variable names.

use crate::capacity::{Bounds, Capacity, OpeningProfile};
use oxrdf::Term;

pub type Var = u32;
pub type NodeId = u32;
pub type ParameterId = u32;

/// Constant membership-list capacity; its active length is a public parameter.
pub const IN_CAPACITY: usize = 16;

/// An occurrence of a public term constant, or a variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    Variable(Var),
    Parameter(ParameterId),
}

/// Supported value comparisons; equality follows value semantics, joins use identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compare {
    Equal,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

/// Initial expression contract. Evaluation has an explicit error lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Term(Slot),
    Compare(Compare, Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    In(Box<Expr>, Vec<Expr>),
    InList {
        value: Box<Expr>,
        start: ParameterId,
        count: ParameterId,
    },
}

/// Public parameters are separate from the circuit shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parameter {
    Term(Term),
    Index(u32),
}

/// Every operator produces lexicographically ordered term encodings in schema order.
/// Equal rows retain multiplicity unless the operator is explicitly DISTINCT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalOrder {
    TermEncoding,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schema {
    pub variables: Vec<Var>,
    pub capacity: u32,
    pub order: CanonicalOrder,
}

impl Schema {
    /// Noir entry points use a canonical unbound column for an empty schema.
    pub fn abi_width(&self) -> usize {
        self.variables.len().max(1)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operator {
    Closed(crate::plan_closed::ClosedOperator),
    Unit,
    Scan([Slot; 3]),
    Join(NodeId, NodeId),
    Filter(NodeId, Expr),
    Project(NodeId),
    Distinct(NodeId),
    Slice {
        input: NodeId,
        offset: ParameterId,
        limit: Option<ParameterId>,
    },
    Values(Vec<Vec<Option<ParameterId>>>),
}

impl Operator {
    pub fn inputs(&self) -> Vec<NodeId> {
        match self {
            Self::Closed(op) => op.inputs(),
            Self::Join(a, b) => vec![*a, *b],
            Self::Filter(a, _) | Self::Project(a) | Self::Distinct(a) => vec![*a],
            Self::Slice { input, .. } => vec![*input],
            Self::Unit | Self::Scan(_) | Self::Values(_) => vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub operator: Operator,
    pub output: Schema,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultForm {
    Select,
    Ask,
}

/// A plan is constructed only by lowering, with topologically ordered nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub(crate) bounds: Bounds,
    pub(crate) capacity: Capacity,
    pub(crate) opening_profile: OpeningProfile,
    pub(crate) credential_quad_bounds: Vec<u32>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) parameters: Vec<Parameter>,
    pub(crate) variable_names: Vec<String>,
    pub(crate) result: NodeId,
    pub(crate) form: ResultForm,
}

impl Plan {
    /// Choose opening bounds from the verifier's request, independently of witnesses.
    pub fn with_opening_profile(mut self, profile: OpeningProfile) -> Self {
        self.opening_profile = profile;
        self
    }
    pub fn opening_profile(&self) -> OpeningProfile {
        self.opening_profile
    }
    /// Refine public per-credential bounds without using private cardinalities.
    pub fn with_credential_quad_bounds(mut self, bounds: &[u32]) -> Result<Self, crate::Error> {
        if bounds.len() != self.bounds.credentials as usize
            || bounds
                .iter()
                .any(|&n| n == 0 || n > self.bounds.quads_per_credential)
        {
            return Err(crate::Error::Capacity("per-credential quad bounds".into()));
        }
        self.credential_quad_bounds = bounds.to_vec();
        Ok(self)
    }
    pub fn credential_quad_bounds(&self) -> Vec<u32> {
        if self.credential_quad_bounds.is_empty() {
            vec![self.bounds.quads_per_credential; self.bounds.credentials as usize]
        } else {
            self.credential_quad_bounds.clone()
        }
    }
    pub fn credential_quad_capacities(&self) -> Vec<u32> {
        let mut capacities: Vec<_> = self
            .credential_quad_bounds()
            .into_iter()
            .map(|n| {
                crate::capacity::Bounds {
                    quads_per_credential: n,
                    ..self.bounds
                }
                .capacity()
                .expect("validated bounds")
                .quads_per_credential
            })
            .collect();
        capacities.resize(self.capacity.credentials as usize, 4);
        capacities
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    pub fn capacity(&self) -> Capacity {
        self.capacity
    }
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }
    pub fn variable_names(&self) -> &[String] {
        &self.variable_names
    }
    pub fn result_node(&self) -> NodeId {
        self.result
    }
    pub fn form(&self) -> ResultForm {
        self.form
    }
    pub fn result_schema(&self) -> &Schema {
        &self.nodes[self.result as usize].output
    }

    /// Length-prefixed, tagged, little-endian shape encoding. No serializer defaults.
    /// Parameter kinds and occurrence positions are encoded; their values are not.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut w = Encoding(b"sparq.noir-query.plan\0\x03".to_vec());
        for n in [
            self.capacity.credentials,
            self.capacity.quads_per_credential,
            self.capacity.rows_per_table,
        ] {
            w.u32(n);
        }
        for n in self.credential_quad_capacities() {
            w.u32(n);
        }
        let limits = self.opening_profile.limits();
        for capacity in [
            limits.resource,
            limits.object,
            limits.scalar,
            limits.datatype,
            limits.language,
            limits.created,
            limits.method,
        ] {
            w.u32(capacity);
        }
        w.u32(crate::credential::ISSUER_DEPTH);
        w.u8(match self.form {
            ResultForm::Select => 0,
            ResultForm::Ask => 1,
        });
        w.u32(self.result);
        w.len(self.parameters.len());
        for p in &self.parameters {
            w.u8(match p {
                Parameter::Term(_) => 0,
                Parameter::Index(_) => 1,
            });
        }
        w.len(self.nodes.len());
        for node in &self.nodes {
            w.u32(node.output.capacity);
            w.u8(match node.output.order {
                CanonicalOrder::TermEncoding => 0,
            });
            w.len(node.output.variables.len());
            for &v in &node.output.variables {
                w.u32(v);
            }
            match &node.operator {
                Operator::Closed(op) => {
                    w.u8(8);
                    let bytes = op.canonical_bytes();
                    w.len(bytes.len());
                    w.0.extend(bytes);
                }
                Operator::Unit => w.u8(0),
                Operator::Scan(slots) => {
                    w.u8(1);
                    for s in slots {
                        w.slot(s);
                    }
                }
                Operator::Join(a, b) => {
                    w.u8(2);
                    w.u32(*a);
                    w.u32(*b);
                }
                Operator::Filter(a, e) => {
                    w.u8(3);
                    w.u32(*a);
                    w.expr(e);
                }
                Operator::Project(a) => {
                    w.u8(4);
                    w.u32(*a);
                }
                Operator::Distinct(a) => {
                    w.u8(5);
                    w.u32(*a);
                }
                Operator::Slice {
                    input,
                    offset,
                    limit,
                } => {
                    w.u8(6);
                    w.u32(*input);
                    w.u32(*offset);
                    w.u8(u8::from(limit.is_some()));
                    if let Some(n) = limit {
                        w.u32(*n);
                    }
                }
                Operator::Values(rows) => {
                    w.u8(7);
                    w.len(rows.len());
                    for row in rows {
                        w.len(row.len());
                        for p in row {
                            w.u8(u8::from(p.is_some()));
                            if let Some(n) = p {
                                w.u32(*n);
                            }
                        }
                    }
                }
            }
        }
        w.0
    }

    pub fn hash(&self) -> [u8; 32] {
        *blake3::hash(&self.canonical_bytes()).as_bytes()
    }
}

struct Encoding(Vec<u8>);
impl Encoding {
    fn u8(&mut self, n: u8) {
        self.0.push(n);
    }
    fn u32(&mut self, n: u32) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).expect("bounded plan length"));
    }
    fn slot(&mut self, s: &Slot) {
        match s {
            Slot::Variable(v) => {
                self.u8(0);
                self.u32(*v);
            }
            Slot::Parameter(p) => {
                self.u8(1);
                self.u32(*p);
            }
        }
    }
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Term(s) => {
                self.u8(0);
                self.slot(s);
            }
            Expr::Compare(op, a, b) => {
                self.u8(1);
                self.u8(match op {
                    Compare::Equal => 0,
                    Compare::Less => 1,
                    Compare::LessOrEqual => 2,
                    Compare::Greater => 3,
                    Compare::GreaterOrEqual => 4,
                });
                self.expr(a);
                self.expr(b);
            }
            Expr::And(a, b) => {
                self.u8(2);
                self.expr(a);
                self.expr(b);
            }
            Expr::Or(a, b) => {
                self.u8(3);
                self.expr(a);
                self.expr(b);
            }
            Expr::Not(a) => {
                self.u8(4);
                self.expr(a);
            }
            Expr::In(a, bs) => {
                self.u8(5);
                self.expr(a);
                self.len(bs.len());
                for b in bs {
                    self.expr(b);
                }
            }
            Expr::InList {
                value,
                start,
                count,
            } => {
                self.u8(6);
                self.expr(value);
                self.u32(*start);
                self.u32(*count);
                self.u32(IN_CAPACITY as u32);
            }
        }
    }
}
