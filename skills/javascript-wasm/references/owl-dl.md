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

## Rechecking a draft (`openOwlDlSession`)

```js
import { openOwlDlSession } from '@sparq-org/sparq';

const session = await openOwlDlSession();   // loads the bundle like checkOwlDl
let r = session.check(draftTurtle);         // synchronous; same arguments as checkOwlDl
r = session.check(draftPlusEditTurtle);     // r.reuse: 'none' | 'identical' | 'extension'
session.free();
```

A session keeps the last decided answer. `session.check` answers what `checkOwlDl`
would, plus `reuse`: `'identical'` when the logical axioms, declarations and imports are the
last decided version's (the
answer is returned without reasoning), `'extension'` when they are a superset (OWL 2 is
monotone, so the old inconsistency, unsatisfiable classes and subsumptions carry over and only
the rest is tested; such an answer may be decided where a fresh check would run out of budget or abstain), and
`'none'` otherwise (a removed or changed axiom, or nothing decided kept). Axioms are compared by
the terms they name, an anonymous individual by its blank-node label, so `[]` nodes (which get
fresh labels on every parse) make every version count as changed. A version that is refused,
undecided or does not parse leaves the kept answer as it was, and the session holds only the
kept version's terms. The raw glue class is `OwlDlSession`
(`new OwlDlSession()`, `.check(tbox, abox, format, optionsJson) -> string`, `.free()`).

## Result shape (`OwlDlResult`)

```ts
{
  consistent: true | false | 'unknown',
  reason: null                                              // unless consistent is 'unknown'
    | { kind: 'OutOfFragment', construct: string, triples: string[] }
    | { kind: 'NotInDl', violations: OwlDlViolation[] }     // the blocking profile violations
    | { kind: 'ResourceBudget' }
    | { kind: 'DatatypeUnknown', triples: string[] }
    | { kind: 'Unconfirmed' }                                // a model with nominals not confirmed
    | { kind: 'ImportsNotLoaded', imports: string[] }        // owl:imports targets (not fetched)
    | { kind: 'Unparsed' | 'Malformed' | 'Ambiguous' | 'Unsupported', detail: string, triples: string[] },
  profile: {
    dl: true | false | 'unknown',            // true only with no violation AND nothing unchecked
    violations: { kind: OwlDlViolationKind, detail: string, triples: string[] }[],
    unchecked: { rule: string, triples: string[] }[],
  },
  unsatisfiable: string[],                   // named-class IRIs, sorted
  inferredSubClassOf: [string, string][],    // entailed [sub, super], not asserted, sorted
  classification: 'complete' | { unknown: 'ResourceBudget' | 'DatatypeUnknown' | 'Unconfirmed' } | null,  // null unless consistent
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
  A consistent one whose classification could not finish has `{ unknown: kind }`:
  `'ResourceBudget'` (the budget ran out), `'DatatypeUnknown'` (a class test met an undecided
  datatype question) or `'Unconfirmed'` (a class test met an unconfirmed model). Only the
  first is helped by a larger budget.

## Scope (honest)

The decided fragment is SROIQ(D), including the built-in properties (`owl:topObjectProperty`,
whose restrictions reach every individual, so an ontology using it is classified with the ABox
and more often answers `'Unconfirmed'`; `owl:bottomObjectProperty` and `owl:bottomDataProperty`,
which relate nothing; a property above `owl:topObjectProperty`, or a key or chain body
using it, is `OutOfFragment`): regular property
chains (`owl:propertyChainAxiom`) and transitivity, every class constructor
including nominals (`oneOf`, `hasValue` on object properties; with nominals the ABox takes
part in classification, so `A ≡ {a}` and `a ∈ B` give `A ⊑ B`), object properties with inverses and hierarchies, the role
characteristics (functional, inverse-functional, reflexive, irreflexive, symmetric,
asymmetric, disjoint), qualified cardinality up to the reasoner's cardinality bound
(`MAX_CARDINALITY`; larger numbers, and axioms needing more successors than the clause limit,
answer `OutOfFragment`; very deeply nested expressions are refused as `Unsupported`), and class / object-property / negative /
`sameAs` / `differentFrom` assertions, and data properties (domain, range, hierarchy,
disjointness, functional, some / all / has-value, data cardinalities, positive and negative
data assertions) over the OWL 2 datatype map with facet restrictions, `DataOneOf`, the data
booleans and acyclic `DatatypeDefinition`s. Literals are compared by value, and an ill-typed
literal in an assertion makes the ontology inconsistent (`"sixteen"^^xsd:nonNegativeInteger`).
Keys (`hasKey`) are decided over named individuals; a key whose data values are not
fixed to literals answers `'Unconfirmed'`. **Keys on a non-simple object property (transitive,
or implied by a chain or a transitive sub-property), a property hierarchy whose automaton
would exceed a fixed size,
datatypes and literals outside the OWL 2 datatype map, and a property used as both an
object and a data property currently answer `consistent: 'unknown'` with
`reason.kind: 'OutOfFragment'`** and the triples of the axiom that uses them; a datatype
question the reasoner does not decide (for example an XSD pattern with a block escape)
answers `reason.kind: 'DatatypeUnknown'` with the triples involved. Nominals are decided
without SHOIQ's NI-rule: when nominals tie a part of the candidate model that blocking
repeats to an individual whose number restrictions the repetition could break, the
answer is `reason.kind: 'Unconfirmed'` instead of a verdict. The answer is never a guess: every verdict is either
decided or `'unknown'` with a reason. Budgets are deterministic work counts (`maxSteps`: rule applications, `maxNodes`: completion-graph
nodes, at most 2^32 - 1), not time limits; the defaults are those of `sparq_reason_dl::owl2dl::Budget::default()`.
