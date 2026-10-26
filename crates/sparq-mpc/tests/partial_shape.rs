use oxrdf::{Literal, Term, Variable};
use sparq_mpc::{DisclosedKeyJoin, GlobalJoin, HolderId, JoinPlan, MpcError, PartialResult};

fn partial(holder: &str) -> PartialResult {
    PartialResult {
        holder: HolderId::new(holder),
        vars: vec![Variable::new_unchecked("key")],
        rows: vec![vec![Some(Term::Literal(Literal::new_simple_literal(
            "shape-term-sentinel",
        )))]],
    }
}

fn shape_error(holder: &str, row: usize) -> MpcError {
    MpcError::Protocol(format!("holder {holder} expected arity 1 at row {row}"))
}

fn join_refuses_width(width: usize) {
    let plan = JoinPlan {
        join_var: Variable::new_unchecked("key"),
        key_disclosed: true,
    };
    let join = DisclosedKeyJoin::new();
    let global: &dyn GlobalJoin = &join;
    for bad_row in [0, 1] {
        let mut malformed = partial("malformed-holder");
        malformed.rows = vec![malformed.rows[0].clone(); 2];
        malformed.rows[bad_row].resize(width, None);
        let valid = partial("valid-holder");
        let mut empty = valid.clone();
        empty.rows.clear();
        // Singleton, either fold side, and a later partial after an empty fold.
        for inputs in [
            vec![malformed.clone()],
            vec![malformed.clone(), valid.clone()],
            vec![valid.clone(), malformed.clone()],
            vec![valid.clone(), empty, malformed.clone()],
        ] {
            let expected = Err(shape_error("malformed-holder", bad_row));
            assert_eq!(join.join(&inputs, &plan), expected);
            assert_eq!(global.join(&inputs, &plan), expected);
        }
    }
}

#[test]
fn disclosed_join_refuses_short_rows() {
    join_refuses_width(0);
}

#[test]
fn disclosed_join_refuses_long_rows() {
    join_refuses_width(2);
}

#[test]
fn disclosed_join_refuses_duplicate_variables() {
    let mut p = partial("duplicate-holder");
    p.vars.push(p.vars[0].clone());
    p.rows[0].push(None);
    let plan = JoinPlan {
        join_var: p.vars[0].clone(),
        key_disclosed: true,
    };
    assert_eq!(
        DisclosedKeyJoin.join(&[p], &plan),
        Err(MpcError::Protocol(
            "holder duplicate-holder repeats a projected variable at column 1 (expected arity 2)"
                .into(),
        ))
    );
}

#[test]
fn valid_join_preserves_schema_rows_and_attribution() {
    let left = partial("left");
    let mut right = partial("right");
    right.vars.push(Variable::new_unchecked("payload"));
    right.rows[0].push(None);
    let plan = JoinPlan {
        join_var: left.vars[0].clone(),
        key_disclosed: true,
    };
    let expected = PartialResult {
        holder: HolderId::new("federation"),
        vars: right.vars.clone(),
        rows: right.rows.clone(),
    };
    assert_eq!(DisclosedKeyJoin.join(&[left, right], &plan), Ok(expected));
}
