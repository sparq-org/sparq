# EXISTS with MINUS solution domains

<!-- [GPT-6] Scoped shared engine correction; broader correlation is unfinished. -->

For EXISTS/NOT EXISTS bodies composed of BGP, Join, UNION, FILTER and MINUS,
the engine applies captured IRI/literal bindings before MINUS observes the child
solution domains. It checks exact RDF term identity and removes the captured
columns. Thus, given `ex:a ex:p 1`, this query retains `ex:a`:

```sparql
PREFIX ex: <http://example.org/>
SELECT ?s { ?s ex:p ?n
  FILTER EXISTS { ?s ex:p ?m MINUS { ?s ex:p ?z } }
}
```

After replacing `?s` by its IRI, the remaining `?m` and `?z` domains are disjoint.
This follows the published [2013 substitution](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#defn_substitute)
and [MINUS](https://www.w3.org/TR/2013/REC-sparql11-query-20130321/#defn_algMinus)
definitions. The [EXISTS community report](https://w3c.github.io/sparql-exists/docs/sparql-exists.html)
explicitly identifies the resulting domain change as Issue 4. Its alternative
binding-injection proposal changes this result; it is not silently selected here.
The retained [differential fixtures](../../../crates/sparq-engine/tests/fixtures/exists_minus_differential.json)
record that PyOxigraph 0.5.11 returns no mapping for this discriminator, so interoperability
with that implementation does not establish agreement with this selected rule.

Only bound outer cells are substituted. Unbound cells retain variable domains;
uncaptured shared variables still allow MINUS to remove compatible solutions.
A variable occurring only in the right branch is constrained there before
subtraction. Duplicate outer mappings and zero-column inner mappings retain their
multiplicity. Reused integer literals are compared as RDF terms, not by numeric
value (for example, `10` and `"010"^^xsd:integer` differ as terms).

The repair does not cross subquery projections, VALUES, BIND, OPTIONAL, GRAPH,
path operators, nested EXISTS or triple-term patterns inside the body. Captured
blank terms and captured BOUND positions are also outside this rule. Those shapes
retain existing native practical behavior, which is not a complete published-2013
semantics claim. The proof profile must enforce its own narrower supported shapes;
in particular it must not interpret the native fallback as explicit support.
EXISTS running inside an outer GRAPH continues to use that active named graph.

The current implementation evaluates this admitted body per correlated outer row,
checks materialized intermediate widths/rows against the query budget, then
restricts captured values and projects their columns before parent operators run.
It can scan and allocate more than a future constant-substituted plan; capacity
errors remain errors rather than a false EXISTS result. Ordinary operators incur
one local mode check, and the unchanged positive-pattern/SIP branches retain their
existing behavior. No latency or performance-neutrality claim is made. Constant
scan pushdown and reuse across equal outer bindings remain optimization work.
