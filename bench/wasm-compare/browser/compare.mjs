#!/usr/bin/env node
// [FABLE-5] sq-hmd7l.17 — cross-LIBRARY comparison orchestrator, layered on
// the sq-3ul2n.1 harness (same server, same workload module, same envelope
// discipline — NOT a second harness).
//
// Compares sparq's shipped WASM bundle against the pinned competitor stacks
// from bench/competitors.json on the SAME deterministic workload:
//   sparq            — @sparq-org/sparq shipped bundle (js/ build:wasm output)
//   oxigraph         — the official npm WASM package (node.js / web.js builds)
//   n3js-quadstore   — N3.js + quadstore(memory-level) + quadstore-comunica
//                      (Node runtime only, per the registry entry)
//
//   node compare.mjs                                  # node runtime, all libraries
//   node compare.mjs --runtime chromium               # in-browser (headless Chrome)
//   node compare.mjs --runtime all --quick            # smoke tier
//   node compare.mjs --lib sparq,oxigraph
//
// Competitor packages are GATHER-ONLY installs (never committed):
// run the INSTALL_HINT from compare-workload.mjs in this directory first;
// missing packages SKIP WITH NOTICE (exit stays 0), never fabricate.
//
// INVARIANT: no latency row without row-count agreement — each library's
// counts are oracle-checked in-workload, and this orchestrator re-checks
// them ACROSS libraries/runtimes before reporting (divergence → exit 1).
//
// Every number is ADVISORY / NON-CANONICAL except the row counts themselves.
// The DETERMINISTIC comparison (bundle bytes) lives in ../bundle.mjs.

import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { startServer } from "./server.mjs";
import { runCompareWorkload, INSTALL_HINT } from "./compare-workload.mjs";
import { makeAdapter, pkgVersion, LIBRARIES } from "./adapters.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..", "..");
const SUITE = "wasm-compare/compare";
const BEAD = "sq-hmd7l.17";
const RUNTIMES = ["node", "chromium"];
const ADVISORY_NOTE =
  "ADVISORY / NON-CANONICAL — cross-library latency measured on the running host. The " +
  "deliverable is the ratio structure over an oracle-checked identical workload; do not bake " +
  "any ms value into committed docs, tests, or gates. Deterministic gates = the per-library " +
  "row-count oracle + the cross-library row-count agreement check. The deterministic " +
  "bundle-bytes comparison is bench/wasm-compare/bundle.mjs.";

// ---------- CLI ----------
function parseArgs(argv) {
  const opts = {
    libs: [],
    runtimes: [],
    quick: false,
    out: path.join(HERE, "results"),
    timeoutMs: 900_000,
    child: false,
    childOut: undefined,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--lib") {
      for (const l of (argv[++i] ?? usage("--lib requires a value")).split(",")) {
        if (l === "all") opts.libs.push(...LIBRARIES);
        else if (LIBRARIES.includes(l)) opts.libs.push(l);
        else usage(`unknown library '${l}' (expected ${LIBRARIES.join("/")}/all)`);
      }
    } else if (a === "--runtime") {
      for (const r of (argv[++i] ?? usage("--runtime requires a value")).split(",")) {
        if (r === "all") opts.runtimes.push(...RUNTIMES);
        else if (RUNTIMES.includes(r)) opts.runtimes.push(r);
        else usage(`unknown runtime '${r}' (expected ${RUNTIMES.join("/")}/all)`);
      }
    } else if (a === "--quick") opts.quick = true;
    else if (a === "--out") opts.out = path.resolve(argv[++i] ?? usage("--out requires a value"));
    else if (a === "--timeout-ms") opts.timeoutMs = Number(argv[++i]) || opts.timeoutMs;
    else if (a === "--child") opts.child = true;
    else if (a === "--child-out") opts.childOut = argv[++i];
    else usage(`unknown argument '${a}'`);
  }
  if (opts.libs.length === 0) opts.libs = [...LIBRARIES];
  if (opts.runtimes.length === 0) opts.runtimes = ["node"];
  opts.libs = [...new Set(opts.libs)];
  opts.runtimes = [...new Set(opts.runtimes)];
  return opts;
}
function usage(msg) {
  if (msg) console.error(`error: ${msg}`);
  console.error(
    "usage: node compare.mjs [--lib sparq|oxigraph|n3js-quadstore|all]... " +
      "[--runtime node|chromium|all]... [--quick] [--out <dir>] [--timeout-ms <n>]",
  );
  process.exit(2);
}

// ---------- child mode: one library, in-process, fresh V8 per library ----------
async function childMain(opts) {
  const library = opts.libs[0];
  const adapter = await makeAdapter(library);
  let result;
  if (adapter.missing) {
    result = { ok: true, skipped: true, reason: adapter.reason, library };
  } else {
    try {
      const wl = await runCompareWorkload({
        adapter,
        quick: opts.quick,
        log: (m) => console.error(`[${library}] ${m}`),
      });
      result = { ok: true, library, version: adapter.version, rows: wl.rows, skipped_phases: wl.skipped };
    } catch (err) {
      result = { ok: false, library, error: String(err?.stack ?? err) };
    }
  }
  await writeFile(opts.childOut, JSON.stringify(result, null, 2));
  process.exit(result.ok ? 0 : 1);
}

// ---------- parent runners ----------
async function runNodeLibrary(library, opts) {
  const outFile = path.join(os.tmpdir(), `wasm-compare-child-${process.pid}-${library}.json`);
  const args = [fileURLToPath(import.meta.url), "--child", "--lib", library, "--child-out", outFile];
  if (opts.quick) args.push("--quick");
  const r = spawnSync(process.execPath, args, { stdio: ["ignore", "inherit", "inherit"], timeout: opts.timeoutMs });
  let parsed;
  try {
    parsed = JSON.parse(await readFile(outFile, "utf8"));
  } catch {
    return { ok: false, library, error: `child exited ${r.status} without a result file` };
  }
  parsed.version ??= parsed.skipped ? undefined : null;
  return parsed;
}

async function runChromiumLibrary(library, opts, port, playwright) {
  if (library === "n3js-quadstore") {
    return {
      ok: true,
      skipped: true,
      library,
      reason:
        "n3js-quadstore is a Node-runtime column (bench/competitors.json): the quadstore stack " +
        "needs a bundler for browser use — skipped in chromium, not approximated",
    };
  }
  if (library === "oxigraph" && !(await pkgVersion("oxigraph"))) {
    return {
      ok: true,
      skipped: true,
      library,
      reason: `oxigraph npm package not installed — gather-only install: \`${INSTALL_HINT}\``,
    };
  }
  let browser;
  try {
    browser = await playwright.chromium.launch();
  } catch (err) {
    const firstLine = String(err?.message ?? err).split("\n")[0];
    return {
      ok: true,
      skipped: true,
      library,
      reason: `chromium could not launch: ${firstLine} — \`npx playwright install chromium\``,
    };
  }
  try {
    const version = `chromium ${browser.version()} (playwright)`;
    const page = await browser.newPage();
    page.on("console", (m) => {
      const text = m.text();
      if (text.startsWith("[compare]") || m.type() === "error") console.error(`[chromium/${library}] ${text}`);
    });
    page.on("pageerror", (e) => console.error(`[chromium/${library}] pageerror: ${e}`));
    const q = opts.quick ? "&quick=1" : "";
    await page.goto(`http://127.0.0.1:${port}/harness/page/compare.html?lib=${library}${q}`);
    await page.waitForFunction("window.__WASM_COMPARE_DONE__ === true", null, { timeout: opts.timeoutMs });
    const result = await page.evaluate("window.__WASM_COMPARE_RESULT__");
    if (!result.ok) return { ok: false, library, error: result.error };
    const version_str =
      library === "sparq"
        ? JSON.parse(await readFile(path.join(REPO, "js", "package.json"), "utf8")).version + " (local shipped bundle)"
        : await pkgVersion("oxigraph");
    return { ok: true, library, version: version_str, engine_version: version, rows: result.rows, skipped_phases: result.skipped };
  } finally {
    await browser.close();
  }
}

// ---------- reporting ----------
const rowKey = (r) => [r.phase, r.format ?? "", r.triples ?? "", r.query ?? "", r.kind ?? ""].join("|");
const rowLabel = (r) => {
  let l = r.phase;
  if (r.format) l += ` ${r.format}@${r.triples}`;
  if (r.query) l += ` ${r.query}`;
  if (r.kind && r.kind !== "total") l += ` [${r.kind}]`;
  return l;
};

function printTables(envelopes) {
  const ran = envelopes.filter((e) => e.rows);
  if (ran.length === 0) return { divergent: false };
  const cols = ran.map((e) => `${e.runtime}/${e.library}`);
  const byCol = new Map(ran.map((e) => [`${e.runtime}/${e.library}`, new Map(e.rows.map((r) => [rowKey(r), r]))]));
  const keys = [];
  for (const e of ran) for (const r of e.rows) if (!keys.includes(rowKey(r))) keys.push(rowKey(r));

  const baseCol = cols.find((c) => c.endsWith("/sparq")) ?? cols[0];
  const w0 = 34;
  const wc = Math.max(14, ...cols.map((c) => c.length + 2));
  console.log(`\n== cross-library wall time, ms (ADVISORY / NON-CANONICAL) ==`);
  console.log("phase".padEnd(w0) + cols.map((c) => c.padStart(wc)).join("") + `   vs ${baseCol}`);
  let divergent = false;
  for (const k of keys) {
    const sample = ran.flatMap((e) => byCol.get(`${e.runtime}/${e.library}`).get(k) ?? []).at(0);
    const cells = cols.map((c) => {
      const r = byCol.get(c).get(k);
      return r ? r.ms.toFixed(r.ms < 1 ? 3 : 1).padStart(wc) : "—".padStart(wc);
    });
    const base = byCol.get(baseCol).get(k);
    const ratios = cols
      .filter((c) => c !== baseCol)
      .map((c) => {
        const r = byCol.get(c).get(k);
        return r && base && base.ms > 0 ? `×${(r.ms / base.ms).toFixed(2)}` : `×—`;
      })
      .join(" ");
    console.log(rowLabel(sample).padEnd(w0) + cells.join("") + "   " + ratios);
    // Deterministic oracle: row counts must agree across libraries AND runtimes.
    const counts = new Set(cols.map((c) => byCol.get(c).get(k)?.rows).filter((v) => v !== undefined));
    if (counts.size > 1) {
      divergent = true;
      console.log(`  !! ROW-COUNT DIVERGENCE at ${k}: ${[...counts].join(" vs ")}`);
    }
  }
  return { divergent };
}

// ---------- main ----------
const opts = parseArgs(process.argv.slice(2));
if (opts.child) {
  if (!opts.childOut || opts.libs.length !== 1) usage("--child requires --lib <one> and --child-out <file>");
  await childMain(opts);
}

// sparq shipped-bundle artifacts must exist (raw Store class only).
const WASM = path.join(REPO, "js", "wasm", "sparq_wasm_bg.wasm");
try {
  await stat(WASM);
} catch {
  console.error(
    `[compare] missing shipped bundle ${path.relative(REPO, WASM)} — build it first:\n` +
      `  cd ${REPO} && npm ci --ignore-scripts && cd js && npm run build:wasm`,
  );
  process.exit(2);
}
await mkdir(opts.out, { recursive: true });

const wasmBytes = await readFile(WASM);
const bundle = {
  file: path.relative(REPO, WASM),
  bytes: wasmBytes.length,
  sha256: createHash("sha256").update(wasmBytes).digest("hex"),
};
const git = spawnSync("git", ["-C", REPO, "rev-parse", "HEAD"], { encoding: "utf8" });
const host = {
  platform: os.platform(),
  arch: os.arch(),
  cpus: os.cpus().length,
  cpu_model: os.cpus()[0]?.model ?? "unknown",
  node: process.version,
  canonical: false,
};

const wantsChromium = opts.runtimes.includes("chromium");
let playwright;
if (wantsChromium) {
  try {
    playwright = await import("playwright");
  } catch {
    console.error("[compare] playwright is not installed — run `npm ci` in bench/wasm-compare/browser/ first");
    process.exit(2);
  }
}
const require = createRequire(import.meta.url);
const playwrightVersion = wantsChromium ? require("playwright/package.json").version : undefined;

const { server, port } = wantsChromium
  ? await startServer({ "/js/": path.join(REPO, "js"), "/harness/": HERE, "/nm/": path.join(HERE, "node_modules") })
  : { server: undefined, port: 0 };

const utc = new Date().toISOString().replace(/[:.]/g, "-");
const envelopes = [];
let failed = false;
try {
  for (const runtime of opts.runtimes) {
    for (const library of opts.libs) {
      console.error(`\n[compare] ${runtime}/${library}${opts.quick ? " (quick)" : ""}`);
      const t0 = performance.now();
      const result =
        runtime === "node"
          ? await runNodeLibrary(library, opts)
          : await runChromiumLibrary(library, opts, port, playwright);
      const envelope = {
        suite: SUITE,
        bead: BEAD,
        advisory: true,
        note: ADVISORY_NOTE,
        runtime,
        library,
        library_version: result.version ?? null,
        engine_version: result.engine_version ?? (runtime === "node" ? `node ${process.version}` : null),
        playwright: runtime === "chromium" ? playwrightVersion : undefined,
        run_utc: new Date().toISOString(),
        git_commit: git.status === 0 ? git.stdout.trim() : null,
        host,
        bundle: library === "sparq" ? bundle : undefined,
        quick: opts.quick,
        harness_wall_ms: performance.now() - t0,
        ...(result.skipped === true
          ? { skipped: true, reason: result.reason }
          : result.ok
            ? { rows: result.rows, skipped_phases: result.skipped_phases }
            : { failed: true, error: result.error }),
      };
      if (result.ok === false) failed = true;
      envelopes.push(envelope);
      const file = path.join(opts.out, `compare-${runtime}-${library}-${utc}.json`);
      await writeFile(file, JSON.stringify(envelope, null, 2));
      console.error(
        `[compare] wrote ${path.relative(process.cwd(), file)}` +
          `${envelope.skipped ? " (SKIPPED-WITH-NOTICE)" : envelope.failed ? " (FAILED)" : ""}`,
      );
      if (envelope.skipped) console.error(`[compare]   reason: ${envelope.reason}`);
    }
  }
} finally {
  server?.close();
}

const { divergent } = printTables(envelopes);
const skippedCols = envelopes.filter((e) => e.skipped).map((e) => `${e.runtime}/${e.library}`);
if (skippedCols.length > 0)
  console.log(`\nskipped-with-notice: ${skippedCols.join(", ")} (see envelope \`reason\` fields)`);
console.log(`\n${ADVISORY_NOTE}`);
if (divergent) {
  console.error("\n[compare] FAIL: cross-library row-count divergence (correctness, not timing)");
  process.exit(1);
}
if (failed) {
  console.error("\n[compare] FAIL: at least one library run errored (see envelopes)");
  process.exit(1);
}
