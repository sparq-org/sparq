# Policy parsing limits

`default_policy_limits() -> sparq_core::meter::Limits` returns a template. The
ordinary string and graph wrappers size it to their input; explicit metered calls
use the caller's limits exactly. One ledger covers both permission and prohibition
rules, all compounds, atomic constraints, cached RDF lists and extraction queries.

- **Depth:** 32 nested descents below the outer compound, retaining the earlier
  parser's bound. The existing policy and bridge corpus needs only shallow nesting.
  The shared meter's independent guard for nesting of triple terms reports the
  same `Depth` kind; raising compound depth does not raise that fixed guard.
- **Nodes:** the earlier 1024-operand allowance is shared across the policy, up to
  a finite ceiling derived from repeated-compound probes against main. Ordinary
  wrappers size the allowance by the larger of rule count and constraint-edge
  count, with an allowance for first-walk list cells. Main reset its budget for
  each attached compound, including several compounds on one rule, so counting
  only rules would refuse existing inputs. Expansion totals for individually
  legacy-sized compounds are checked before assembly. A caller can raise `nodes`
  to admit larger totals. The ceiling classifies expansion beyond the measured
  practical parsing envelope as an input-work refusal.
- **Bytes:** the base is four times source length for string wrappers, or 4 MiB
  for a loaded graph. Ordinary wrappers also cover retained terms using the graph's
  triple count and largest stored term, then add a bounded allowance for expanded
  atom copies using the node allowance and the whole stored-term vocabulary.
  A folded set can be larger than any individual stored term, so sizing copies
  by only the largest source term would refuse compact shared-set policies. A strict source-only
  multiplier refused an existing shared-compound policy; the expansion allowance
  preserves its result. Explicit caller byte limits remain strict. Term bytes
  are charged before cloning retained atoms or expanded copies.
- **Query rows:** ordinary wrappers allow twice the maximum bulk-query count times
  `max(1, triples)`. The graph-independent template uses the node ceiling as its
  input-size reference. Every extraction query runs with the ledger's remaining
  row allowance and charges its returned rows. Constraint fields, duty fields and
  list fields are queried separately. Engine intermediate rows share the query's
  row cap, even if a later projection would reduce them.

RDF list heads cache resolved members as `Rc<[Term]>`; repeated uses share that
resolution. Each cell consumes nodes on its first walk; subsequent member copies
still consume bytes. Raw field and typed atom tables are retained once for both
rule kinds, so atomic constraint fields are queried once across the policy.

```rust
use sparq_core::{Graph, meter::Ledger};
use sparq_policy::{default_policy_limits, parse_policy_metered};

fn main() -> Result<(), String> {
    let graph = Graph::load_str(
        "<urn:p> <http://www.w3.org/ns/odrl/2/permission> <urn:r> .",
        "turtle",
    )?;
    let mut limits = default_policy_limits();
    limits.bytes = 8 * 1024 * 1024;
    let mut ledger = Ledger::new(limits);
    let policy = parse_policy_metered(&graph, &mut ledger)?;
    assert_eq!(policy.permissions.len(), 1);
    Ok(())
}
```

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
