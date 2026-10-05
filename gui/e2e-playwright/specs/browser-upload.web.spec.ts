// (sq-eydh9) [SONNET-4.6] — browser upload + drag-and-drop RDF import: multi-file File tab on
// web, multi-select native dialog, global dropzone.
//
// Runs under the chromium-web project (support/web-fixtures.ts): no window.__TAURI__ /
// __TAURI_INTERNALS__, so isTauriRuntime() === false and the page takes the pure-browser code
// paths. The webPersona auto-fixture blocks external network, navigates to "/", and waits for
// "Engine ready" before yielding to each test body.
//
// The three acceptance tests from sq-eydh9:
//   (a) Multi-file upload via the file input: two .ttl fixtures → both are imported, store size
//       grows by the sum of their triples (non-vacuous count assertion — must be > 0).
//   (b) Drag-and-drop: the global drop zone's data-global-drop-input exercises the same import
//       code path the window-level drop uses; files set via setInputFiles() are imported.
//   (c) Malformed file in a batch: a syntactically invalid .ttl alongside a valid one — the
//       bad file surfaces a per-file error (`data-file-error` attribute) WITHOUT aborting the
//       valid file's import (the valid file's row shows "quads imported").
//
// (sq-ljc12) [OPUS-5] Two compressed-upload acceptance tests. Both exercise the REAL browser
// codec path — `readFilesWithDecompress` → `maybeDecompressFile` → `@sparq/client`'s
// `decompressDatasetBytes` — in the served static export, with no server and no Tauri global:
//   (d) A `.zst` upload imports. This is the only test that proves the LAZY `fzstd` chunk
//       (`import(/* webpackChunkName: "codec-zstd" */ "fzstd")`, kept out of the first-load
//       bundle per #1046) actually resolves and runs in a real browser — the unit tests in
//       packages/sparq-client run under Node.
//   (e) A `.zip` upload takes its RDF format from the archive's INNER MEMBER NAME. This is the
//       regression test for the fix: `rdf-format.ts#stripCompression` only unwraps
//       `.gz`/`.bz2`/`.zst[d]`, so the previous code's `guessFormat(<outer name>)` resolved
//       `bundle.zip` to the Turtle fallback. The member here is N-Quads with a graph term,
//       which is a SYNTAX ERROR in Turtle — so on the pre-fix code this test fails with a
//       per-file parse error instead of importing into the named graph.
//
// Stable selectors (declared E2E hooks — role/data-* only):
//   [data-import-trigger="rail"]  — left rail's "+ Import data…"
//   [data-import-drawer]          — Import drawer dialog content
//   [data-import-tab="file"]      — File tab button
//   [data-web-file-input]         — stable hidden <input type="file"> in WebFilePane
//   [data-global-drop-input]      — stable hidden <input type="file"> in GlobalDropOverlay
//   [data-import-feedback="ok"]   — aggregate success banner
//   [data-import-feedback="error"]— aggregate error/partial banner
//   [data-file-error="<name>"]    — per-file error span
//   [data-file-row="<name>"]      — per-file list row
//   button "Run query"            — SPARQL run trigger
//   [data-result-kind="select"]   — SELECT result container
//   #repl-query                   — the SPARQL editor textarea
//
// Determinism rules: NO waitForTimeout; NO exact numeric assertions except storeSize > 0;
//   web-first assertions only; store-size checked via a SPARQL COUNT query.

import { webTest as test, webExpect as expect } from "../support/index.ts";

// ── Inline Turtle fixtures (no filesystem access; content is deterministic) ─────────────────

// fixture-a.ttl — 2 triples (Alice and Bob with names)
const FIXTURE_A: string = `
@prefix ex: <http://example.org/upload-a#> .
ex:Alice ex:name "Alice Upload A" .
ex:Bob   ex:name "Bob Upload A" .
`.trim();

// fixture-b.ttl — 3 triples (Carol, Dave, Eve with names)
const FIXTURE_B: string = `
@prefix ex: <http://example.org/upload-b#> .
ex:Carol ex:name "Carol Upload B" .
ex:Dave  ex:name "Dave Upload B" .
ex:Eve   ex:name "Eve Upload B" .
`.trim();

// fixture-bad.ttl — syntactically invalid Turtle (unpaired < for the predicate)
const FIXTURE_BAD: string = `
@prefix ex: <http://example.org/bad#> .
ex:Subject <UNCLOSED_IRI_NO_CLOSE ex:Object .
`.trim();

// ── (sq-ljc12) Compressed fixtures ───────────────────────────────────────────────────────────

// A reference zstd frame, byte-for-byte the `ZSTD_SAMPLE` fixture from
// packages/sparq-client/test/decompress.test.mjs — that suite (gated in gui.yml's "shared TS
// client typecheck" job) asserts it decodes to exactly the two N-Triples below. Reusing the
// same bytes keeps one reference frame in the repo: there is no zstd COMPRESSOR in the toolchain
// (Node 20 has no `zlib.zstdCompressSync`), so this frame cannot be regenerated inline.
const ZSTD_FIXTURE_BASE64: string =
  "KLUv/SSHDQIAJAM8aHR0cDovL2V4YW1wbGUub3JnL3M+IHA+ICJvYmplY3QgdmFsdWUiIC4KcW8yPiAuCgQAOooIuAaWolmlZxOc62uy";

// The two triples ZSTD_FIXTURE_BASE64 decodes to. Their subject <http://example.org/s> does not
// occur in the seeded sample graph (data/sample-graph.ts uses ex:alice … ex:dan), so the import
// is guaranteed to add quads rather than re-assert existing ones.
const ZSTD_FIXTURE_SUBJECT = "http://example.org/s";
const ZSTD_FIXTURE_OBJECT = "object value";

// The zip member: N-Quads. The trailing graph term makes this a SYNTAX ERROR in Turtle, so the
// import can only succeed when the format is derived from the member name `dataset.nq`.
const ZIP_MEMBER_NAME = "dataset.nq";
const ZIP_MEMBER_NQUADS: string =
  '<http://example.org/zip#Frank> <http://example.org/zip#name> "Frank Zip" <http://example.org/zip/graph> .\n';
const ZIP_GRAPH = "http://example.org/zip/graph";

// ── Fixture builders ─────────────────────────────────────────────────────────────────────────

/** A minimal STORED (method 0) single-member zip — no compressor dependency, real CRC-32. */
function makeStoredZip(memberName: string, text: string): Buffer {
  const name = new TextEncoder().encode(memberName);
  const data = new TextEncoder().encode(text);

  let crc = ~0;
  for (const byte of data) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  const checksum = ~crc >>> 0;

  // Local file header (PKZIP APPNOTE.TXT §4.3.7) + the stored bytes.
  const local = new Uint8Array(30 + name.length);
  const localView = new DataView(local.buffer);
  localView.setUint32(0, 0x04034b50, true); // signature
  localView.setUint16(4, 10, true); // version needed (1.0 = STORED)
  localView.setUint16(8, 0, true); // method 0 = STORED
  localView.setUint32(14, checksum, true);
  localView.setUint32(18, data.length, true); // compressed size
  localView.setUint32(22, data.length, true); // uncompressed size
  localView.setUint16(26, name.length, true);
  local.set(name, 30);

  // Central directory file header (§4.3.12).
  const central = new Uint8Array(46 + name.length);
  const centralView = new DataView(central.buffer);
  centralView.setUint32(0, 0x02014b50, true); // signature
  centralView.setUint16(4, 20, true); // version made by
  centralView.setUint16(6, 10, true); // version needed
  centralView.setUint16(10, 0, true); // method 0 = STORED
  centralView.setUint32(16, checksum, true);
  centralView.setUint32(20, data.length, true);
  centralView.setUint32(24, data.length, true);
  centralView.setUint16(28, name.length, true);
  centralView.setUint32(42, 0, true); // local header offset
  central.set(name, 46);

  // End of central directory (§4.3.16). No ZIP64 locator — the reader rejects those.
  const centralOffset = local.length + data.length;
  const eocd = new Uint8Array(22);
  const eocdView = new DataView(eocd.buffer);
  eocdView.setUint32(0, 0x06054b50, true); // signature
  eocdView.setUint16(8, 1, true); // entries on this disk
  eocdView.setUint16(10, 1, true); // total entries
  eocdView.setUint32(12, central.length, true);
  eocdView.setUint32(16, centralOffset, true);

  const zip = new Uint8Array(centralOffset + central.length + eocd.length);
  zip.set(local, 0);
  zip.set(data, local.length);
  zip.set(central, centralOffset);
  zip.set(eocd, centralOffset + central.length);
  return Buffer.from(zip);
}

// ── Helper: read the live store size from the top bar ────────────────────────────────────────
//
// The TopBar renders "{storeSize.toLocaleString()} quads" in a <span>. Reading this is more
// reliable than running a COUNT(*) SPARQL query (which returns cells as `"21"^^xsd:integer`
// via formatTerm, requiring extra parsing). The locale is "en-US" per playwright.config.ts, so
// the number may include thousands commas ("1,234 quads").

async function countQuads(page: import("@playwright/test").Page): Promise<number> {
  // The top bar "N quads" span is always visible after engine ready.
  const quadsSpan = page.locator("span", { hasText: /\d[\d,]* quads/ }).first();
  await expect(quadsSpan).toBeVisible();
  const text = await quadsSpan.textContent();
  // Strip thousands commas, then extract the leading number.
  const digits = text?.replace(/,/g, "").match(/^\d+/);
  return digits ? parseInt(digits[0], 10) : 0;
}

// ── Helper: open the Import drawer to the File tab ──────────────────────────────────────────

async function openFileTab(page: import("@playwright/test").Page) {
  await page.locator('[data-import-trigger="rail"]').click();
  const drawer = page.locator("[data-import-drawer]");
  await expect(drawer).toBeVisible();
  await drawer.locator('[data-import-tab="file"]').click();
  await expect(drawer.locator('[data-import-tab="file"]')).toHaveAttribute(
    "aria-selected",
    "true",
  );
  return drawer;
}

// ── Tests ─────────────────────────────────────────────────────────────────────────────────────

test.describe("browser-upload (sq-eydh9)", () => {
  // The webPersona fixture navigates to "/" and waits for "Engine ready" before each test.

  // ── (a) Multi-file upload via the file input ───────────────────────────────────────────────

  test("(a) multi-file upload via file input imports ALL selected files' triples", async ({
    page,
  }) => {
    // Measure baseline store size (sample graph is already loaded).
    const baseline = await countQuads(page);
    expect(baseline).toBeGreaterThan(0); // sample graph sanity-check

    // Open the drawer, switch to the File tab.
    const drawer = await openFileTab(page);

    // Set two .ttl fixtures on the stable hidden input (data-web-file-input).
    const fileInput = drawer.locator("[data-web-file-input]");
    await expect(fileInput).toBeAttached();
    await fileInput.setInputFiles([
      { name: "fixture-a.ttl", mimeType: "text/turtle", buffer: Buffer.from(FIXTURE_A) },
      { name: "fixture-b.ttl", mimeType: "text/turtle", buffer: Buffer.from(FIXTURE_B) },
    ]);

    // Both files appear in the per-file list before import.
    await expect(drawer.locator('[data-file-row="fixture-a.ttl"]')).toBeVisible();
    await expect(drawer.locator('[data-file-row="fixture-b.ttl"]')).toBeVisible();

    // The Import button is now enabled.
    const importBtn = drawer.getByRole("button", { name: /^Import/i });
    await expect(importBtn).toBeEnabled();

    // Click Import and wait for success feedback.
    await importBtn.click();
    await expect(drawer.locator('[data-import-feedback="ok"]')).toBeVisible();

    // Both rows show "quads imported" (non-vacuous: their added counts > 0).
    await expect(
      drawer.locator('[data-file-row="fixture-a.ttl"]').getByText(/quads? imported/),
    ).toBeVisible();
    await expect(
      drawer.locator('[data-file-row="fixture-b.ttl"]').getByText(/quads? imported/),
    ).toBeVisible();

    // Close the drawer and verify the store grew by at least the two files' triples (5 total).
    await page.getByRole("button", { name: "Close import drawer" }).click();
    await expect(drawer).not.toBeVisible();

    const after = await countQuads(page);
    // fixture-a = 2 triples, fixture-b = 3 triples → store must grow by at least 5.
    expect(after).toBeGreaterThan(baseline);
    expect(after - baseline).toBeGreaterThanOrEqual(5);

    // Specific subjects from BOTH fixtures must be queriable — non-vacuous presence check.
    const editor = page.locator("#repl-query");
    await editor.fill(
      `SELECT * WHERE { <http://example.org/upload-a#Alice> ?p ?o } LIMIT 1`,
    );
    await page.getByRole("button", { name: "Run query" }).click();
    const aliceResult = page.locator('[data-result-kind="select"]');
    await expect(aliceResult).toBeVisible();
    await expect(aliceResult.getByText("Alice Upload A")).toBeVisible();

    await editor.fill(
      `SELECT * WHERE { <http://example.org/upload-b#Carol> ?p ?o } LIMIT 1`,
    );
    await page.getByRole("button", { name: "Run query" }).click();
    await expect(page.locator('[data-result-kind="select"]')).toBeVisible();
    await expect(page.locator('[data-result-kind="select"]').getByText("Carol Upload B")).toBeVisible();
  });

  // ── (b) Drag-and-drop via the global drop input ────────────────────────────────────────────

  test("(b) drag-and-drop of files onto the window imports them", async ({ page }) => {
    // The global drop zone's data-global-drop-input exercises the same import code path that
    // window-level drag-and-drop uses. setInputFiles on this input simulates a file drop per
    // Playwright's recommended approach for drag-and-drop file upload testing.
    const baseline = await countQuads(page);

    const globalInput = page.locator("[data-global-drop-input]");
    await expect(globalInput).toBeAttached();

    // "Drop" fixture-a onto the workbench via the global drop input.
    await globalInput.setInputFiles([
      { name: "drop-fixture-a.ttl", mimeType: "text/turtle", buffer: Buffer.from(FIXTURE_A) },
    ]);

    // The import drawer should open automatically (GlobalDropOverlay calls setOpen(true)).
    const drawer = page.locator("[data-import-drawer]");
    await expect(drawer).toBeVisible();

    // The store must have grown by at least the 2 triples from fixture-a (non-vacuous).
    // Wait for the import to complete by polling the store via the Query tool.
    await page.getByRole("button", { name: "Close import drawer" }).click();
    await expect(drawer).not.toBeVisible();

    // Verify fixture-a's triples are queryable.
    const editor = page.locator("#repl-query");
    await editor.fill(
      `SELECT * WHERE { <http://example.org/upload-a#Bob> ?p ?o } LIMIT 1`,
    );
    await page.getByRole("button", { name: "Run query" }).click();
    const result = page.locator('[data-result-kind="select"]');
    await expect(result).toBeVisible();
    await expect(result.getByText("Bob Upload A")).toBeVisible();

    const after = await countQuads(page);
    expect(after).toBeGreaterThan(baseline);
  });

  // ── (c) Malformed file surfaces a per-file error without aborting the batch ───────────────

  test("(c) a malformed file surfaces a per-file error without aborting valid files", async ({
    page,
  }) => {
    const baseline = await countQuads(page);

    // Open the drawer, switch to the File tab.
    const drawer = await openFileTab(page);

    const fileInput = drawer.locator("[data-web-file-input]");
    await expect(fileInput).toBeAttached();

    // Set two files: one valid (fixture-b) and one malformed (fixture-bad).
    await fileInput.setInputFiles([
      { name: "fixture-b.ttl", mimeType: "text/turtle", buffer: Buffer.from(FIXTURE_B) },
      { name: "fixture-bad.ttl", mimeType: "text/turtle", buffer: Buffer.from(FIXTURE_BAD) },
    ]);

    // Both rows appear.
    await expect(drawer.locator('[data-file-row="fixture-b.ttl"]')).toBeVisible();
    await expect(drawer.locator('[data-file-row="fixture-bad.ttl"]')).toBeVisible();

    // Import.
    const importBtn = drawer.getByRole("button", { name: /^Import/i });
    await expect(importBtn).toBeEnabled();
    await importBtn.click();

    // The aggregate feedback is an error/partial-error banner (not a full ok).
    await expect(drawer.locator('[data-import-feedback="error"]')).toBeVisible();

    // The bad file's row has a per-file error attribute — error is surfaced, not swallowed.
    await expect(drawer.locator('[data-file-error="fixture-bad.ttl"]')).toBeAttached();

    // The valid file's row shows "quads imported" — the batch was NOT aborted by the bad file.
    await expect(
      drawer.locator('[data-file-row="fixture-b.ttl"]').getByText(/quads? imported/),
    ).toBeVisible();

    // Close and verify fixture-b's triples ARE in the store (the valid file was imported).
    await page.getByRole("button", { name: "Close import drawer" }).click();
    await expect(drawer).not.toBeVisible();

    const editor = page.locator("#repl-query");
    await editor.fill(
      `SELECT * WHERE { <http://example.org/upload-b#Dave> ?p ?o } LIMIT 1`,
    );
    await page.getByRole("button", { name: "Run query" }).click();
    const result = page.locator('[data-result-kind="select"]');
    await expect(result).toBeVisible();
    await expect(result.getByText("Dave Upload B")).toBeVisible();

    // fixture-b's triples are new (bad file contributed zero).
    const after = await countQuads(page);
    expect(after - baseline).toBeGreaterThanOrEqual(3); // fixture-b has 3 triples
  });

  // ── (d) A .zst upload decodes in-tab via the lazily-loaded fzstd chunk ─────────────────────

  test("(d) a .zst upload is decompressed in-tab and imported", async ({ page }) => {
    const baseline = await countQuads(page);

    const drawer = await openFileTab(page);
    const fileInput = drawer.locator("[data-web-file-input]");
    await expect(fileInput).toBeAttached();

    // The outer name is `.nt.zst`; the inner document is N-Triples.
    await fileInput.setInputFiles([
      {
        name: "compressed.nt.zst",
        mimeType: "application/zstd",
        buffer: Buffer.from(ZSTD_FIXTURE_BASE64, "base64"),
      },
    ]);

    // The row appears with the INNER format (ntriples), not the container extension. If the
    // decompression had failed, readFilesWithDecompress would have produced a rejection row
    // instead of a file row, so the presence of this row is itself part of the assertion.
    const row = drawer.locator('[data-file-row="compressed.nt.zst"]');
    await expect(row).toBeVisible();
    await expect(row.getByText("ntriples")).toBeVisible();

    const importBtn = drawer.getByRole("button", { name: /^Import/i });
    await expect(importBtn).toBeEnabled();
    await importBtn.click();

    await expect(drawer.locator('[data-import-feedback="ok"]')).toBeVisible();
    await expect(row.getByText(/quads? imported/)).toBeVisible();

    // Non-vacuous: the decompressed triples must be queryable, not merely "no error".
    await page.getByRole("button", { name: "Close import drawer" }).click();
    await expect(drawer).not.toBeVisible();

    await page
      .locator("#repl-query")
      .fill(`SELECT * WHERE { <${ZSTD_FIXTURE_SUBJECT}> ?p ?o }`);
    await page.getByRole("button", { name: "Run query" }).click();
    const result = page.locator('[data-result-kind="select"]');
    await expect(result).toBeVisible();
    await expect(result.getByText(ZSTD_FIXTURE_OBJECT)).toBeVisible();

    expect(await countQuads(page)).toBeGreaterThan(baseline);
  });

  // ── (e) A .zip upload takes its RDF format from the inner member name ─────────────────────

  test("(e) a .zip upload derives its RDF format from the inner member name", async ({
    page,
  }) => {
    const baseline = await countQuads(page);

    const drawer = await openFileTab(page);
    const fileInput = drawer.locator("[data-web-file-input]");
    await expect(fileInput).toBeAttached();

    // `bundle.zip` carries `dataset.nq`. Guessing from the OUTER name yields the Turtle
    // fallback, under which this member is a syntax error — so this upload can only import
    // when the inner member name drives the format.
    await fileInput.setInputFiles([
      {
        name: "bundle.zip",
        mimeType: "application/zip",
        buffer: makeStoredZip(ZIP_MEMBER_NAME, ZIP_MEMBER_NQUADS),
      },
    ]);

    const row = drawer.locator('[data-file-row="bundle.zip"]');
    await expect(row).toBeVisible();
    // The row previews `nquads` — the format the member name resolves to, NOT `turtle`.
    await expect(row.getByText("nquads")).toBeVisible();

    const importBtn = drawer.getByRole("button", { name: /^Import/i });
    await expect(importBtn).toBeEnabled();
    await importBtn.click();

    // No per-file parse error, and the aggregate banner is a clean success.
    await expect(drawer.locator('[data-import-feedback="ok"]')).toBeVisible();
    await expect(drawer.locator('[data-file-error="bundle.zip"]')).toHaveCount(0);
    await expect(row.getByText(/quads? imported/)).toBeVisible();

    // Non-vacuous: the quad landed in its NAMED GRAPH, which only an N-Quads parse produces.
    await page.getByRole("button", { name: "Close import drawer" }).click();
    await expect(drawer).not.toBeVisible();

    await page
      .locator("#repl-query")
      .fill(`SELECT * WHERE { GRAPH <${ZIP_GRAPH}> { ?s ?p ?o } }`);
    await page.getByRole("button", { name: "Run query" }).click();
    const result = page.locator('[data-result-kind="select"]');
    await expect(result).toBeVisible();
    await expect(result.getByText("Frank Zip")).toBeVisible();

    expect(await countQuads(page)).toBeGreaterThan(baseline);
  });
});
