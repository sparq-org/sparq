// sq-1sa9r — the MINIMAL canonical-competitor ingestion path.
//
// PURPOSE. Turn the raw canonical "sparql-same-box-comparison" gather envelopes
// (bench/canonical-competitor-results/<date>/canonical-<suite>-<ts>.json — the exact
// output of the dedicated quiet-box gather) into the `same_box_comparisons` array that
// bench/dashboard/competitors.json carries and that site/scripts/sync-benchmarks.mjs then
// copies into site/src/data/benchmarks.generated.json. This is the established data flow;
// before this script the same_box_comparisons rows were hand-transcribed. Committing the
// raw envelopes + this transform makes a canonical ingest reproducible + reviewable.
//
// WHAT IT DOES (honestly, never fabricating):
//   * Groups the envelopes by `suite`. Each suite was gathered TWICE back-to-back.
//   * COUNTS are identical across the two gathers (asserted); if they ever differ the
//     script fails loudly rather than silently picking one.
//   * TIMINGS: best-of — per engine, per query, the MIN best_us across the two gathers.
//     Both gathers are already min-of-5 on a loaded store; best-of is min-of-10, the
//     least-contended estimate the "smaller-is-better" methodology targets, and it removes
//     a transient contention spike (e.g. sp2b virtuoso q09 in the later gather) uniformly
//     for every engine — NOT cherry-picking.
//   * A per-query "ERROR" (engine ran but the query failed) → null (renders "n/a").
//   * A whole-engine `status: "failed"` (e.g. fuseki load timeout) → status is carried so
//     the site shows the engine column as "failed", never blank; its cells stay null.
//   * count_match per row = the gather's count_crosscheck all_agree (honest DIFF where the
//     engines disagreed, e.g. sp2b q08 / q12b).
//   * MODE asymmetry (sparq/oxigraph = CLI in-process, fuseki/virtuoso/qlever = HTTP SPARQL
//     adapter) is carried per engine so the site can label it.
//
// USAGE:  node scripts/bench/ingest-canonical-competitors.mjs [<results-dir>...]
//   default results-dirs = EVERY dated directory under bench/canonical-competitor-results/
//   (sq-7d3dj.34: the HTTP/TTFB panel lands in a sibling dated dir, e.g.
//   2026-07-07-http/, whose envelopes carry DISTINCT suite ids like "sp2b-http" — so
//   multiple dirs combine into one same_box_comparisons array without collisions).
// It rewrites ONLY the `same_box_comparisons` key of bench/dashboard/competitors.json;
// every other key (schema_version, engines, values, references, …) is preserved verbatim.
// Re-run site/scripts/sync-benchmarks.mjs afterwards to refresh the site JSON.
//
// sq-7d3dj.34 — 6-col HTTP-profile envelopes: when an envelope's TSVs carry the
// extended columns (`<query>\t<rows>\t<ka_best_us>\t<ka_ttfb_us>\t<fresh_us>\t<fresh_ttfb_us>`)
// the row keeps `values` = keep-alive full-request best (col 3, backward-compatible) and
// ADDITIONALLY carries values_ttfb / values_fresh / values_fresh_ttfb; the envelope's
// `connection` note (keep-alive vs fresh-connect semantics) is carried onto the entry.
// Engine measurement MODE is preferred from the envelope's engines map when present.
import { readFileSync, writeFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..");
const resultsRoot = join(repoRoot, "bench", "canonical-competitor-results");
const resultsDirs = process.argv.length > 2
  ? process.argv.slice(2)
  : readdirSync(resultsRoot)
      .map((d) => join(resultsRoot, d))
      .filter((p) => statSync(p).isDirectory())
      .sort();
const competitorsPath = join(repoRoot, "bench", "dashboard", "competitors.json");

// Human labels + measurement MODE for each engine id (the CLI-vs-HTTP asymmetry the site
// must label). Digests come from the envelope; we keep them on the engine for provenance.
const ENGINE_META = {
  sparq: {
    label: "sparq",
    mode: "CLI in-process (sparq-cli bench, COUNT mode)",
  },
  oxigraph: {
    label: "Oxigraph",
    mode: "CLI in-process (prebuilt on-disk store, sha256-pinned)",
  },
  fuseki: {
    label: "Apache Jena Fuseki",
    mode: "HTTP SPARQL adapter (tdb2.tdbloader → fuseki-server)",
  },
  virtuoso: {
    label: "Virtuoso Open-Source 7",
    mode: "HTTP SPARQL adapter (isql ld_dir)",
  },
  qlever: {
    label: "QLever",
    mode: "HTTP SPARQL adapter (qlever-index → qlever-server)",
  },
  // sq-hmd7l.28 — the new comparative axes' competitor engines. Each axis'
  // same-box harness (scripts/bench/{fts,geo,hdt,update,materialize}-same-box.sh) records
  // these ids in its envelope `engines` map; adding them here gives the site a stable human
  // label + measurement-MODE string per column. A brand-new competitor a harness adds later
  // still surfaces automatically (see engineIdsOf below) — this table is only for the label.
  "jena-text": {
    label: "Apache Jena (jena-text / Lucene)",
    mode: "HTTP SPARQL adapter (jena-text Lucene index over a Fuseki endpoint)",
  },
  "jena-geosparql": {
    label: "Apache Jena (GeoSPARQL)",
    mode: "HTTP SPARQL adapter (jena-fuseki-geosparql spatial index)",
  },
  "hdt-cpp": {
    label: "hdt-cpp (rdfhdt)",
    mode: "in-container decode (Docker rdfhdt/hdt-cpp, hdt2rdf: decode .hdt → N-Triples; container spawn excluded, hdt2rdf process start included)",
  },
  jena: {
    label: "Apache Jena (rule reasoner)",
    mode: "in-process (Jena InfModel materialisation — a profile-DIFFERENT rule set; see note)",
  },
  vlog: {
    label: "VLog (Datalog)",
    mode: "VLog Datalog on the VALIDATED bench/reason-encodings program (closure set-identical to sparq's); timed = VLog's self-reported materialization (EDB load + store excluded)",
  },
  nemo: {
    label: "Nemo (Datalog)",
    mode: "Nemo Datalog on the VALIDATED bench/reason-encodings program (closure set-identical to sparq's); timed = Nemo's self-reported `Reasoning:` breakdown (import + export excluded)",
  },
};
// Column order (sparq first, then the SPARQL matrix's fixed order; any engine not listed
// here — e.g. a new-axis competitor — is appended in the envelope's own key order via
// engineIdsOf, so a new competitor never gets silently dropped).
const ENGINE_ORDER = ["sparq", "oxigraph", "fuseki", "virtuoso", "qlever"];

// sq-hmd7l.28 — the engine ids to render for ONE envelope, sparq first, then the
// fixed SPARQL-matrix order for any of those present, then any REMAINING engine the envelope
// declares (a new-axis competitor like jena-text / hdt-cpp / vlog) in its own key order. This
// is what makes a new competitor column flow through WITHOUT editing this script per gather.
function engineIdsOf(envelope) {
  const declared = Object.keys(envelope.engines || {});
  const ordered = ENGINE_ORDER.filter((id) => declared.includes(id));
  const extra = declared.filter((id) => !ENGINE_ORDER.includes(id));
  return [...ordered, ...extra];
}

// Per-row count agreement can be recorded as `all_agree` (sp2b/watdiv) OR a plain boolean
// `agree` (geo) OR `same_ruleset_agree` (materialize: sparq + the validated-encoding
// Datalog engines run the SAME rule set and must agree; Jena's profile-different closure
// is a documented caveat, deliberately outside the flag) OR left absent (fts, which
// cross-checks engine-vs-engine differently). Read whichever is present; a missing flag
// stays null (renders "—", never a fabricated ✓).
function rowCountMatch(cc) {
  if (typeof cc.all_agree === "boolean") return cc.all_agree;
  if (typeof cc.agree === "boolean") return cc.agree;
  if (typeof cc.same_ruleset_agree === "boolean") return cc.same_ruleset_agree;
  return null;
}

function parseTsv(tsv) {
  // 3-col "<query>\t<rows|ERROR>\t<best_us|engine>" per line → { query: { rows, us|null } }.
  // 6-col HTTP-profile lines additionally yield { ttfb_us, fresh_us, fresh_ttfb_us }.
  const out = {};
  for (const line of (tsv || "").trim().split("\n")) {
    if (!line) continue;
    const parts = line.split("\t");
    const [query, rowsField, usField] = parts;
    const num = (f) => (f != null && /^-?\d/.test(f) ? Number(f) : null);
    const cell = { rows: num(rowsField), us: num(usField) };
    if (parts.length >= 6) {
      cell.ttfb_us = num(parts[3]);
      cell.fresh_us = num(parts[4]);
      cell.fresh_ttfb_us = num(parts[5]);
    }
    out[query] = cell;
  }
  return out;
}

// sq-hmd7l.28 — an envelope's per-engine TSV key is `<id>_tsv`, but some harnesses
// (fts, geo) sanitise hyphenated ids to underscores (`jena-text` → `jena_text_tsv`). Resolve
// whichever form the envelope actually carries so a hyphenated competitor id is not read as an
// all-null column.
function tsvFor(envelope, id) {
  const direct = envelope[id + "_tsv"];
  if (typeof direct === "string") return direct;
  const under = envelope[id.replace(/-/g, "_") + "_tsv"];
  if (typeof under === "string") return under;
  return "";
}

function shortDigest(engine) {
  const d = engine && engine.image_digest;
  if (!d) return null;
  const m = /sha256:([0-9a-f]{12})/.exec(d);
  return m ? "sha256:" + m[1] + "…" : null;
}

function buildEntry(gathers) {
  // gathers: array of envelope objects for ONE suite (>=1; expect 2 canonical gathers).
  const primary = gathers[gathers.length - 1]; // latest for the metadata/version fields
  const suite = primary.suite;
  const engineIds = engineIdsOf(primary);

  // Parse every gather's per-engine TSV once.
  const parsed = gathers.map((g) => {
    const p = {};
    for (const id of engineIds) p[id] = parseTsv(tsvFor(g, id));
    return { g, p };
  });

  // Query order = the crosscheck key order of the primary gather (deterministic).
  const queries = Object.keys(primary.count_crosscheck || {});

  const rows = queries.map((q) => {
    const cc = primary.count_crosscheck[q] || {};
    const values = {};
    const extended = { ttfb_us: {}, fresh_us: {}, fresh_ttfb_us: {} };
    let anyExtended = false;
    let rowsCount = null;
    for (const id of engineIds) {
      // best-of: MIN non-null across gathers, independently per measured field.
      let best = null;
      const ext = { ttfb_us: null, fresh_us: null, fresh_ttfb_us: null };
      for (const { p } of parsed) {
        const cell = p[id] && p[id][q];
        if (cell && typeof cell.us === "number") {
          best = best == null ? cell.us : Math.min(best, cell.us);
          if (rowsCount == null && typeof cell.rows === "number") rowsCount = cell.rows;
        }
        for (const k of Object.keys(ext)) {
          if (cell && typeof cell[k] === "number") {
            ext[k] = ext[k] == null ? cell[k] : Math.min(ext[k], cell[k]);
          }
        }
      }
      values[id] = best;
      for (const k of Object.keys(ext)) {
        if (ext[k] != null) {
          extended[k][id] = ext[k];
          anyExtended = true;
        }
      }
    }
    // Prefer the crosscheck's expected/sparq count for the honest "rows" column.
    const ccCount =
      cc.expected != null && /^-?\d/.test(String(cc.expected))
        ? Number(cc.expected)
        : cc.sparq != null && /^-?\d/.test(String(cc.sparq))
          ? Number(cc.sparq)
          : rowsCount;
    const row = {
      query: q,
      unit: "µs",
      rows: ccCount,
      values,
      count_match: rowCountMatch(cc),
    };
    if (cc.sparq_oracle_workload && cc.sparq_oracle_workload !== q) {
      row.sparq_oracle_workload = cc.sparq_oracle_workload;
    }
    if (cc.corpus_variant) {
      row.corpus_variant = cc.corpus_variant;
    }
    if (anyExtended) {
      // HTTP-profile extras: values = keep-alive full-request best (primary, above);
      // TTFB + fresh-connect twins carried alongside, same best-of-gathers rule.
      row.values_ttfb = extended.ttfb_us;
      row.values_fresh = extended.fresh_us;
      row.values_fresh_ttfb = extended.fresh_ttfb_us;
    }
    return row;
  });

  // sq-7d3dj.34: an HTTP-profile envelope (6-col ttfb TSVs) records the true
  // per-engine HTTP mode itself — prefer it. CLI-matrix envelopes keep the curated
  // ENGINE_META display labels (stable output for the committed 2026-07-07 matrix).
  const isHttpProfile = !!(primary.tsv_format && /ttfb/.test(primary.tsv_format));
  const engines = engineIds.map((id) => {
    const src = primary.engines[id] || {};
    const meta = ENGINE_META[id] || { label: id, mode: src.mode || "" };
    const digest = shortDigest(src);
    const status = (primary.statuses && primary.statuses[id]) || "ok";
    const version = src.version || "n/a";
    const e = {
      id,
      label: meta.label,
      version: digest ? `${version} (${digest})` : version,
      mode: isHttpProfile ? src.mode || meta.mode : meta.mode || src.mode,
      status,
    };
    if (status === "failed") {
      const wall =
        primary.load && primary.load[id + "_wall_s"]
          ? ` (~${primary.load[id + "_wall_s"]}s wall)`
          : "";
      e.failure = `engine run failed${wall} — no query timings captured; shown as failed, not blank`;
    }
    return e;
  });

  const tsList = gathers
    .map((g) => {
      const m = /canonical-.+-(\d{8}T\d{6}Z)/.exec(g.__file || "") || [];
      return m[1] || g.env?.gathered_at_utc || "?";
    })
    .join(" + ");

  return {
    ...(primary.connection ? { connection: primary.connection } : {}),
    ...(primary.tsv_format && /ttfb/.test(primary.tsv_format)
      ? { profile: "http-ttfb (keep-alive + fresh-connect; values = keep-alive full-request best)" }
      : {}),
    suite,
    scale: primary.scale,
    iters: primary.iters,
    git_commit: primary.git_commit,
    gathered_at_utc: (primary.env && primary.env.gathered_at_utc) || primary.gathered_at_utc,
    canonical: primary.canonical === true,
    // sq-hmd7l.28 — the sp2b/watdiv matrix keeps its exact reviewed prose (incl. the
    // real virtuoso-q09 example) for a byte-stable committed snapshot; other axes get a generic,
    // accurate combine string (no fabricated engine/query name) + the gather's OWN dated dir.
    combine: /^(sp2b|watdiv)(-http)?$/i.test(suite)
      ? `best-of the ${gathers.length} back-to-back canonical gathers (${tsList}): per-engine per-query MIN best_us; solution COUNTS are identical across the gathers. best-of = min-of-${gathers.length}×${primary.iters}, the least-contended estimate; it also removes transient contention spikes (e.g. a virtuoso q09 blip) uniformly for every engine.`
      : `best-of the ${gathers.length} back-to-back canonical gather${gathers.length === 1 ? "" : "s"} (${tsList}): per-engine per-query MIN best_us; solution COUNTS are cross-checked across the gathers. best-of = min-of-${gathers.length}×${primary.iters}, the least-contended estimate.`,
    source: /^(sp2b|watdiv)(-http)?$/i.test(suite)
      ? `canonical gather '${primary.gather}' (${primary.wave}) — raw envelopes committed at bench/canonical-competitor-results/2026-07-07/, ingested by scripts/bench/ingest-canonical-competitors.mjs`
      : `canonical gather '${primary.gather}' (${primary.wave}) — raw envelopes committed under bench/canonical-competitor-results/, ingested by scripts/bench/ingest-canonical-competitors.mjs`,
    count_crosscheck_note: primary.count_crosscheck_note,
    env: {
      host_class: primary.env.host_class,
      quiet_box: primary.env.quiet_box === true,
      gathered_at_utc: primary.env.gathered_at_utc,
      cpu_model: primary.env.cpu_model,
      kernel: primary.env.kernel,
      note: primary.load && primary.load.note,
    },
    engines,
    rows,
  };
}

// ---- sq-hmd7l.28 — bespoke-shape axis adapters ----------------------------
// A few axes do NOT emit the query-row `<engine>_tsv` + `count_crosscheck` layout the
// generic buildEntry consumes. Rather than hand-transcribe them, we normalise each into the
// SAME same_box_comparison shape (engines + rows) here, keyed off the envelope `suite`, so a
// new canonical gather of these axes flows to the site with no further edit. Every adapter is
// DEFENSIVE: if a gather's documented fields are absent (a shape the parallel wave has not
// finalised) it returns null and the ingest SKIPS that suite with a warning — it never
// fabricates a row and never crashes the build.

// Common env/meta projection shared by the bespoke adapters (mirrors buildEntry's tail).
function commonMeta(primary, gathers) {
  return {
    suite: primary.suite,
    scale: primary.scale,
    iters: primary.iters,
    git_commit: primary.git_commit,
    gathered_at_utc: (primary.env && primary.env.gathered_at_utc) || primary.gathered_at_utc,
    canonical: primary.canonical === true,
    combine:
      gathers.length > 1
        ? `best-of ${gathers.length} back-to-back gathers (per-engine MIN); counts cross-checked engine-vs-engine.`
        : "single gather; counts cross-checked engine-vs-engine.",
    source: `canonical gather '${primary.gather}' (${primary.wave}) — raw envelope committed under bench/canonical-competitor-results/, ingested by scripts/bench/ingest-canonical-competitors.mjs`,
    count_crosscheck_note: primary.count_crosscheck_note || primary.caveat,
    env: {
      host_class: (primary.env && (primary.env.host_class || primary.env.host)) || "quiet box",
      quiet_box: !!(primary.env && primary.env.quiet_box),
      gathered_at_utc: primary.env && primary.env.gathered_at_utc,
      cpu_model: primary.env && primary.env.cpu_model,
      kernel: primary.env && primary.env.kernel,
      note: primary.mode || primary.caveat,
    },
  };
}

function metaEngines(primary, ids) {
  return ids.map((id) => {
    const src = (primary.engines && primary.engines[id]) || {};
    const meta = ENGINE_META[id] || { label: id, mode: src.mode || "" };
    return {
      id,
      label: meta.label,
      version: src.version || "n/a",
      mode: meta.mode || src.mode || "",
      status: src.status === "absent" || src.status === "failed" ? "failed" : "ok",
    };
  });
}

// HDT decode-only (suite "hdt"): sparq decodes .hdt → native Graph, hdt-cpp decodes → N-Triples;
// the ONE like-for-like metric is decode wall-clock, cross-checked on the decoded triple count.
function normalizeHdt(primary, gathers) {
  const sm = primary.sparq_metrics || {};
  const decodeCell = sm.decode_s || sm.load_decode_s || sm.snikmeta_decode_s;
  const sparqDecodeS = decodeCell && typeof decodeCell.value !== "undefined" ? Number(decodeCell.value) : null;
  const cc = primary.count_agreement || {};
  const ids = engineIdsOf(primary).length ? engineIdsOf(primary) : ["sparq", "hdt-cpp"];
  // decode seconds → µs for a uniform unit with the other axes. hdt-cpp:
  // sq-hmd7l.33 — the harness now records a NUMERIC best-of-N in-container decode_us
  // (container spawn excluded) under hdt_cpp_metrics; absent (an older envelope, or the
  // in-container timing fell back) → null (renders n/a, never a fabricated number).
  const cppCell = (primary.hdt_cpp_metrics || {}).decode_us;
  const cppUs = cppCell && typeof cppCell.value !== "undefined" ? Number(cppCell.value) : null;
  const values = {
    sparq: sparqDecodeS != null && Number.isFinite(sparqDecodeS) ? Math.round(sparqDecodeS * 1e6) : null,
    "hdt-cpp": cppUs != null && Number.isFinite(cppUs) ? Math.round(cppUs) : null,
  };
  const row = {
    query: "decode",
    unit: "µs",
    rows: cc.expected != null ? Number(cc.expected) : null,
    values,
    count_match: typeof cc.all_agree === "boolean" ? cc.all_agree : null,
  };
  return {
    ...commonMeta(primary, gathers),
    engines: metaEngines(primary, ids),
    rows: [row],
  };
}

// PSS update parity (suite "pss-update-parity"): per-engine p99 update latency over the LDP-CRUD
// stream, cross-checked on the post-workload quad count. One row per recorded latency metric.
function normalizeUpdate(primary, gathers) {
  const lat = primary.latency_ms || {};
  const ids = engineIdsOf(primary).length ? engineIdsOf(primary) : Object.keys(lat);
  if (!ids.length) return null;
  const cc = primary.count_crosscheck || {};
  const mkRow = (query, pick) => {
    const values = {};
    for (const id of ids) {
      const cell = lat[id];
      const v = cell && typeof cell === "object" ? cell[pick] : undefined;
      values[id] = typeof v === "number" ? Math.round(v * 1000) : null; // ms → µs
    }
    return {
      query,
      unit: "µs",
      rows: cc.sparq != null && /^-?\d/.test(String(cc.sparq)) ? Number(cc.sparq) : null,
      values,
      count_match: typeof cc.all_agree === "boolean" ? cc.all_agree : null,
    };
  };
  const rows = ["p50", "p99", "max"]
    .filter((p) => ids.some((id) => lat[id] && typeof lat[id][p] === "number"))
    .map((p) => mkRow(`update ${p}`, p));
  if (!rows.length) return null;
  return {
    ...commonMeta(primary, gathers),
    engines: metaEngines(primary, ids),
    rows,
  };
}

// suite → bespoke adapter. Any suite not listed uses the generic query-row buildEntry.
const BESPOKE_ADAPTERS = {
  hdt: normalizeHdt,
  "pss-update-parity": normalizeUpdate,
};

// ---- load envelopes (across every results dir), group by suite ----------------------
const bySuite = new Map();
let fileCount = 0;
for (const dir of resultsDirs) {
  for (const f of readdirSync(dir).filter((x) => x.endsWith(".json"))) {
    const env = JSON.parse(readFileSync(join(dir, f), "utf8"));
    env.__file = f;
    fileCount += 1;
    const key = env.suite;
    (bySuite.get(key) || bySuite.set(key, []).get(key)).push(env);
  }
}
if (!fileCount) {
  console.error(`[ingest-canonical] no envelope JSON in: ${resultsDirs.join(", ")}`);
  process.exit(1);
}

// Assert counts identical across gathers of a suite (fail loud, never silently pick one).
// Bespoke-shape suites (hdt/update) carry no per-query `<engine>_tsv`, so this TSV-level
// assertion does not apply; their adapters do their own cross-check. Skip them here.
for (const [suite, gathers] of bySuite) {
  gathers.sort((a, b) => (a.__file < b.__file ? -1 : 1));
  if (BESPOKE_ADAPTERS[suite]) continue;
  const ref = gathers[0];
  for (const g of gathers.slice(1)) {
    for (const id of engineIdsOf(ref)) {
      const a = parseTsv(tsvFor(ref, id));
      const b = parseTsv(tsvFor(g, id));
      for (const q of Object.keys(a)) {
        // sq-7d3dj.34: an ERROR row (rows=null, e.g. a per-query timeout that
        // fired in only one of the gathers) is a MISSING count, not a DISAGREEING count —
        // only two conflicting numeric counts refuse the ingest.
        if (b[q] && a[q].rows != null && b[q].rows != null && a[q].rows !== b[q].rows) {
          console.error(
            `[ingest-canonical] COUNT MISMATCH ${suite} ${id} ${q}: ${a[q].rows} vs ${b[q].rows} — refusing to ingest.`,
          );
          process.exit(1);
        }
      }
    }
  }
}

// Deterministic suite order: sp2b-ish first, then the rest alphabetically.
const suiteOrder = [...bySuite.keys()].sort((a, b) => {
  const rank = (s) => (/sp2b/i.test(s) ? 0 : /watdiv/i.test(s) ? 1 : 2);
  return rank(a) - rank(b) || (a < b ? -1 : 1);
});
// sq-hmd7l.28 — dispatch each suite to its bespoke adapter or the generic query-row
// buildEntry. A defensive adapter that returns null (an unfinalised gather shape) is skipped
// with a warning so the ingest never crashes and never fabricates a row.
const sameBox = [];
for (const s of suiteOrder) {
  const gathers = bySuite.get(s);
  const adapter = BESPOKE_ADAPTERS[s];
  const entry = adapter ? adapter(gathers[gathers.length - 1], gathers) : buildEntry(gathers);
  if (entry) sameBox.push(entry);
  else console.warn(`[ingest-canonical] suite '${s}': adapter produced no comparable rows (unfinalised gather shape) — skipped, not fabricated.`);
}

// ---- rewrite ONLY same_box_comparisons in competitors.json --------------------------
const competitors = JSON.parse(readFileSync(competitorsPath, "utf8"));
competitors.same_box_comparisons = sameBox;
writeFileSync(competitorsPath, JSON.stringify(competitors, null, 2) + "\n");
console.log(
  `[ingest-canonical] wrote ${sameBox.length} canonical same_box_comparisons ` +
    `(${sameBox.map((c) => `${c.suite}:${c.rows.length}q`).join(", ")}) → bench/dashboard/competitors.json`,
);
