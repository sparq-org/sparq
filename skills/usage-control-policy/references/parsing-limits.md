# Policy parsing limits

`default_policy_limits_for(&graph, source_bytes) -> sparq_core::meter::Limits`
returns the input-sized limits used by the ordinary wrappers. Pass `Some(rdf.len())`
when source text is available, or `None` for an already-loaded graph. Explicit
metered calls use the caller's limits. The older `default_policy_limits()` remains
an input-independent template for callers choosing their own limits.

One ledger covers permission and prohibition rules, compounds, atomic constraints,
RDF lists, predicate scans and extraction queries.

- **Depth:** the existing compound descent cap is retained. The shared meter's
  independent triple-term nesting guard reports the same `Depth` refusal kind.
- **Nodes:** defaults cover the former per-compound operand allowance times the
  larger of distinct rule count and constraint-edge count, plus authored combinator
  references and first-walk list cells, capped by the policy-wide operand ceiling.
  The input-reference allowance also covers valid unattached combinator definitions.
  Each compound independently
  retains the former operand cap, even with padding rules or raised caller limits.
  Combined expansion counts are checked before assembling compound operands.
  Operand references consume nodes; this ceiling bounds operand count, rather
  than elapsed work. List reuse consumes its reference node once, including when
  an enclosing operand reservation or direct constraint has already counted it.
- **Bytes:** four times source length when known. For a graph, the base is the
  summed term bytes of its triples, including term-node overhead, counted in one
  charged sizing scan. Each distinct atom and resolved list is charged when built.
  Fixed list slots count towards Nodes; their lexical storage counts towards Bytes,
  and per-term checks still include term-node overhead. Compound reuse shares the
  retained atom without charging or copying its terms again. Generated constraint
  guards charge their owned terms before construction.
- **Query rows:** defaults allow twice the fixed extraction-pass allowance times
  the graph's triple count (at least one). Predicate scans charge their exact range
  size before opening it, including pending insertions and deletions. Candidates
  later filtered by rule membership still count. Wrapper ledgers include the sizing
  pass. SPARQL queries use the remaining row cap for intermediate results as well
  as charging their returned rows.

Atoms are built only when reachable from rule constraints. Parser-local atoms and
resolved list members use `Rc` and `Rc<[Term]>`; public shared leaves and collection text
values use `Arc` to preserve thread transfer. A memoized list or folded value costs
one reference node per reuse. Unattached atomic fields incur scans without building
atoms or folding their right operands. Raw fields, folded values and compound
shapes are cached for the duration of one parse.

Combinator collections also retain shared member-key sequences. Their members are
validated and deduplicated once per head; compound definitions keep the authored
operand references. Expansion traverses these sequences with ordered deduplication,
preserving the position of an explicitly authored direct operand. The compound
operand count is checked before its result vector is reserved.

```rust
use sparq_core::{Graph, meter::Ledger};
use sparq_policy::{default_policy_limits_for, parse_policy_metered};

fn main() -> Result<(), String> {
    let graph = Graph::load_str(
        "<urn:p> <http://www.w3.org/ns/odrl/2/permission> <urn:r> .",
        "turtle",
    )?;
    let limits = default_policy_limits_for(&graph, None);
    let mut ledger = Ledger::new(limits);
    let policy = parse_policy_metered(&graph, &mut ledger)?;
    assert_eq!(policy.permissions.len(), 1);
    Ok(())
}
```

## Reproducing the operand calibration

The ignored `calibrate_operand_ceiling` test in
`crates/sparq-policy/tests/parse_differential.rs` runs the frozen main parser against
repeated real-atom compounds. The scalar case matches the original probe: distinct
atoms in one compound shared by every rule, with RDF loading included in each timed
parse. The folded-set case additionally checks sensitivity to atom size. The test
takes the median of repeated parses, verifies its upper rule bracket crosses the
declared elapsed envelope, then bisects rule count within that bracket. Multiply
the upper rule count by expanded operands per compound to re-derive the candidate
ceiling. If the upper bracket still fits, increase that bracket and rerun; the test
reports this without claiming a ceiling. The folded-set bracket is smaller because
main retains owned copies of the set. Run on an otherwise idle host and keep
observations in an external report:

```sh
cargo test -p sparq-policy --release --test parse_differential \
  calibrate_operand_ceiling -- --ignored --nocapture
```

This calibration selects an operand-count ceiling. Different atom sizes have
separate elapsed costs; the calibration does not specify a runtime bound.

Read refusals with `constraint_budget_refusal(&err)`, including wrapped errors.
The human-readable prefix stays `CONSTRAINT_BUDGET_REFUSAL`. Tags are:

| Kind | Tag |
|---|---|
| `Depth` | `constraint-depth` |
| `Expansion` (compatible alias for nodes refused during compound assembly) | `constraint-expansion` |
| `Nodes` | `constraint-nodes` |
| `Bytes` | `policy-bytes` |
| `Rows` | `policy-query-rows` |
| `Unspecified` | unrecognized tags |

Per-term and document byte caps also report `Bytes`. A failed reservation reports
its dimension's kind, retaining the expansion alias during compound assembly.

A refusal returns an error for the whole policy. It does not return a partial model.
A failed call retains spent work, while compound depth is balanced on return.
