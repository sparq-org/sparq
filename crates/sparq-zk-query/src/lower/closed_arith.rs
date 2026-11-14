//! Arithmetic shape lowering for the shared expression gadgets.
use super::Builder;
use crate::{
    closed_arith::{Comparison, Integer},
    plan::{Compare, Expr},
    Error,
};
use spargebra::algebra::Expression as E;
pub(super) fn has_arithmetic(e: &E) -> bool {
    match e {
        E::Add(..) | E::Subtract(..) | E::Multiply(..) => true,
        E::Equal(a, b)
        | E::Less(a, b)
        | E::LessOrEqual(a, b)
        | E::Greater(a, b)
        | E::GreaterOrEqual(a, b)
        | E::And(a, b)
        | E::Or(a, b) => has_arithmetic(a) || has_arithmetic(b),
        E::Not(a) => has_arithmetic(a),
        _ => false,
    }
}
impl Builder {
    fn closed_integer(&mut self, e: &E) -> Result<Integer, Error> {
        Ok(match e {
            E::Add(a, b) => Integer::Add(
                Box::new(self.closed_integer(a)?),
                Box::new(self.closed_integer(b)?),
            ),
            E::Subtract(a, b) => Integer::Subtract(
                Box::new(self.closed_integer(a)?),
                Box::new(self.closed_integer(b)?),
            ),
            E::Multiply(a, b) => Integer::Multiply(
                Box::new(self.closed_integer(a)?),
                Box::new(self.closed_integer(b)?),
            ),
            _ => match self.expression(e)? {
                Expr::Term(s) => {
                    if let crate::plan::Slot::Parameter(p) = s {
                        if let crate::plan::Parameter::Term(oxrdf::Term::Literal(l)) =
                            &self.plan.parameters[p as usize]
                        {
                            if !matches!(
                                l.datatype().as_str(),
                                "http://www.w3.org/2001/XMLSchema#integer"
                                    | "http://www.w3.org/2001/XMLSchema#decimal"
                            ) {
                                return Err(Error::Unsupported(
                                    "lane D integer arithmetic operands".into(),
                                ));
                            }
                        }
                    }
                    Integer::Term(s)
                }
                _ => {
                    return Err(Error::Unsupported(
                        "lane D integer arithmetic expression".into(),
                    ))
                }
            },
        })
    }
    pub(super) fn closed_arithmetic(&mut self, e: &E) -> Result<Comparison, Error> {
        let (op, a, b) = match e {
            E::Equal(a, b) => (Compare::Equal, a, b),
            E::Less(a, b) => (Compare::Less, a, b),
            E::LessOrEqual(a, b) => (Compare::LessOrEqual, a, b),
            E::Greater(a, b) => (Compare::Greater, a, b),
            E::GreaterOrEqual(a, b) => (Compare::GreaterOrEqual, a, b),
            _ => return Err(Error::Unsupported(format!("arithmetic expression {e}"))),
        };
        Ok(Comparison {
            op,
            left: self.closed_integer(a)?,
            right: self.closed_integer(b)?,
        })
    }
}
