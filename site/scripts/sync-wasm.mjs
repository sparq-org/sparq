// [OPUS-4.8] sq-8thu — copy the lean sparq wasm bundle into public/wasm/ for the
// live REPL. Source of truth is the wasm-pack `--target web` output that `js/`'s
// `build:wasm` produces (crates/sparq-wasm → js/wasm). Run after that build.
//
// Kept out of the bundler: the REPL loads these as plain static assets from
// /public at runtime (see src/lib/sparq-wasm.ts), so they must live in public/.
//
// [SONNET-4.6] sq-b66fc — content-hashing added: every .js/.wasm runtime file gets
// a hashed copy written alongside the unhashed original, and a wasm-manifest.json is
// emitted to public/wasm/ so the @sparq/client resolveWasmAsset() loader can request
// content-addressed URLs (defeating GitHub Pages' ~10 min max-age after redeployment).
import { mkdir, copyFile, access, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  hashFile,
  hashedName,
} from "../../packages/sparq-client/scripts/hash-asset.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, "..", "..", "js", "wasm");
const dest = join(here, "..", "public", "wasm");
// Runtime files (.js + .wasm) are content-hashed; .d.ts files are dev-only declarations
// that nothing fetches at runtime — do NOT hash them.
const runtimeFiles = ["sparq_wasm.js", "sparq_wasm_bg.wasm"];
const devFiles = ["sparq_wasm.d.ts"];
const files = [...runtimeFiles, ...devFiles];

try {
  await access(join(src, "sparq_wasm_bg.wasm"));
} catch {
  console.error(
    `[sync-wasm] ${src}/sparq_wasm_bg.wasm not found.\n` +
      `Build it first:  (cd ../js && npm ci && npm run build:wasm)`,
  );
  process.exit(1);
}

await mkdir(dest, { recursive: true });
for (const f of files) {
  await copyFile(join(src, f), join(dest, f));
}
console.log(`[sync-wasm] copied ${files.length} files → public/wasm/`);

// [SONNET-4.6] sq-b66fc — accumulate manifest entries: logicalName → hashed filename.
// The logical name is the subdirectory-relative path (e.g. "sparq_wasm.js" for the
// core bundle, "reason/sparq_reason_wasm.js" for the W-reason tier-b bundle).
/** @type {Record<string, string>} */
const manifest = {};

// Hash core runtime files and write hashed copies alongside the originals.
for (const f of runtimeFiles) {
  const destPath = join(dest, f);
  const hash = await hashFile(destPath);
  const hashed = hashedName(f, hash);
  await copyFile(destPath, join(dest, hashed));
  manifest[f] = hashed;
}

// [OPUS-4.8] sq-0po6 — also sync the tier-b "W-reason" bundle that drives
// /surface/inference (crates/sparq-reason-wasm, built with `--features explain` by
// `js/`'s `build:reason-wasm`). Its wasm-pack output stays in the crate's own pkg/
// (it is NOT part of the published @sparq-org/sparq package), so we copy it from there
// into public/wasm/reason/. This bundle is OPTIONAL: if it has not been built (e.g. a
// quick `next dev` that skips it), we WARN and skip rather than fail — the inference
// page surfaces a clear "bundle failed to load" state at runtime in that case.
const reasonSrc = join(here, "..", "..", "crates", "sparq-reason-wasm", "pkg");
const reasonDest = join(dest, "reason");
const reasonRuntime = ["sparq_reason_wasm.js", "sparq_reason_wasm_bg.wasm"];
const reasonFiles = [
  "sparq_reason_wasm.js",
  "sparq_reason_wasm_bg.wasm",
  "sparq_reason_wasm.d.ts",
];

try {
  await access(join(reasonSrc, "sparq_reason_wasm_bg.wasm"));
  await mkdir(reasonDest, { recursive: true });
  for (const f of reasonFiles) {
    await copyFile(join(reasonSrc, f), join(reasonDest, f));
  }
  console.log(
    `[sync-wasm] copied ${reasonFiles.length} files → public/wasm/reason/ (W-reason)`,
  );
  // [SONNET-4.6] sq-b66fc — hash W-reason runtime files.
  for (const f of reasonRuntime) {
    const destPath = join(reasonDest, f);
    const hash = await hashFile(destPath);
    const hashed = hashedName(f, hash);
    await copyFile(destPath, join(reasonDest, hashed));
    manifest[`reason/${f}`] = hashed;
  }
} catch {
  console.warn(
    `[sync-wasm] W-reason bundle not found at ${reasonSrc}; /surface/inference will not\n` +
      `            run live. Build it:  (cd ../js && npm run build:reason-wasm)`,
  );
}

// [OPUS-4.8] sq-11zy — also sync the tier-b "W-rsp" bundle that drives
// /surface/streaming-rsp (crates/sparq-rsp-wasm, the windowed RSP-QL stream processor,
// built by `js/`'s `build:rsp-wasm`). Like W-reason, its wasm-pack output stays in the
// crate's own pkg/ (it is NOT part of the published @sparq-org/sparq package), so we copy it
// into public/wasm/rsp/. OPTIONAL: a quick `next dev` that skips the build WARNs and
// skips rather than failing — the streaming page then surfaces a clear load-failure state.
const rspSrc = join(here, "..", "..", "crates", "sparq-rsp-wasm", "pkg");
const rspDest = join(dest, "rsp");
const rspRuntime = ["sparq_rsp_wasm.js", "sparq_rsp_wasm_bg.wasm"];
const rspFiles = [
  "sparq_rsp_wasm.js",
  "sparq_rsp_wasm_bg.wasm",
  "sparq_rsp_wasm.d.ts",
];

try {
  await access(join(rspSrc, "sparq_rsp_wasm_bg.wasm"));
  await mkdir(rspDest, { recursive: true });
  for (const f of rspFiles) {
    await copyFile(join(rspSrc, f), join(rspDest, f));
  }
  console.log(
    `[sync-wasm] copied ${rspFiles.length} files → public/wasm/rsp/ (W-rsp)`,
  );
  // [SONNET-4.6] sq-b66fc — hash W-rsp runtime files.
  for (const f of rspRuntime) {
    const destPath = join(rspDest, f);
    const hash = await hashFile(destPath);
    const hashed = hashedName(f, hash);
    await copyFile(destPath, join(rspDest, hashed));
    manifest[`rsp/${f}`] = hashed;
  }
} catch {
  console.warn(
    `[sync-wasm] W-rsp bundle not found at ${rspSrc}; /surface/streaming-rsp will not\n` +
      `            run live. Build it:  (cd ../js && npm run build:rsp-wasm)`,
  );
}

// [OPUS-4.8] sq-xoxu — also sync the tier-b "W-text" bundle that drives
// /surface/full-text (crates/sparq-text-wasm, the BM25 inverted index + `text:` magic
// predicates, built by `js/`'s `build:text-wasm`). Like W-reason / W-rsp, its wasm-pack
// output stays in the crate's own pkg/ (it is NOT part of the published @sparq-org/sparq
// package), so we copy it into public/wasm/text/. OPTIONAL: a quick `next dev` that skips
// the build WARNs and skips rather than failing — the full-text page then surfaces a clear
// load-failure state.
const textSrc = join(here, "..", "..", "crates", "sparq-text-wasm", "pkg");
const textDest = join(dest, "text");
const textRuntime = ["sparq_text_wasm.js", "sparq_text_wasm_bg.wasm"];
const textFiles = [
  "sparq_text_wasm.js",
  "sparq_text_wasm_bg.wasm",
  "sparq_text_wasm.d.ts",
];

try {
  await access(join(textSrc, "sparq_text_wasm_bg.wasm"));
  await mkdir(textDest, { recursive: true });
  for (const f of textFiles) {
    await copyFile(join(textSrc, f), join(textDest, f));
  }
  console.log(
    `[sync-wasm] copied ${textFiles.length} files → public/wasm/text/ (W-text)`,
  );
  // [SONNET-4.6] sq-b66fc — hash W-text runtime files.
  for (const f of textRuntime) {
    const destPath = join(textDest, f);
    const hash = await hashFile(destPath);
    const hashed = hashedName(f, hash);
    await copyFile(destPath, join(textDest, hashed));
    manifest[`text/${f}`] = hashed;
  }
} catch {
  console.warn(
    `[sync-wasm] W-text bundle not found at ${textSrc}; /surface/full-text will not\n` +
      `            run live. Build it:  (cd ../js && npm run build:text-wasm)`,
  );
}

// [SONNET-4.6] sq-b66fc — write the manifest. The manifest maps each logical runtime
// filename (relative to public/wasm/, e.g. "sparq_wasm.js") to its content-hashed copy
// (just the filename, not the full path). The @sparq/client resolveWasmAsset() loader
// reads this once and caches it to resolve content-addressed URLs, falling back to the
// unhashed name when the manifest is absent (dev mode / Tauri / local disk).
const manifestPath = join(dest, "wasm-manifest.json");
await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");
console.log(
  `[sync-wasm] wrote wasm-manifest.json with ${Object.keys(manifest).length} entries → public/wasm/`,
);
