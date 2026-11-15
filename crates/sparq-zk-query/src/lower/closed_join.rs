//! Deterministic association of pure inner joins avoids unnecessary cross products.
use super::Builder;
use crate::{plan::NodeId, Error};
use spargebra::algebra::GraphPattern as G;
fn factors<'a>(g: &'a G, out: &mut Vec<&'a G>) {
    if let G::Join { left, right } = g {
        factors(left, out);
        factors(right, out);
    } else {
        out.push(g);
    }
}
impl Builder {
    pub(super) fn closed_join(
        &mut self,
        g: &G,
        context: Option<NodeId>,
        graph: Option<u32>,
    ) -> Result<NodeId, Error> {
        let mut patterns = vec![];
        factors(g, &mut patterns);
        let mut nodes = patterns
            .into_iter()
            .map(|p| self.closed_in(p, context, graph))
            .collect::<Result<Vec<_>, _>>()?;
        let seed = (0..nodes.len())
            .max_by_key(|&i| (self.schema(nodes[i]).len(), std::cmp::Reverse(i)))
            .expect("join factors");
        let mut input = nodes.remove(seed);
        while !nodes.is_empty() {
            let vars = self.schema(input);
            let next = (0..nodes.len())
                .max_by_key(|&i| {
                    (
                        self.schema(nodes[i])
                            .iter()
                            .filter(|v| vars.contains(v))
                            .count(),
                        std::cmp::Reverse(i),
                    )
                })
                .unwrap();
            input = self.join(input, nodes.remove(next))?;
        }
        Ok(input)
    }
}
