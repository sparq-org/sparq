// The native adapter retains oxrdf's interning and indexes. The guest needs only
// set membership and traversal; lexical keys avoid randomized interner ids.
#[cfg(target_os = "zkvm")]
pub(super) use ordered::Dataset;
#[cfg(not(target_os = "zkvm"))]
pub(super) use oxrdf02::Dataset;

#[cfg(any(target_os = "zkvm", test))]
mod ordered {
    use oxrdf02::{Quad, QuadRef};
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub struct Dataset(BTreeMap<String, Quad>);

    impl Dataset {
        pub fn iter(&self) -> impl Iterator<Item = QuadRef<'_>> {
            self.0.values().map(Quad::as_ref)
        }
    }

    impl FromIterator<Quad> for Dataset {
        fn from_iter<T: IntoIterator<Item = Quad>>(iter: T) -> Self {
            Self(
                iter.into_iter()
                    .map(|quad| (quad.to_string(), quad))
                    .collect(),
            )
        }
    }

    impl<'a> FromIterator<&'a Quad> for Dataset {
        fn from_iter<T: IntoIterator<Item = &'a Quad>>(iter: T) -> Self {
            iter.into_iter().cloned().collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ordered::Dataset;
    use oxrdf02::{GraphName, Literal, NamedNode, Quad};

    #[test]
    fn ordered_adapter_preserves_the_native_quad_set() {
        let p = NamedNode::new_unchecked("urn:p");
        let s = NamedNode::new_unchecked("urn:s");
        let first = Quad::new(
            s.clone(),
            p.clone(),
            Literal::new_simple_literal("a\n\""),
            GraphName::DefaultGraph,
        );
        let second = Quad::new(
            s,
            p,
            Literal::new_language_tagged_literal("a", "en").unwrap(),
            NamedNode::new_unchecked("urn:g"),
        );
        let quads = vec![second.clone(), first.clone(), second];
        let native = oxrdf02::Dataset::from_iter(&quads);
        let ordered = Dataset::from_iter(&quads);
        let mut expected: Vec<_> = native.iter().map(|quad| quad.to_string()).collect();
        expected.sort();
        assert_eq!(
            ordered
                .iter()
                .map(|quad| quad.to_string())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            Dataset::from_iter(quads.into_iter().rev())
                .iter()
                .map(|quad| quad.to_string())
                .collect::<Vec<_>>(),
            expected
        );
        assert!(Dataset::default().iter().next().is_none());
    }
}
