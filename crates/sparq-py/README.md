<!-- [OPUS-4.8] sq-inzv: full-template README — the published sparq-rdf Python bindings. -->
# sparq (Python bindings)

Python bindings for the [sparq](https://github.com/sparq-org/sparq) RDF + SPARQL engine:
a dictionary-encoded triplestore with six permutation indexes, a SPARQL 1.1 query
engine (SELECT / ASK / CONSTRUCT / DESCRIBE), SPARQL Update over the full dataset
(named graphs included), opt-in RDFS / OWL-RL / Notation3 reasoning with OWL
inconsistency reporting, and opt-in BM25 full-text search — compiled to a native
extension with [pyo3](https://pyo3.rs) and packaged with [maturin](https://www.maturin.rs)
(abi3: one wheel per platform covers CPython ≥ 3.9).

> Distributed via PyPI, not crates.io (`publish = false`). It ships to **PyPI as the
> distribution `sparq-rdf`** (`pip install sparq-rdf`), with **import name `sparq`**
> (`import sparq`) — the bare `sparq` PyPI name is taken by an unrelated package; the
> importable module is unaffected.

## 🚀 Quickstart

```sh
pip install sparq-rdf      # then: import sparq
```

```python
import sparq

g = sparq.Graph.load("""
    @prefix ex: <http://ex/> .
    ex:alice ex:knows ex:bob ; ex:age 30 .
    ex:bob   ex:age 25 .
""")                                   # format defaults to "turtle"
len(g)                                 # number of triples

# SELECT -> QueryResult: .vars + .rows (list of {var: Term} dicts).
res = g.query("PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY ?a")
for row in res:
    print(row["s"].value, row["a"].value)   # Term: .kind, .value, .language, .datatype

g.query_json("SELECT * WHERE { ?s ?p ?o }")  # fast path: SPARQL 1.1 JSON str
g.ask("PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }")   # -> True
```

## ✨ Features

- **All four query forms are native.** SELECT (`query` / `query_json`), ASK (`ask`),
  CONSTRUCT (`construct` → `(s, p, o)` Term triples), DESCRIBE (`describe`,
  concise-bounded-description semantics). Named graphs from N-Quads/TriG or updates
  are queryable via `GRAPH`.
- **Load Turtle / N-Triples / N-Quads / TriG / JSON-LD.** `Graph.load(source,
  format=...)` (default: inferred from the `.ttl`/`.nt`/`.nq`/`.trig`/`.jsonld`
  extension, else `"turtle"`). **JSON-LD is on by default** (`format="jsonld"` and a
  `.jsonld` path both work, `@graph` named graphs preserved) — a wheel built with
  `--no-default-features` drops it (then `format="jsonld"` errors).
- **SPARQL Update, applied in place** — INSERT/DELETE DATA, DELETE/INSERT … WHERE,
  CLEAR/DROP/CREATE/ADD/COPY/MOVE. Named graphs are fully supported (GRAPH-scoped
  data and templates, USING (NAMED), graph management). `update()` / `reason()` /
  `reason_n3_with()` rebuild the immutable store (O(n) per call).
- **Opt-in reasoning** — `g.reason("rdfs")` / `g.reason("owl")` materialize the
  closure over the **default graph** in place (named graphs carried across
  untouched) and return the entailed-triple count; `g.inconsistencies()` reports the
  OWL 2 RL clash list. Notation3 rules load via `sparq.Graph.load_n3(...)` or apply
  to an existing graph with `g.reason_n3_with(rules)` — the graph's blank nodes are
  renamed under a reserved `sparqg` prefix first, so a rule's blank-node label
  cannot alias an existing data node; RDF 1.2 triple terms have no N3 form and are
  rejected there.
- **Opt-in BM25 full-text search** — `g.text_search("ali*")` (ranked
  `[(Term, score), …]`; `any=True` for OR, `limit=n` for top-n) and the `text:`
  magic predicates inside plain SPARQL via `g.query_text(...)` (`text:matches` AND,
  `text:matchesAny` OR, `*`-suffix prefixes, `text:score` binds BM25). The index
  covers the **default graph's** string literals, is built lazily, cached, and
  invalidated by every mutating call.
- **Cheap structural copy + persistence.** `g.copy()` is a logically-independent
  Arc-shared snapshot (O(pending delta), not O(triples)); the original and copy
  mutate separately. `g.save("./mydb")` / `sparq.Graph.open("./mydb")` persist and
  reopen with memory-mapped indexes (out-of-core path).
- **GIL-friendly.** Long-running calls (load, query, update, reason, text search)
  release the GIL. `Graph.load` treats a `str` as a **file path** only when a file
  with that name exists and the string has no newline; `os.PathLike` is always a
  path; otherwise the string is parsed as RDF content.
- **Opt-in Arrow export** — `g.query_arrow(sparql)` returns a `pyarrow.Table` (one
  `struct<kind,value,datatype,language,direction>` column per variable; unbound → null
  struct slot) over the merged `sparq-arrow` projection, bridged through the Arrow C
  Data Interface (no re-serialisation). OFF by default, so the lean wheel pays nothing.
  v1 boundary: no numeric narrowing yet, RDF 1.2 triple terms stringified. The `arrow`
  extra (`pip install sparq-rdf[arrow]`) pulls in the `pyarrow ≥ 14` consumer side, but
  `query_arrow` only EXISTS in a wheel built with the cargo `arrow` feature
  (`maturin build --features arrow`). Which published PyPI wheels ship with `arrow`
  baked in is the broader release-matrix question (sq-v286); until then, build/install
  an `--features arrow` wheel yourself.

## 📚 Learn more

- **How-to** — [`skills/python/SKILL.md`](../../skills/python/SKILL.md).
- **Develop** — `maturin develop` (debug build into the venv) + `pytest tests`.
  Release wheels: `maturin build --profile python-release` (release with unwinding
  panics — the workspace default `release` profile is `panic = "abort"`, which would
  turn a Rust panic into a hard interpreter abort).
- **Contribute** — [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
