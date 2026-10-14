# OWL 2 DL reasoning from JavaScript (`checkOwlDl`)

`@sparq-org/sparq` exposes the OWL 2 DL reasoner of `sparq-reason-dl` (the `owl2dl` module,
see [`../../inference/SKILL.md`](../../inference/SKILL.md)) as one async function:

```js
import { checkOwlDl, SparqStore } from '@sparq-org/sparq';

const result = await checkOwlDl(tboxTurtle, aboxTurtle, {
  format: 'turtle',                         // 'turtle' (default) | 'ntriples' | 'nquads' | 'trig'
  budget: { maxSteps: 100_000 },  // optional: maxSteps / maxNodes; omitted members keep Budget::default()
});
if (result.consistent === 'unknown') console.log(result.reason);

// Convenience: the store's DEFAULT graph is the TBox (serialised to Turtle; named graphs
// are not included); the optional ABox must be Turtle or N-Triples.
const store = await SparqStore.fromString(tboxTurtle, 'turtle');
await store.checkOwlDl(aboxTurtle, { budget: { maxSteps: 100_000 } });
```

## What it does

`tbox` and the optional `abox` are parsed in `format` and merged into ONE graph (the ABox is
just more assertions; blank-node labels never join across the two documents). The graph is
read as an OWL 2 ontology by the fail-closed reverse RDF mapping, then the OWL 2 DL profile
check, consistency (TBox + ABox) and, for a consistent ontology, classification run.

It **rejects** (a thrown `Error`/`TypeError`) only on a parse error (message starts with
`TBox parse error` / `ABox parse error`), malformed options (a `TypeError`: options or `budget` that
are not plain objects, an unknown key, a `format` other than `turtle` / `ntriples` / `nquads` /
`trig`, a `format` passed to `SparqStore.checkOwlDl`, or a budget limit that is not a
non-negative integer — never a silent fall-back to the defaults), or if the DL bundle cannot be
loaded. A graph the OWL mapping refuses
is **not** a rejection: it resolves with `consistent: "unknown"` and the refusal as `reason`.

## Lazy loading

The reasoner is its own wasm bundle (`wasm-dl/`, built from `crates/sparq-dl-wasm`), never
imported statically: the first `checkOwlDl` call does `import('../wasm-dl/sparq_dl_wasm.js')`
and initialises it (Node reads the `.wasm` from disk; browsers fetch it next to the glue).
The load is memoised; a failed initialisation (for example the `.wasm` fetch) is retried on the next call, but a browser keeps a failed fetch of the glue module itself in its module map, so that case needs a reload. A page that never calls `checkOwlDl` never
fetches the bundle. The raw glue is also exported as `@sparq-org/sparq/wasm-dl/*`
(`checkOwlDl(tbox, abox, format, optionsJson) -> string`).

## Result shape (`OwlDlResult`)

```ts
{
  consistent: true | false | 'unknown',
  reason: null                                              // unless consistent is 'unknown'
    | { kind: 'OutOfFragment', construct: string, triples: string[] }
    | { kind: 'NotInDl', violations: OwlDlViolation[] }     // the blocking profile violations
    | { kind: 'ResourceBudget' }
    | { kind: 'ImportsNotLoaded', imports: string[] }        // owl:imports targets (not fetched)
    | { kind: 'Unparsed' | 'Malformed' | 'Ambiguous' | 'Unsupported', detail: string, triples: string[] },
  profile: {
    dl: true | false | 'unknown',            // true only with no violation AND nothing unchecked
    violations: { kind: OwlDlViolationKind, detail: string, triples: string[] }[],
    unchecked: { rule: string, triples: string[] }[],
  },
  unsatisfiable: string[],                   // named-class IRIs, sorted
  inferredSubClassOf: [string, string][],    // entailed [sub, super], not asserted, sorted
  classification: 'complete' | { unknown: 'ResourceBudget' | … } | null,  // null unless consistent
}
```

- `triples` are the input triples behind the answer, each one N-Triples line
  (`<s> <p> <o> .`). IRIs and literals are the input's; blank-node labels are per document and
  may be renamed (an ABox label that also occurs in the TBox) or generated (`[]` nodes). For
  `store.checkOwlDl` the TBox lines come from the re-serialised default graph.
- The four refusal kinds come from the reverse mapping: `Unparsed` (a triple no OWL 2
  mapping rule reads), `Malformed` (recognisably OWL but ill-formed, e.g. a restriction with
  two fillers), `Ambiguous` (a property whose kind cannot be settled), `Unsupported` (valid
  RDF the mapping deliberately refuses, e.g. RDF 1.2 triple terms). The profile is then
  `{ dl: 'unknown', violations: [], unchecked: [] }`.
- `OwlDlViolationKind`: `Undeclared`, `IllegalPunning`, `ReservedVocabulary`,
  `NonSimpleProperty`, `IrregularPropertyHierarchy`, `AnonymousIndividualCycle`,
  `IllTypedLiteral`, `UnknownDatatype`, `InvalidFacet`, `InvalidDatatypeDefinition`,
  `TopDataPropertyMisuse`, `ExtractionNote`, `InsufficientOperands`. Only `NonSimpleProperty`
  and `IrregularPropertyHierarchy` block reasoning (`reason.kind: 'NotInDl'`); the others are
  reported in `profile` while consistency is still decided. The verdict is about the
  ontology as the OWL 2 RDF mapping reads it: an assertion with an undeclared (e.g. misspelt)
  property is read as an annotation and plays no part in consistency, so check
  `profile.violations` for `Undeclared` / `ExtractionNote` when you pass a separate ABox.
- `unsatisfiable` and `inferredSubClassOf` are empty unless `classification` is
  `'complete'`. An inconsistent ontology has `classification: null` (it entails everything).
  A consistent one whose classification ran out of budget has `{ unknown: 'ResourceBudget' }`.

## Scope (honest)

The decided fragment is ALCHIQ with `hasSelf`, transitive roles and `R∘R ⊑ R`: class
constructors except nominals, object properties with inverses and hierarchies, the role
characteristics (functional, inverse-functional, reflexive, irreflexive, symmetric,
asymmetric, disjoint), qualified cardinality up to the reasoner's cardinality bound
(`MAX_CARDINALITY`; larger numbers, and axioms needing more successors than the clause limit,
answer `OutOfFragment`; very deeply nested expressions are refused as `Unsupported`), and class / object-property / negative /
`sameAs` / `differentFrom` assertions. **Data properties and datatypes, nominals
(`oneOf`, `hasValue`), other property chains, keys, and the top/bottom object properties
currently answer `consistent: 'unknown'` with `reason.kind: 'OutOfFragment'`** and the
triples of the axiom that uses them. The answer is never a guess: every verdict is either
decided or `'unknown'` with a reason. Budgets are deterministic work counts (`maxSteps`: rule applications, `maxNodes`: completion-graph
nodes), not time limits; the defaults are those of `sparq_reason_dl::owl2dl::Budget::default()`.
