//! [GPT-6] Checked numeric consumers for an explicitly bounded evaluation.

use super::{budget, xsd, ArithOp, Dec, Literal, Num};

pub(super) fn miss<T>() -> Option<T> {
    let _ = budget::fail_capacity("numeric-representation");
    None
}

pub(super) fn operand(literal: &Literal) -> Option<Num> {
    let result = Num::of_literal(literal);
    if budget::strict_numeric()
        && sparq_core::numeric_literal_valid(literal.value(), literal.datatype().as_str())
        && (result.is_none()
            || (sparq_core::is_integer_datatype(literal.datatype().as_str())
                && !matches!(result, Some(Num::Int(_)))))
    {
        return miss();
    }
    result
}

pub(super) fn binop(left: Num, right: Num, op: ArithOp) -> Option<Num> {
    let result = left.binop(right, op);
    if budget::strict_numeric()
        && left.rank() < 2
        && right.rank() < 2
        && matches!(result, Some(Num::Float(_) | Num::Double(_)))
    {
        return miss();
    }
    result
}

pub(super) fn unary(value: Num, operation: impl FnOnce(Num) -> Num) -> Option<Num> {
    let result = operation(value);
    if budget::strict_numeric() && value.rank() < 2 && result.rank() >= 2 {
        return miss();
    }
    Some(result)
}

pub(super) fn comparable(left: Num, right: Num) -> bool {
    if budget::strict_numeric() {
        if let (Some(a), Some(b)) = (left.to_dec(), right.to_dec()) {
            if a.cmp(b).is_none() {
                return miss::<()>().is_some();
            }
        }
    }
    true
}

pub(super) fn decimal_cast(lexical: &str) -> Option<Dec> {
    let valid = sparq_core::numeric_literal_valid(lexical, xsd::DECIMAL.as_str());
    let result = valid.then(|| Dec::parse_lexical(lexical)).flatten();
    if budget::strict_numeric() && valid && result.is_none() {
        return miss();
    }
    result
}

pub(super) fn representable<T>(valid: bool, value: Option<T>) -> Option<T> {
    if budget::strict_numeric() && valid && value.is_none() {
        return miss();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueryBudget;

    #[test]
    fn reentrant_numeric_capacity_restores_parent_without_poisoning_next_query() {
        let strict = QueryBudget {
            strict_numeric_capacity: true,
            ..QueryBudget::unlimited()
        };
        budget::with_budget(&strict, || {
            assert!(budget::strict_numeric());
            assert!(miss::<()>().is_none());
            budget::with_budget(&QueryBudget::unlimited(), || {
                assert!(!budget::strict_numeric());
                assert!(budget::check(0).is_ok());
            });
            assert!(budget::strict_numeric());
            assert!(budget::check(0)
                .unwrap_err()
                .contains("numeric-representation"));
        });
        budget::with_budget(&strict, || assert!(budget::check(0).is_ok()));
        assert!(!budget::strict_numeric());
    }
}
