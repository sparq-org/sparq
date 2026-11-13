//! Expression IR. The plan layer supplies variable slots and static types.
//! Values and numeric bounds belong to public inputs; gadget structure is compiled.
use std::collections::BTreeMap;
use std::fmt;

use oxrdf::Term;
use spargebra::algebra::{Expression, Function};

/// Typed lanes; `Dynamic` requests the circuit's runtime scalar tag.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum LaneType {
    Boolean,
    Integer,
    Decimal,
    Float,
    Double,
    String,
    Date,
    DateTime,
    Term,
    Dynamic,
}

/// The first expression group, matching the corresponding ExprOp typing rules.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ExprOp {
    And,
    Or,
    Not,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    SameTerm,
    In,
    NotIn,
    If,
    Coalesce,
    Add,
    Sub,
    Mul,
    Div,
    UnaryPlus,
    UnaryMinus,
    Unsupported(String),
}

impl ExprOp {
    pub fn name(&self) -> &str {
        match self {
            Self::And => "&&",
            Self::Or => "||",
            Self::Not => "!",
            Self::Eq => "=",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::SameTerm => "sameTerm",
            Self::In => "IN",
            Self::NotIn => "NOT IN",
            Self::If => "IF",
            Self::Coalesce => "COALESCE",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::UnaryPlus => "unary +",
            Self::UnaryMinus => "unary -",
            Self::Unsupported(name) => name,
        }
    }
}

/// Neutral tree for integration with the canonical query plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Expr {
    Variable {
        slot: u32,
        ty: LaneType,
    },
    Constant(Term),
    /// A constrained boolean from the correlated-subquery lane.
    BooleanInput {
        slot: u32,
    },
    Bound {
        slot: u32,
    },
    Call {
        op: ExprOp,
        args: Vec<Expr>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refusal {
    pub function: String,
    pub reason: String,
}
impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.function, self.reason)
    }
}
impl std::error::Error for Refusal {}
fn refuse(function: &str, reason: &str) -> Refusal {
    Refusal {
        function: function.into(),
        reason: reason.into(),
    }
}

/// Each constant occurrence is a public parameter, never embedded Noir source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicConstant {
    pub value: Term,
    pub ty: LaneType,
}

/// Explicit conversion nodes keep EBV and term identity distinct from value equality.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Gadget {
    Variable(u32),
    BooleanInput(u32),
    PublicInput(u32),
    Bound(u32),
    Ebv,
    AsNumeric,
    /// Retain an existing cell; serialize only a newly computed typed value.
    ToTerm,
    Call(ExprOp),
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Node {
    pub gadget: Gadget,
    pub inputs: Vec<usize>,
    pub ty: LaneType,
}
/// Topological DAG. Every node carries the value, error, and capacity lanes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TypedDag {
    pub nodes: Vec<Node>,
    pub constants: Vec<PublicConstant>,
    pub root: usize,
}

pub fn constant_type(term: &Term) -> LaneType {
    let Term::Literal(literal) = term else {
        return LaneType::Term;
    };
    let dt = literal.datatype();
    match dt
        .as_str()
        .strip_prefix("http://www.w3.org/2001/XMLSchema#")
    {
        Some("boolean") => LaneType::Boolean,
        Some("decimal") => LaneType::Decimal,
        Some("float") => LaneType::Float,
        Some("double") => LaneType::Double,
        Some("string") => LaneType::String,
        Some("date") => LaneType::Date,
        Some("dateTime") => LaneType::DateTime,
        Some(
            "integer" | "long" | "int" | "short" | "byte" | "unsignedLong" | "unsignedInt"
            | "unsignedShort" | "unsignedByte" | "positiveInteger" | "nonNegativeInteger"
            | "negativeInteger" | "nonPositiveInteger",
        ) => LaneType::Integer,
        _ if literal.language().is_some() => LaneType::String,
        _ => LaneType::Term,
    }
}
fn numeric(ty: LaneType) -> bool {
    matches!(
        ty,
        LaneType::Integer | LaneType::Decimal | LaneType::Float | LaneType::Double
    )
}
pub fn promote(a: LaneType, b: LaneType, division: bool) -> LaneType {
    if !numeric(a) || !numeric(b) {
        return LaneType::Dynamic;
    }
    let ty = a.max(b);
    if division && ty == LaneType::Integer {
        LaneType::Decimal
    } else {
        ty
    }
}

/// Lower BIND for computed boolean, integer and decimal results.
pub fn lower_bind(expr: &Expr) -> Result<TypedDag, Refusal> {
    let mut dag = lower(expr)?;
    if !matches!(
        dag.nodes[dag.root].ty,
        LaneType::Boolean | LaneType::Integer | LaneType::Decimal
    ) {
        return Err(refuse(
            "BIND",
            "computed result type is outside this term-construction bucket",
        ));
    }
    dag.nodes.push(Node {
        gadget: Gadget::ToTerm,
        inputs: vec![dag.root],
        ty: LaneType::Term,
    });
    dag.root = dag.nodes.len() - 1;
    Ok(dag)
}

pub fn lower(expr: &Expr) -> Result<TypedDag, Refusal> {
    let mut builder = Builder {
        nodes: Vec::new(),
        constants: Vec::new(),
        interned: BTreeMap::new(),
    };
    let root = builder.expr(expr)?;
    Ok(TypedDag {
        nodes: builder.nodes,
        constants: builder.constants,
        root,
    })
}
struct Builder {
    nodes: Vec<Node>,
    constants: Vec<PublicConstant>,
    interned: BTreeMap<Node, usize>,
}
impl Builder {
    fn node(&mut self, gadget: Gadget, inputs: Vec<usize>, ty: LaneType) -> usize {
        let node = Node { gadget, inputs, ty };
        if let Some(&index) = self.interned.get(&node) {
            return index;
        }
        let index = self.nodes.len();
        self.interned.insert(node.clone(), index);
        self.nodes.push(node);
        index
    }
    fn convert(&mut self, input: usize, gadget: Gadget, ty: LaneType) -> usize {
        self.node(gadget, vec![input], ty)
    }
    fn expr(&mut self, expr: &Expr) -> Result<usize, Refusal> {
        match expr {
            Expr::Variable { slot, ty } => Ok(self.node(Gadget::Variable(*slot), vec![], *ty)),
            Expr::BooleanInput { slot } => {
                Ok(self.node(Gadget::BooleanInput(*slot), vec![], LaneType::Boolean))
            }
            Expr::Bound { slot } => Ok(self.node(Gadget::Bound(*slot), vec![], LaneType::Boolean)),
            Expr::Constant(value) => {
                let ty = constant_type(value);
                let slot = u32::try_from(self.constants.len())
                    .map_err(|_| refuse("constant", "public input capacity"))?;
                self.constants.push(PublicConstant {
                    value: value.clone(),
                    ty,
                });
                Ok(self.node(Gadget::PublicInput(slot), vec![], ty))
            }
            Expr::Call { op, args } => {
                if let ExprOp::Unsupported(name) = op {
                    return Err(refuse(name, "outside the first expression group"));
                }
                let valid_arity = match op {
                    ExprOp::Not | ExprOp::UnaryPlus | ExprOp::UnaryMinus => args.len() == 1,
                    ExprOp::If => args.len() == 3,
                    ExprOp::Coalesce => true,
                    ExprOp::In | ExprOp::NotIn => !args.is_empty(),
                    _ => args.len() == 2,
                };
                if !valid_arity {
                    return Err(refuse(op.name(), "argument count"));
                }
                let mut inputs = args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let types = inputs.iter().map(|&i| self.nodes[i].ty).collect::<Vec<_>>();
                let ty = match op {
                    ExprOp::And | ExprOp::Or | ExprOp::Not => {
                        for input in &mut inputs {
                            *input = self.convert(*input, Gadget::Ebv, LaneType::Boolean);
                        }
                        LaneType::Boolean
                    }
                    ExprOp::If => {
                        inputs[0] = self.convert(inputs[0], Gadget::Ebv, LaneType::Boolean);
                        if types[1] == types[2] {
                            types[1]
                        } else {
                            LaneType::Dynamic
                        }
                    }
                    ExprOp::Coalesce => types
                        .first()
                        .copied()
                        .filter(|t| types.iter().all(|v| v == t))
                        .unwrap_or(LaneType::Dynamic),
                    ExprOp::Add | ExprOp::Sub | ExprOp::Mul | ExprOp::Div => {
                        for input in &mut inputs {
                            let ty = self.nodes[*input].ty;
                            *input = self.convert(
                                *input,
                                Gadget::AsNumeric,
                                if numeric(ty) { ty } else { LaneType::Dynamic },
                            );
                        }
                        promote(types[0], types[1], *op == ExprOp::Div)
                    }
                    ExprOp::UnaryPlus | ExprOp::UnaryMinus => {
                        let ty = if numeric(types[0]) {
                            types[0]
                        } else {
                            LaneType::Dynamic
                        };
                        if *op == ExprOp::UnaryMinus {
                            inputs[0] = self.convert(inputs[0], Gadget::AsNumeric, ty);
                        }
                        ty
                    }
                    ExprOp::SameTerm => {
                        for input in &mut inputs {
                            *input = self.convert(*input, Gadget::ToTerm, LaneType::Term);
                        }
                        LaneType::Boolean
                    }
                    _ => LaneType::Boolean,
                };
                Ok(self.node(Gadget::Call(op.clone()), inputs, ty))
            }
        }
    }
}

/// Adapter from a parsed expression; structural query lowering remains in the plan layer.
/// Variable names resolve to plan slots even when that slot is currently unbound.
pub fn from_spargebra(
    expr: &Expression,
    slots: &BTreeMap<String, (u32, LaneType)>,
) -> Result<Expr, Refusal> {
    from_spargebra_with_boolean_inputs(expr, slots, &[])
}

/// Match correlated EXISTS subplans to constrained boolean input slots from lane C.
/// The caller owns correlation and completeness; this layer only combines their values.
pub fn from_spargebra_with_boolean_inputs(
    expr: &Expression,
    slots: &BTreeMap<String, (u32, LaneType)>,
    booleans: &[(spargebra::algebra::GraphPattern, u32)],
) -> Result<Expr, Refusal> {
    let slot = |name: &str| {
        slots
            .get(name)
            .copied()
            .ok_or_else(|| refuse(name, "variable has no plan slot"))
    };
    let call = |op, args: Vec<&Expression>| -> Result<Expr, Refusal> {
        Ok(Expr::Call {
            op,
            args: args
                .into_iter()
                .map(|a| from_spargebra_with_boolean_inputs(a, slots, booleans))
                .collect::<Result<_, _>>()?,
        })
    };
    use Expression as E;
    match expr {
        E::Variable(v) => {
            let (slot, ty) = slot(v.as_str())?;
            Ok(Expr::Variable { slot, ty })
        }
        E::NamedNode(n) => Ok(Expr::Constant(n.clone().into())),
        E::Literal(l) => Ok(Expr::Constant(l.clone().into())),
        E::Bound(v) => Ok(Expr::Bound {
            slot: slot(v.as_str())?.0,
        }),
        E::And(a, b) => call(ExprOp::And, vec![a, b]),
        E::Or(a, b) => call(ExprOp::Or, vec![a, b]),
        E::Not(a) => call(ExprOp::Not, vec![a]),
        E::Equal(a, b) => call(ExprOp::Eq, vec![a, b]),
        E::SameTerm(a, b) => call(ExprOp::SameTerm, vec![a, b]),
        E::Greater(a, b) => call(ExprOp::Gt, vec![a, b]),
        E::GreaterOrEqual(a, b) => call(ExprOp::Ge, vec![a, b]),
        E::Less(a, b) => call(ExprOp::Lt, vec![a, b]),
        E::LessOrEqual(a, b) => call(ExprOp::Le, vec![a, b]),
        E::Add(a, b) => call(ExprOp::Add, vec![a, b]),
        E::Subtract(a, b) => call(ExprOp::Sub, vec![a, b]),
        E::Multiply(a, b) => call(ExprOp::Mul, vec![a, b]),
        E::Divide(a, b) => call(ExprOp::Div, vec![a, b]),
        E::UnaryPlus(a) => call(ExprOp::UnaryPlus, vec![a]),
        E::UnaryMinus(a) => call(ExprOp::UnaryMinus, vec![a]),
        E::If(a, b, c) => call(ExprOp::If, vec![a, b, c]),
        E::Coalesce(args) => call(ExprOp::Coalesce, args.iter().collect()),
        E::In(lhs, args) => call(
            ExprOp::In,
            std::iter::once(lhs.as_ref()).chain(args.iter()).collect(),
        ),
        E::FunctionCall(function, args) => {
            let name = match function {
                Function::Custom(iri) => iri.as_str().to_owned(),
                _ => function.to_string(),
            };
            let _ = args;
            Err(refuse(&name, "outside the first expression group"))
        }
        E::Exists(pattern) => booleans
            .iter()
            .find(|(p, _)| p == pattern.as_ref())
            .map(|(_, slot)| Expr::BooleanInput { slot: *slot })
            .ok_or_else(|| refuse("EXISTS", "no constrained boolean input for this subplan")),
    }
}
