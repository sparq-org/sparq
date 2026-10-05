---
name: python
description: "Use the sparq RDF + SPARQL engine from Python (PyPI distribution `sparq-rdf`; `import sparq`). Reach for this when an agent or developer needs to load RDF (Turtle/N-Triples/N-Quads/TriG/JSON-LD), run SPARQL 1.1 SELECT/ASK/CONSTRUCT/DESCRIBE, apply SPARQL Update, do opt-in RDFS/OWL-RL/Notation3 reasoning + OWL inconsistency checks, run BM25 full-text search (text: magic predicates), or persist/memory-map a triplestore — all via the pyo3 sparq.Graph class."
---

# sparq (Python)

Python bindings (pyo3 + maturin) for the sparq RDF+SPARQL engine: a dictionary-encoded triplestore with SPARQL 1.1 query/update over the full dataset (named graphs included), opt-in RDFS / OWL-RL / Notation3 reasoning with OWL inconsistency reporting, and opt-in BM25 full-text search. Everything is reached through one class, `sparq.Graph`. The PyPI **distribution** name is `sparq-rdf` (`pip install sparq-rdf`), but the **import** name is `sparq` (`import sparq`) — the two intentionally differ because the bare `sparq` distribution name is taken on PyPI by an unrelated package.

[GPT-6] Query entry points and structural rewrites retain VERSION announcements;
see the [version-pinned EBV rules](../sparql-query/ebv-dialects.md). Unsupported
or incompatible labels do not silently select REC 2013.

## Quickstart

Install (no published wheel may be available yet — Alpha; build from the repo with maturin):

```sh
python3 -m venv .venv-py && . .venv-py/bin/activate
pip install maturin pytest
cd crates/sparq-py
maturin develop            # debug build into the venv (use `maturin develop --release` for speed)
pytest tests               # run the test suite
```

Then:

```python
import sparq

# Load from an RDF string OR a file path (str / pathlib.Path). format defaults to "turtle".
g = sparq.Graph.load("""
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
    ex:bob   ex:age 25 ; ex:name "Bob"@en .
    ex:carol ex:age 35 .
""")
print(len(g))                          # 7  (default-graph triple count)

# SELECT -> QueryResult: .vars (projection order) + .rows (list of {var: Term} dicts).
res = g.query("PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY ?a")
print(res.vars)                        # ['s', 'a']
for row in res:                        # QueryResult is iterable/indexable (sequence protocol)
    print(row["s"].value, row["a"].value)   # http://ex/bob 25  ...
# Unbound variables (e.g. from OPTIONAL) are simply ABSENT from the row dict — use `"k" in row`.
```

## Key APIs

All on the `sparq.Graph` class (the only entry point). Constructors are static methods:

- `Graph.load(source, format=None) -> Graph` — `source` is an RDF string OR a file path (`str` is a path only if a file with that name exists and the string has no newline; `os.PathLike`/`pathlib.Path` is always a path). `format` ∈ `"turtle"` | `"ntriples"` | `"nquads"` | `"trig"` | `"jsonld"` (default: inferred from extension `.nt`/`.nq`/`.trig`/`.jsonld`, else `"turtle"`). Named graphs from N-Quads/TriG/JSON-LD are preserved and queryable via `GRAPH`. **JSON-LD is on by default** in the wheel (`format="jsonld"`, `"json-ld"`, or `"application/ld+json"`; a `.jsonld` path); a wheel built `--no-default-features` drops it and `format="jsonld"` then errors (fail-closed, not mis-parsed as Turtle).
- `Graph.load_n3(text) -> Graph` — parse an N3 document (facts + `{ premise } => { conclusion }` rules) and forward-chain to fixpoint; the result holds the ground closure.
- `Graph.open(dir) -> Graph` — reopen a graph persisted by `save()`; permutation indexes are memory-mapped (paged on demand, so larger-than-RAM is fine).
- `g.save(dir) -> None` — persist indexes + dictionary into `dir` for later `Graph.open`.
- `g.query(sparql) -> QueryResult` — SPARQL SELECT, materialised.
- `g.query_json(sparql) -> str` — SELECT as a SPARQL 1.1 JSON results document string (fast path; skips per-cell Term objects).
- `g.query_arrow(sparql) -> pyarrow.Table` — SELECT as a `pyarrow.Table` (**opt-in**: a wheel built with the `arrow` feature / `pip install sparq-rdf[arrow]`, and `pyarrow` ≥ 14 installed). One Arrow `struct<kind, value, datatype, language, direction>` column per variable — the faithful, round-trippable RDF-term projection of the `sparq-arrow` crate, bridged through the Arrow C Data Interface (no re-serialisation). An UNBOUND binding is a null struct slot (distinct from a bound empty-string literal). Same v1 boundary as `sparq-arrow`: no numeric narrowing yet (`42^^xsd:integer` is `"42"` + a `datatype`, not an Arrow `Int64`), RDF 1.2 triple terms stringified to N-Triples in `value`. Only present when the wheel was built with the feature; a missing `pyarrow` raises `ImportError`.
- `g.ask(sparql) -> bool` — ASK query (a SELECT is also accepted: True iff it yields ≥1 row).
- `g.construct(sparql) -> list[(Term, Term, Term)]` — CONSTRUCT; deduplicated, first-production order.
- `g.describe(sparql) -> list[(Term, Term, Term)]` — DESCRIBE with concise-bounded-description (CBD) semantics.
- `g.update(sparql) -> None` — SPARQL Update applied IN PLACE (INSERT/DELETE DATA, DELETE/INSERT…WHERE, CLEAR/DROP/CREATE/ADD/COPY/MOVE; GRAPH-scoped data + templates, USING (NAMED)). On error the graph is left unchanged.
- `g.reason(profile) -> int` — materialise the closure in place for `profile` ∈ `"rdfs"` | `"owl"` (OWL 2 RL subset, includes RDFS); returns number of NEW triples added; idempotent. `"n3"` is rejected here (use `load_n3`/`reason_n3_with`).
- `g.reason_n3_with(rules) -> int` — forward-chain caller-supplied N3 `rules` over this graph in place; returns triples added.
- `g.inconsistencies() -> list[str]` — OWL 2 RL clash descriptions over ASSERTED triples (run `reason("owl")` first to surface entailed clashes); `[]` = none detected.
- `g.text_search(query, any=False, limit=None) -> list[(Term, float)]` — BM25 search over the default graph's string literals; best-first `(literal Term, score)`. AND of tokens by default; `any=True` is OR; `*`-suffix token = prefix match; `limit` keeps top-n.
- `g.query_text(sparql) -> QueryResult` — SELECT that may use `text:` magic predicates (see recipe below).
- `g.build_text_index() -> int` / `g.drop_text_index() -> None` — eagerly build (returns indexed-literal count) / free the cached full-text index.
- `g.copy() -> Graph` — a cheap, logically-independent copy (over the core's Arc-shared structural snapshot, O(pending delta), not O(triples)); the original and the copy can then be mutated separately (`update`/`reason`/`reason_n3_with` on one leaves the other untouched). Named graphs are copied; the copy lazily rebuilds its own full-text index.
- `len(g)` — default-graph triple count; `repr(g)` → `"Graph(N triples)"`.

`QueryResult`: `.vars: list[str]`, `.rows: list[dict[str, Term]]`, plus `len(res)`, `res[i]` (negative indices/slices ok), and iteration.

`Term` (frozen, value-based `==`/`hash`): `.kind` ∈ `"uri"` | `"literal"` | `"bnode"` | `"triple"` (RDF 1.2 triple term); `.value` (IRI / lexical form / bnode label); `.language` (literals only — the bare BCP-47 tag, else `None`); `.datatype` (literals only — always set: plain → `xsd:string`, lang-tagged → `rdf:langString`, directional → `rdf:dirLangString`; else `None`); `.direction` (RDF 1.2 base direction `"ltr"` / `"rtl"` of a directional language string — the SPARQL 1.2 `its:dir`; `None` otherwise). `str(term)` → bare `.value`; `repr(term)` → N-Triples-ish (e.g. `Term("Bob"@en)`, `Term("hi"@en--ltr)`).

`sparq.__version__` is the package version string.

## Common recipes

ASK and the JSON fast path:

```python
g.ask('PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }')   # True
import json
doc = json.loads(g.query_json("SELECT ?s WHERE { ?s ?p ?o }"))      # SPARQL 1.1 JSON results
```

Arrow / dataframes (opt-in `arrow` wheel + `pyarrow`): one struct column per variable, ready for Polars/DuckDB/pandas without a CSV round-trip:

```python
# pip install sparq-rdf[arrow]    (wheel built with --features arrow; needs pyarrow >= 14)
table = g.query_arrow("PREFIX ex: <http://ex/> SELECT ?s ?age WHERE { ?s ex:age ?age }")
table.column_names                    # ['s', 'age'] — each an Arrow struct<kind,value,datatype,language,direction>
table.to_pylist()[0]["age"]           # {'kind': 'literal', 'value': '30', 'datatype': '...#integer', ...}
# An unbound (OPTIONAL) cell reads back as a null struct slot (None), not an empty literal.
import polars as pl; pl.from_arrow(table)     # straight into a dataframe
```

CONSTRUCT / DESCRIBE return Term triples (template triples with an unbound var or an illegal RDF position are silently dropped):

```python
for s, p, o in g.construct(
    "PREFIX ex: <http://ex/> CONSTRUCT { ?s a ex:Person } WHERE { ?s ex:age ?a }"
):
    print(s.value, p.value, o.value)
g.describe("DESCRIBE <http://ex/alice>")        # CBD: recurses through blank-node objects
```

Update in place, including named graphs (default graph survives named-graph ops and vice versa):

```python
g.update('INSERT DATA { <http://ex/dave> <http://ex/age> 40 }')
g.update('INSERT DATA { GRAPH <http://ex/g1> { <http://ex/a> <http://ex/p> <http://ex/b> } }')
g.ask('ASK { GRAPH <http://ex/g1> { ?s ?p ?o } }')                  # True
g.update('DROP GRAPH <http://ex/g1>')
```

Reasoning (RDFS / OWL-RL / N3) and consistency:

```python
added = g.reason("rdfs")             # entailed triples added; reason("owl") includes RDFS
g.inconsistencies()                  # OWL 2 RL clash report (run reason("owl") first for entailed clashes)

# N3 rules + facts in one document:
g2 = sparq.Graph.load_n3("""
    @prefix ex: <http://ex/> .
    ex:socrates a ex:Man .
    { ?x a ex:Man } => { ?x a ex:Mortal } .
""")
g2.ask("PREFIX ex: <http://ex/> ASK { ex:socrates a ex:Mortal }")   # True

# ...or apply N3 rules to an already-loaded graph in place (returns triples added):
g3 = sparq.Graph.load("@prefix ex: <http://ex/> . ex:plato a ex:Man .")
g3.reason_n3_with("@prefix ex: <http://ex/> . { ?x a ex:Man } => { ?x a ex:Mortal } .")
```

Full-text search — direct ranked hits, and the `text:` magic predicates inside SPARQL:

```python
g.text_search("ali*")                          # [(Term, score), ...]; AND of tokens, * = prefix
g.text_search("alice bob", any=True, limit=5)  # OR, top-5

res = g.query_text("""
    PREFIX text: <http://sparq.dev/text#>
    SELECT ?s ?score WHERE {
        ?s ?p ?lit .
        ?lit text:matches "ali*" .          # AND of tokens (text:matchesAny = OR)
        ?lit text:score ?score              # binds BM25 score as xsd:double
    } ORDER BY DESC(?score)                 # hit table carries no order through joins — sort explicitly
""")
```

The `text:matches` literal must be a constant string (not a variable); unknown `text:` predicates raise `ValueError`.

Persist and reopen (out-of-core, memory-mapped indexes):

```python
g.save("./mydb")
g4 = sparq.Graph.open("./mydb")
```

## Gotchas / prerequisites

- **Alpha + build-from-source.** Package is Development Status 3 - Alpha; `requires-python >= 3.9`. The wheel is abi3-py39 (one wheel per platform covers CPython ≥ 3.9). Build it with `maturin develop` (dev) from `crates/sparq-py`; for distributable wheels use `maturin build --profile python-release` — the workspace's plain `release` profile is `panic = "abort"`, which would turn any Rust panic into a hard interpreter abort, so do NOT use a plain release build for the wheel.
- **Errors map to `ValueError`** (parse/eval failures) and **`IOError`/`OSError`** (file read / `open` missing dir). Catch `ValueError` around `query`/`update`/etc.
- **`load(str)` path-vs-data heuristic:** a `str` is treated as a file path ONLY if a file by that name exists and the string has no newline; otherwise it is parsed as RDF. Pass `pathlib.Path` to force file semantics.
- **`update()` / `reason()` / `reason_n3_with()` rebuild the immutable store (O(n) per call)** and mutate the `Graph` in place. Reasoning materialises over the **default graph**; named graphs are carried across untouched. `len(g)` counts default-graph triples only.
- **Full-text indexes the default graph's plain + language-tagged string literals only** (typed literals like `42`/`xsd:integer` and named graphs are NOT indexed). The index is built lazily on first `text_search`/`query_text`, cached, and **invalidated by every mutating call** (`update`/`reason`/`reason_n3_with`) — the next text call rebuilds it. `query_text` without any `text:` pattern behaves exactly like `query`.
- **`reason_n3_with(rules)`:** the graph's blank nodes are renamed under a reserved `sparqg` prefix before composition, so a blank-node label in the rules can NOT alias an existing data node. RDF 1.2 triple terms have no N3 form and are rejected there.
- **No Cargo feature flags to set as a user:** the wheel already enables `sparq-core` `mmap` (for `save`/`open`), plus `sparq-reason` and `sparq-text`. The ZK / Noir estate is NOT part of the Python surface.
- **GIL:** long-running calls (load, query, update, reason, text search) release the GIL, so other Python threads keep running.

## See also

- `cli` — the `sparq` command-line tool over the same engine.
- `js` / `wasm` — the browser/Node WASM bindings (mirror surface).
- `core` / `engine` — the underlying Rust crates (`sparq-core`, `sparq-engine`) if you need the native API.
- `reasoning` / `text-search` — deeper coverage of RDFS/OWL-RL/N3 and BM25 `text:` semantics.
