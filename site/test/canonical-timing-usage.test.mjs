import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, test } from "node:test";

import {
  typstCommonArgs,
  validateCanonicalTimingUsage,
} from "../scripts/build-papers.mjs";

const roots = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function fixture(source) {
  const papersDir = mkdtempSync(join(tmpdir(), "timing-usage-test-"));
  roots.push(papersDir);
  writeFileSync(join(papersDir, "demo.typ"), source, "utf8");
  const papers = [{ slug: "demo", source: "demo.typ" }];
  const evidence = {
    records: {
      "ac.ratio": {
        value: 1.01,
        environment: "canonical-timing",
        kind: "canonical-timing",
        papers: ["demo"],
      },
      "ac.figure.scaling": {
        value: "b".repeat(64),
        environment: "canonical-timing",
        kind: "canonical-timing-figure",
        papers: ["demo"],
        figure: {
          typst_path: "/papers/figures/canonical-timing/ac/scaling.svg",
          media_type: "image/svg+xml",
          alt: "Scaling plot.",
        },
      },
      "ac.h2_verdict": {
        value: true,
        unit: "boolean",
        environment: "canonical-timing",
        kind: "canonical-timing-verdict",
        hypothesis: "H2",
        papers: ["demo"],
      },
    },
  };
  return { papersDir, papers, evidence };
}

test("literal timing helpers declared for the paper pass", () => {
  const fx = fixture(`
    #headline_timing("ac.ratio", digits: 2)
    #timing_provenance("ac.ratio")
    #timing_verdict("ac.h2_verdict", yes: [meets], no: [does not meet])
    #timing_table(
      "ac.ratio",
      header: ([Metric], [Value]),
      rows: (([ratio], "ac.ratio"),),
      caption: [H2 results.],
    )
    #timing_figure(
      "ac.figure.scaling",
      caption: [Scaling by Pod count.],
    )
  `);
  assert.doesNotThrow(() => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir));
});

test("a digest-bound SVG cannot be embedded by raw path", () => {
  const fx = fixture('#image("/papers/figures/canonical-timing/ac/scaling.svg")\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /direct path into the reserved canonical timing SVG tree/,
  );
});

test("an unregistered SVG in the reserved tree cannot bypass timing_figure", () => {
  const fx = fixture('#image("figures/canonical-timing/ac/not-in-the-ledger.svg")\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /reserved canonical timing SVG tree/,
  );
});

test("dynamic timing keys fail closed", () => {
  const fx = fixture('#let key = "ac.ratio"\n#headline_timing(key)\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /literal first-argument key/,
  );
});

test("paper scoping and helper kind are checked", () => {
  const fx = fixture('#headline_timing("ac.figure.scaling")\n');
  fx.evidence.records["ac.figure.scaling"].papers = ["another-paper"];
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /not declared for 'demo'.*does not name a numeric record/s,
  );
});

test("a valid numeric key declared for another paper is rejected", () => {
  const fx = fixture('#headline_timing("ac.ratio")\n');
  fx.evidence.records["ac.ratio"].papers = ["another-paper"];
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /not declared for 'demo'/,
  );
});

test("every Typst compile receives the merged ledger path and current paper slug", () => {
  const args = typstCommonArgs(
    { slug: "demo" },
    "/src/generated/papers/_paper-evidence.merged.json",
    "/fixture/site",
  );
  assert.deepEqual(args, [
    "--root",
    "/fixture/site",
    "--input",
    "data=/src/generated/papers/_paper-evidence.merged.json",
    "--input",
    "paper=demo",
  ]);
});

test("paper source cannot parse the raw injected evidence", () => {
  const fx = fixture('#let evidence = json(bytes(sys.inputs.data))\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not access the injected evidence ledger directly/,
  );
});

test("paper source cannot parse the injected evidence path directly", () => {
  const fx = fixture('#let evidence = json(sys.inputs.data)\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not access the injected evidence ledger directly/,
  );
});

test("paper source cannot alias the injected ledger path before parsing it", () => {
  const fx = fixture('#let injected = sys.inputs.data\n#let evidence = json(injected)\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not access the injected evidence ledger directly/,
  );
});

test("paper source cannot obtain the injected ledger with dictionary lookup syntax", () => {
  const fx = fixture('#let injected = sys.inputs.at("data")\n#let evidence = json(injected)\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not access the injected evidence ledger directly/,
  );
});

test("paper source cannot explicitly import a raw timing implementation lookup", () => {
  const fx = fixture('#import "_lib/timing.typ": _timing_rec\n#_timing_rec("ac.ratio").value\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not import or call canonical timing implementation internals/,
  );
});

test("paper source cannot explicitly import the legacy raw record lookup", () => {
  const fx = fixture('#import "_lib/bench.typ": _rec\n#_rec("ac.ratio").value\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not import or call canonical timing implementation internals/,
  );
});

test("paper source cannot read either evidence ledger by literal path", () => {
  const fx = fixture('#let evidence = json("/src/data/paper-evidence.canonical-timing.generated.json")\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not read an evidence-ledger path directly/,
  );
});

test("paper source cannot read the generated merged ledger by literal path", () => {
  const fx = fixture('#let evidence = json("/src/generated/papers/_paper-evidence.merged.json")\n');
  assert.throws(
    () => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir),
    /may not read an evidence-ledger path directly/,
  );
});

test("comments do not create helper calls or raw-path violations", () => {
  const fx = fixture(`
    // #headline_timing(dynamic_key)
    /* #image("/papers/figures/canonical-timing/ac/scaling.svg") */
    The fixture contains no timing result.
  `);
  assert.doesNotThrow(() => validateCanonicalTimingUsage(fx.papers, fx.evidence, fx.papersDir));
});
