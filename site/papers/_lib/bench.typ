// [OPUS-4.8] sq-gum8 — shared paper-factory helpers. ONE source of numbers.
//
// The build writes one ignored merged ledger and injects its paper-bound path via
// `--input data=...`. Every paper reads it through the accessors below and
// NEVER hard-codes a number — so the PDF and the in-site HTML cannot disagree, and a paper
// auto-updates when the evidence refreshes.
//
// HONESTY GATE (enforced in build-papers.mjs, mirrored here as a hard fail):
// `headline(key)` is the ONLY accessor allowed inside a headline result table/figure. It
// REFUSES to render a record whose `environment` is not "canonical" — so a non-canonical
// (work-box / indicative) number can never appear as a paper's headline evidence. Use the
// plain `ev(key)` accessor only for clearly-labelled "indicative" callouts. Canonical machine
// timings use timing.typ, whose helpers make the value and its acquisition provenance inseparable.

#let _evidence = json(sys.inputs.data)
#let anon = sys.inputs.at("anon", default: "false") == "true"

// Raw record lookup (panics loudly if the key is missing — a typo must fail the build,
// never silently render an empty string).
#let _rec(key) = {
  let r = _evidence.records.at(key, default: none)
  if r == none {
    panic("paper-factory: unknown evidence key '" + key + "' — add it to site/src/data/paper-evidence.json")
  }
  r
}

// Plain values are available only for non-timing records. A timing value cannot be separated
// from its acquisition provenance by choosing the legacy accessor.
#let ev(key) = {
  let r = _rec(key)
  if r.environment == "canonical-timing" {
    panic(
      "paper-factory: ev('" + key + "') cannot expose canonical timing without provenance; " +
      "import timing.typ and use headline_timing(...) instead",
    )
  }
  r.value
}

// HEADLINE accessor — the honesty gate. Returns the value ONLY if the record is canonical.
#let headline(key) = {
  let r = _rec(key)
  if r.environment != "canonical" {
    panic(
      "paper-factory honesty gate: headline('" + key + "') references an environment='" +
      r.environment + "' record. Headline tables may cite ONLY environment='canonical' " +
      "numbers (deterministic, machine-independent). Move it to an 'indicative' callout " +
      "or supply a canonical record.",
    )
  }
  r.value
}

// A provenance line for a headline number (source test + environment).
// [FABLE-5] sq-gum8.3 render fix: `#r.source` inside raw backticks printed LITERALLY
// (raw content does not interpolate); use #raw(r.source) so the actual source renders.
#let provenance(key) = {
  let r = _rec(key)
  if r.environment == "canonical-timing" {
    panic(
      "paper-factory: provenance('" + key + "') is incomplete for canonical timing; " +
      "import timing.typ and use timing_provenance(...) instead",
    )
  }
  text(size: 0.85em, fill: gray)[evidence: #raw(r.source) (environment: #r.environment)]
}

// Author block honours the double-blind anon toggle.
#let authors() = if anon {
  align(center)[#text(style: "italic")[Author(s) omitted for double-blind review]]
} else {
  align(center)[Jesse Wright · the sparq project]
}

// [OPUS-4.8] sq-iixdh — canonical heading-numbering function for all papers.
//
// Papers use level-2 headings (== Section) as their top-level sections (the page h1 or
// document title occupies the conceptual level-1 slot). A plain `#set heading(numbering:
// "1.")` over level-2 headings with no level-1 ancestor renders them as "0.1", "0.2",
// because Typst counts the never-incremented level-1 counter as 0.
//
// This function drops the level-1 component so == headings render as "1.", "2.", "3." and
// === headings as "1.1.", "1.2." — the venue-conventional numbering.
//
// Usage in a paper:
//   #import "_lib/bench.typ": ..., paper_heading_numbering
//   #set heading(numbering: paper_heading_numbering)
//   // Abstract must be explicitly un-numbered:
//   #heading(level: 2, numbering: none, outlined: false)[Abstract]
#let paper_heading_numbering = (..n) => {
  let ns = n.pos()
  // Drop the leading level-1 counter (always 0 for papers) so == sections
  // are "1.", "2.", ... and === sub-sections are "1.1.", "1.2.", ...
  numbering("1.", ..if ns.len() > 1 { ns.slice(1) } else { ns })
}
