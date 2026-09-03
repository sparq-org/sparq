import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, test } from "node:test";

import {
  CanonicalTimingError,
  synchronize,
} from "../scripts/sync-canonical-timing.mjs";

const roots = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function write(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, value, "utf8");
}

function writeJson(path, value) {
  write(path, `${JSON.stringify(value, null, 2)}\n`);
}

function fixture() {
  const root = mkdtempSync(join(tmpdir(), "canonical-timing-test-"));
  roots.push(root);
  const protocolRel = "bench/ac/scaling/ANALYSIS-PROTOCOL.md";
  const scriptRel = "bench/ac/scaling/analyze.py";
  const publisherRel = "bench/ac/scaling/publish_paper_summary.py";
  const figureRel = "site/papers/figures/canonical-timing/ac/pod-scaling.svg";
  const envelopeRel = "bench/canonical-competitor-results/ac-sparql/run-1/paper-summary.json";
  const rawArchiveRel = "bench/canonical-competitor-results/ac-sparql/run-1/raw-sanitized.tar.zst";
  const sourcesPath = join(root, "site/src/data/canonical-timing-sources.json");
  const outputPath = join(root, "site/src/data/paper-evidence.canonical-timing.generated.json");
  write(join(root, protocolRel), "# frozen protocol\n");
  write(join(root, scriptRel), "# deterministic analyzer\n");
  write(join(root, publisherRel), "# deterministic publisher\n");
  write(join(root, figureRel), '<svg xmlns="http://www.w3.org/2000/svg"/>\n');
  write(join(root, rawArchiveRel), "fixture sanitized raw archive\n");
  const figureBytes = readFileSync(join(root, figureRel)).length;
  const rawArchiveBytes = readFileSync(join(root, rawArchiveRel)).length;
  const envelope = {
    schema_version: 1,
    canonical: true,
    study_id: "fixture-study",
    run_id: "run-1",
    collected_at_utc: "2026-09-03T12:00:00Z",
    source: { git_commit: "a".repeat(40), tree_clean: true },
    environment: {
      class: "controlled-single-process-ec2",
      host_label: "fixture EC2 host",
      tenancy: "shared",
      noise_limitation: "Shared-cloud tenancy cannot exclude noisy-neighbour interference.",
      os: "linux",
      cpu_affinity: "3",
      rayon_threads: 1,
    },
    workload: {
      label: "fixture workload",
      dataset: "fixture synthetic dataset",
      query_scope: "fixture point query",
      protocol: {
        path: protocolRel,
        version: "1.0",
        sha256: sha256(join(root, protocolRel)),
      },
    },
    analysis: {
      git_commit: "d".repeat(40),
      tree_clean: true,
      canonical_validation_passed: true,
      script: { path: scriptRel, sha256: sha256(join(root, scriptRel)) },
      publisher: { path: publisherRel, sha256: sha256(join(root, publisherRel)) },
      bootstrap_draws: 10000,
      bootstrap_seed: 7,
      input_files: [{
        path: "archived/raw.jsonl",
        sha256: "b".repeat(64),
        archive_member: "inputs/raw.jsonl",
      }],
      output_files: [
        { name: "h2.json", sha256: "e".repeat(64), bytes: 4321 },
        { name: "pod-scaling.svg", sha256: sha256(join(root, figureRel)), bytes: figureBytes },
      ],
      result_bindings: {
        "/results/h2/routed/meets_h2_criterion": [{
          artifact: "h2.json",
          sha256: "e".repeat(64),
          locator: "/results/h2/routed/meets_h2_criterion",
        }],
        "/results/h2/routed/wall_ratio": [{
          artifact: "h2.json",
          sha256: "e".repeat(64),
          locator: "/results/h2/routed/wall_ratio",
        }],
      },
      raw_archive: {
        path: rawArchiveRel,
        sha256: sha256(join(root, rawArchiveRel)),
        bytes: rawArchiveBytes,
      },
      raw_archive_verification: {
        manifest_member: "manifest.json",
        manifest: { sha256: "c".repeat(64), bytes: 512, entry_count: 1 },
        regular_members: 2,
        all_members_rehashed: true,
        exact_member_set: true,
        sanitization_scan_passed: true,
        deterministic_tar_headers: true,
        zstd: { version: "1.5.7", executable_sha256: "f".repeat(64) },
      },
    },
    correctness: { passed: true, records: 1 },
    results: { h2: { routed: { wall_ratio: 1.0125, meets_h2_criterion: true } } },
    figures: {
      scaling: {
        path: figureRel,
        sha256: sha256(join(root, figureRel)),
        media_type: "image/svg+xml",
        output_name: "pod-scaling.svg",
      },
    },
    paper_evidence: {
      "fixture.wall_ratio": {
        pointer: "/results/h2/routed/wall_ratio",
        unit: "ratio",
        estimator: "fixture ratio",
        note: "Fixture-only timing record.",
        papers: ["fixture-paper"],
      },
      "fixture.h2_verdict": {
        pointer: "/results/h2/routed/meets_h2_criterion",
        unit: "boolean",
        estimator: "fixture decision rule",
        hypothesis: "H2",
        note: "Fixture-only derived verdict.",
        papers: ["fixture-paper"],
      },
    },
    paper_figures: {
      "fixture.figure.scaling": {
        figure: "scaling",
        alt: "A fixture scaling plot used to test digest-bound publication.",
        note: "Fixture-only digest-bound timing figure.",
        papers: ["fixture-paper"],
      },
    },
  };
  writeJson(join(root, envelopeRel), envelope);
  writeJson(sourcesPath, { schemaVersion: 1, envelopes: [envelopeRel] });
  return { root, envelope, envelopeRel, sourcesPath, outputPath, figureRel, rawArchiveRel };
}

test("the JSON schema pins publisher, archived-input, and member-verification fields", () => {
  const schema = JSON.parse(
    readFileSync(new URL("../../bench/canonical-timing-envelope.schema.json", import.meta.url), "utf8"),
  );
  assert.ok(schema.properties.analysis.required.includes("publisher"));
  assert.ok(schema.properties.analysis.required.includes("result_bindings"));
  assert.ok(schema.properties.analysis.required.includes("raw_archive_verification"));
  assert.deepEqual(
    schema.$defs.archivedInputFile.required,
    ["path", "sha256", "archive_member"],
  );
  assert.deepEqual(
    schema.$defs.rawArchiveVerification.properties.all_members_rehashed,
    { const: true },
  );
  assert.deepEqual(
    schema.$defs.rawArchiveVerification.properties.exact_member_set,
    { const: true },
  );
});

test("the Typst timing API fails closed on missing or cross-paper evidence", () => {
  const helper = readFileSync(new URL("../papers/_lib/timing.typ", import.meta.url), "utf8");
  assert.match(helper, /sys\.inputs\.at\("paper", default: none\)/);
  assert.match(helper, /paper == none or not \(paper in papers\)/);
  assert.match(helper, /is not declared for paper/);
  assert.match(helper, /#let timing_figure\(/);
  assert.match(helper, /image\(path, width: width, alt: alt\)/);
});

test("the AC template freezes the guarded-stack/content-reference names and comparison counts", () => {
  const template = JSON.parse(
    readFileSync(
      new URL("../../bench/ac/scaling/canonical/paper-summary.template.json", import.meta.url),
      "utf8",
    ),
  );
  assert.ok(Object.hasOwn(template.results, "guarded_stack_content_reference"));
  assert.ok(Object.hasOwn(template.figures, "guarded_stack_content_reference"));
  assert.equal(
    template.figures.guarded_stack_content_reference.output_name,
    "guarded-stack-content-reference.svg",
  );
  assert.equal(template.correctness.oracle_comparisons, 5760);
  assert.equal(template.correctness.materialized_oracle_comparisons, 4608);
  assert.ok(
    Object.keys(template.paper_evidence).some((key) =>
      key.startsWith("ac_sparql.guarded_stack_content_reference."),
    ),
  );
  assert.ok(!JSON.stringify(template).includes("guarded-interface premium"));
});

test("write derives the value and complete provenance; check is byte-exact", () => {
  const fx = fixture();
  const result = synchronize({ ...fx, mode: "write" });
  assert.equal(result.records, 3);
  const generated = JSON.parse(readFileSync(fx.outputPath, "utf8"));
  const record = generated.records["fixture.wall_ratio"];
  assert.equal(record.value, 1.0125);
  assert.equal(record.environment, "canonical-timing");
  assert.equal(record.binding.file, fx.envelopeRel);
  assert.equal(record.binding.pointer, "/results/h2/routed/wall_ratio");
  assert.deepEqual(record.result_bindings, [{
    artifact: "h2.json",
    sha256: "e".repeat(64),
    locator: "/results/h2/routed/wall_ratio",
  }]);
  assert.equal(record.timing_provenance.source_git_commit, "a".repeat(40));
  assert.equal(record.timing_provenance.analysis_git_commit, "d".repeat(40));
  assert.equal(record.timing_provenance.publisher_path, "bench/ac/scaling/publish_paper_summary.py");
  assert.equal(record.timing_provenance.publisher_sha256, sha256(join(fx.root, "bench/ac/scaling/publish_paper_summary.py")));
  assert.equal(record.timing_provenance.raw_archive_kind, "committed");
  assert.equal(record.timing_provenance.raw_archive_build_verification, "local-rehash");
  assert.equal(record.timing_provenance.raw_archive_member_verification_authority, "publisher-recorded");
  assert.equal(record.timing_provenance.raw_archive_all_members_rehashed, true);
  assert.equal(record.timing_provenance.raw_archive_manifest_entry_count, 1);
  assert.deepEqual(record.papers, ["fixture-paper"]);
  assert.equal(generated.records["fixture.h2_verdict"].value, true);
  assert.equal(generated.records["fixture.h2_verdict"].kind, "canonical-timing-verdict");
  const figure = generated.records["fixture.figure.scaling"];
  assert.equal(figure.value, fx.envelope.figures.scaling.sha256);
  assert.equal(figure.kind, "canonical-timing-figure");
  assert.equal(figure.binding.pointer, "/figures/scaling/sha256");
  assert.equal(figure.figure.typst_path, "/papers/figures/canonical-timing/ac/pod-scaling.svg");
  assert.equal(figure.figure.output_name, "pod-scaling.svg");
  assert.equal(figure.figure.bytes, readFileSync(join(fx.root, fx.figureRel)).length);
  assert.match(figure.figure.alt, /digest-bound publication/);
  assert.deepEqual(synchronize({ ...fx, mode: "check" }), { records: 3, changed: false });
});

test("check fails when the generated file is stale", () => {
  const fx = fixture();
  synchronize({ ...fx, mode: "write" });
  write(fx.outputPath, "{}\n");
  assert.throws(
    () => synchronize({ ...fx, mode: "check" }),
    (error) => error instanceof CanonicalTimingError && /stale/.test(error.message),
  );
});

test("a source envelope that does not self-declare canonical is rejected", () => {
  const fx = fixture();
  fx.envelope.canonical = false;
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /canonical must be true/.test(error.message),
  );
});

test("the envelope run_id must match its immutable parent directory", () => {
  const fx = fixture();
  fx.envelope.run_id = "another-run";
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /run_id.*must equal its envelope directory/.test(error.message),
  );
});

test("a registered canonical envelope must use the paper-summary.json basename", () => {
  const fx = fixture();
  const badRel = "bench/canonical-competitor-results/ac-sparql/run-1/summary.json";
  writeJson(join(fx.root, badRel), fx.envelope);
  writeJson(fx.sourcesPath, { schemaVersion: 1, envelopes: [badRel] });
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /<run-id>\/paper-summary\.json/.test(error.message),
  );
});

test("the non-evidence template warning cannot survive into a canonical envelope", () => {
  const fx = fixture();
  fx.envelope._template_warning = "NOT EVIDENCE";
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /unsupported field.*_template_warning/.test(error.message),
  );
});

test("the post-run analysis commit is independently required", () => {
  const fx = fixture();
  delete fx.envelope.analysis.git_commit;
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /analysis.git_commit/.test(error.message),
  );
});

test("the locally hashed publisher is independently required", () => {
  const fx = fixture();
  fx.envelope.analysis.publisher.sha256 = "0".repeat(64);
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /analysis\.publisher hash drift/.test(error.message),
  );
});

test("shared EC2 tenancy cannot be mislabeled dedicated", () => {
  const fx = fixture();
  fx.envelope.environment.class = "dedicated-quiet-ec2";
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /supported canonical host class/.test(error.message),
  );
});

test("free-form result strings cannot enter canonical timing evidence", () => {
  const fx = fixture();
  fx.envelope.results.h2.routed.prose = "12x faster";
  fx.envelope.paper_evidence["fixture.prose"] = {
    pointer: "/results/h2/routed/prose",
    unit: "text",
    note: "Must be rejected.",
    papers: ["fixture-paper"],
  };
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /finite number or derived boolean/.test(error.message),
  );
});

test("every finite result scalar needs a hashed source-artifact binding", () => {
  const fx = fixture();
  delete fx.envelope.analysis.result_bindings["/results/h2/routed/wall_ratio"];
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /exactly cover every finite numeric\/boolean/.test(error.message),
  );
});

test("result bindings may name archived inputs but must match a declared digest", () => {
  const fx = fixture();
  const pointer = "/results/h2/routed/wall_ratio";
  fx.envelope.analysis.result_bindings[pointer] = [{
    artifact: "archived/raw.jsonl",
    sha256: "b".repeat(64),
    locator: "row=fixture; field=wall_ratio",
  }];
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.doesNotThrow(() => synchronize({ ...fx, mode: "write" }));
  fx.envelope.analysis.result_bindings[pointer][0].sha256 = "0".repeat(64);
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /sha256 disagrees with the descriptor/.test(error.message),
  );
});

test("result-binding pointer keys must be in lexical order", () => {
  const fx = fixture();
  const bindings = fx.envelope.analysis.result_bindings;
  fx.envelope.analysis.result_bindings = {
    "/results/h2/routed/wall_ratio": bindings["/results/h2/routed/wall_ratio"],
    "/results/h2/routed/meets_h2_criterion": bindings["/results/h2/routed/meets_h2_criterion"],
  };
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /serialized in lexical order/.test(error.message),
  );
});

test("canonical envelopes cannot retain unresolved result placeholders", () => {
  const fx = fixture();
  fx.envelope.results.h2.routed.wall_ratio = null;
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /must not contain unresolved null placeholders/.test(error.message),
  );
});

test("derived boolean verdicts must declare boolean units", () => {
  const fx = fixture();
  fx.envelope.paper_evidence["fixture.h2_verdict"].unit = "ratio";
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /unit='boolean'/.test(error.message),
  );
});

test("only H1 and H2 may become mechanical verdicts", () => {
  const fx = fixture();
  fx.envelope.paper_evidence["fixture.h2_verdict"].hypothesis = "H4";
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /hypothesis='H1' or 'H2'/.test(error.message),
  );
});

test("figure drift fails before timing evidence is emitted", () => {
  const fx = fixture();
  write(join(fx.root, fx.figureRel), '<svg xmlns="http://www.w3.org/2000/svg"><text>changed</text></svg>\n');
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /hash drift/.test(error.message),
  );
});

test("canonical artifacts cannot obtain bytes through symbolic links", () => {
  const fx = fixture();
  const targetRel = "site/papers/figures/elsewhere.svg";
  write(join(fx.root, targetRel), '<svg xmlns="http://www.w3.org/2000/svg"/>\n');
  unlinkSync(join(fx.root, fx.figureRel));
  symlinkSync(join(fx.root, targetRel), join(fx.root, fx.figureRel));
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /not a symbolic link/.test(error.message),
  );
});

test("a committed raw archive is re-hashed and byte-counted", () => {
  const fx = fixture();
  write(join(fx.root, fx.rawArchiveRel), "changed sanitized archive\n");
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /raw_archive.*hash drift/.test(error.message),
  );
});

test("the committed raw archive must be beside its immutable run envelope", () => {
  const fx = fixture();
  const elsewhere = "bench/canonical-competitor-results/ac-sparql/another-run/raw-sanitized.tar.zst";
  write(join(fx.root, elsewhere), "another archive\n");
  fx.envelope.analysis.raw_archive = {
    path: elsewhere,
    sha256: sha256(join(fx.root, elsewhere)),
    bytes: readFileSync(join(fx.root, elsewhere)).length,
  };
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /must be beside its run envelope/.test(error.message),
  );
});

test("each input descriptor must name a unique safe archive member", () => {
  const fx = fixture();
  fx.envelope.analysis.input_files.push({
    path: "archived/second.jsonl",
    sha256: "9".repeat(64),
    archive_member: "../raw.jsonl",
  });
  fx.envelope.analysis.raw_archive_verification.manifest.entry_count = 2;
  fx.envelope.analysis.raw_archive_verification.regular_members = 3;
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /safe relative archive member path/.test(error.message),
  );
});

test("publisher-recorded archive verification must be complete and internally consistent", () => {
  const fx = fixture();
  fx.envelope.analysis.raw_archive_verification.all_members_rehashed = false;
  fx.envelope.analysis.raw_archive_verification.regular_members = 7;
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /regular_members must equal/.test(error.message),
  );
  fx.envelope.analysis.raw_archive_verification.regular_members = 2;
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /all_members_rehashed must be true/.test(error.message),
  );
});

test("an external archive remains an explicit descriptor-only fallback", () => {
  const fx = fixture();
  fx.envelope.analysis.raw_archive = {
    url: "https://example.invalid/sparq/fixture-run.tar.zst",
    sha256: "c".repeat(64),
    bytes: 1234,
    build_verification: "descriptor-only",
  };
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  synchronize({ ...fx, mode: "write" });
  const generated = JSON.parse(readFileSync(fx.outputPath, "utf8"));
  const provenance = generated.records["fixture.wall_ratio"].timing_provenance;
  assert.equal(provenance.raw_archive_kind, "external");
  assert.equal(provenance.raw_archive_build_verification, "descriptor-only");
});

test("a multi-file raw bundle must be named .tar.zst", () => {
  const fx = fixture();
  fx.envelope.analysis.raw_archive = {
    url: "https://example.invalid/sparq/fixture-run.jsonl.zst",
    sha256: "c".repeat(64),
    bytes: 1234,
    build_verification: "descriptor-only",
  };
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /must name a \.tar\.zst bundle/.test(error.message),
  );
});

test("a timing SVG must match its exact analyzer output digest", () => {
  const fx = fixture();
  fx.envelope.analysis.output_files.find((output) => output.name === "pod-scaling.svg").sha256 =
    "f".repeat(64);
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /hash disagrees with analysis\.output_files/.test(error.message),
  );
});

test("a paper figure cannot name an undeclared SVG", () => {
  const fx = fixture();
  fx.envelope.paper_figures["fixture.figure.scaling"].figure = "not-there";
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /unknown envelope figure/.test(error.message),
  );
});

test("canonical paper figures must live in the reserved timing-figure tree", () => {
  const fx = fixture();
  const badRel = "site/papers/figures/ac/unreserved.svg";
  write(join(fx.root, badRel), '<svg xmlns="http://www.w3.org/2000/svg"/>\n');
  fx.envelope.figures.scaling.path = badRel;
  fx.envelope.figures.scaling.sha256 = sha256(join(fx.root, badRel));
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /canonical-timing/.test(error.message),
  );
});

test("an envelope outside the canonical-results tree is rejected", () => {
  const fx = fixture();
  const badRel = "bench/ac/scaling/results/run-1/paper-summary.json";
  writeJson(join(fx.root, badRel), fx.envelope);
  writeJson(fx.sourcesPath, { schemaVersion: 1, envelopes: [badRel] });
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /canonical-competitor-results/.test(error.message),
  );
});

test("canonical path prefixes cannot be escaped with parent traversal", () => {
  const fx = fixture();
  const outsideRel = "bench/outside-archive.tar.zst";
  write(join(fx.root, outsideRel), "outside\n");
  fx.envelope.analysis.raw_archive = {
    path: "bench/canonical-competitor-results/../../outside-archive.tar.zst",
    sha256: sha256(join(fx.root, outsideRel)),
    bytes: readFileSync(join(fx.root, outsideRel)).length,
  };
  writeJson(join(fx.root, fx.envelopeRel), fx.envelope);
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /must remain below/.test(error.message),
  );
});

test("duplicate timing keys across envelopes fail closed", () => {
  const fx = fixture();
  const secondRel = "bench/canonical-competitor-results/ac-sparql/run-2/paper-summary.json";
  const secondArchiveRel = "bench/canonical-competitor-results/ac-sparql/run-2/raw-sanitized.tar.zst";
  write(join(fx.root, secondArchiveRel), "fixture sanitized raw archive\n");
  fx.envelope.run_id = "run-2";
  fx.envelope.analysis.raw_archive = {
    path: secondArchiveRel,
    sha256: sha256(join(fx.root, secondArchiveRel)),
    bytes: readFileSync(join(fx.root, secondArchiveRel)).length,
  };
  writeJson(join(fx.root, secondRel), fx.envelope);
  writeJson(fx.sourcesPath, {
    schemaVersion: 1,
    envelopes: [fx.envelopeRel, secondRel],
  });
  assert.throws(
    () => synchronize({ ...fx, mode: "write" }),
    (error) => error instanceof CanonicalTimingError && /duplicate generated/.test(error.message),
  );
});
