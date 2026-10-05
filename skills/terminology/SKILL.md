---
name: terminology
license: MIT
description: 'The authoritative RDF/SPARQL terminology guide for all sparq documentation — the single source of truth that keeps wording consistent with the W3C specs. Use BEFORE writing or editing any doc, README, SKILL.md, research record, book page, or site copy that mentions RDF or SPARQL features. It lists the preferred term, the deprecated/banned term, and the spec citation for each — most importantly, say RDF 1.2 (the version/spec) and triple term / reifier / reified triple (the constructs), NEVER the community-era RDF-star / RDF* / SPARQL-star / quoted triple / embedded triple. Enforced in CI by scripts/check-terminology.py (the terminology HARD gate).'
---

# sparq terminology guide (RDF 1.2 / SPARQL 1.2)

This is the **single source of truth** for the RDF/SPARQL vocabulary used across
sparq's documentation. Its job is narrow and load-bearing: keep every doc, README,
`SKILL.md`, `research/` record, book page, and site string using the **current,
standardised** wording from the W3C specifications — not the older community-group
names that pre-date standardisation.

When you write or edit any doc that mentions an RDF or SPARQL feature, check the
preferred term here first. A `scripts/check-terminology.py` HARD gate (the
`terminology` job in `docs-quality.yml`) fails the build on the banned spellings, so
using the wrong term blocks the merge — fix the wording or, for a *legitimate*
historical / proper-noun mention, carry the inline `terminology-allow: <why>` marker
(see [§ Allowed exceptions](#allowed-exceptions)).

## Why this exists

The feature commonly nicknamed **"RDF-star"** / **"RDF\*"** began life in a W3C
*Community Group* (Hartig & Thompson, *"Foundations of RDF★ and SPARQL★ / Reification
Done Right"*, 2014). It is now **standardised in RDF 1.2** — and the standardised model
**renamed and re-scoped** the construct. Carrying the community-era names into our docs
is imprecise on two counts: it cites a non-spec name for the *version*, and it implies
the *old* (subject-and-object, `<<s p o>>`) model rather than the RDF 1.2
**object-position-only `<<( s p o )>>` triple term + `rdf:reifies` reifier** model.

## The terminology table

| Use this (spec-correct) | NOT this (deprecated / banned) | What it means | Spec citation |
|---|---|---|---|
| **RDF 1.2** | RDF-star, RDF\*, RDF star, RDF 1.2-star | The version/specification that standardised triple terms + reification. Use when naming the spec/version. | [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/) |
| **SPARQL 1.2** | SPARQL-star, SPARQL\*, SPARQL star | The query-language version that queries triple terms. | [SPARQL 1.2 Query](https://www.w3.org/TR/sparql12-query/) |
| **triple term** | quoted triple, embedded triple | "An RDF triple used as the object of another triple is called a triple term." Serialised `<<( s p o )>>` (the `tripleTerm` production). Object position only in RDF 1.2. | [RDF 1.2 Concepts § triple terms](https://www.w3.org/TR/rdf12-concepts/); [Turtle 1.2 `tripleTerm`](https://www.w3.org/TR/rdf12-turtle/) |
| **reifier** | — (no older name; do not call it a "quoted-triple subject") | "A reifying triple is a triple whose predicate is `rdf:reifies` and whose object is a triple term. The subject of that triple is the reifier." | [RDF 1.2 Concepts § reification](https://www.w3.org/TR/rdf12-concepts/) |
| **reified triple** | quoted triple (when reification is meant) | The `reifiedTriple` syntactic sugar `<< s p o >>` (note: **no parentheses** — distinct from a triple term's `<<( s p o )>>`). Expands to a reifier + `rdf:reifies` triple term. | [Turtle 1.2 `reifiedTriple`](https://www.w3.org/TR/rdf12-turtle/) |
| **`rdf:reifies`** | rdf:Statement / rdf:subject/predicate/object (RDF-1.1 reification) when RDF 1.2 reification is meant | The RDF 1.2 reification predicate linking a reifier to a triple term. | [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/) |

### Syntax: mind the parentheses

The two `<<…>>` forms are **not** interchangeable — this is a documented, load-bearing
distinction in Turtle 1.2:

- `<<( s p o )>>` — a **triple term** (the `tripleTerm` production). The thing itself.
- `<< s p o >>` — a **reified triple** (the `reifiedTriple` production; syntactic
  sugar that mints a reifier and a `rdf:reifies` triple term).

> "Note the difference in syntax between the syntactic sugar of `reifiedTriple` (i.e.
> `<< [...] >>`) and the regular `tripleTerm` (i.e. `<<( [...] )>>`)."
> — [Turtle 1.2](https://www.w3.org/TR/rdf12-turtle/)

When sparq's docs show the term form they should write `<<( s p o )>>`.

## Other precision rules (smaller, but spec-anchored)

These are not banned by the CI gate (too prone to false positives for a grep gate), but
prefer the precise term:

- **IRI** vs **URI**: RDF 1.2 graphs contain **IRIs** (RFC 3987), not URIs. Use "IRI"
  for an RDF term; reserve "URI"/"URL" for an HTTP endpoint or a protocol-level address.
  [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/)
- **RDF graph** vs **RDF dataset**: an **RDF graph** is a *set of RDF triples*; an **RDF
  dataset** is *one default graph + zero or more named graphs*. Do not say "dataset" when
  you mean a single graph, or "graph" when you mean the whole default-plus-named
  collection. [RDF 1.2 Concepts § datasets](https://www.w3.org/TR/rdf12-concepts/)
- **named graph**: a pair of *(an IRI or blank node — the graph name, an RDF graph)*.
  Use "named graph" only for the named members of a dataset, never as a synonym for the
  default graph. [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/)
- **blank node** (two words, lowercase): not "bnode" in prose (the `bnode` token is fine
  in code/identifiers), not "anonymous node". [RDF 1.2 Concepts](https://www.w3.org/TR/rdf12-concepts/)

## Banned terms beyond RDF/SPARQL wording

The gate is **data-driven**: every banned term lives in `scripts/banned-terminology.json`,
and each one declares its **own file surface**. Adding a term is one object there — no code
change. Terms currently enforced beyond the RDF 1.2 / SPARQL 1.2 table above:

| Banned | Say instead | Surface |
|---|---|---|
| the trust-container term *(`TrustEnvelope`, `trust_envelope`, `trust-envelope`, "trust envelope")* <!-- terminology-allow: the guide must name the banned term in order to ban it --> | **"trust requirements"** for the TR document; a request/response-symmetric carrier name (e.g. `ContractRequest`) for the *(query, requirements, nonce)* triple | source **and** vocabulary: `.rs`, `.ttl`, `.md`, `.typ`, `.nr`, manifests, workflows |

Two things that rule out a blind find-and-replace here:

- "trust requirements" already names a **distinct** type (`TrustRequirements`, the TR
  graph). The carrier is a *superset* of TR, so it needs its own non-banned name.
- Only the **compound** is banned. Bare "envelope" is legitimate and common elsewhere
  (leakage envelope, JSON envelope, SD-JWT envelope, noise envelope, console envelope) and
  is deliberately **not** matched.

Why the surface matters: this gate used to scan `*.md` only, so a banned term reached a
merged-ready PR as a `pub` Rust type **and** a published `rdfs:comment` with fully
green CI (issue #3811). A term banned from public API must be checked in the
files that carry public API.

## Allowed exceptions

The banned spellings are *correct* in a few legitimate places. The CI gate
(`scripts/check-terminology.py`) carries built-in exemptions for them, and any other
genuine case can carry an inline `terminology-allow: <why>` marker on the line:

1. **Proper nouns** — the W3C **"RDF-star Working Group"** (and its repo
   `w3c/rdf-star-wg`, the **"RDF-star Community Group"**) are the real, citable names of
   those bodies. Do not rewrite them.
2. **Prior-art titles** — Hartig & Thompson's paper *"Foundations of RDF★ and SPARQL★"*
   is a fixed citation title. Quote it verbatim.
3. **Third-party product/doc names** — when describing another system's historical
   support, its own documentation name is a fact (e.g. RDF4J's `rdfstar` doc path,
   GraphDB's `rdf-sparql-star` page, Jena's `rdf-star` docs). Cite the real name, then
   describe the standardised feature in our own words as "triple terms (RDF 1.2)".
4. **External URLs / API identifiers** — a link or an identifier that literally contains
   `rdf-star` is the resource's real address; do not edit the URL.

In all four cases the *surrounding sparq prose* should still use the spec-correct term;
only the proper noun / title / URL itself keeps the historical spelling.

## When you cannot decide

If you are unsure whether a mention is a banned usage or a legitimate historical /
proper-noun reference, **do not guess** — leave it, flag it in your report, and let a
maintainer decide. Silent flattening of a precise prior-art reference is as much a
documentation defect as leaving a deprecated term in current-state copy.
