// [OPUS-5.5] Display helpers shared by the zkSPARQL papers: zksparql-architecture.typ, the ESWC
// variant (zksparql-eswc.typ) and its supplementary material (zksparql-eswc-supplement.typ). Every
// value still comes from paper-evidence.json through the accessors of bench.typ; these helpers only
// format it. Each paper passes its own `anon` switch, because the ESWC files default to the
// double-blind build and the site paper does not.

#import "bench.typ": headline, ev

// The double-blind build shows a placeholder for every value that identifies our repository:
// proof-method identifiers, commits, and the digests and image IDs of guest binaries, which its
// public records list. The normal build shows the value unchanged.
#let withheld = "[withheld for review]"
#let repo-id(anon, body) = if anon { withheld } else { body }

// Short commit / digest rendering for bound identity records (first twelve hex characters). The
// record is looked up in both builds, so the anonymous build still checks it.
#let short-id(anon, key) = repo-id(anon, raw(headline(key).slice(0, 12)))

// Fixed three-decimal rendering for the preliminary pilot values.
#let fmt3(x) = {
  let s = str(calc.round(x, digits: 3))
  let parts = s.split(".")
  if parts.len() == 1 { s + ".000" } else { s + "0" * calc.max(0, 3 - parts.at(1).len()) }
}
#let pilot(key) = fmt3(ev(key))

// Executor cycle counts in millions, rounded to two decimals for display only (the value itself
// comes from the evidence file through headline, so it must be canonical).
#let mcycles(key) = {
  let s = str(calc.round(headline(key) / 1000000, digits: 2))
  let parts = s.split(".")
  if parts.len() == 1 { s + ".00" } else { s + "0" * calc.max(0, 2 - parts.at(1).len()) }
}
