use super::*;
use std::sync::Arc;

const F: &str = "http://ex/f";

fn registry(tag: &'static str) -> crate::FunctionRegistry {
    let mut reg = crate::FunctionRegistry::new();
    reg.register(F, move |_: &[Term]| Ok(Term::Literal(oxrdf::Literal::new_simple_literal(tag))));
    reg
}

fn call() -> Option<Term> {
    functions::lookup(F).map(|f| f(&[]).unwrap())
}

fn lit(tag: &str) -> Option<Term> {
    Some(Term::Literal(oxrdf::Literal::new_simple_literal(tag)))
}

#[test]
fn nested_function_registry_restores_the_outer_one() {
    crate::with_functions(&registry("outer"), || {
        assert_eq!(call(), lit("outer"));
        crate::with_functions(&registry("inner"), || assert_eq!(call(), lit("inner")));
        assert_eq!(call(), lit("outer"), "outer registry lost after the inner scope");
        let g = sparq_core::Graph::load_str("<http://ex/s> <http://ex/p> <http://ex/o> .", "turtle").unwrap();
        let r = crate::query(&g, "SELECT ?v WHERE { BIND(<http://ex/f>() AS ?v) }").unwrap();
        assert_eq!(r.rows[0][0], lit("outer"));
    });
    assert_eq!(call(), None, "registry leaked past the outermost scope");
}

#[test]
fn nested_function_registry_restores_the_outer_one_on_unwind() {
    crate::with_functions(&registry("outer"), || {
        let unwound = std::panic::catch_unwind(|| {
            crate::with_functions(&registry("inner"), || panic!("inner scope panics"))
        });
        assert!(unwound.is_err());
        assert_eq!(call(), lit("outer"));
    });
    assert_eq!(call(), None);
}

#[cfg(feature = "window-functions")]
#[test]
fn nested_aggregate_registry_restores_the_outer_one() {
    let reg = |tag: &'static str| {
        let mut reg = crate::CustomAggregateRegistry::new();
        reg.register(F, move |_: &[Option<Term>]| Ok(lit(tag)));
        reg
    };
    let call = || aggregates::lookup(F).map(|f| f(&[]).unwrap());
    crate::aggregate::with_aggregates(&reg("outer"), || {
        crate::aggregate::with_aggregates(&reg("inner"), || assert_eq!(call(), Some(lit("inner"))));
        assert_eq!(call(), Some(lit("outer")));
    });
    assert_eq!(call(), None);
}

struct NoIndex;
impl crate::SpatialProvider for NoIndex {
    fn candidates(&self, _: &crate::SpatialQuery) -> Option<Vec<Term>> {
        None
    }
    fn is_indexed(&self, _: &Term) -> bool {
        false
    }
}

#[test]
fn nested_spatial_index_restores_the_outer_one() {
    let outer: Arc<dyn crate::SpatialProvider> = Arc::new(NoIndex);
    let inner: Arc<dyn crate::SpatialProvider> = Arc::new(NoIndex);
    let is = |want: &Arc<dyn crate::SpatialProvider>| {
        spatial::active().is_some_and(|a| std::ptr::addr_eq(Arc::as_ptr(&a), Arc::as_ptr(want)))
    };
    crate::with_spatial_index(outer.clone(), || {
        crate::with_spatial_index(inner.clone(), || assert!(is(&inner)));
        assert!(is(&outer), "outer spatial index lost after the inner scope");
    });
    assert!(spatial::active().is_none());
}
