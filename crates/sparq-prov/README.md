# sparq-prov

W3C **[PROV-O]** data-lineage records for data **derived** by a sparq operation —
a small, **opt-in** public API over sparq's term model.

When a sparq operation *derives* new triples from existing data, this crate
optionally records *who/what/when* produced them, as standard PROV-O RDF:
a `prov:Activity` (the operation, time-stamped), a `prov:Entity` (the result),
and the lineage edges `prov:wasGeneratedBy`, `prov:used`, `prov:wasDerivedFrom`.

[PROV-O]: https://www.w3.org/TR/prov-o/

> Bead sq-ntcg · CDMC capability gap **CD-1** (first-class data lineage).

## 🚀 Quickstart

```rust
use sparq_core::Graph;
use sparq_prov::{derive_construct, ProvConfig};
use oxrdf::NamedNode;

let g = Graph::load_str(
    "@prefix ex: <http://ex/> . ex:alice ex:age 30 .", "turtle",
).unwrap();

// Run a CONSTRUCT and capture its PROV-O lineage, naming the input source.
let config = ProvConfig::with_inputs([NamedNode::new_unchecked("http://ex/src")]);
let d = derive_construct(
    &g,
    "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
    config,
).unwrap();

let derived = d.triples();          // the data the operation produced
let lineage = d.prov_graph();       // its PROV-O record (Vec<Triple>)
let turtle  = d.prov_turtle();      // …serialised as prefix-compacted Turtle
let nt      = d.prov_ntriples();    // …or canonical N-Triples
```

## ✨ Features

- **CONSTRUCT/DESCRIBE lineage** — [`derive_construct`] wraps the engine's
  `CONSTRUCT`/`DESCRIBE` evaluation, times it, and returns a [`Derivation`]
  carrying both the derived triples and a PROV-O lineage graph.
- **SPARQL UPDATE lineage** — [`derive_update`] applies an `INSERT … WHERE` /
  `INSERT DATA` / `DELETE …` in place, captures the engine's *resolved* effect log,
  and returns an [`UpdateDerivation`]. The two-sided PROV reading: the **inserted**
  triples are *generated* (a `prov:Entity` `wasGeneratedBy` the update,
  `wasDerivedFrom` the matched inputs); the **deleted** triples are *invalidated*
  (`prov:wasInvalidatedBy`) — retraction is not derivation, so deletes are never
  claimed as generated/derived. A pure-delete update generates nothing (no result
  entity). Structural ops (`CLEAR`/`DROP`/`CREATE`) are recorded as an activity kind
  but carry no per-triple entity (a deliberate honesty boundary — see below).
- **Standard PROV-O shape** — for result entity `E`, activity `A`, inputs `Iᵢ`:
  `A a prov:Activity` · `E a prov:Entity` ·
  `A prov:startedAtTime/endedAtTime "…"^^xsd:dateTime` ·
  `E prov:wasGeneratedBy A` · `A prov:used Iᵢ` · `E prov:wasDerivedFrom Iᵢ` ·
  (optional) `A prov:wasAssociatedWith <agent>`. Every IRI is absolute, so the
  graph is valid PROV-O that round-trips through any RDF parser.
- **Opt-in, zero core overhead** — a standalone member (like `sparq-canon`):
  nothing in sparq's default build or the wasm artifact depends on it, so the
  capability is **off by default** and the lean core is byte-identical without
  it. Pull it in explicitly only where you need lineage.
- **Configurable identity** — IRIs default to stable, content-addressed
  `urn:sparq:prov:` nodes (same derivation ⇒ same IRIs); set `ProvConfig`
  `activity`/`entity`/`used`/`agent` to integrate with an external provenance
  store or named-graph scheme. The clock is injectable for deterministic tests.
- **Reasoner-materialization lineage** (`reason` feature) — `prov_from_proof`
  maps a `sparq-reason` `why()` proof tree to PROV-O: one `prov:Entity` per
  inferred fact, one `prov:Activity` per rule firing (labelled `cax-sco` /
  `rdfs9` / `prp-trp` / `n3-rule-i` / …), with `wasGeneratedBy` / `used` /
  `wasDerivedFrom` edges. Inference *is* derivation, and the proof tree is a
  *finer-grained* provenance than a single CONSTRUCT activity (it names the rule
  and exact premises for each fact). Entity/activity IRIs are content-addressed,
  so lineage from overlapping proofs **stitches** into one DAG. Non-default
  feature: the `sparq-reason` dep is pulled only when you ask for `reason`.
- **Missing-answer explanation** (`why-not` feature) — `why_not` accepts one
  basic graph pattern (BGP) plus a fully-ground target binding and reports every
  substituted triple absent from the graph, in BGP order. It returns an empty
  vector when every conjunct is present. `why_not_report_ntriples` /
  `why_not_report_turtle` turn that vector into deterministic RDF 1.2: one
  `prov:Entity` per missing conjunct carrying its exact grounded triple term and
  an `urn:sparq:prov:absent true` marker. `OPTIONAL`, `UNION`, `FILTER`, property
  paths, named graphs, and all other algebra fail closed as unsupported.
  <!-- sq-lsp7k -->
- **Dependency-light** — `xsd:dateTime` is formatted in-crate (no `chrono`/
  `time` dep); the formatter is the inverse of `sparq-core`'s dateTime parser,
  so a recorded timestamp parses back to the same instant (tested).

## Scope — covered vs deferred

| Derivation path | Status |
|---|---|
| `CONSTRUCT` / `DESCRIBE` (query → new graph) | ✅ covered here (`derive_construct`) |
| Reasoner materialization (RDFS / OWL-RL / N3) | ✅ covered (`reason` feature) — reuses `sparq-reason`'s per-fact `why()` proof trees: a *finer-grained* derivation provenance than PROV-O alone |
| SPARQL UPDATE data ops (`INSERT … WHERE`, `INSERT DATA`, `DELETE …`, `LOAD`) | ✅ covered here (`derive_update`) — inserts ⇒ generated/derived, deletes ⇒ `wasInvalidatedBy` |
| SPARQL UPDATE structural ops (`CLEAR` / `DROP` / `CREATE`) | ⛔ no per-triple entity (deliberate boundary — they change graph existence/emptiness, not triples-as-data; recorded only as the activity kind) |
| Missing target binding for one BGP | ✅ covered under the non-default `why-not` feature; other algebra is unsupported |

The CONSTRUCT path is the cleanest, best-tested derivation in the engine and the
natural first PROV-O target; reasoner materialization reuses the existing proof
trees; UPDATE lineage reads the engine's resolved effect log so capture is exact
even for non-deterministic update text. sq-m3i0, sq-xwdd

## 📚 Learn more

- W3C PROV-O: <https://www.w3.org/TR/prov-o/>
- Skill: `skills/prov-lineage/SKILL.md`
- Hartig provenance research: `research/feature-research-hartig.md` (§6)
- CDMC CD-1 gap: `compliance/cdmc/gap-register.md`

## License

MIT — `publish = false` workspace member.
