// [OPUS-5] sq-gum8.16 (paper factory F4) — derive the `canonical-timing` evidence class from
// the committed canonical measurement envelopes, so a paper can headline a MEASURED wall-clock
// number without a human ever typing one.
//
// WHY THIS EXISTS (research/paper-factory-2026-07.md §2 "second gap", §3.2 option (c)):
// `site/src/data/paper-evidence.json` holds only deterministic, machine-independent values
// (conformance floors, recall floors, boolean invariants). Papers therefore could not cite ANY
// measured timing — not even the provenance-complete envelopes the dedicated quiet-EC2
// protocol already commits under `bench/canonical-competitor-results/**` — leaving the
// systems-paper evaluation sections weaker than the data the project actually has.
//
// THE POLICY THIS IMPLEMENTS (papers MAY headline measured timings, under strict provenance):
//   1. Values are DERIVED here, never hand-typed. There is no author-writable timing file.
//   2. An envelope is admitted ONLY if it self-declares `"canonical": true` — so a work-box
//      timing cannot enter the class by construction (work-box numbers are NON-CANONICAL and
//      stay excluded; `benchmarks.generated.json` remains `environment: indicative` and may
//      never back a headline).
//   3. Every admitted record carries the envelope's provenance (host class, cpu, git commit,
//      gather date, dataset/scale, aggregate), and a paper can only reach it through
//      `site/papers/_lib/timing.typ`'s `headline_timing()` / `timing_provenance()`, which
//      always render that provenance alongside the number. Typst cannot enforce that on its
//      own (it has no private module bindings), so the boundary is a build gate:
//      `site/scripts/timing-source-gate.mjs`.
//   4. COUNT BEFORE TIME. The envelopes themselves state the rule ("this is why COUNT is
//      checked before trusting any timing"): a row is admitted only if that engine's own
//      result count matches the suite's COMMITTED `bench/<suite>/expected-rows.tsv`, which is
//      loaded here INDEPENDENTLY of the envelope — an envelope's own
//      `count_crosscheck.expected` is a self-assertion, so trusting it would let an envelope
//      carrying a wrong-but-internally-consistent expected value admit its own bad rows.
//      When the committed file has no row for that query, the fallback does NOT trust the
//      envelope's `all_agree` flag either: agreement is RE-DERIVED from the per-engine
//      crosscheck counts (at least two engines must have produced a count, all of them equal
//      to the TSV row). So a fast-but-WRONG comparator (e.g. an engine returning 0 rows on
//      SP2Bench q08) can never be published as a timing.
//
// HONEST SCOPE — read before extending:
//   - Only ONE envelope family is derived today: the same-box per-query SPARQL comparison,
//     `tsv_format == "<query>\t<rows|ERROR>\t<best_us|engine>"`. The HTTP family (6-column,
//     keep-alive vs fresh + TTFB), the materialization family (per-profile closure timings)
//     and the HDT family (per-metric) are RECOGNISED AND SKIPPED WITH A REASON rather than
//     guessed at — each needs its own key scheme. `skipped[]` in the output is the honest
//     record of what was left out.
//   - An envelope carrying a free-text `disqualified_note` is SKIPPED WHOLE. The note scopes a
//     disqualification in prose a machine cannot bound, so the fail-closed direction is to
//     refuse the envelope and surface the reason for a human.
//   - This is a MECHANICAL derivation: it guarantees value↔envelope provenance, not that the
//     comparison is fair or that the paper's framing of it is honest. Semantic framing stays
//     the Stage-5 claims↔evidence review in `skills/academic-paper/SKILL.md`.
//
// OUTPUT: `site/src/data/paper-timing.generated.json`, committed so the derivation is
// reviewable in a diff (the repo's committed-generated-artifact pattern) and re-derived on
// every `prebuild` so a newly-landed canonical envelope updates the papers automatically.
// `site/test/paper-timing.test.mjs` byte-compares the committed file against a fresh
// derivation, which is the drift guard.
//
// USAGE:
//   node scripts/sync-canonical-timing.mjs            # write the generated file
//   node scripts/sync-canonical-timing.mjs --check     # exit 1 if the committed file is stale

import { readFileSync, writeFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { dirname, join, relative, basename, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SITE = resolve(__dirname, "..");
const REPO_ROOT = resolve(SITE, "..");
const ENVELOPE_DIR = join(REPO_ROOT, "bench", "canonical-competitor-results");
const OUT_PATH = join(SITE, "src", "data", "paper-timing.generated.json");

export const SCHEMA_VERSION = 1;

// The ONE envelope family derived today (see "HONEST SCOPE" above). Compared after
// normalisation because the committed envelopes are inconsistently escaped: some spell the
// separator as a real tab, others as a literal backslash-t inside the JSON string.
const SUPPORTED_TSV_FORMAT = "<query>\t<rows|ERROR>\t<best_us|engine>";

function normalizeTsvFormat(fmt) {
  return typeof fmt === "string" ? fmt.replace(/\\t/g, "\t").trim() : "";
}

// ---- envelope discovery -------------------------------------------------------------------
// Sorted so the derivation is deterministic regardless of directory-read order.
function findEnvelopes(dir) {
  if (!existsSync(dir)) return [];
  const out = [];
  for (const entry of readdirSync(dir).sort()) {
    const p = join(dir, entry);
    if (statSync(p).isDirectory()) out.push(...findEnvelopes(p));
    else if (entry.endsWith(".json")) out.push(p);
  }
  return out;
}

// ---- admission of a whole envelope --------------------------------------------------------
// Returns { ok: true, meta } or { ok: false, reason } — never throws on a malformed envelope,
// because an unrelated new file under bench/ must not break the site build; it is skipped
// with a reason instead.
function admitEnvelope(env, relPath) {
  if (env?.canonical !== true) {
    return { ok: false, reason: "does not self-declare `\"canonical\": true`" };
  }
  if (env.disqualified_note !== undefined || env.disqualified !== undefined) {
    return {
      ok: false,
      reason:
        "carries a free-text disqualification note (a machine cannot bound its scope; " +
        "refusing the whole envelope is the fail-closed direction)",
    };
  }
  const fmt = normalizeTsvFormat(env.tsv_format);
  if (fmt !== SUPPORTED_TSV_FORMAT) {
    return {
      ok: false,
      reason: `tsv_format ${JSON.stringify(env.tsv_format ?? null)} is not the derived ` +
        `same-box per-query family ${JSON.stringify(SUPPORTED_TSV_FORMAT)}`,
    };
  }
  if (typeof env.suite !== "string" || !env.suite) return { ok: false, reason: "missing `suite`" };
  if (typeof env.scale !== "string" || !env.scale) return { ok: false, reason: "missing `scale`" };
  if (typeof env.git_commit !== "string" || !env.git_commit) {
    return { ok: false, reason: "missing `git_commit` (provenance is mandatory)" };
  }
  const e = env.env;
  if (!e || typeof e !== "object" || typeof e.host_class !== "string" || !e.gathered_at_utc) {
    return {
      ok: false,
      reason: "missing `env.host_class` / `env.gathered_at_utc` (the forced provenance line " +
        "cannot be rendered without them)",
    };
  }
  // The aggregate is READ OFF the envelope's own methodology note rather than assumed, and
  // cross-checked against `iters` — so a note/iters disagreement fails closed instead of
  // silently mislabelling how the number was aggregated.
  const m = /min-of-(\d+)/.exec(String(env.canonical_note ?? ""));
  if (!m) {
    return {
      ok: false,
      reason: "`canonical_note` does not state a `min-of-N` aggregate (the aggregate is read " +
        "off the envelope, never assumed)",
    };
  }
  if (Number(m[1]) !== Number(env.iters)) {
    return {
      ok: false,
      reason: `aggregate/iters disagreement: canonical_note says min-of-${m[1]} but iters=${env.iters}`,
    };
  }
  return {
    ok: true,
    meta: {
      path: relPath,
      suite: env.suite,
      scale: env.scale,
      iters: env.iters,
      aggregate: `min-of-${m[1]}`,
      gitCommit: env.git_commit,
      gatheredAtUtc: e.gathered_at_utc,
      hostClass: e.host_class,
      cpuModel: e.cpu_model ?? null,
      nproc: e.nproc ?? null,
      quietBox: e.quiet_box ?? null,
      canonicalNote: env.canonical_note ?? null,
    },
  };
}

// ---- the COMMITTED expected-rows oracle ----------------------------------------------------
// `bench/<suite>/expected-rows.tsv` is the repo's per-commit correctness oracle, committed and
// reviewed independently of any measurement run. It — not the envelope's self-declared
// `count_crosscheck.expected` — is the authority for COUNT BEFORE TIME.
//
// Format: `<query>\t<rows>` with `#` comments; extra columns (LUBM spells a third `regime`
// column) are ignored. A query listed twice with DIFFERENT counts is genuinely ambiguous, so it
// is recorded as AMBIGUOUS and refuses every row for that query rather than picking one.
const AMBIGUOUS = Symbol("ambiguous expected-rows entry");

function loadExpectedRows(repoRoot, suite) {
  const rel = join("bench", suite, "expected-rows.tsv");
  const abs = join(repoRoot, rel);
  if (!existsSync(abs)) return { path: rel, rows: null };
  const rows = new Map();
  for (const raw of readFileSync(abs, "utf8").split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#")) continue;
    const [query, expected] = line.split("\t");
    if (!query || expected === undefined) continue;
    const count = expected.trim();
    if (!/^\d+$/.test(count)) continue;
    const held = rows.get(query);
    rows.set(query, held !== undefined && held !== count ? AMBIGUOUS : count);
  }
  return { path: rel, rows };
}

// ---- COUNT BEFORE TIME --------------------------------------------------------------------
// A per-(engine, query) admission check: publish a timing only for a row whose result count is
// verified correct. Returns null when admitted, else the reason it was refused.
//
// `expected` is the INDEPENDENTLY loaded committed oracle for this envelope's suite (see
// loadExpectedRows); `engines` is the envelope's own engine list, used to re-derive agreement
// from the per-engine counts instead of trusting the envelope's `all_agree` flag.
function refuseOnCount(crosscheck, engine, query, count, engines, expected) {
  const cc = crosscheck?.[query];
  if (!cc || typeof cc !== "object") {
    return "no count_crosscheck entry (an unverified count may not back a published timing)";
  }
  const declared = cc.expected === undefined || cc.expected === null ? null : String(cc.expected);

  // (a) The committed oracle covers this query — it decides, and the envelope may not contradict
  //     it (a disagreement means one of the two is wrong, so neither may back a timing).
  const committed = expected.rows ? expected.rows.get(query) : undefined;
  if (committed === AMBIGUOUS) {
    return `${expected.path} lists '${query}' more than once with different counts`;
  }
  if (committed !== undefined) {
    if (committed !== count) {
      return `count ${count} != expected ${committed}`;
    }
    if (declared !== null && declared !== committed) {
      return `envelope asserts expected ${declared} but ${expected.path} says ${committed}`;
    }
    if (String(cc[engine] ?? "") !== count) {
      return `count_crosscheck disagrees with the TSV row (${JSON.stringify(cc[engine] ?? null)} vs ${count})`;
    }
    return null;
  }

  // (b) No committed count for this query: fall back to engine agreement, RE-DERIVED from the
  //     per-engine counts. `all_agree` is the envelope's own summary of that data and is not
  //     trusted on its own.
  const participating = engines.filter((e) => /^\d+$/.test(String(cc[e] ?? "")));
  const where = expected.rows
    ? `${expected.path} has no row for '${query}'`
    : `${expected.path} is not committed`;
  if (String(cc[engine] ?? "") !== count) {
    return `${where} and count_crosscheck disagrees with the TSV row ` +
      `(${JSON.stringify(cc[engine] ?? null)} vs ${count})`;
  }
  if (declared !== null && declared !== count) {
    return `${where} and the envelope's own expected ${declared} != count ${count}`;
  }
  const disagreeing = participating.filter((e) => String(cc[e]) !== count);
  if (disagreeing.length) {
    return `${where} and the engines did not all agree (${disagreeing.join(", ")} != ${count})`;
  }
  if (participating.length < 2) {
    return `${where} and only ${engine} produced a count, so nothing cross-checked it`;
  }
  return null;
}

// ---- the derivation -----------------------------------------------------------------------
export function deriveCanonicalTiming({ repoRoot = REPO_ROOT } = {}) {
  const envelopes = {};
  const records = {};
  const skipped = [];
  // suite -> { scale, path }: two envelopes keyed into the same suite namespace with DIFFERENT
  // datasets would let `latest wins` silently conflate two corpora, so that is a hard error.
  const suiteScale = new Map();
  // key -> the winning envelope's sort tuple, for the latest-wins resolution below.
  const winner = new Map();
  // suite -> the committed expected-rows oracle, loaded once (the COUNT BEFORE TIME authority).
  const expectedBySuite = new Map();

  for (const abs of findEnvelopes(join(repoRoot, "bench", "canonical-competitor-results"))) {
    const relPath = relative(repoRoot, abs);
    let env;
    try {
      env = JSON.parse(readFileSync(abs, "utf8"));
    } catch (e) {
      skipped.push({ path: relPath, reason: `unparseable JSON: ${e.message}` });
      continue;
    }
    const verdict = admitEnvelope(env, relPath);
    if (!verdict.ok) {
      skipped.push({ path: relPath, reason: verdict.reason });
      continue;
    }
    const meta = verdict.meta;

    const prior = suiteScale.get(meta.suite);
    if (prior && prior.scale !== meta.scale) {
      throw new Error(
        `sync-canonical-timing: suite '${meta.suite}' appears with two different datasets — ` +
          `'${prior.scale}' (${prior.path}) vs '${meta.scale}' (${meta.path}). ` +
          "Latest-wins would conflate them; give one of the runs a distinct `suite`.",
      );
    }
    suiteScale.set(meta.suite, { scale: meta.scale, path: meta.path });

    const envelopeId = basename(relPath).replace(/\.json$/, "");
    if (envelopes[envelopeId] && envelopes[envelopeId].path !== relPath) {
      throw new Error(
        `sync-canonical-timing: envelope id collision '${envelopeId}' ` +
          `(${envelopes[envelopeId].path} vs ${relPath}) — rename one file.`,
      );
    }

    if (!expectedBySuite.has(meta.suite)) {
      expectedBySuite.set(meta.suite, loadExpectedRows(repoRoot, meta.suite));
    }
    const expected = expectedBySuite.get(meta.suite);
    meta.expectedRows = expected.rows ? expected.path : null;

    const engines = Object.keys(env)
      .filter((k) => k.endsWith("_tsv"))
      .map((k) => k.slice(0, -4))
      .sort();
    let admittedFromThisEnvelope = 0;
    const refusedRows = [];

    for (const engine of engines) {
      if (env.statuses?.[engine] !== "ok") {
        refusedRows.push(`${engine}: whole engine (status=${JSON.stringify(env.statuses?.[engine] ?? null)})`);
        continue;
      }
      for (const raw of String(env[`${engine}_tsv`]).split("\n")) {
        const line = raw.trim();
        if (!line) continue;
        const [query, count, best] = line.split("\t");
        if (!query || count === undefined || best === undefined) continue;
        if (!/^\d+$/.test(count)) continue; // ERROR / timeout row — never fabricated, never used
        const us = Number(best);
        if (!Number.isFinite(us) || us <= 0) continue; // the engine-name column of an ERROR row
        const refusal = refuseOnCount(env.count_crosscheck, engine, query, count, engines, expected);
        if (refusal) {
          refusedRows.push(`${engine}/${query}: ${refusal}`);
          continue;
        }
        const key = `timing.${meta.suite}.${engine}.${query}`;
        // Latest-wins per RECORD (not per envelope), so a narrow re-run of one query supersedes
        // just that query and the broader earlier run still supplies the rest. The provenance
        // rendered next to the number always names the envelope that actually won.
        const rank = [meta.gatheredAtUtc, relPath];
        const held = winner.get(key);
        if (held && (held[0] > rank[0] || (held[0] === rank[0] && held[1] > rank[1]))) continue;
        winner.set(key, rank);
        records[key] = {
          value: us,
          unit: "us",
          environment: "canonical-timing",
          kind: "measured-wall-clock",
          engine,
          suite: meta.suite,
          query,
          rows: Number(count),
          aggregate: meta.aggregate,
          envelope: envelopeId,
          source: `${relPath}#/${engine}_tsv (${query})`,
        };
        admittedFromThisEnvelope += 1;
      }
    }

    if (admittedFromThisEnvelope === 0) {
      skipped.push({
        path: relPath,
        reason: `admitted as canonical but no row passed the count check (${refusedRows.join("; ") || "no parseable rows"})`,
      });
      continue;
    }
    envelopes[envelopeId] = meta;
    if (refusedRows.length) meta.refusedRows = refusedRows.sort();
  }

  // Drop envelopes every one of whose records lost the latest-wins race, so `envelopes` is
  // exactly the set the records actually cite.
  const cited = new Set(Object.values(records).map((r) => r.envelope));
  for (const id of Object.keys(envelopes)) {
    if (!cited.has(id)) {
      skipped.push({
        path: envelopes[id].path,
        reason: "every row it contributed was superseded by a later canonical envelope",
      });
      delete envelopes[id];
    }
  }

  const sortKeys = (o) =>
    Object.fromEntries(Object.keys(o).sort().map((k) => [k, o[k]]));

  return {
    _comment:
      "GENERATED by site/scripts/sync-canonical-timing.mjs from the committed canonical " +
      "measurement envelopes under bench/canonical-competitor-results/**. DO NOT HAND-EDIT: " +
      "regenerate with `npm run sync-canonical-timing`. Every record is environment=" +
      "'canonical-timing' — a MEASURED wall-clock value from an envelope that self-declares " +
      "`\"canonical\": true` (the dedicated quiet-EC2 protocol), whose result count is verified " +
      "against the suite's committed bench/<suite>/expected-rows.tsv (loaded independently of " +
      "the envelope, which cannot bless its own counts). Work-box timings are NON-CANONICAL and " +
      "cannot enter this file. Papers may render these ONLY through headline_timing() / " +
      "timing_provenance() in site/papers/_lib/timing.typ, which always print the provenance " +
      "alongside the number — enforced at build time by site/scripts/timing-source-gate.mjs. " +
      "`skipped` is the honest record of which envelopes were left out and why.",
    schemaVersion: SCHEMA_VERSION,
    generatedBy: "site/scripts/sync-canonical-timing.mjs",
    envelopes: sortKeys(envelopes),
    records: sortKeys(records),
    skipped: skipped.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0)),
  };
}

// Single serialisation point so the writer and the drift-guard test compare identical bytes.
export function serializeCanonicalTiming(derived) {
  return JSON.stringify(derived, null, 2) + "\n";
}

export const TIMING_OUT_PATH = OUT_PATH;

function main(argv) {
  const check = argv.includes("--check");
  const derived = deriveCanonicalTiming();
  const text = serializeCanonicalTiming(derived);
  const nRecords = Object.keys(derived.records).length;
  const nEnvelopes = Object.keys(derived.envelopes).length;

  if (check) {
    const current = existsSync(OUT_PATH) ? readFileSync(OUT_PATH, "utf8") : null;
    if (current !== text) {
      console.error(
        "\n[canonical-timing] STALE: site/src/data/paper-timing.generated.json does not match a " +
          "fresh derivation from bench/canonical-competitor-results/**.\n" +
          "  Run `npm run sync-canonical-timing` and commit the result. Never hand-edit it.\n",
      );
      process.exit(1);
    }
    console.log(
      `[canonical-timing] up to date: ${nRecords} record(s) from ${nEnvelopes} canonical envelope(s).`,
    );
    return;
  }

  writeFileSync(OUT_PATH, text, "utf8");
  console.log(
    `[canonical-timing] wrote ${relative(SITE, OUT_PATH)}: ${nRecords} record(s) from ` +
      `${nEnvelopes} canonical envelope(s); ${derived.skipped.length} envelope(s) skipped.`,
  );
  for (const s of derived.skipped) console.log(`  skipped ${s.path}: ${s.reason}`);
}

// Only run the CLI when invoked directly (the module is imported by the build step + tests).
if (process.argv[1] && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url))) {
  main(process.argv.slice(2));
}
