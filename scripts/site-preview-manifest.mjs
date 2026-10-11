#!/usr/bin/env node
// Writes the `_preview.json` manifest that ships inside a PR site-preview artifact
// (.github/workflows/site-preview.yml). The deploy side
// (scripts/site-preview-overlay.sh + site-preview-deploy.yml) reads it to link the PR's
// changed specs and papers straight from the preview comment.
//
// Usage: site-preview-manifest.mjs <out-file> <pr-number> <head-sha> < changed-paths.txt
//   changed-paths.txt: one repo-relative path per line (the PR's changed files).
import { readFileSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = resolve(dirname(fileURLToPath(import.meta.url)), "..", "site");
const [outFile, pr, sha] = process.argv.slice(2);
if (!outFile || !/^\d+$/.test(pr ?? "") || !/^[0-9a-f]{40}$/.test(sha ?? "")) {
  console.error("usage: site-preview-manifest.mjs <out-file> <pr-number> <head-sha> < changed-paths.txt");
  process.exit(2);
}

// The registries declare `slug:` before `source:` on single-line string fields (the same
// shape site/scripts/build-specs.mjs parses), so pair them in declaration order.
function registry(file) {
  const src = readFileSync(join(SITE, "src", "data", file), "utf8");
  const field = (name) => [...src.matchAll(new RegExp(`\\b${name}:\\s*"([^"]*)"`, "g"))].map((m) => m[1]);
  const slugs = field("slug");
  const sources = field("source");
  if (slugs.length !== sources.length) throw new Error(`${file}: slug/source count mismatch`);
  return new Map(sources.map((s, i) => [s, slugs[i]]));
}

const changed = readFileSync(0, "utf8").split("\n").map((l) => l.trim()).filter(Boolean);
function touched(dir, file) {
  const bySource = registry(file);
  const slugs = new Set();
  for (const p of changed) {
    if (dirname(p) !== `site/${dir}` || !p.endsWith(".typ")) continue;
    const slug = bySource.get(basename(p));
    if (slug) slugs.add(slug);
  }
  return [...slugs].sort();
}

const manifest = {
  pr: Number(pr),
  sha,
  specs: touched("specs", "specs.ts"),
  papers: touched("papers", "papers.ts"),
};
writeFileSync(outFile, JSON.stringify(manifest, null, 2) + "\n");
console.log(JSON.stringify(manifest));
