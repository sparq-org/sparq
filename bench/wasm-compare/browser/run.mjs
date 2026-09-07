#!/usr/bin/env node
// [FABLE-5] sq-3ul2n.1 — cross-engine BROWSER wasm measurement harness (orchestrator).
//
// Drives the SAME shipped @sparq-org/sparq web bundle (js `build:wasm` output) +
// the SAME deterministic workload through headless Chromium, Firefox and
// WebKit (via Playwright) plus a plain-Node baseline, timing each PHASE
// separately — fetch / compile / instantiate (and the shipped overlapped
// instantiateStreaming path), store load/parse per format+size, query shapes
// cold (tier-up-inclusive) vs warm, serialization out, and the
// boundary-marshalling probe set — so a regression attributes to a LAYER,
// not just a total.
//
//   node run.mjs --engine chromium            # one engine
//   node run.mjs --engine all                 # node + chromium + firefox + webkit
//   node run.mjs --engine webkit --quick      # smoke tier (25k triples, few iters)
//
// Browsers that cannot launch on this host SKIP WITH AN EXPLICIT NOTICE
// (an envelope with skipped:true) — never fabricated numbers. Exit codes:
//   0 = every requested engine ran or skipped-with-notice
//   1 = a run FAILED (page error / row-count oracle divergence across engines)
//   2 = usage / missing build artifacts
//
// Every emitted number is ADVISORY / NON-CANONICAL: measured on whatever host
// runs this (typically a shared EC2 work box). The deliverable is repeatable
// per-phase ATTRIBUTION and the cross-engine ratio structure, not headline
// numbers. Never bake emitted values into docs/tests/gates (repo rule).

import http from "node:http";
import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { startServer } from "./server.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..", "..");
const SUITE = "wasm-compare/browser";
const BEAD = "sq-3ul2n.1";
const ADVISORY_NOTE =
  "ADVISORY / NON-CANONICAL — every `ms` is best-effort wall time measured on the running " +
  "host (typically a non-canonical work box). The per-phase attribution and cross-engine " +
  "ratios are the deliverable, not the absolute numbers; do not bake any value into " +
  "committed docs, tests, or gates. Deterministic gate = the row/triple-count oracle.";

const BROWSER_ENGINES = ["chromium", "firefox", "webkit"];
const ALL_ENGINES = ["node", ...BROWSER_ENGINES];

// ---------- CLI ----------
function parseArgs(argv) {
  const opts = { engines: [], quick: false, out: path.join(HERE, "results"), build: false, timeoutMs: 900_000 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--engine") {
      const v = argv[++i];
      if (!v) usage(`--engine requires a value`);
      for (const e of v.split(",")) {
        if (e === "all") opts.engines.push(...ALL_ENGINES);
        else if (ALL_ENGINES.includes(e)) opts.engines.push(e);
        else usage(`unknown engine '${e}' (expected ${ALL_ENGINES.join("/")}/all)`);
      }
    } else if (a === "--quick") opts.quick = true;
    else if (a === "--build") opts.build = true;
    else if (a === "--out") opts.out = path.resolve(argv[++i] ?? usage("--out requires a value"));
    else if (a === "--timeout-ms") opts.timeoutMs = Number(argv[++i]) || opts.timeoutMs;
    else usage(`unknown argument '${a}'`);
  }
  if (opts.engines.length === 0) opts.engines = [...ALL_ENGINES];
  opts.engines = [...new Set(opts.engines)];
  return opts;
}
function usage(msg) {
  if (msg) console.error(`error: ${msg}`);
  console.error(
    "usage: node run.mjs [--engine node|chromium|firefox|webkit|all]... [--quick] [--build] [--out <dir>] [--timeout-ms <n>]",
  );
  process.exit(2);
}

// ---------- artifact checks ----------
const WASM = path.join(REPO, "js", "wasm", "sparq_wasm_bg.wasm");
const GLUE = path.join(REPO, "js", "wasm", "sparq_wasm.js");
const DIST = path.join(REPO, "js", "dist", "index.js");

async function ensureArtifacts(build) {
  const missing = [];
  for (const f of [WASM, GLUE, DIST]) {
    try {
      await stat(f);
    } catch {
      missing.push(path.relative(REPO, f));
    }
  }
  if (missing.length === 0) return;
  if (build) {
    console.error(`[run] building missing artifacts (${missing.join(", ")})…`);
    for (const script of ["build:wasm", "build:ts"]) {
      const r = spawnSync("npm", ["run", script], { cwd: path.join(REPO, "js"), stdio: "inherit" });
      if (r.status !== 0) {
        console.error(`[run] npm run ${script} failed — is the repo-root \`npm ci\` done and wasm-pack installed?`);
        process.exit(2);
      }
    }
    return ensureArtifacts(false);
  }
  console.error(
    `[run] missing shipped-bundle artifacts:\n  ${missing.join("\n  ")}\n` +
      `build them first (or pass --build):\n` +
      `  cd ${REPO} && npm ci --ignore-scripts\n` +
      `  cd js && npm run build:wasm && npx tsc`,
  );
  process.exit(2);
}

// ---------- static server (serves the repo's js/ tree + this harness dir) ----------
// Implementation extracted to server.mjs (shared with compare.mjs, sq-hmd7l.17).
const ROOTS = { "/js/": path.join(REPO, "js"), "/harness/": HERE };

// ---------- engine runners ----------
async function runNode(opts) {
  const outFile = path.join(os.tmpdir(), `wasm-compare-node-${process.pid}.json`);
  const args = [path.join(HERE, "node-baseline.mjs"), "--out", outFile];
  if (opts.quick) args.push("--quick");
  const r = spawnSync(process.execPath, args, { stdio: ["ignore", "inherit", "inherit"], timeout: opts.timeoutMs });
  if (r.status !== 0) return { ok: false, error: `node-baseline exited ${r.status}` };
  const result = JSON.parse(await readFile(outFile, "utf8"));
  result.version = `node ${process.version} (V8 ${process.versions.v8})`;
  return result;
}

async function runBrowser(engine, opts, port, playwright) {
  const type = playwright[engine];
  const rows = [];
  const skipped = [];
  let version;
  // One FRESH browser process per mode: wasm compilation caches are
  // per-process, so running `main` (decomposed WebAssembly.compile) in the
  // same process as `streaming` would let whichever page runs second hit a
  // warm compile cache and mis-attribute the compile phase.
  for (const mode of ["streaming", "main"]) {
    let browser;
    try {
      browser = await type.launch(
        engine === "firefox"
          ? { firefoxUserPrefs: { "privacy.reduceTimerPrecision": false } } // finer perf.now(); measure() batching covers engines that stay coarse
          : {},
      );
    } catch (err) {
      const firstLine =
        String(err?.message ?? err)
          .split("\n")
          .map((l) => l.trim())
          .filter((l) => l && !l.startsWith("browserType.launch"))[0] ?? String(err?.message ?? err).slice(0, 200);
      const reason =
        `${engine} could not launch on this host: ${firstLine} — ` +
        `install it with \`npx playwright install ${engine}\` (+ \`sudo npx playwright install-deps ${engine}\` for OS libraries)`;
      console.error(`[run] SKIP-WITH-NOTICE: ${reason}`);
      return { skipped: true, reason };
    }
    try {
      version = `${engine} ${browser.version()} (playwright)`;
      const page = await browser.newPage();
      page.on("console", (m) => {
        const text = m.text();
        if (text.startsWith("[harness]") || m.type() === "error") console.error(`[${engine}/${mode}] ${text}`);
      });
      page.on("pageerror", (e) => console.error(`[${engine}/${mode}] pageerror: ${e}`));
      const q = opts.quick ? "&quick=1" : "";
      await page.goto(`http://127.0.0.1:${port}/harness/page/index.html?mode=${mode}${q}`);
      await page.waitForFunction("window.__WASM_COMPARE_DONE__ === true", null, { timeout: opts.timeoutMs });
      const result = await page.evaluate("window.__WASM_COMPARE_RESULT__");
      if (!result.ok) return { ok: false, error: `${engine}/${mode}: ${result.error}`, version };
      rows.push(...result.rows);
      skipped.push(...(result.skipped ?? []));
    } finally {
      await browser.close();
    }
  }
  return { ok: true, rows, skipped, version };
}

// ---------- reporting ----------
const rowKey = (r) =>
  [r.phase, r.format ?? "", r.triples ?? "", r.query ?? "", r.kind ?? ""].join("|");
const rowLabel = (r) => {
  let l = r.phase;
  if (r.format) l += ` ${r.format}@${r.triples}`;
  if (r.query) l += ` ${r.query}`;
  if (r.kind && r.kind !== "total") l += ` [${r.kind}]`;
  return l;
};

function printTables(envelopes) {
  const ran = envelopes.filter((e) => !e.skipped);
  if (ran.length === 0) return { divergent: false };
  const keys = [];
  const byEngine = new Map(ran.map((e) => [e.engine, new Map(e.rows.map((r) => [rowKey(r), r]))]));
  for (const r of ran[0].rows) keys.push(rowKey(r));
  for (const e of ran.slice(1))
    for (const r of e.rows) if (!keys.includes(rowKey(r))) keys.push(rowKey(r));

  const baseline = ran.find((e) => e.engine === "node") ?? ran[0];
  const engines = ran.map((e) => e.engine);
  const w0 = 46, wc = 14;
  console.log(`\n== per-phase wall time, ms (${ADVISORY_NOTE.split("—")[0].trim()}) ==`);
  console.log("phase".padEnd(w0) + engines.map((e) => e.padStart(wc)).join("") + `   vs ${baseline.engine}`);
  let divergent = false;
  for (const k of keys) {
    const sample = ran.flatMap((e) => byEngine.get(e.engine).get(k) ?? []).at(0);
    const cells = engines.map((eng) => {
      const r = byEngine.get(eng).get(k);
      return r ? r.ms.toFixed(r.ms < 1 ? 3 : 1).padStart(wc) : "—".padStart(wc);
    });
    const base = byEngine.get(baseline.engine).get(k);
    const ratios = engines
      .filter((eng) => eng !== baseline.engine)
      .map((eng) => {
        const r = byEngine.get(eng).get(k);
        return r && base && base.ms > 0 ? `${eng[0]}×${(r.ms / base.ms).toFixed(2)}` : `${eng[0]}×—`;
      })
      .join(" ");
    console.log(rowLabel(sample).padEnd(w0) + cells.join("") + "   " + ratios);
    // deterministic oracle: row counts must agree across engines
    const counts = new Set(
      engines.map((eng) => byEngine.get(eng).get(k)?.rows).filter((v) => v !== undefined),
    );
    if (counts.size > 1) {
      divergent = true;
      console.log(`  !! ROW-COUNT DIVERGENCE at ${k}: ${[...counts].join(" vs ")}`);
    }
  }
  return { divergent };
}

// ---------- main ----------
const opts = parseArgs(process.argv.slice(2));
await ensureArtifacts(opts.build);
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
  canonical: false, // this harness never produces canonical numbers by itself
};

const wantsBrowser = opts.engines.some((e) => BROWSER_ENGINES.includes(e));
let playwright;
if (wantsBrowser) {
  try {
    playwright = await import("playwright");
  } catch {
    console.error(
      "[run] playwright is not installed — run `npm ci` (or `npm install`) in bench/wasm-compare/browser/ first",
    );
    process.exit(2);
  }
}
const require = createRequire(import.meta.url);
const playwrightVersion = wantsBrowser ? require("playwright/package.json").version : undefined;

const utc = new Date().toISOString().replace(/[:.]/g, "-");
const envelopes = [];
let failed = false;

const { server, port } = wantsBrowser ? await startServer(ROOTS) : { server: undefined, port: 0 };
if (server) {
  // Prewarm: one throwaway GET of the biggest asset so the FIRST engine's
  // fetch phase does not also pay the server/OS cold file read.
  await new Promise((resolve, reject) => {
    http.get(`http://127.0.0.1:${port}/js/wasm/sparq_wasm_bg.wasm`, (res) => {
      res.resume();
      res.on("end", resolve);
    }).on("error", reject);
  });
}
try {
  for (const engine of opts.engines) {
    console.error(`\n[run] engine: ${engine}${opts.quick ? " (quick)" : ""}`);
    const t0 = performance.now();
    const result =
      engine === "node" ? await runNode(opts) : await runBrowser(engine, opts, port, playwright);
    const envelope = {
      suite: SUITE,
      bead: BEAD,
      advisory: true,
      note: ADVISORY_NOTE,
      engine,
      engine_version: result.version ?? null,
      playwright: engine === "node" ? undefined : playwrightVersion,
      run_utc: new Date().toISOString(),
      git_commit: git.status === 0 ? git.stdout.trim() : null,
      host,
      bundle,
      quick: opts.quick,
      harness_wall_ms: performance.now() - t0,
      ...(result.skipped === true
        ? { skipped: true, reason: result.reason }
        : result.ok
          ? { rows: result.rows, skipped_phases: result.skipped }
          : { failed: true, error: result.error }),
    };
    if (result.ok === false) failed = true;
    envelopes.push(envelope);
    const file = path.join(opts.out, `${engine}-${utc}.json`);
    await writeFile(file, JSON.stringify(envelope, null, 2));
    console.error(`[run] wrote ${path.relative(process.cwd(), file)}${envelope.skipped ? " (SKIPPED-WITH-NOTICE)" : envelope.failed ? " (FAILED)" : ""}`);
  }
} finally {
  server?.close();
}

const { divergent } = printTables(envelopes.filter((e) => e.rows));
const skippedEngines = envelopes.filter((e) => e.skipped).map((e) => e.engine);
if (skippedEngines.length > 0)
  console.log(`\nskipped-with-notice: ${skippedEngines.join(", ")} (see envelope \`reason\` fields)`);
console.log(`\n${ADVISORY_NOTE}`);
if (divergent) {
  console.error("\n[run] FAIL: cross-engine row-count divergence (correctness, not timing)");
  process.exit(1);
}
if (failed) {
  console.error("\n[run] FAIL: at least one engine run errored (see envelopes)");
  process.exit(1);
}
