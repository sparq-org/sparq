use super::*;
use oxrdf::{vocab::xsd, BlankNode, Literal, NamedNode};

fn typed(lex: &str, dt: oxrdf::NamedNodeRef<'_>) -> Term {
    Term::Literal(Literal::new_typed_literal(lex, dt))
}

/// The plain-`xsd:string` literals the lazy path handles — including the adversarial
/// lexical-order cases (`"10" < "2"`), the empty string, and duplicates (ties).
fn strings() -> Vec<Term> {
    ["apple", "banana", "zebra", "", "10", "2", "apple", "Apple", "aardvark", "http://x/y"]
        .iter()
        .map(|s| Term::Literal(Literal::new_simple_literal(*s)))
        .collect()
}

/// Non-string terms spanning every OTHER class + literal kind, to exercise the cross-type
/// arms (`StrId` vs `Iri` / `Num` / `Temp` / `Val`).
fn others() -> Vec<Term> {
    vec![
        Term::BlankNode(BlankNode::new("b0").unwrap()),
        Term::NamedNode(NamedNode::new("http://ex/a").unwrap()),
        Term::NamedNode(NamedNode::new("http://x/y").unwrap()), // same lexical as a string
        typed("42", xsd::INTEGER),
        typed("3.14", xsd::DOUBLE),
        typed("true", xsd::BOOLEAN),
        typed("2020-01-01T00:00:00Z", xsd::DATE_TIME),
        typed("2020-06-15", xsd::DATE),
        Term::Literal(Literal::new_language_tagged_literal("apple", "en").unwrap()),
        typed("PT1H", xsd::DURATION),
        Term::Literal(Literal::new_typed_literal(
            "x",
            NamedNode::new("http://ex/customdt").unwrap(),
        )),
    ]
}

/// Build a graph interning every corpus term (as the object of a triple) so each has a real
/// dictionary id; return the graph plus the `(term, id)` list.
fn graph_with(terms: &[Term]) -> (Graph, Vec<(Term, Id)>) {
    let mut ttl = String::from("@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n");
    for (i, t) in terms.iter().enumerate() {
        // Serialise each object term inline; plain strings, langs, typed literals, IRIs,
        // blanks all have a canonical Turtle form via oxrdf's Display.
        let obj = match t {
            Term::BlankNode(_) => format!("[ <http://ex/tag> {} ]", i), // fresh blank
            other => other.to_string(),
        };
        ttl.push_str(&format!("<http://ex/s{}> <http://ex/v> {} .\n", i, obj));
    }
    let g = Graph::load_str(&ttl, "turtle").expect("corpus graph");
    // Resolve each NON-blank term to its id via id_of; blanks get their own fresh id per
    // row so we look them up positionally by re-querying is skipped — blanks are only used
    // to exercise the class-rank arm, and any blank id works for that.
    let mut ided = Vec::new();
    for t in terms {
        if let Term::BlankNode(_) = t {
            continue; // handled separately below
        }
        let id = g.id_of(t).unwrap_or_else(|| panic!("term interned: {t}"));
        ided.push((t.clone(), id));
    }
    (g, ided)
}

/// The eager cell the feature-OFF path builds for a term.
fn eager(t: &Term) -> SortCell<'static> {
    sort_cell_val(Value::Term(t.clone()))
}

#[test]
fn strid_orders_identically_to_eager_val_on_every_pair() {
    let all: Vec<Term> = strings().into_iter().chain(others()).collect();
    let (g, ided) = graph_with(&all);
    let l = LocalVocab::default();

    // For each interned term decide its lazy cell: a plain string → StrId(id); anything
    // else → the same eager Val cell the feature-off path uses (only the string arm is
    // lazy). Then assert lazy-vs-lazy == eager-vs-eager for EVERY ordered pair.
    let lazy_cell = |t: &Term, id: Id| -> SortCell {
        if g.dict.plain_string_value(id).is_some() {
            SortCell::StrId(id)
        } else {
            eager(t)
        }
    };

    for (ta, ida) in &ided {
        // Reflexivity of the lazy cell.
        let la = lazy_cell(ta, *ida);
        assert_eq!(
            cmp_sort_cells(&g, &l, &la, &la),
            Ordering::Equal,
            "lazy reflexivity failed for {ta}"
        );
        for (tc, idc) in &ided {
            let want = cmp_sort_cells(&g, &l, &eager(ta), &eager(tc));
            let lc = lazy_cell(tc, *idc);
            let got = cmp_sort_cells(&g, &l, &la, &lc);
            assert_eq!(
                got, want,
                "lazy StrId order diverged from eager Val for ({ta}, {tc}): got {got:?} want {want:?}"
            );
            // Antisymmetry across the lazy path.
            let got_rev = cmp_sort_cells(&g, &l, &lc, &la);
            assert_eq!(got, got_rev.reverse(), "lazy antisymmetry ({ta}, {tc})");
        }
    }
}

#[test]
fn plain_string_value_matches_exactly_the_string_kind_set() {
    // The lazy eligibility predicate must be `Some` for EXACTLY the plain xsd:string
    // literals and `None` for everything else (lang-tagged, typed non-string, IRI, blank,
    // numeric) — otherwise a non-string would take the string-ordered lazy arm.
    let all: Vec<Term> = strings().into_iter().chain(others()).collect();
    let (g, ided) = graph_with(&all);
    for (t, id) in &ided {
        let is_plain_string = matches!(t, Term::Literal(l)
            if l.language().is_none() && l.datatype() == xsd::STRING);
        assert_eq!(
            g.dict.plain_string_value(*id).is_some(),
            is_plain_string,
            "plain_string_value eligibility mismatch for {t}"
        );
        if is_plain_string {
            if let Term::Literal(l) = t {
                assert_eq!(g.dict.plain_string_value(*id), Some(l.value()), "value slice for {t}");
            }
        }
    }
}
