// [OPUS-4.8] sq-gum8 — the paper-factory build step.
//
// For every paper registered in src/data/papers.ts, this:
//   1. checks/synchronizes the committed canonical-timing ledger, merges it with
//      src/data/paper-evidence.json, and runs the build-time HONESTY GATE, then
//   2. compiles the paper's .typ to BOTH a PDF (public/papers/<slug>.pdf — the download)
//      and a semantic HTML fragment (src/generated/papers/<slug>.html — the in-site render),
//      passing the SAME generated merged-evidence path via `--input data=...` so the two
//      artifacts cannot show different numbers. A path avoids Linux's per-argument size limit
//      when a dense timing study declares hundreds of paper values.
//
// It is wired into `prebuild` (after sync-benchmarks) so `next build` always regenerates the
// papers against fresh data, and into `dev`. The typst binary is resolved from PATH or
// ~/.local/bin or ~/.cargo/bin; CI installs it (see the workflow note at the bottom).
//
// The page route imports the generated HTML fragment at build time, so the in-site render is
// a static asset (no WASM compiler shipped to the browser). See the route + papers.ts.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { homedir } from "node:os";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SITE = resolve(__dirname, "..");
const REPO_ROOT = resolve(SITE, ".."); // [OPUS-4.8] sq-mraf: to invoke the shared gates.
const EVIDENCE_PATH = join(SITE, "src", "data", "paper-evidence.json");
const TIMING_EVIDENCE_PATH = join(
  SITE,
  "src",
  "data",
  "paper-evidence.canonical-timing.generated.json",
);
const TIMING_SYNC = join(SITE, "scripts", "sync-canonical-timing.mjs");
const TIMING_GENERATOR = "site/scripts/sync-canonical-timing.mjs";
const PAPERS_DIR = join(SITE, "papers");
const PDF_OUT_DIR = join(SITE, "public", "papers");
const HTML_OUT_DIR = join(SITE, "src", "generated", "papers");
const MERGED_EVIDENCE_OUT = join(HTML_OUT_DIR, "_paper-evidence.merged.json");
const MERGED_EVIDENCE_TYPST_PATH = "/src/generated/papers/_paper-evidence.merged.json";

// [OPUS-4.8] sq-d8or — anti-drift "GENERATED at build time" header for the build-copied
// HTML fragments under src/generated/papers/. The fragment is the in-site render compiled
// from the tracked papers/<slug>.typ source on every `prebuild`; it is git-ignored
// (site/.gitignore: /src/generated/) and must NEVER be hand-edited. A leading HTML comment
// is invisible when the route injects the fragment via dangerouslySetInnerHTML, so it adds
// no render cost while making the provenance unmistakable to anyone who opens the file.
// `source` is the .typ this fragment was compiled from.
function generatedHeader(source) {
  return (
    `<!-- GENERATED at build time by site/scripts/build-papers.mjs from papers/${source} -->\n` +
    `<!-- DO NOT EDIT: regenerate with \`npm run build-papers\` (or \`npm run build\`). -->\n`
  );
}

// ---- resolve the typst binary -------------------------------------------------------------
function resolveTypst() {
  const candidates = [
    process.env.TYPST_BIN,
    "typst",
    join(homedir(), ".local", "bin", "typst"),
    join(homedir(), ".cargo", "bin", "typst"),
  ].filter(Boolean);
  for (const c of candidates) {
    try {
      execFileSync(c, ["--version"], { stdio: "ignore" });
      return c;
    } catch {
      /* try next */
    }
  }
  return null;
}

// ---- the registry (parsed from papers.ts without importing TS) ----------------------------
// papers.ts is the single source of truth; we read the slug + source list from it via a tiny
// regex so this plain .mjs build step needs no TS toolchain. The shape is asserted below.
function readRegistry() {
  const src = readFileSync(join(SITE, "src", "data", "papers.ts"), "utf8");
  const slugs = [...src.matchAll(/slug:\s*"([^"]+)"/g)].map((m) => m[1]);
  const sources = [...src.matchAll(/source:\s*"([^"]+)"/g)].map((m) => m[1]);
  if (slugs.length === 0 || slugs.length !== sources.length) {
    throw new Error(
      `build-papers: could not parse papers.ts (slugs=${slugs.length}, sources=${sources.length})`,
    );
  }
  return slugs.map((slug, i) => ({ slug, source: sources[i] }));
}

// ---- the build-time honesty gate (schema + canonical/indicative invariant) ----------------
// The .typ-level gate (headline() in papers/_lib/bench.typ) already panics the compile if a
// headline cites a non-canonical record. This is the data-layer guard that runs FIRST so the
// failure is a clear, early, build-level message rather than a Typst stack trace, and so the
// evidence file itself is validated even before any paper references a given key.
function readMergedEvidence() {
  const base = JSON.parse(readFileSync(EVIDENCE_PATH, "utf8"));
  const timing = JSON.parse(readFileSync(TIMING_EVIDENCE_PATH, "utf8"));
  const baseRecords = base.records || {};
  const timingRecords = timing.records || {};
  const duplicates = Object.keys(timingRecords).filter((key) => Object.hasOwn(baseRecords, key));
  if (duplicates.length) {
    throw new Error(`paper evidence key collision: ${duplicates.sort().join(", ")}`);
  }
  return {
    ...base,
    records: { ...baseRecords, ...timingRecords },
  };
}

function runCanonicalTimingSyncCheck() {
  if (!existsSync(TIMING_SYNC)) {
    throw new Error(`canonical timing sync missing: ${TIMING_SYNC}`);
  }
  // Publication is read-only: deriving values is an explicit maintainer action. A build only
  // proves that the tracked generated ledger is byte-identical to every registered envelope.
  execFileSync(process.execPath, [TIMING_SYNC, "--check"], {
    cwd: REPO_ROOT,
    stdio: ["ignore", "inherit", "inherit"],
  });
}

function runHonestyGate(raw, papers) {
  const records = raw.records || {};
  const VALID_ENV = new Set(["canonical", "canonical-timing", "indicative"]);
  const TIMING_KINDS = new Set([
    "canonical-timing",
    "canonical-timing-verdict",
    "canonical-timing-figure",
  ]);
  const paperSlugs = new Set(papers.map((paper) => paper.slug));
  const problems = [];
  for (const [key, r] of Object.entries(records)) {
    if (!VALID_ENV.has(r.environment)) {
      problems.push(
        `record '${key}' has environment='${r.environment}' — must be ` +
          "'canonical', 'canonical-timing', or 'indicative'",
      );
    }
    if (!r.source || typeof r.source !== "string") {
      problems.push(`record '${key}' is missing a 'source' (every number must trace to a real test/dataset)`);
    }
    if (r.value === undefined) {
      problems.push(`record '${key}' is missing a 'value'`);
    }
    if (r.environment === "canonical-timing") {
      if (!TIMING_KINDS.has(r.kind)) {
        problems.push(`record '${key}' has invalid canonical timing kind '${r.kind}'`);
      }
      if (r.binding?.kind !== "json-pointer") {
        problems.push(`record '${key}' canonical timing must use a json-pointer binding`);
      }
      if (!r.timing_provenance || typeof r.timing_provenance !== "object") {
        problems.push(`record '${key}' canonical timing lacks timing_provenance`);
      }
      if (r._generated_by !== TIMING_GENERATOR) {
        problems.push(`record '${key}' canonical timing was not generated by the sync`);
      }
      if (
        !Array.isArray(r.papers) ||
        r.papers.length === 0 ||
        r.papers.some((slug) => typeof slug !== "string" || !paperSlugs.has(slug))
      ) {
        problems.push(`record '${key}' canonical timing has an empty or unknown paper slug`);
      }
      if (r.kind === "canonical-timing-figure") {
        if (!/^[0-9a-f]{64}$/.test(r.value) || r.unit !== "sha256") {
          problems.push(`record '${key}' timing figure value must be a SHA-256 digest`);
        }
        if (
          r.figure?.media_type !== "image/svg+xml" ||
          !r.figure?.typst_path?.startsWith("/papers/figures/canonical-timing/")
        ) {
          problems.push(`record '${key}' timing figure must use the reserved SVG tree`);
        }
      } else if (r.kind === "canonical-timing-verdict") {
        if (
          typeof r.value !== "boolean" ||
          r.unit !== "boolean" ||
          !["H1", "H2"].includes(r.hypothesis)
        ) {
          problems.push(
            `record '${key}' timing verdict must be boolean, unit='boolean', and scoped to H1/H2`,
          );
        }
      } else if (typeof r.value !== "number" || !Number.isFinite(r.value)) {
        problems.push(`record '${key}' canonical timing value must be finite numeric`);
      }
    }
  }
  if (problems.length) {
    console.error("\n[paper-factory] HONESTY GATE FAILED:\n  - " + problems.join("\n  - ") + "\n");
    process.exit(1);
  }
  const nCanonical = Object.values(records).filter((r) => r.environment === "canonical").length;
  const nTiming = Object.values(records).filter((r) => r.environment === "canonical-timing").length;
  const nIndicative = Object.values(records).filter((r) => r.environment === "indicative").length;
  console.log(
    `[paper-factory] honesty gate passed: ${Object.keys(records).length} evidence records ` +
      `(${nCanonical} deterministic canonical, ${nTiming} canonical timing, ` +
      `${nIndicative} indicative).`,
  );
}

// ---- the build-BOUNDARY honesty assertion (bead sq-mraf, Option C) -------------------------
// [OPUS-4.8] The data-layer runHonestyGate() above validates the evidence-record *envelope*
// (environment/source/value) but never reads PROSE — the .typ paper text or the human `note`
// fields. To guarantee the factory can NEVER serve an un-scanned paper, we RE-RUN the two real
// CI honesty gates here, at the build boundary, over the exact paper sources we are about to
// compile + the evidence file:
//   - scripts/check-no-perf-numbers.py --enforce <each .typ> <paper-evidence.json>
//       (the Python gate dispatches a `.typ` path → the accessor-aware Typst scan, and a
//        paper-evidence.json path → the prose-field scan — beads sq-mkza + sq-4hga).
//   - scripts/check-privacy-claims.sh  (whole-tree; its git-ls-files surface already includes
//       site/papers/**/*.typ + paper-evidence.json — beads sq-mkza + sq-4hga).
// Both gates consume the SINGLE shared forbidden-phrase list (scripts/honesty-phrases.json),
// so there is ONE source of truth and zero drift between the CI gate and this build boundary.
// FAIL-CLOSED: a non-zero gate exit aborts the build (the artifacts are never written). This
// is deliberately strict — a published paper is the highest-stakes outward surface, so an
// un-scanned or violating paper must not compile. python3 + bash are stdlib-level CI deps
// (the gates already run in docs-quality.yml); a genuinely missing interpreter is itself a
// build failure, not a silent skip.
//
// HONEST SCOPE: re-running the gates here catches the same COARSE class (forbidden phrase /
// hard-coded result number); it does NOT catch a subtle semantic overclaim — that remains the
// Stage-5 claims↔evidence human review in skills/academic-paper/SKILL.md.
function runBuildBoundaryHonestyScan(papers) {
  const perfGate = join(REPO_ROOT, "scripts", "check-no-perf-numbers.py");
  const privacyGate = join(REPO_ROOT, "scripts", "check-privacy-claims.sh");
  const sharedList = join(REPO_ROOT, "scripts", "honesty-phrases.json");
  for (const p of [perfGate, privacyGate, sharedList]) {
    if (!existsSync(p)) {
      console.error(`\n[paper-factory] BUILD-BOUNDARY HONESTY SCAN: required gate missing: ${p}\n`);
      process.exit(1);
    }
  }
  const typPaths = papers.map((p) => join(PAPERS_DIR, p.source)).filter((t) => existsSync(t));

  const run = (label, cmd, cmdArgs) => {
    try {
      // inherit stderr so a finding is visible; capture nothing — the gate self-reports.
      execFileSync(cmd, cmdArgs, { cwd: REPO_ROOT, stdio: ["ignore", "inherit", "inherit"] });
    } catch (e) {
      const code = typeof e.status === "number" ? e.status : 1;
      console.error(
        `\n[paper-factory] BUILD-BOUNDARY HONESTY SCAN FAILED (${label}, exit ${code}).\n` +
          "  A paper .typ source or an evidence note carries a forbidden ZK/MPC claim or a\n" +
          "  hard-coded result number. Hedge the wording / route the number through the\n" +
          "  paper-evidence.json accessor, or add the inline allow marker. The factory will\n" +
          "  not serve an un-scanned paper. (Shared list: scripts/honesty-phrases.json.)\n",
      );
      process.exit(1);
    }
  };

  // Perf gate over the exact paper sources + the evidence file (explicit paths => --enforce).
  run("no-perf-numbers", "python3", [
    perfGate,
    "--enforce",
    ...typPaths,
    EVIDENCE_PATH,
    TIMING_EVIDENCE_PATH,
  ]);
  // Privacy gate (whole-tree; already covers the paper surface + the evidence file).
  run("privacy-claims", "bash", [privacyGate]);

  console.log(
    `[paper-factory] build-boundary honesty scan passed: ${typPaths.length} paper source(s) ` +
      "+ paper-evidence.json scanned by both honesty gates (shared phrase list).",
  );
}

// ---- the fail-closed EVIDENCE-BINDING verifier (bead sq-gum8.13, paper factory F1) --------
// [OPUS-4.8] runHonestyGate() above validates the record ENVELOPE (environment/source/value
// present) but never checks that a record's `value` still MATCHES its committed source — a
// ratchet/floor rise, or a rename, silently republishes a STALE number. This step runs
// scripts/verify-paper-evidence.py at the build boundary: every environment=canonical record
// must carry a machine-verified `binding` (json-pointer / rust-anchor / doc-anchor) whose
// source resolves AND whose value matches, OR sit on the shrink-only allowlist
// (scripts/paper-evidence-binding-allowlist.json). FAIL-CLOSED: a drifted value, a
// missing/renamed source, or a grown allowlist ABORTS the build (no artifact written).
//
// HONEST SCOPE (do not oversell): the verifier is MECHANICAL — value<->source EQUALITY /
// EXISTENCE only. A semantic overclaim (a true number framed misleadingly) is NOT caught here;
// that stays the Stage-5 claims<->evidence human review (skills/academic-paper/SKILL.md,
// sq-dxi3). It does not touch the ZK/MPC posture (external audit pending, sq-qhy4).
function runEvidenceBindingVerifier() {
  const verifier = join(REPO_ROOT, "scripts", "verify-paper-evidence.py");
  if (!existsSync(verifier)) {
    console.error(
      `\n[paper-factory] EVIDENCE-BINDING VERIFY: required verifier missing: ${verifier}\n`,
    );
    process.exit(1);
  }
  try {
    // Runs against the committed evidence file + allowlist, resolving sources under the repo
    // root (the verifier's defaults). The gate self-reports any drift/missing-anchor to stderr.
    execFileSync("python3", [verifier, "--supplement", TIMING_EVIDENCE_PATH], {
      cwd: REPO_ROOT,
      stdio: ["ignore", "inherit", "inherit"],
    });
  } catch (e) {
    const code = typeof e.status === "number" ? e.status : 1;
    console.error(
      `\n[paper-factory] EVIDENCE-BINDING VERIFY FAILED (exit ${code}).\n` +
        "  A canonical paper-evidence value no longer matches its committed source, a bound\n" +
        "  source was renamed/removed, or the shrink-only allowlist grew. Fix the number at its\n" +
        "  source-of-truth (do not hand-edit the paper value), re-point the binding, or shrink\n" +
        "  the allowlist. The factory will not republish a stale number. (Mechanical value<->\n" +
        "  source match only; semantic framing stays the Stage-5 human review.)\n",
    );
    process.exit(1);
  }
  console.log(
    "[paper-factory] evidence-binding verify passed: every canonical record is machine-bound or allowlisted.",
  );
}

// ---- canonical-timing accessor discipline ------------------------------------------------
// The Typst helpers enforce provenance at render time. This source-level check closes the two
// accidental bypasses that matter in review: dynamically choosing a timing key, and embedding a
// digest-bound SVG by path instead of through timing_figure(). This is not a hostile-code sandbox;
// it is a fail-closed paper-factory lint backed by the runtime checks in timing.typ.
export function stripTypstComments(source) {
  let out = "";
  let block = false;
  let line = false;
  let string = false;
  let escaped = false;
  for (let i = 0; i < source.length; i += 1) {
    const char = source[i];
    const next = source[i + 1];
    if (line) {
      if (char === "\n") {
        line = false;
        out += char;
      }
      continue;
    }
    if (block) {
      if (char === "*" && next === "/") {
        block = false;
        i += 1;
      } else if (char === "\n") {
        out += char;
      }
      continue;
    }
    if (string) {
      out += char;
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') string = false;
      continue;
    }
    if (char === '"') {
      string = true;
      out += char;
    } else if (char === "/" && next === "/") {
      line = true;
      i += 1;
    } else if (char === "/" && next === "*") {
      block = true;
      i += 1;
    } else {
      out += char;
    }
  }
  return out;
}

export function validateCanonicalTimingUsage(papers, evidence, papersDir = PAPERS_DIR) {
  const timingRecords = Object.fromEntries(
    Object.entries(evidence.records || {}).filter(([, record]) =>
      record.environment === "canonical-timing",
    ),
  );
  const anyCall = /\b(headline_timing|timing_verdict|timing_table|timing_provenance|timing_figure)\s*\(/g;
  const literalCall = /\b(headline_timing|timing_verdict|timing_table|timing_provenance|timing_figure)\s*\(\s*"((?:\\.|[^"\\])*)"/g;
  const problems = [];

  for (const paper of papers) {
    const path = join(papersDir, paper.source);
    if (!existsSync(path)) continue;
    const source = stripTypstComments(readFileSync(path, "utf8"));
    const calls = [...source.matchAll(anyCall)];
    const literalCalls = [...source.matchAll(literalCall)];
    if (calls.length !== literalCalls.length) {
      problems.push(
        `${paper.source}: every canonical timing helper requires a literal first-argument key`,
      );
    }
    for (const match of literalCalls) {
      const [, helper, key] = match;
      const record = timingRecords[key];
      if (!record) {
        problems.push(`${paper.source}: ${helper} names unknown canonical timing key '${key}'`);
        continue;
      }
      if (!Array.isArray(record.papers) || !record.papers.includes(paper.slug)) {
        problems.push(`${paper.source}: timing key '${key}' is not declared for '${paper.slug}'`);
      }
      if (helper === "headline_timing" && record.kind !== "canonical-timing") {
        problems.push(`${paper.source}: headline_timing('${key}') does not name a numeric record`);
      }
      if (helper === "timing_verdict" && record.kind !== "canonical-timing-verdict") {
        problems.push(`${paper.source}: timing_verdict('${key}') does not name a boolean verdict`);
      }
      if (helper === "timing_table" && record.kind === "canonical-timing-figure") {
        problems.push(`${paper.source}: timing_table('${key}') cannot use a figure as its anchor`);
      }
      if (helper === "timing_figure" && record.kind !== "canonical-timing-figure") {
        problems.push(`${paper.source}: timing_figure('${key}') does not name a figure record`);
      }
    }
    // Typst modules do not make underscore-prefixed definitions private: an author can import
    // them explicitly. Keep the paper surface on the supported API by refusing both direct access
    // to the injected path and references to timing/ledger implementation names. The helper
    // modules themselves are outside `papers`, so their intentional uses are unaffected.
    if (/\bsys\.inputs(?:\.data|\.at\(\s*"data")/.test(source)) {
      problems.push(`${paper.source}: paper source may not access the injected evidence ledger directly`);
    }
    if (
      /\b(?:_evidence|_rec|_timing_rec|_provenance_text|_render_number|_timing_fingerprint|_table_spec_record|_render_table_spec)\b/.test(
        source,
      )
    ) {
      problems.push(
        `${paper.source}: paper source may not import or call canonical timing implementation internals`,
      );
    }
    if (
      /(?:json|read|bytes)\s*\(\s*"[^"]*(?:_?paper-evidence)[^"]*\.json"/.test(
        source,
      )
    ) {
      problems.push(`${paper.source}: paper source may not read an evidence-ledger path directly`);
    }
    if (source.includes("figures/canonical-timing/")) {
      problems.push(
        `${paper.source}: direct path into the reserved canonical timing SVG tree is forbidden; ` +
          "use timing_figure(key) so digest verification and provenance are inseparable",
      );
    }
  }
  if (problems.length) {
    throw new Error(`canonical timing accessor gate failed:\n  - ${problems.join("\n  - ")}`);
  }
}

// ---- compile one paper to PDF + HTML ------------------------------------------------------
export function typstCommonArgs(paper, evidencePath, siteRoot = SITE) {
  return [
    "--root",
    siteRoot,
    "--input",
    `data=${evidencePath}`,
    "--input",
    `paper=${paper.slug}`,
  ];
}

function compilePaper(typst, paper, evidencePath) {
  const typPath = join(PAPERS_DIR, paper.source);
  if (!existsSync(typPath)) {
    throw new Error(`build-papers: paper source not found: ${typPath}`);
  }
  const pdfOut = join(PDF_OUT_DIR, `${paper.slug}.pdf`);
  const htmlOut = join(HTML_OUT_DIR, `${paper.slug}.html`);
  const common = typstCommonArgs(paper, evidencePath);

  // PDF (the download). A headline() gate violation panics here and aborts the build.
  execFileSync(typst, ["compile", typPath, pdfOut, ...common], { stdio: ["ignore", "ignore", "inherit"] });

  // HTML (the in-site render) — Typst native HTML export. `--features html` is required; the
  // experimental-feature + page-rule warnings on stderr are expected and harmless.
  execFileSync(
    typst,
    ["compile", typPath, htmlOut, "--format", "html", "--features", "html", ...common],
    { stdio: ["ignore", "ignore", "inherit"] },
  );

  // Extract just the <body> inner HTML so the React route can inject it as a fragment.
  const full = readFileSync(htmlOut, "utf8");
  const body = full.match(/<body[^>]*>([\s\S]*)<\/body>/i);
  const fragment = body ? body[1] : full;
  // [OPUS-4.8] sq-d8or — stamp the build-time provenance header (invisible HTML comment).
  writeFileSync(htmlOut, generatedHeader(paper.source) + fragment, "utf8");

  console.log(`[paper-factory] built ${paper.slug}: ${pdfOut} + ${htmlOut}`);
}

// ---- main ---------------------------------------------------------------------------------
function main() {
  runCanonicalTimingSyncCheck();
  const evidence = readMergedEvidence();
  const papers = readRegistry();
  runHonestyGate(evidence, papers);

  // [OPUS-4.8] sq-gum8.13 (paper factory F1): fail-closed evidence-BINDING verify — every
  // canonical record's value must still MATCH its committed source (or sit on the shrink-only
  // allowlist). Runs right after the envelope gate + before any compile/placeholder is written,
  // so a stale/renamed number aborts the build. Independent of typst (a pure source check).
  runEvidenceBindingVerifier();

  const typst = resolveTypst();
  validateCanonicalTimingUsage(papers, evidence);

  // [OPUS-4.8] sq-mraf: build-boundary honesty assertion — re-run the two shared honesty
  // gates over the exact paper sources + evidence file BEFORE any compile/placeholder is
  // written, so the factory can never serve an un-scanned or violating paper. Runs even when
  // typst is absent (the scan is independent of compilation). FAIL-CLOSED.
  runBuildBoundaryHonestyScan(papers);

  mkdirSync(PDF_OUT_DIR, { recursive: true });
  mkdirSync(HTML_OUT_DIR, { recursive: true });

  if (!typst) {
    // Graceful degradation: a contributor without Typst installed can still run the site.
    // CI MUST have Typst (the workflow installs it) so the real PDFs/HTML are produced; here
    // we emit honest placeholders so `next build` does not hard-fail on a missing binary, and
    // we surface a loud warning.
    console.warn(
      "\n[paper-factory] WARNING: `typst` not found — emitting placeholder paper artifacts.\n" +
        "  Install Typst 0.15+ (https://github.com/typst/typst/releases) so real PDFs/HTML build.\n" +
        "  CI installs it; this fallback is for local dev without Typst only.\n",
    );
    for (const p of papers) {
      const placeholder = `<p class="paper-placeholder">This paper renders from <code>papers/${p.source}</code>. ` +
        `Install the Typst CLI to build it locally; CI builds it automatically.</p>`;
      // [OPUS-4.8] sq-d8or — same build-time provenance header on the placeholder fragment.
      writeFileSync(join(HTML_OUT_DIR, `${p.slug}.html`), generatedHeader(p.source) + placeholder, "utf8");
      // a 1-line text PDF stand-in is not produced; the route guards a missing PDF asset.
    }
    return;
  }

  const evidenceJson = `${JSON.stringify(evidence, null, 2)}\n`;
  writeFileSync(MERGED_EVIDENCE_OUT, evidenceJson, "utf8");
  for (const p of papers) compilePaper(typst, p, MERGED_EVIDENCE_TYPST_PATH);

  console.log(`[paper-factory] done: ${papers.length} paper(s).`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main();
}
