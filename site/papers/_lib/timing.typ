// [OPUS-5] sq-gum8.16 (paper factory F4) — the `canonical-timing` accessor.
//
// `_lib/bench.typ` gates DETERMINISTIC evidence (`headline()` refuses anything that is not
// `environment: "canonical"`). This file is the separate, stricter door for MEASURED
// wall-clock numbers, which carry an extra obligation deterministic values do not: a timing is
// meaningless without the machine, the corpus, the aggregate and the commit it was taken on.
//
// THE INVARIANT: the only way a timing reaches the page is `headline_timing(key)` or
// `timing_provenance(key)`, both of which always render the provenance alongside the number.
// So a paper cannot show a measured number stripped of its context, and cannot show one at all
// unless it came from an envelope that self-declared `"canonical": true` and passed the count
// check in `site/scripts/sync-canonical-timing.mjs`. Work-box timings are NON-CANONICAL: they
// live in `benchmarks.generated.json` as `environment: indicative` and have no path into here.
//
// HOW THE INVARIANT IS ENFORCED — read this before adding a binding. Typst has no private
// module bindings: EVERY top-level `#let` in this file is importable, so the `_`-prefix on
// `_timing_data` / `_trec` / `_envelope` is a CONVENTION and cannot by itself stop a paper
// writing `#import "_lib/timing.typ": _trec` and publishing `_trec(key).value`. The real,
// mechanical enforcement is the build-time SOURCE GATE `runTimingSourceGate()` in
// `site/scripts/build-papers.mjs`, which fails the build when a paper source imports anything
// from this file other than the provenance-rendering entry points, imports it as a whole
// module, or reads the generated JSON directly. `site/test/paper-timing.test.mjs` tests that
// gate against a negative fixture that tries exactly those bypasses.
//
// `timing_keys()` is deliberately the ONLY other export: it returns record KEYS, never values,
// so the self-check fixture can pick a key to exercise without ever holding a raw number.
//
// DATA SOURCE: `site/src/data/paper-timing.generated.json`, read from the Typst project root
// (`--root site/`) rather than injected via `--input`. Both the PDF and the in-site HTML
// compile from that same committed file in the same build, so the two artifacts still cannot
// disagree — and unlike a `--input` payload the file is not bounded by the OS single-argument
// size limit, which a growing set of canonical envelopes would eventually hit.
//
// HONEST SCOPE: this accessor guarantees value↔envelope provenance. It does NOT make a
// comparison fair, and it deliberately offers no ratio/speed-up helper — a cross-engine ratio
// is a CLAIM that needs prose framing (which corpus, which queries, what was excluded), so it
// belongs in the paper's own text under the Stage-5 claims↔evidence review, not in a helper
// that would make it look derived.
//
// USAGE:
//   #import "_lib/timing.typ": headline_timing, timing_provenance
//   sparq answers SP2Bench q01 in #headline_timing("timing.sp2b.sparq.q01").
//   #timing_provenance("timing.sp2b.sparq.q01")   // the full block-level provenance

#let _timing_data = json("/src/data/paper-timing.generated.json")

// The derived record keys, sorted. Keys only — a caller cannot reach a value through this.
#let timing_keys() = _timing_data.records.keys().sorted()

// Raw record lookup — panics loudly on an unknown key so a typo (or a record that dropped out
// of the derivation because its envelope stopped qualifying) fails the build instead of
// silently rendering nothing.
#let _trec(key) = {
  let r = _timing_data.records.at(key, default: none)
  if r == none {
    panic(
      "paper-factory: unknown canonical-timing key '" + key + "'. This file is DERIVED — " +
      "run `npm run sync-canonical-timing` and check that a canonical envelope under " +
      "bench/canonical-competitor-results/** actually carries this suite/engine/query " +
      "(see the `skipped` list in site/src/data/paper-timing.generated.json).",
    )
  }
  if r.environment != "canonical-timing" {
    panic(
      "paper-factory honesty gate: headline_timing('" + key + "') references an environment='" +
      r.environment + "' record. This accessor renders ONLY environment='canonical-timing' " +
      "values (a measured wall-clock from an envelope self-declaring canonical: true).",
    )
  }
  r
}

#let _envelope(r) = {
  let e = _timing_data.envelopes.at(r.envelope, default: none)
  if e == none {
    panic(
      "paper-factory: canonical-timing record cites envelope '" + r.envelope +
      "' which is not in the derived envelope table — regenerate with " +
      "`npm run sync-canonical-timing`.",
    )
  }
  e
}

// The long host-class string is "<class> (CANONICAL 16 vCPU / … )"; the leading class alone is
// the readable short form for an inline provenance tag. The full string is in the block form.
#let _host_short(host_class) = {
  let parts = host_class.split(" (")
  parts.at(0)
}

// Microseconds rendered at a human scale. Fixed, predictable thresholds (no significant-figure
// heuristics) so the same value always renders the same way in both artifacts.
#let _fmt_us(v) = {
  if v < 1000.0 {
    str(calc.round(v, digits: 1)) + " µs"
  } else if v < 1000000.0 {
    str(calc.round(v / 1000.0, digits: 1)) + " ms"
  } else {
    str(calc.round(v / 1000000.0, digits: 2)) + " s"
  }
}

// THE ONLY value accessor for the class. Renders the number and, unconditionally, a compact
// provenance tag (aggregate · host class · git commit). Use `timing_provenance(key)` for the
// full block form next to a table or figure.
//
// The helper calls are kept in CODE mode (plain `let` bindings) and only the resulting strings
// are interpolated into markup — `#_underscore_fn(...)` at the start of a markup expression is
// needlessly close to Typst's `_emphasis_` syntax to rely on.
#let headline_timing(key) = {
  let r = _trec(key)
  let e = _envelope(r)
  let value = _fmt_us(r.value)
  let tag = "(" + r.aggregate + " · " + _host_short(e.hostClass) + " · " + e.gitCommit + ")"
  [#value~#text(size: 0.8em, fill: gray)[#tag]]
}

// Full block-level provenance for a measured number: what was run, on what corpus, on what
// machine, aggregated how, at which commit, from which committed envelope.
#let timing_provenance(key) = {
  let r = _trec(key)
  let e = _envelope(r)
  text(size: 0.85em, fill: gray)[
    measured: #r.engine on #r.suite #r.query (#str(r.rows) rows), #r.aggregate ·
    corpus: #e.scale ·
    host: #e.hostClass ·
    commit: #e.gitCommit, gathered #e.gatheredAtUtc ·
    envelope: #raw(e.path)
  ]
}
