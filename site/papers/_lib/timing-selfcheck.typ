// [OPUS-5] sq-gum8.16 (paper factory F4) — compile self-check for `_lib/timing.typ`.
//
// NOT A PAPER. It is not in the `src/data/papers.ts` registry, so it never gets a route, a
// slug, a PDF in `public/`, or an index card. `scripts/build-papers.mjs`
// (`runTimingLibSelfCheck`) compiles it to a temp dir in BOTH export modes on every build, so
// the measured-timing accessor is verified against the real derived JSON even while no paper
// imports it yet (the first consumer is a separate bead).
//
// It exercises the accessors on a key picked from the derived data itself rather than a
// hard-coded one, so it keeps working as canonical envelopes come and go. It picks that key via
// `timing_keys()` — which yields KEYS ONLY — so even this fixture never holds a raw timing
// value; the raw dataset is not exported at all (see timing.typ "HOW THE INVARIANT IS
// ENFORCED"), and the build's source gate forbids importing anything else from the lib.

#import "timing.typ": headline_timing, timing_provenance, timing_keys

= timing.typ self-check

#let keys = timing_keys()

#if keys.len() == 0 [
  No canonical-timing records are derived right now. That is a legitimate state (no committed
  envelope under `bench/canonical-competitor-results/**` currently qualifies), and it means no
  paper can headline a measured timing until one does. Accessors are not exercised.
] else [
  Records derived: #str(keys.len()). Exercising the first key, #raw(keys.at(0)):

  Value with its forced inline provenance: #headline_timing(keys.at(0)).

  #timing_provenance(keys.at(0))
]
