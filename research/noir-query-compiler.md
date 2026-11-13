# Query compiler architecture

This research implementation is not externally audited. The query method uses
a canonical bounded plan, an interpreter over complete credential tables and
a deterministic generator that wires library gadgets into a query program.

The plan assigns variables by first encounter and gives every constant occurrence
a public parameter slot. Its tagged, length-prefixed shape encoding contains
operator wiring, input references, output schemas, compiled capacities, result
form, parameter kinds and canonical orders. It omits constant values and variable
names, allowing the same program to serve requests with the same shape. Exact
query text belongs to the stored request and its digest.

Public request bounds choose the smallest supported credential, quad and table
buckets. Private cardinalities only cause a refusal; they cannot choose another
key. A bucket reveals an upper bound and the query structure. Each intermediate
table is checked against the public request's row bound as well as its compiled
capacity. A limit on the final result does not waive an intermediate bound.

Each table contains a bounded row array and its actual length. Padding uses the
canonical unbound cell in every column. Operators order rows lexicographically
by bound status, field term encoding and credential scope in schema order.
Projection retains multiplicity and sorts the projected rows; DISTINCT removes
equal mappings. Scan collapses duplicate triples in the default merge before
matching a pattern. Blank nodes from separate credentials remain distinct.
Join implements compatibility on bound shared variables, including VALUES UNDEF.

The initial expression seam carries explicit value and error lanes for key
comparison, boolean connectives and membership. FILTER retains only a true,
non-error root. Unimplemented operand families receive named refusals rather
than a different comparison. Expression-library integration uses the same seam.

The current execution ABI declares the stored request digest, public request
bounds, public term parameters, public slice indices and released result, in
that order. Intermediate tables and complete credential cells are private.
The digest binding is a public output. The credential opening is the programme's
explicit draft stand-in pending suite integration; the complete method statement
is owned with the method adapter. The generated program declares inputs and
calls gadgets in topological order; it emits no constraint logic.

The generated-package adapter uses the existing proving driver, private witness
lifecycle, filename validation and pinned-version matcher. Verification keys are
derived from local generated source. The composed manifest is superseded by the
query method.
