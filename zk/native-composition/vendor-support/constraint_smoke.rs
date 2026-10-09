//! [GPT-6] Executed compatibility checks for the native Ark instrumentation patch.
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

// This export makes the no-default-features check resolve the actual patched API.
pub use ark_relations::r1cs::SynthesisError;

#[cfg(all(test, feature = "std"))]
mod tests {
    use ark_bls12_381::Fr;
    use ark_relations::{
        lc,
        r1cs::{ConstraintLayer, ConstraintSystem, ConstraintTrace, TracingMode},
    };
    use tracing_subscriber::{layer::SubscriberExt, Registry};

    #[test]
    fn registry_captures_real_constraint_spans() {
        let subscriber =
            Registry::default().with(ConstraintLayer::new(TracingMode::OnlyConstraints));
        tracing::subscriber::with_default(subscriber, || {
            let outer = tracing::info_span!(target: "r1cs", "outer_constraint");
            let _outer = outer.enter();
            let inner = tracing::info_span!(target: "r1cs", "inner_constraint");
            let _inner = inner.enter();
            let trace = ConstraintTrace::capture().expect("active constraint span");
            let path = trace.path();
            assert_eq!(path.len(), 2);
            assert_eq!(path[0].name, "outer_constraint");
            assert_eq!(path[1].name, "inner_constraint");
            assert!(trace.to_string().contains("inner_constraint"));
        });
    }

    #[test]
    fn constraint_satisfaction_and_failure_are_unchanged() {
        for (product, expected) in [(6_u64, true), (7_u64, false)] {
            let cs = ConstraintSystem::<Fr>::new_ref();
            let left = cs.new_witness_variable(|| Ok(Fr::from(2_u64))).unwrap();
            let right = cs.new_witness_variable(|| Ok(Fr::from(3_u64))).unwrap();
            let output = cs.new_input_variable(|| Ok(Fr::from(product))).unwrap();
            cs.enforce_constraint(lc!() + left, lc!() + right, lc!() + output)
                .unwrap();
            assert_eq!(cs.is_satisfied().unwrap(), expected);
            assert_eq!(cs.num_constraints(), 1);
        }
    }
}
