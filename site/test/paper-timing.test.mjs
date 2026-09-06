// [OPUS-5] sq-gum8.16 (paper factory F4) — unit tests for the derived `canonical-timing`
// evidence class: the drift guard, the class invariants, and — the load-bearing ones — the two
// honesty properties the class exists to guarantee:
//   * a NON-canonical (work-box) envelope cannot contribute a number, and
//   * COUNT BEFORE TIME: a comparator that returned the wrong number of rows cannot have its
//     timing published, even though the timing itself is present in the envelope.
// Run via `npm run test:unit`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";

import {
  deriveCanonicalTiming,
  serializeCanonicalTiming,
  TIMING_OUT_PATH,
} from "../scripts/sync-canonical-timing.mjs";
import { auditTimingSources, ALLOWED_TIMING_IMPORTS } from "../scripts/timing-source-gate.mjs";

const SITE = dirname(fileURLToPath(import.meta.url)).replace(/\/test$/, "");

// A minimal envelope in the derived family. `canonical` and the count columns are the knobs the
// honesty tests below turn.
function envelope({ canonical, suite = "demo", counts, tsv }) {
  return {
    canonical,
    canonical_note: "CANONICAL: dedicated quiet EC2. min-of-5 on loaded stores.",
    git_commit: "deadbeef",
    suite,
    scale: "demo corpus, 10 triples",
    iters: 5,
    tsv_format: "<query>\t<rows|ERROR>\t<best_us|engine>",
    engines: { sparq: { version: "deadbeef" }, other: { version: "1.0" } },
    statuses: { sparq: "ok", other: "ok" },
    count_crosscheck: counts,
    env: {
      host_class: "dedicated-quiet-ec2-c6i.4xlarge (CANONICAL 16 vCPU / 32 GiB x86_64)",
      cpu_model: "Demo CPU",
      nproc: 16,
      quiet_box: true,
      gathered_at_utc: "2026-01-01T00:00:00Z",
    },
    ...tsv,
  };
}

// Writes envelopes into a throwaway repo root so the derivation can be exercised on crafted
// inputs without touching the real bench/ tree. `expectedRows` seeds the COMMITTED oracle
// (`bench/<suite>/expected-rows.tsv`) that COUNT BEFORE TIME actually consults — it is the
// authority, deliberately independent of anything the envelope asserts about itself.
function deriveFromFixture(envelopes, expectedRows = {}) {
  const root = mkdtempSync(join(tmpdir(), "sparq-timing-fixture-"));
  const dir = join(root, "bench", "canonical-competitor-results", "fixture");
  mkdirSync(dir, { recursive: true });
  for (const [name, env] of Object.entries(envelopes)) {
    writeFileSync(join(dir, `${name}.json`), JSON.stringify(env, null, 2), "utf8");
  }
  for (const [suite, rows] of Object.entries(expectedRows)) {
    mkdirSync(join(root, "bench", suite), { recursive: true });
    writeFileSync(
      join(root, "bench", suite, "expected-rows.tsv"),
      "# fixture expected-rows\n" +
        Object.entries(rows).map(([q, n]) => `${q}\t${n}`).join("\n") + "\n",
      "utf8",
    );
  }
  return deriveCanonicalTiming({ repoRoot: root });
}

// A throwaway `papers/` tree for the publication-boundary source gate, so a negative fixture
// (a paper trying to publish a bare value) can be run through the real gate.
function auditFixturePapers(sources) {
  const dir = mkdtempSync(join(tmpdir(), "sparq-timing-papers-"));
  mkdirSync(join(dir, "_lib"), { recursive: true });
  for (const [name, text] of Object.entries(sources)) {
    writeFileSync(join(dir, name), text, "utf8");
  }
  return auditTimingSources(dir);
}

test("the committed paper-timing.generated.json matches a fresh derivation (drift guard)", () => {
  const expected = serializeCanonicalTiming(deriveCanonicalTiming());
  const actual = readFileSync(TIMING_OUT_PATH, "utf8");
  assert.equal(
    actual,
    expected,
    "src/data/paper-timing.generated.json is stale or hand-edited — run `npm run sync-canonical-timing`",
  );
});

test("every derived record satisfies the canonical-timing class invariants", () => {
  const d = deriveCanonicalTiming();
  assert.ok(Object.keys(d.records).length > 0, "the committed envelopes must yield some records");
  for (const [key, r] of Object.entries(d.records)) {
    assert.equal(r.environment, "canonical-timing", `${key}: wrong environment`);
    assert.equal(r.unit, "us", `${key}: every derived timing is in microseconds`);
    assert.ok(Number.isFinite(r.value) && r.value > 0, `${key}: value must be a positive number`);
    assert.equal(key, `timing.${r.suite}.${r.engine}.${r.query}`, `${key}: key/fields disagree`);
    assert.match(r.aggregate, /^min-of-\d+$/, `${key}: aggregate must be read off the envelope`);
    // The forced provenance line cannot render without a resolvable, fully-described envelope.
    const e = d.envelopes[r.envelope];
    assert.ok(e, `${key}: cites unknown envelope ${r.envelope}`);
    for (const field of ["path", "scale", "hostClass", "gitCommit", "gatheredAtUtc", "aggregate"]) {
      assert.ok(e[field], `envelope ${r.envelope}: missing provenance field ${field}`);
    }
    assert.match(e.path, /^bench\/canonical-competitor-results\//, "envelopes come only from the canonical tree");
  }
});

test("a non-canonical (work-box) envelope cannot contribute a timing", () => {
  const counts = { q01: { sparq: "1", other: "1", all_agree: true, expected: "1" } };
  const tsv = { sparq_tsv: "q01\t1\t42.0\n", other_tsv: "q01\t1\t99.0\n" };
  const d = deriveFromFixture({
    good: envelope({ canonical: true, suite: "yes", counts, tsv }),
    workbox: envelope({ canonical: false, suite: "no", counts, tsv }),
  });
  assert.ok(d.records["timing.yes.sparq.q01"], "the canonical envelope must be derived");
  assert.equal(
    Object.keys(d.records).filter((k) => k.startsWith("timing.no.")).length,
    0,
    "an envelope that does not self-declare `canonical: true` must contribute nothing",
  );
  assert.ok(
    d.skipped.some((s) => /workbox\.json$/.test(s.path) && /canonical/.test(s.reason)),
    "the refusal must be recorded in `skipped` with a reason",
  );
});

test("COUNT BEFORE TIME: a wrong-count engine's timing is refused, a correct one's is kept", () => {
  // `other` is FASTER (1.0 µs) but returned 0 rows where 358 were expected — publishing that
  // timing would be the archetypal dishonest comparison.
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: {
        q08: { sparq: "358", other: "0", all_agree: false, expected: "358", matches_expected: false },
      },
      tsv: { sparq_tsv: "q08\t358\t153318.0\n", other_tsv: "q08\t0\t1.0\n" },
    }),
  }, { demo: { q08: "358" } });
  assert.ok(d.records["timing.demo.sparq.q08"], "the engine with the correct count is published");
  assert.equal(
    d.records["timing.demo.other.q08"],
    undefined,
    "the engine with the wrong row count must NOT have its timing published",
  );
  const env = d.envelopes[Object.keys(d.envelopes)[0]];
  assert.ok(
    env.refusedRows.some((r) => r.startsWith("other/q08:")),
    "the per-row refusal must be recorded on the envelope, not silently dropped",
  );
});

test("this repo's real envelopes exercise the count rule (the guard is not vacuous)", () => {
  const d = deriveCanonicalTiming();
  // SP2Bench q08/q12b: one comparator returns 0 rows against a committed expected count, so its
  // timing must be absent while the engines that agreed with expected-rows are present.
  assert.ok(d.records["timing.sp2b.sparq.q08"], "sparq q08 (correct count) must be present");
  assert.equal(d.records["timing.sp2b.qlever.q08"], undefined, "a 0-row q08 result must be refused");
  assert.equal(d.records["timing.sp2b.qlever.q12b"], undefined, "a 0-row q12b result must be refused");
  const refused = Object.values(d.envelopes).flatMap((e) => e.refusedRows ?? []);
  assert.ok(
    refused.some((r) => /^qlever\/q08: count 0 != expected 358$/.test(r)),
    "the refusal reason must be recorded verbatim",
  );
});

test("an ERROR / timeout row is never turned into a number", () => {
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: { q01: { sparq: "1", other: "1", all_agree: true, expected: "1" } },
      tsv: { sparq_tsv: "q01\t1\t42.0\n", other_tsv: "q01\tERROR\tother\n" },
    }),
  });
  assert.ok(d.records["timing.demo.sparq.q01"]);
  assert.equal(d.records["timing.demo.other.q01"], undefined, "an ERROR row must not be derived");
});

test("an envelope carrying a free-text disqualification note is refused whole", () => {
  const base = envelope({
    canonical: true,
    counts: { q01: { sparq: "1", other: "1", all_agree: true, expected: "1" } },
    tsv: { sparq_tsv: "q01\t1\t42.0\n" },
  });
  const d = deriveFromFixture({ e: { ...base, disqualified_note: "engine X is not a valid comparator here" } });
  assert.equal(Object.keys(d.records).length, 0, "a disqualification note the machine cannot scope fails closed");
  assert.ok(d.skipped.some((s) => /disqualification/.test(s.reason)));
});

test("an envelope whose stated aggregate disagrees with `iters` is refused", () => {
  const base = envelope({
    canonical: true,
    counts: { q01: { sparq: "1", all_agree: true, expected: "1" } },
    tsv: { sparq_tsv: "q01\t1\t42.0\n" },
  });
  const d = deriveFromFixture({ e: { ...base, iters: 3 } }); // note still says min-of-5
  assert.equal(Object.keys(d.records).length, 0);
  assert.ok(d.skipped.some((s) => /aggregate\/iters disagreement/.test(s.reason)));
});

test("latest-wins is per record, so a narrow re-run supersedes only the queries it covers", () => {
  const counts = {
    q01: { sparq: "1", all_agree: true, expected: "1" },
    q02: { sparq: "2", all_agree: true, expected: "2" },
  };
  const older = envelope({ canonical: true, counts, tsv: { sparq_tsv: "q01\t1\t10.0\nq02\t2\t20.0\n" } });
  const newer = envelope({ canonical: true, counts, tsv: { sparq_tsv: "q01\t1\t5.0\n" } });
  newer.env = { ...newer.env, gathered_at_utc: "2026-02-02T00:00:00Z" };
  const d = deriveFromFixture({ a_older: older, b_newer: newer }, { demo: { q01: "1", q02: "2" } });
  assert.equal(d.records["timing.demo.sparq.q01"].value, 5.0, "the later envelope wins its own query");
  assert.equal(d.records["timing.demo.sparq.q02"].value, 20.0, "the earlier envelope still supplies q02");
  assert.equal(Object.keys(d.envelopes).length, 2, "both envelopes are cited, each by the records it won");
});

test("two envelopes claiming one suite with different datasets is a hard error", () => {
  const counts = { q01: { sparq: "1", all_agree: true, expected: "1" } };
  const tsv = { sparq_tsv: "q01\t1\t10.0\n" };
  const a = envelope({ canonical: true, counts, tsv });
  const b = envelope({ canonical: true, counts, tsv });
  b.scale = "a completely different corpus, 10000000 triples";
  assert.throws(
    () => deriveFromFixture({ a, b }),
    /two different datasets/,
    "latest-wins across differing corpora would conflate them, so it must fail closed",
  );
});

// ---- COUNT BEFORE TIME: the committed expected-rows file is the AUTHORITY -------------------
// The envelope's own `count_crosscheck` is a self-assertion. These are the mutation tests for
// that: each one keeps the envelope internally consistent and only breaks its agreement with
// the committed oracle (or removes the oracle), and each must refuse.

test("an envelope whose asserted `expected` contradicts the committed expected-rows is refused", () => {
  // Internally consistent — every engine reports 9, the envelope declares expected 9, all_agree
  // — but bench/demo/expected-rows.tsv says the right answer is 358. Trusting the envelope here
  // is exactly the hole: a bad run could bless its own wrong counts.
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: { q08: { sparq: "9", other: "9", all_agree: true, expected: "9", matches_expected: true } },
      tsv: { sparq_tsv: "q08\t9\t42.0\n", other_tsv: "q08\t9\t99.0\n" },
    }),
  }, { demo: { q08: "358" } });
  assert.equal(Object.keys(d.records).length, 0, "the committed expected-rows file decides, not the envelope");
  assert.ok(
    d.skipped.some((s) => /count 9 != expected 358/.test(s.reason)),
    `the refusal must name the committed count; got ${JSON.stringify(d.skipped)}`,
  );
});

test("an envelope asserting a DIFFERENT expected than the committed file is refused even when its rows match", () => {
  // The TSV rows agree with the committed oracle, but the envelope declares a different
  // `expected`. One of the two is wrong about what this query means, so neither may back a
  // timing until a human reconciles them.
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: { q01: { sparq: "1", other: "1", all_agree: true, expected: "7" } },
      tsv: { sparq_tsv: "q01\t1\t42.0\n", other_tsv: "q01\t1\t99.0\n" },
    }),
  }, { demo: { q01: "1" } });
  assert.equal(Object.keys(d.records).length, 0);
  assert.ok(
    d.skipped.some((s) => /envelope asserts expected 7 but bench\/demo\/expected-rows\.tsv says 1/.test(s.reason)),
    `the contradiction must be named; got ${JSON.stringify(d.skipped)}`,
  );
});

test("all_agree=true cannot admit a row whose crosscheck count differs from the TSV row", () => {
  // No committed oracle for this suite, so the fallback applies — and the envelope's `all_agree`
  // flag is a summary that disagrees with its own per-engine data. Agreement is RE-DERIVED, so
  // the flag cannot launder the inconsistency.
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: { q01: { sparq: "1", other: "1", all_agree: true } },
      tsv: { sparq_tsv: "q01\t5\t42.0\n", other_tsv: "q01\t1\t99.0\n" },
    }),
  });
  assert.equal(
    d.records["timing.demo.sparq.q01"],
    undefined,
    "a TSV row that disagrees with its own crosscheck entry must not be published",
  );
  assert.ok(
    d.skipped.concat(Object.values(d.envelopes).flatMap((e) => e.refusedRows ?? []))
      .some((r) => /count_crosscheck disagrees with the TSV row/.test(r.reason ?? r)),
  );
  // ...and the honest engine in the same envelope is unaffected.
  assert.ok(d.records["timing.demo.other.q01"], "the consistent row still publishes");
});

test("all_agree=true cannot admit a row the other engines actually contradict", () => {
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: { q01: { sparq: "1", other: "2", all_agree: true } },
      tsv: { sparq_tsv: "q01\t1\t42.0\n", other_tsv: "q01\t2\t99.0\n" },
    }),
  });
  assert.equal(Object.keys(d.records).length, 0, "a lying all_agree flag must not launder a disagreement");
  assert.ok(d.skipped.some((s) => /did not all agree/.test(s.reason)));
});

test("with no committed expected-rows and only one engine, nothing cross-checked the count", () => {
  const d = deriveFromFixture({
    e: envelope({
      canonical: true,
      counts: { q01: { sparq: "1", all_agree: true } },
      tsv: { sparq_tsv: "q01\t1\t42.0\n" },
    }),
  });
  assert.equal(Object.keys(d.records).length, 0);
  assert.ok(d.skipped.some((s) => /nothing cross-checked it/.test(s.reason)));
});

test("the committed expected-rows file is what the real derivation consulted", () => {
  // Non-vacuity for the whole authority path: the real records must cite a real committed
  // oracle, and that file must actually carry the count the record published.
  const d = deriveCanonicalTiming();
  const envs = Object.values(d.envelopes);
  assert.ok(envs.length > 0);
  for (const e of envs) {
    assert.match(
      e.expectedRows ?? "",
      /^bench\/[^/]+\/expected-rows\.tsv$/,
      `envelope ${e.path}: the committed count oracle must be named`,
    );
  }
  const sample = d.records["timing.sp2b.sparq.q08"];
  assert.ok(sample, "sp2b q08 is the load-bearing example");
  const tsv = readFileSync(join(SITE, "..", "bench", "sp2b", "expected-rows.tsv"), "utf8");
  assert.ok(
    tsv.split("\n").some((l) => l.trim() === `q08\t${sample.rows}`),
    "the published row count must appear verbatim in the committed expected-rows file",
  );
});

// ---- the publication boundary: no path to a bare value -------------------------------------

test("timing.typ exports only provenance-rendering entry points (no raw data binding)", () => {
  const src = readFileSync(join(SITE, "papers", "_lib", "timing.typ"), "utf8");
  // EVERY top-level `#let` is importable in Typst, so enumerate them all — data bindings,
  // zero-arg helpers and differently-shaped functions included — not just `(key)` accessors.
  const bindings = [...src.matchAll(/^#let\s+([A-Za-z_][A-Za-z0-9_]*)/gm)].map((m) => m[1]);
  const exported = bindings.filter((b) => !b.startsWith("_"));
  assert.deepEqual(
    exported.sort(),
    [...ALLOWED_TIMING_IMPORTS].sort(),
    "a non-underscore binding in timing.typ is part of its published surface — it must be one " +
      "the source gate allows, and it must not be able to hand back a bare timing value",
  );
  // No exported binding may be a bare data binding (the `timing_data` escape hatch).
  for (const name of exported) {
    assert.match(
      src,
      new RegExp(`^#let\\s+${name}\\(`, "m"),
      `${name} must be a function, not an exported data structure`,
    );
  }
  // `headline_timing` must actually emit provenance, not just be named as though it does.
  const body = src.slice(src.indexOf("#let headline_timing(key) ="));
  assert.match(body, /aggregate/, "headline_timing must render the aggregate");
  assert.match(body, /hostClass/, "headline_timing must render the host class");
  assert.match(body, /gitCommit/, "headline_timing must render the git commit");
});

test("the source gate rejects every known way to publish a bare timing value", () => {
  // Each fixture is a paper that renders a measured number WITHOUT its provenance — the exact
  // escape hatches a narrow "no bare accessor declared" assertion cannot see.
  const bypasses = {
    "raw-data-import.typ":
      '#import "_lib/timing.typ": headline_timing, _timing_data\n' +
      '#_timing_data.records.at("timing.sp2b.sparq.q01").value\n',
    "internal-accessor.typ":
      '#import "_lib/timing.typ": _trec\n#str(_trec("timing.sp2b.sparq.q01").value)\n',
    "wildcard-import.typ": '#import "_lib/timing.typ": *\n',
    "module-import.typ":
      '#import "_lib/timing.typ"\n#str(timing.records.at("timing.sp2b.sparq.q01").value)\n',
    "direct-json.typ":
      '#let d = json("/src/data/paper-timing.generated.json")\n' +
      '#str(d.records.at("timing.sp2b.sparq.q01").value)\n',
    // The path is CONSTRUCTED, so it spells neither the generated file name nor any internal
    // binding — the bypass a substring denylist cannot see. The rule is on the loader itself.
    "concatenated-path.typ":
      '#let d = json("/src/data/paper-" + "timing.generated.json")\n' +
      '#str(d.records.at("timing.sp2b.sparq.q01").value)\n',
    // ...same construct, renamed, so the call form no longer names a loader either.
    "aliased-loader.typ":
      '#let grab = json\n#let d = grab("/src/data/paper-" + "timing.generated.json")\n' +
      '#str(d.records.at("timing.sp2b.sparq.q01").value)\n',
    // ...and reached as raw text rather than parsed data.
    "raw-read.typ": '#let s = read("/src/data/paper-" + "timing.generated.json")\n#s.len()\n',
    // ...and CONSTRUCTED reflectively, so the source spells no loader name at all: `eval` builds
    // `json` from fragments and the path is assembled from fragments too. No loader-name rule can
    // see this, which is why the reflective construct itself is refused.
    "reflective-eval.typ":
      '#let grab = eval("j" + "son", mode: "code")\n' +
      '#let d = grab("/src/data/paper-" + "timing.generated.json")\n' +
      '#str(d.records.at("timing.sp2b.sparq.q01").value)\n',
    // ...same, via the std namespace, which the loader-call lookbehind skips as a field access.
    "std-namespace-loader.typ":
      '#let d = std.json("/src/data/paper-" + "timing.generated.json")\n' +
      '#str(d.records.at("timing.sp2b.sparq.q01").value)\n',
    // A computed module path defeats the literal-path import rules, so it is refused outright.
    "computed-import.typ": '#import "_lib/timing" + ".typ": headline_timing\n',
  };
  for (const [name, text] of Object.entries(bypasses)) {
    const { problems } = auditFixturePapers({ [name]: text });
    assert.ok(problems.length > 0, `the gate must reject ${name}`);
    assert.ok(problems.every((p) => p.startsWith(name)), `the finding must name the offending file`);
  }
  // ...while the sanctioned usage passes, so the gate is a boundary and not a blanket ban.
  const ok = auditFixturePapers({
    "good.typ":
      '#import "_lib/timing.typ": headline_timing, timing_provenance\n' +
      'q01 runs in #headline_timing("timing.sp2b.sparq.q01").\n' +
      // The reflection rule matches code position only, so SPARQL notation stays writable: real
      // papers say eval(P) in prose and `eval(P)` in raw spans, neither of which evaluates.
      'A solution mapping is in eval(P) iff it is in `eval(Join(P1,P2))`.\n',
  });
  assert.deepEqual(ok.problems, [], "the provenance-rendering accessors must remain usable");
});

test("the evidence library's data-loading exemption is pinned to its audited expression", () => {
  // `_lib/bench.typ` must load the deterministic evidence payload, so it is allowed exactly the
  // one audited line — and nothing else. A blanket per-file exemption would just relocate the
  // constructed-path bypass into the library.
  const audited = '#let evidence = json(bytes(sys.inputs.data))\n';
  assert.deepEqual(
    auditFixturePapers({ "_lib/bench.typ": audited }).problems,
    [],
    "the audited evidence loader must keep working",
  );
  const { problems } = auditFixturePapers({
    "_lib/bench.typ": audited + '#let t = json("/src/data/paper-" + "timing.generated.json")\n',
  });
  assert.equal(problems.length, 1, `the extra loader must be the only finding; got ${problems}`);
  assert.match(problems[0], /^_lib\/bench\.typ: uses the data loader 'json'/);
});

test("the real paper tree passes the publication-boundary source gate", () => {
  const { scanned, problems } = auditTimingSources(join(SITE, "papers"));
  assert.deepEqual(problems, [], "a registered paper source bypasses forced timing provenance");
  assert.ok(scanned.length > 0, "the gate must actually have scanned the paper sources");
  // The lib and its own self-check are the implementation of the boundary, not consumers.
  assert.ok(
    !scanned.some((f) => /timing(-selfcheck)?\.typ$/.test(f)),
    "timing.typ / timing-selfcheck.typ are exempt; every other paper source is scanned",
  );
});
