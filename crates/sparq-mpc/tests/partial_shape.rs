use oxrdf::{Literal, Term, Variable};
use sparq_mpc::{
    BatchedHiddenInput, DisclosedKeyJoin, Fp, GlobalJoin, HiddenValueJoin, HolderId, JoinPlan,
    MpcError, PartialResult, RowBinding, ShamirBackend,
};

fn hidden_input(holder: &str, width: usize) -> BatchedHiddenInput {
    BatchedHiddenInput {
        holder: HolderId::new(holder),
        payload_vars: vec![Variable::new_unchecked(holder)],
        rows: vec![(
            Fp::new(7),
            vec![
                Some(Term::Literal(Literal::new_simple_literal(
                    "shape-term-sentinel",
                )));
                width
            ],
        )],
        binding: RowBinding::Keyed(vec!["common".into()]),
    }
}

fn hidden_join_refuses_compensating_payload_arities(fully_oblivious: bool) {
    let join = HiddenValueJoin::new(ShamirBackend::new(3).unwrap());
    let mut left = hidden_input("left", 0);
    let right = hidden_input("right", 2);
    let run = |left: &BatchedHiddenInput| {
        if fully_oblivious {
            join.fully_oblivious_batched_join(left, &right, 1)
        } else {
            join.batched_join(left, &right, 1)
        }
    };
    assert_eq!(run(&left).unwrap_err(), shape_error("left", 0));
    left.rows[0].1.push(None);
    assert_eq!(run(&left).unwrap_err(), shape_error("right", 0));
}

#[test]
fn batched_join_refuses_compensating_payload_arities() {
    hidden_join_refuses_compensating_payload_arities(false);
}

#[test]
fn fully_oblivious_batched_join_refuses_compensating_payload_arities() {
    hidden_join_refuses_compensating_payload_arities(true);
}

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
    for (names, column) in [(vec!["a", "a"], 1), (vec!["a", "b", "a"], 2)] {
        let mut p = partial("duplicate-holder");
        p.vars = names.into_iter().map(Variable::new_unchecked).collect();
        p.rows[0].resize(p.vars.len(), None);
        let plan = JoinPlan {
            join_var: p.vars[0].clone(),
            key_disclosed: true,
        };
        let expected = MpcError::Protocol(format!(
            "holder duplicate-holder repeats a projected variable at column {column} (expected arity {})",
            p.vars.len(),
        ));
        assert_eq!(DisclosedKeyJoin.join(&[p], &plan), Err(expected));
    }
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
