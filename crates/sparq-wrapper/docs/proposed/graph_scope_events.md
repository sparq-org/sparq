# Graph scope events proposal

Implementation: `src/proposed/graph_scope_events.rs` (gated behind the default-off
`proposed-graph-scope-events` feature).
Feature: `proposed-graph-scope-events` (default off).

`ObservableDataset` owns a dataset and reports quad mutations through a
graph scope's read `Projection` (`GraphScope::projection()` or
`Projection::new`). A projected listener sees the union change: an add only for
the first in-scope copy of a triple, a delete only for the last, and nothing
for graphs outside the projection; each event carries the configured
projection. A named graph must be an IRI or a blank node; any other name is
rejected with `ObserveError::InvalidGraphName` before a graph is created. The feature implies `proposed-graph-scope` and `proposed-observe`.
Source: rdfjs/wrapper draft PR #96. <!-- sq-1rg2q.7 -->
