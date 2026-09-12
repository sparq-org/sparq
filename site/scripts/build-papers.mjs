// [OPUS-4.8] sq-gum8 — the paper-factory build step.
//
// For every paper registered in src/data/papers.ts, this:
//   1. runs the build-time HONESTY GATE on src/data/paper-evidence.json (schema + the
//      canonical/indicative invariant), then
//   2. compiles the paper's .typ to BOTH a PDF (public/papers/<slug>.pdf — the download)
//      and a semantic HTML fragment (src/generated/papers/<slug>.html — the in-site render),
//      injecting the SAME evidence JSON via `--input data=...` so the two artifacts cannot
//      show different numbers.
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
import { homedir, tmpdir } from "node:os";

// [OPUS-5] sq-gum8.16 — the derived `canonical-timing` evidence class (paper factory F4).
import {
  deriveCanonicalTiming,
  serializeCanonicalTiming,
  TIMING_OUT_PATH,
} from "./sync-canonical-timing.mjs";
import { auditTimingSources } from "./timing-source-gate.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const SITE = resolve(__dirname, "..");
const REPO_ROOT = resolve(SITE, ".."); // [OPUS-4.8] sq-mraf: to invoke the shared gates.
const EVIDENCE_PATH = join(SITE, "src", "data", "paper-evidence.json");
const PAPERS_DIR = join(SITE, "papers");
const PDF_OUT_DIR = join(SITE, "public", "papers");
const HTML_OUT_DIR = join(SITE, "src", "generated", "papers");

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
function runHonestyGate() {
  const raw = JSON.parse(readFileSync(EVIDENCE_PATH, "utf8"));
  const records = raw.records || {};
  const VALID_ENV = new Set(["canonical", "indicative"]);
  const problems = [];
  for (const [key, r] of Object.entries(records)) {
    if (!VALID_ENV.has(r.environment)) {
      problems.push(
        `record '${key}' has environment='${r.environment}' — must be 'canonical' or 'indicative'`,
      );
    }
    if (!r.source || typeof r.source !== "string") {
      problems.push(`record '${key}' is missing a 'source' (every number must trace to a real test/dataset)`);
    }
    if (r.value === undefined) {
      problems.push(`record '${key}' is missing a 'value'`);
    }
  }
  if (problems.length) {
    console.error("\n[paper-factory] HONESTY GATE FAILED:\n  - " + problems.join("\n  - ") + "\n");
    process.exit(1);
  }
  const nCanonical = Object.values(records).filter((r) => r.environment === "canonical").length;
  const nIndicative = Object.values(records).length - nCanonical;
  console.log(
    `[paper-factory] honesty gate passed: ${Object.keys(records).length} evidence records ` +
      `(${nCanonical} canonical, ${nIndicative} indicative).`,
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
  run("no-perf-numbers", "python3", [perfGate, "--enforce", ...typPaths, EVIDENCE_PATH]);
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
    execFileSync("python3", [verifier], { cwd: REPO_ROOT, stdio: ["ignore", "inherit", "inherit"] });
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

// ---- the derived canonical-timing gate (bead sq-gum8.16, paper factory F4) -----------------
// [OPUS-5] `paper-evidence.json` above is HAND-MAINTAINED deterministic evidence. Measured
// wall-clock numbers live in a separate, fully DERIVED file — `src/data/paper-timing.generated
// .json`, produced by sync-canonical-timing.mjs from the committed canonical envelopes under
// `bench/canonical-competitor-results/**` and read by `papers/_lib/timing.typ`.
//
// This step re-runs that derivation and byte-compares it against the committed file, so the
// factory can never publish a HAND-EDITED or STALE timing: the file is either exactly what the
// committed envelopes imply, or the build aborts. It also asserts the class invariant (every
// record is `environment: "canonical-timing"`) directly, so the property the papers rely on is
// checked here and not only inside the generator.
//
// `prebuild` regenerates the file (the live-update path: a new canonical envelope lands on
// main → the papers pick it up on the next deploy); this is the verification half.
//
// HONEST SCOPE: mechanical value↔envelope provenance only. It does not judge whether a
// comparison is fair or whether a paper frames it honestly — that is the Stage-5
// claims↔evidence review (skills/academic-paper/SKILL.md).
function runCanonicalTimingGate() {
  let derived;
  try {
    derived = deriveCanonicalTiming();
  } catch (e) {
    console.error(`\n[paper-factory] CANONICAL-TIMING DERIVATION FAILED: ${e.message}\n`);
    process.exit(1);
  }
  const expected = serializeCanonicalTiming(derived);
  const current = existsSync(TIMING_OUT_PATH) ? readFileSync(TIMING_OUT_PATH, "utf8") : null;
  if (current !== expected) {
    // Distinguish the one non-obvious cause: a checkout without the envelope tree derives
    // NOTHING, which looks identical to "stale" unless we say so. We still fail — a derivation
    // that cannot see its sources cannot verify anything, and trusting the committed file in
    // that state would be the un-fail-closed direction.
    const envelopeDir = join(REPO_ROOT, "bench", "canonical-competitor-results");
    const hint = existsSync(envelopeDir)
      ? "  Run `npm run sync-canonical-timing` and commit the result. A measured number a\n" +
        "  paper headlines must be exactly what the committed canonical envelopes say.\n"
      : `  CAUSE: ${envelopeDir} is absent in this checkout, so the derivation produced\n` +
        "  nothing to compare against. This build needs the full tree (no sparse checkout).\n";
    console.error(
      "\n[paper-factory] CANONICAL-TIMING GATE FAILED: src/data/paper-timing.generated.json " +
        "does not match a fresh derivation.\n" +
        hint,
    );
    process.exit(1);
  }
  const bad = Object.entries(derived.records).filter(
    ([, r]) => r.environment !== "canonical-timing",
  );
  if (bad.length) {
    console.error(
      "\n[paper-factory] CANONICAL-TIMING GATE FAILED: non-'canonical-timing' record(s) in the " +
        `derived timing file: ${bad.map(([k]) => k).join(", ")}\n`,
    );
    process.exit(1);
  }
  const n = Object.keys(derived.records).length;
  const nEnv = Object.keys(derived.envelopes).length;
  console.log(
    `[paper-factory] canonical-timing gate passed: ${n} derived record(s) from ${nEnv} ` +
      `canonical envelope(s); ${derived.skipped.length} envelope(s) skipped (see \`skipped\`).`,
  );
}

// ---- the measured-timing PUBLICATION-BOUNDARY source gate ---------------------------------
// [OPUS-5] sq-gum8.16 — runCanonicalTimingGate() above proves the DATA is exactly what the
// committed envelopes imply. This proves the RENDERING PATH: that no paper source reaches a
// measured value except through the two provenance-rendering accessors.
//
// It is a separate gate because Typst cannot enforce it — the lib's parsed dataset and its
// internal record lookup are importable like any other top-level binding, and a paper can also
// `json()` the generated file itself. See site/scripts/timing-source-gate.mjs for the rules.
// FAIL-CLOSED: any bypass aborts the build before an artifact is written.
function runTimingSourceGate() {
  const { scanned, problems } = auditTimingSources(PAPERS_DIR);
  if (problems.length) {
    console.error(
      "\n[paper-factory] MEASURED-TIMING SOURCE GATE FAILED:\n  - " +
        problems.join("\n  - ") +
        "\n\n  A measured wall-clock number may only reach a page via headline_timing(key) or\n" +
        "  timing_provenance(key), which always render the machine, aggregate and commit\n" +
        "  beside it. Route the number through an accessor instead of the raw data.\n",
    );
    process.exit(1);
  }
  console.log(
    `[paper-factory] measured-timing source gate passed: ${scanned.length} paper source(s) ` +
      "reach timings only through the provenance-rendering accessors.",
  );
}

// ---- the timing-lib compile self-check ----------------------------------------------------
// [OPUS-5] sq-gum8.16 — `papers/_lib/timing.typ` is infrastructure that no paper imports yet
// (the first consumer is a separate bead). Without this, a syntax error in the lib or a shape
// change in the derived JSON would only surface later, inside whichever paper adopts it first.
// So on every build we compile a tiny fixture that imports the lib and exercises BOTH
// accessors, in BOTH export modes, to a temp dir (never into public/). Cheap, and it makes the
// accessor's contract a build-enforced property rather than an untested promise.
function runTimingLibSelfCheck(typst) {
  const fixture = join(PAPERS_DIR, "_lib", "timing-selfcheck.typ");
  if (!existsSync(fixture)) {
    console.error(`\n[paper-factory] TIMING SELF-CHECK: fixture missing: ${fixture}\n`);
    process.exit(1);
  }
  const scratch = join(tmpdir(), "sparq-paper-timing-selfcheck");
  mkdirSync(scratch, { recursive: true });
  const common = ["--root", SITE, "--input", `data=${readFileSync(EVIDENCE_PATH, "utf8")}`];
  for (const [label, args] of [
    ["pdf", [join(scratch, "selfcheck.pdf")]],
    ["html", [join(scratch, "selfcheck.html"), "--format", "html", "--features", "html"]],
  ]) {
    try {
      execFileSync(typst, ["compile", fixture, ...args, ...common], {
        stdio: ["ignore", "ignore", "inherit"],
      });
    } catch (e) {
      const code = typeof e.status === "number" ? e.status : 1;
      console.error(
        `\n[paper-factory] TIMING SELF-CHECK FAILED (${label} export, exit ${code}).\n` +
          "  papers/_lib/timing.typ does not compile against the derived\n" +
          "  src/data/paper-timing.generated.json. Fix the lib (or the derivation shape)\n" +
          "  before any paper can headline a measured timing.\n",
      );
      process.exit(1);
    }
  }
  console.log("[paper-factory] timing-lib self-check passed: timing.typ compiles to PDF + HTML.");
}

// ---- compile one paper to PDF + HTML ------------------------------------------------------
function compilePaper(typst, paper, evidenceJson) {
  const typPath = join(PAPERS_DIR, paper.source);
  if (!existsSync(typPath)) {
    throw new Error(`build-papers: paper source not found: ${typPath}`);
  }
  const pdfOut = join(PDF_OUT_DIR, `${paper.slug}.pdf`);
  const htmlOut = join(HTML_OUT_DIR, `${paper.slug}.html`);
  const common = ["--root", SITE, "--input", `data=${evidenceJson}`];

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
  runHonestyGate();

  // [OPUS-4.8] sq-gum8.13 (paper factory F1): fail-closed evidence-BINDING verify — every
  // canonical record's value must still MATCH its committed source (or sit on the shrink-only
  // allowlist). Runs right after the envelope gate + before any compile/placeholder is written,
  // so a stale/renamed number aborts the build. Independent of typst (a pure source check).
  runEvidenceBindingVerifier();

  // [OPUS-5] sq-gum8.16 (paper factory F4): the derived MEASURED-timing class. Also a pure
  // source check (re-derive + byte-compare), so it runs before any compile/placeholder too.
  runCanonicalTimingGate();

  // [OPUS-5] sq-gum8.16: ...and the matching PUBLICATION-BOUNDARY check — no paper source may
  // reach a measured value except through the provenance-rendering accessors.
  runTimingSourceGate();

  const typst = resolveTypst();
  const papers = readRegistry();

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

  // [OPUS-5] sq-gum8.16: prove the measured-timing accessor compiles before the papers do.
  runTimingLibSelfCheck(typst);

  const evidenceJson = readFileSync(EVIDENCE_PATH, "utf8");
  for (const p of papers) compilePaper(typst, p, evidenceJson);

  console.log(`[paper-factory] done: ${papers.length} paper(s).`);
}

main();
