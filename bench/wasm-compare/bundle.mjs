#!/usr/bin/env node
// [FABLE-5] sq-hmd7l.17 — DETERMINISTIC WASM bundle-size comparison:
// sparq's shipped web bundle vs the official Oxigraph npm WASM package.
//
//   node bundle.mjs [--pin <oxigraph-npm-version>] [--out <dir>]
//
// Determinism (why these bytes CAN be canonical, unlike timings):
//   - oxigraph: `npm pack oxigraph@<pin>` downloads the registry tarball for
//     the PINNED version — npm registry artifacts are immutable, and npm
//     verifies the tarball against the registry integrity hash — so the byte
//     counts are exactly reproducible on any box. The local tarball sha256 is
//     recorded in the envelope.
//   - sparq: the shipped js/wasm/sparq_wasm_bg.wasm (the `js/ build:wasm`
//     wasm-pack output, full feature set — the `--features` list in
//     js/package.json `build:wasm` is authoritative — @sparq-org/sparq is not yet
//     published to npm, so the pin is the repo
//     commit + the wasm-pack/binaryen versions recorded in the envelope).
//     The pre-bindgen artifact is separately hard-gated by scripts/ci-bench.sh
//     `wasm_bundle_bytes` (untouched by this script).
//
// Like-for-like: both numbers are "the .wasm a browser user downloads" —
// each project's SHIPPED web artifact with its default published feature set,
// NOT a minimal-engine comparison (sparq also has `build:wasm:lean`).
// gzip wire-bytes are computed with Node zlib level 9 — informative
// (zlib-version-dependent), the raw byte counts are the canonical metric.

import os from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import { spawnSync } from "node:child_process";
import { readFile, writeFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { COMPETITOR_PINS } from "./browser/compare-workload.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..");

/** Default pinned oxigraph npm version (single-sourced; also in bench/competitors.json). */
const OXIGRAPH_NPM_PIN = COMPETITOR_PINS.oxigraph;

function arg(name) {
  const i = process.argv.indexOf(name);
  return i === -1 ? undefined : process.argv[i + 1];
}
const PIN = arg("--pin") ?? process.env.OXIGRAPH_NPM_PIN ?? OXIGRAPH_NPM_PIN;
const OUT_DIR = path.resolve(arg("--out") ?? path.join(HERE, "results"));

const sha256 = (buf) => createHash("sha256").update(buf).digest("hex");
const gzBytes = (buf) => gzipSync(buf, { level: 9 }).length;

async function artifactRow(engine, artifact, file, extra = {}) {
  const buf = await readFile(file);
  return { engine, artifact, bytes: buf.length, gzip_bytes_informative: gzBytes(buf), sha256: sha256(buf), ...extra };
}

// ---- sparq: the shipped web bundle at this commit. ----
const SPARQ_WASM = path.join(REPO, "js", "wasm", "sparq_wasm_bg.wasm");
const SPARQ_GLUE = path.join(REPO, "js", "wasm", "sparq_wasm.js");
let sparqRows;
try {
  sparqRows = [
    await artifactRow("sparq", "web wasm (sparq_wasm_bg.wasm)", SPARQ_WASM),
    await artifactRow("sparq", "web js glue (sparq_wasm.js)", SPARQ_GLUE),
  ];
} catch {
  console.error(
    `[bundle] missing shipped bundle js/wasm/sparq_wasm_bg.wasm — build it first:\n` +
      `  cd ${REPO} && npm ci --ignore-scripts && cd js && npm run build:wasm`,
  );
  process.exit(2);
}

// ---- oxigraph: npm pack of the PINNED version into a scratch dir. ----
const tmp = await mkdtemp(path.join(os.tmpdir(), "wasm-compare-bundle-"));
let oxiRows, tarballSha;
try {
  const pack = spawnSync("npm", ["pack", `oxigraph@${PIN}`, "--pack-destination", tmp, "--silent"], {
    encoding: "utf8",
    timeout: 300_000,
  });
  if (pack.status !== 0) {
    console.error(`[bundle] npm pack oxigraph@${PIN} failed (network/registry?):\n${pack.stderr}`);
    process.exit(1);
  }
  const tarball = path.join(tmp, pack.stdout.trim().split("\n").at(-1));
  tarballSha = sha256(await readFile(tarball));
  const untar = spawnSync("tar", ["xzf", tarball, "-C", tmp], { encoding: "utf8" });
  if (untar.status !== 0) {
    console.error(`[bundle] tar extraction failed:\n${untar.stderr}`);
    process.exit(1);
  }
  const pkg = path.join(tmp, "package");
  oxiRows = [
    await artifactRow("oxigraph", "web wasm (web_bg.wasm)", path.join(pkg, "web_bg.wasm")),
    await artifactRow("oxigraph", "web js glue (web.js)", path.join(pkg, "web.js")),
    await artifactRow("oxigraph", "node wasm (node_bg.wasm)", path.join(pkg, "node_bg.wasm")),
  ];
} finally {
  await rm(tmp, { recursive: true, force: true });
}

// ---- envelope + report. ----
const git = spawnSync("git", ["-C", REPO, "rev-parse", "HEAD"], { encoding: "utf8" });
const wasmPack = spawnSync("wasm-pack", ["--version"], { encoding: "utf8" });
const envelope = {
  suite: "wasm-compare/bundle",
  bead: "sq-hmd7l.17",
  deterministic: true,
  note:
    "Raw `bytes` are DETERMINISTIC (pinned immutable npm artifact for oxigraph; repo commit + " +
    "recorded toolchain for sparq's shipped bundle) — the one wasm-compare metric that can be " +
    "canonical without a quiet box. `gzip_bytes_informative` is Node-zlib level 9 (informative only). " +
    "Feature-set caveat: each engine ships its own default feature set; this compares SHIPPED " +
    "artifacts, not equalized feature matrices.",
  run_utc: new Date().toISOString(),
  git_commit: git.status === 0 ? git.stdout.trim() : null,
  sparq_source: `local shipped bundle (js/ build:wasm; @sparq-org/sparq not yet on npm); wasm-pack ${
    wasmPack.status === 0 ? wasmPack.stdout.trim() : "unknown"
  }`,
  oxigraph_source: `npm:oxigraph@${PIN} (tarball sha256 ${tarballSha})`,
  rows: [...sparqRows, ...oxiRows],
};

await mkdir(OUT_DIR, { recursive: true });
const outFile = path.join(OUT_DIR, `bundle-${new Date().toISOString().replace(/[:.]/g, "-")}.json`);
await writeFile(outFile, JSON.stringify(envelope, null, 2));

console.log(`== deterministic wasm bundle bytes (sq-hmd7l.17) ==`);
for (const r of envelope.rows) {
  console.log(
    `bundle_bytes ${r.engine.padEnd(9)} ${String(r.bytes).padStart(9)} bytes  ` +
      `(gzip-9 ${String(r.gzip_bytes_informative).padStart(9)})  ${r.artifact}`,
  );
}
const sparqWeb = sparqRows[0].bytes;
const oxiWeb = oxiRows[0].bytes;
console.log(
  `\nweb .wasm ratio: sparq/oxigraph = ${(sparqWeb / oxiWeb).toFixed(3)} ` +
    `(sparq ${sparqWeb} vs oxigraph@${PIN} ${oxiWeb})`,
);
console.log(`sparq: ${envelope.sparq_source}`);
console.log(`oxigraph: ${envelope.oxigraph_source}`);
console.log(`\nwrote ${path.relative(process.cwd(), outFile)}`);
