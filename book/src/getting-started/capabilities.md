<!-- [OPUS-4.8] sq-g4h0c — capability link-LIST single-sourced (build-time content
injection). The list below is {{#include}}d from the README's `features-core` ANCHOR
and the Interfaces line from the README's `interfaces` ANCHOR — so the docs guide can no
longer drift from the README. The README keeps those per-surface guides as REPO-RELATIVE
`skills/<x>/SKILL.md` links (required by the lychee internal-links gate); the book's
`link-fixup` preprocessor (scripts/mdbook-rewrite-links.py, wired in book.toml) rewrites
them to mount-portable github.com/sparq-org/sparq/blob/main URLs at build time, which is what
resolves the relative-vs-portable conflict that previously forced a hand-maintained table
of absolute URLs here.

[OPUS-4.8] sq-im8u — the two research-scaffold maturity caveats below are {{#include}}d
verbatim from their canonical skills/*/SKILL.md `scaffold-caveat` anchors, so their
not-yet-audited / not-yet-hardened hedges stay intact at the source (privacy-claims gate;
sq-toze.35 / sq-qhy4). Do not weaken the included hedges or restate the guarantees inline.

[OPUS-4.8] sq-tfpq / issue #813 — the SPARQL-Update note documents the 1.2 triple-term
delta (no new ops); honest scoping kept (engine write tests, not a formal conformance
run). It is book-only prose (no README counterpart to single-source from). -->

# Capabilities at a glance

The engine core is always built; every capability below is an **opt-in** crate or feature that the
core does not depend on, so the core stays lean. Each links the standard it implements and its
usage guide.

{{#include ../../../README.md:features-core}}

## A note on SPARQL Update 1.1 vs 1.2

SPARQL 1.1 Update is fully supported: `INSERT DATA`, `DELETE DATA`, `DELETE`/`INSERT … WHERE`
(with `USING` / `WITH`), `LOAD`, `CLEAR`, `CREATE`, `DROP`, `COPY`, `MOVE`, and `ADD`, with
request-level atomicity for `;`-separated bodies.

[SPARQL 1.2 Update](https://www.w3.org/TR/sparql12-update/) adds **no new operations** over 1.1;
its substantive change is the [RDF 1.2](https://www.w3.org/TR/rdf12-concepts/) triple-term
semantics inside `INSERT DATA` / `DELETE DATA` and `DELETE`/`INSERT … WHERE` — inserting or
deleting a reifying triple (`rdf:reifies <<( s p o )>>`) operates on that triple term itself and
does **not** automatically assert or retract the asserted triple it refers to. sparq handles this:
triple terms are stored and matched as structural object terms, so a reifying triple is added or
removed as an exact term with no coupling to its asserted counterpart. This behaviour is exercised
by the engine's write tests (`crates/sparq-engine/tests/rdfstar_write.rs`); it has not been checked
against a formal SPARQL 1.2 conformance suite.

## Research scaffolds (no security guarantee yet)

Two capabilities are **research scaffolds**. They are honest models of the protocols, but they do
**not** yet provide the cryptographic guarantee a relying party would need. Treat any engineering
numbers as indicative, not as an audited cryptographic guarantee. The two maturity caveats below
are single-sourced verbatim (build-time `{{#include}}`) from their canonical guides, so they
cannot drift from the source of truth — see the
[zk-query-proofs guide](https://github.com/sparq-org/sparq/blob/main/skills/zk-query-proofs/SKILL.md),
the [mpc guide](https://github.com/sparq-org/sparq/blob/main/skills/mpc/SKILL.md), and
[SECURITY.md](https://github.com/sparq-org/sparq/blob/main/SECURITY.md) for the full scope.

**Zero-knowledge query proofs** —

{{#include ../../../skills/zk-query-proofs/SKILL.md:scaffold-caveat}}

**Federated MPC** —

{{#include ../../../skills/mpc/SKILL.md:scaffold-caveat}}

## Interfaces

{{#include ../../../README.md:interfaces}}
