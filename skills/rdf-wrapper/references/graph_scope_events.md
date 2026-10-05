# Graph scope events extension

Implemented behind the default-off `proposed-graph-scope-events` feature in
`proposed::graph_scope_events`.

`ObservableDataset` owns a dataset and reports quad mutations through a
graph scope's read `Projection` (`GraphScope::projection()` or
`Projection::new`). A projected listener sees the union change: an add only for
the first in-scope copy of a triple, a delete only for the last, and nothing
for graphs outside the projection; each event carries the configured
projection. The feature implies `proposed-graph-scope` and `proposed-observe`.
Source: rdfjs/wrapper draft PR #96. <!-- sq-1rg2q.7 -->
