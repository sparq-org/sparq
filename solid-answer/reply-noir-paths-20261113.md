# sparq reply to the Solid App Suite: the three absence and totals paths, and the packages

13 November 2026. Answers your note of 18:08 UTC and `noir-closed-world-queries.md`.

## Status in one paragraph

Two packages are ready to use now: joint computation and Node result proofs (below). The
three paths that need absence checks and totals (insurance-quote, trial-eligibility,
recycled-content) are **not built yet**. They are now the first acceptance cases of sparq's
Noir query method, which compiles one circuit per query. Everything in "The three paths"
below is the agreed design the lanes are building to. The target date is 23 November for
the proofs and 4 December for the Node package. Those are plan dates; nothing has been
measured yet. We will tell you when a path actually proves and verifies.

## Packages you can fetch today

Branch `solid-answer-dist` on `sparq-org/sparq`. Check each file against `SHA256SUMS` there.

| File | What | SHA-256 |
|---|---|---|
| `sparq-verifiable-aggregate-61989f5.tgz` | joint count, sum and threshold verdicts (the one you are wiring in) | `9832ef937efbcc8970db3f70ec4e72e3384a867f3ecb40a1be81ea002a6a6830` |
| `sparq-zk-result-7a56f943.tgz` | Node result proofs, built from main after the PR merged; offline install and all 23 installed-package tests pass on Node 24 | `e2b19eead2bcc471661b3698fb43b922caee4f9cf448024687c1f8c98335d86d` |

```sh
gh api -H "Accept: application/vnd.github.raw" "repos/sparq-org/sparq/contents/sparq-zk-result-7a56f943.tgz?ref=solid-answer-dist" > sparq-zk-result-7a56f943.tgz && shasum -a 256 sparq-zk-result-7a56f943.tgz
```

`sparq-zk-result-7a56f943.tgz` has the two public-predicate members and **test-only**
issuer keys (`testIssuerKeygen(seedHex)`). Two more builds follow as reviewed changes merge:

1. Issuer key generation from OS entropy (`issuerKeygen()`), and hidden `xsd:date` and
   full-width unsigned integer comparisons. Negative integer constants are refused.
2. The three things you asked for, for your six result-proof paths: test issuance that is
   reproducible from a seed (a separate test-only call; normal issuance keeps a fresh
   random salt), `verify` reporting which issuer keys the proof relied on, and the
   public-input layout documented in the README.

What result proofs do not show: answer completeness, absence, or holder identity. They
prove that the released rows are supported by credentials signed by trusted issuers.

## The three paths: what the Noir query method will give you

Your two requirements are both met by the design.

1. **Each predicate bound to the issuer you accept for it.** The method gives each
   credential its own named graph and adds one verified-issuer fact per credential. The
   receiver writes the binding into the query: the fact's pattern goes inside `GRAPH ?g`,
   and `?g vcq:verifiedIssuer <issuer>` says who must have signed it. Issuer IRIs in the
   query are constants, constants are public inputs, and your trust set binds each issuer
   IRI to its key. **Your flat queries would run, but they carry no issuer binding, so
   please send the scoped form.** The five scoped queries (insurance-quote in both forms,
   trial-eligibility, recycled-content) are in the appendix below.
2. **Completeness against the issuer's root.** The circuit opens every fact under each
   presented credential's signed root, and the answer is exact over them. So a
   `NOT EXISTS` inside the register's graph is a statement about the whole credential, and
   no separate completeness statement is needed. Two conditions:
   - The issuer must state what the root covers (the subject and the period) as facts
     inside the same credential, and the query checks them.
   - The proof cannot show that the holder presented the issuer's *newest* snapshot.
     That is covered by the status check and the freshness bound, not by the proof.
3. **Predicate IRIs are parameters.** Moving to the awp namespace changes neither the
   circuit nor its key.
4. **recycled-content.** Decimals compare natively, so no fixed-point encoding is needed.
   The lot must be a constant in the query; with a variable, figures for different lots
   would be pooled. With two of the three suppliers, the result is `false`, not "no proof".
5. **`verify`** rebuilds every public input itself from your stored request (query, trust
   set, status roots, times, nonce) and returns the result with a report of what was
   enforced, in your gate's terms: `rule`, `issuers`, `subject`, `fresh`, `complete`.
6. **Limits.** 4 credentials, 3 issuers, 32 facts per credential, integers to 2^48, dates
   as `xsd:date` without a timezone, 16 excluded entries, 32 medicines, one status entry
   per credential. All within the design.
7. **Credentials** are issued in the Poseidon2 Merkle-root suite. Key generation and
   issuance reproducible from a seed, in Node, offline, are a requirement on that package.

Both Node packages (result proofs now, query proofs later) will expose the same call
names and argument shapes for key generation, issuance, `prove` and `verify`, so one
adapter can call both. A field a package cannot establish is reported as not established,
never implied.

## Three questions for you

1. How are claims grouped into credentials: one credential per issuer and subject, or one
   per claim? It decides whether a path uses one graph variable or several per issuer.
2. Will snapshot credentials (the claims register, the medication list) carry their subject
   and period as facts under the same root? Please name those predicates.
3. Is `false` acceptable for "two of three suppliers", or must no proof exist at all?

## Appendix: the scoped queries

The period predicate names (`claims.period-start`, `trial.medications-from` and so on) are
placeholders for you to set. The `vcq:` namespace is the method's and may still change
before release.

Prefixes used below:

```sparql
PREFIX vcq: <https://w3id.org/sparq/vcq#>
PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
```

### 1a. insurance-quote, stated count (the consumer's current form)

```sparql
SELECT DISTINCT ?result WHERE {
  ?lic vcq:verifiedIssuer <urn:fictional:insurance-quote:licensing> .
  ?reg vcq:verifiedIssuer <urn:fictional:insurance-quote:claims-register> .
  GRAPH ?lic { ?subject <urn:claim:licence.years> ?v0 ;
                        <urn:claim:licence.points> ?v1 . }
  GRAPH ?reg { ?subject <urn:claim:claims.five-year-count> ?v2 ;
                        <urn:claim:claims.five-year-complete> ?v3 . }
  BIND((?v0 >= 5 && ?v1 <= 3 && ?v2 = 0 && ?v3 = true) AS ?result)
}
```

Needs only stage N0 operators plus `GRAPH`, `BIND` of a boolean and the issuer quad.

### 1b. insurance-quote, closed world (the consumer's preferred form)

The register issues one credential for the subject and period, with one fact per claim.

```sparql
SELECT DISTINCT ?result WHERE {
  ?lic vcq:verifiedIssuer <urn:fictional:insurance-quote:licensing> .
  ?reg vcq:verifiedIssuer <urn:fictional:insurance-quote:claims-register> .
  GRAPH ?lic { ?subject <urn:claim:licence.years> ?v0 ;
                        <urn:claim:licence.points> ?v1 . }
  GRAPH ?reg { ?subject <urn:claim:claims.period-start> ?from ;
                        <urn:claim:claims.period-end> ?to . }
  BIND((?v0 >= 5 && ?v1 <= 3 &&
        ?from <= "2021-11-13"^^xsd:date && ?to >= "2026-11-13"^^xsd:date &&
        NOT EXISTS { GRAPH ?reg { ?subject <urn:claim:claims.claim> ?c .
                                  ?c <urn:claim:claims.date> ?d .
                                  FILTER(?d >= "2021-11-13"^^xsd:date) } }) AS ?result)
}
```

The period facts are what make the register's root a snapshot "for this subject and
period". Their predicate names are placeholders for the consumer to set. The absence is
evaluated in the same graph `?reg` that carries the period, so it cannot be satisfied by a
different credential. With no register credential there is no solution at all, not `true`.

### 2. trial-eligibility

```sparql
SELECT DISTINCT ?result WHERE {
  ?prac vcq:verifiedIssuer <urn:fictional:trial-eligibility:practice> .
  GRAPH ?prac { ?subject <urn:claim:trial.age> ?age ;
                         <urn:claim:trial.diagnosis> ?dx ;
                         <urn:claim:trial.medications-from> ?from ;
                         <urn:claim:trial.medications-to> ?to . }
  BIND((?age >= 18 && ?age <= 65 && ?dx = "condition-a" &&
        ?from <= "2026-11-01"^^xsd:date && ?to >= "2026-11-13"^^xsd:date &&
        NOT EXISTS { GRAPH ?prac { ?subject <urn:claim:trial.medications> ?m .
                                   FILTER(?m IN ("excluded-x", "excluded-y")) } }) AS ?result)
}
```

This form assumes age, diagnosis and the medication list are in one credential. If the
practice issues them separately, use one graph variable per credential, each pinned to the
practice, and keep the absence in the graph that carries the period. The consumer's
current form with a `trial.medication-complete` fact also runs; replace the two period
patterns with that fact.

### 3. recycled-content

```sparql
SELECT DISTINCT ((MIN(?value) >= 0.3 && COUNT(DISTINCT ?iss) = 3 && COUNT(?value) = 3)
                 AS ?result) WHERE {
  ?g vcq:verifiedIssuer ?iss .
  FILTER(?iss IN (<urn:fictional:recycled-content:tier>,
                  <urn:fictional:recycled-content:tier-2>,
                  <urn:fictional:recycled-content:tier-3>))
  GRAPH ?g { <urn:fictional:recycled-content:lot>
               <urn:claim:material.recycled-fraction> ?value . }
}
```

- The lot is a constant. With a variable subject, figures for different lots would be
  pooled into one minimum.
- `COUNT(DISTINCT ?iss) = 3` is the rule that all three named suppliers contributed. With
  two of the three the result is `false`. `COUNT(?value) = 3` refuses a second figure from
  one supplier; it matters once the aggregate is a sum.
- Values are `xsd:decimal` and compare natively. No fixed-point encoding is needed.
- No holder binding: the request's holder-binding issuer list is explicitly empty.
- The other form the consumer named, for the same shape:
  `SELECT DISTINCT ?result WHERE { { SELECT (SUM(?mass) AS ?total) (COUNT(DISTINCT ?iss) AS ?n) WHERE { ... } } BIND((?total >= 500 && ?n = 3) AS ?result) }`,
  and its `GROUP BY ... HAVING` spelling. Both are acceptance cases.
