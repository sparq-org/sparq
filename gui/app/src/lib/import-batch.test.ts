// (sq-810a0) [FABLE-5] — unit tests for the batch-import mode sequencer (import-batch.ts).
//
// THE REGRESSION UNDER TEST (GPT-5.6 GUI review, adversarially verified): the Import drawer's
// WEB and NATIVE multi-file loops decided each file's import mode by ARRAY INDEX
// (`i === 0 ? mode : "add"`). So a selected `replace` was pinned to file[0]. If file[0] FAILED to
// parse (its importer throws BEFORE any store mutation) but a later file SUCCEEDED, the `replace`
// was silently never applied: the old dataset survived and the later file was merged in as `add`
// — the exact opposite of the user's replace intent.
//
// The load-bearing property proved here: with file[0] failing and file[1] succeeding, file[1] is
// imported under `replace` (NOT `add`), so the store ends REPLACED-with-file[1], not old+file[1].
// The mock importer models `runImport` (engine-context.tsx): it decodes then mutates a fake store
// ONLY on success; a thrown parse error leaves the fake store untouched (as the real one does).
//
// Run via:   npm run test:unit   (gui/app)
import { test } from "node:test";
import assert from "node:assert/strict";

import { nextFileMode, runImportBatch, summariseImportedBatch } from "./import-batch.js";
import type { ImportMode } from "./engine-context.js";

// ── A fake store that mirrors runImport's replace/add semantics (engine-context.tsx §2) ─────────
//
// `replace` → the store becomes exactly the incoming file. `add` → the incoming file is appended.
// A file whose name starts with "BAD" throws at "decode" — BEFORE any store mutation — exactly
// like a parse error in the real importer.

interface FakeItem {
  name: string;
  /** Quads this file contributes (its identity, so we can assert what survived). */
  quads: string[];
}

class FakeStore {
  contents: string[] = [];

  /** Import one file under `mode`, replicating runImport: throw on decode (BAD*) before mutating. */
  importOne = async (item: FakeItem, mode: ImportMode): Promise<{ added: number }> => {
    if (item.name.startsWith("BAD")) {
      // Decode fails first; the store is NOT mutated (storeRef.current stays as-is).
      throw new Error(`parse error in ${item.name}`);
    }
    if (mode === "replace") {
      this.contents = [...item.quads];
    } else {
      this.contents.push(...item.quads);
    }
    return { added: item.quads.length };
  };
}

test("nextFileMode: replace carries until the first success, then add", () => {
  // Not yet replaced → the selected `replace` is offered.
  assert.equal(nextFileMode("replace", false), "replace");
  // Already replaced (a prior success consumed it) → add.
  assert.equal(nextFileMode("replace", true), "add");
  // `add` selected → always add, regardless of state.
  assert.equal(nextFileMode("add", false), "add");
  assert.equal(nextFileMode("add", true), "add");
});

test("REGRESSION sq-810a0: file[0] fails, file[1] succeeds under REPLACE — store ends replaced-with-file[1], NOT old+file[1]", async () => {
  const store = new FakeStore();
  // The pre-existing dataset the user intends to REPLACE.
  store.contents = ["<old> <p> <o> ."];

  const files: FakeItem[] = [
    { name: "BAD-first.ttl", quads: ["<a> <p> <a> ."] }, // fails at decode
    { name: "good-second.ttl", quads: ["<b> <p> <b> ."] }, // succeeds
  ];

  const { statuses, summary } = await runImportBatch(
    files,
    "replace",
    (f) => f.name,
    store.importOne,
  );

  // file[0] failed, file[1] succeeded.
  assert.equal(statuses["BAD-first.ttl"].kind, "error");
  assert.equal(statuses["good-second.ttl"].kind, "ok");

  // THE FIX: the store is REPLACED with file[1] — the stale <old> quad is GONE and file[1]'s
  // quad is the only content. Under the buggy index-based logic file[1] would import as `add`,
  // leaving the store as ["<old> …", "<b> …"] (old + file[1] coexisting).
  assert.deepEqual(
    store.contents,
    ["<b> <p> <b> ."],
    "the old dataset must be dropped and only file[1] must remain (replace applied to first success)",
  );
  assert.ok(!store.contents.includes("<old> <p> <o> ."), "the old quad must NOT survive a replace");

  // Summary reflects that the replace was actually applied.
  assert.equal(summary.replaceRequested, true);
  assert.equal(summary.replaceApplied, true);
  assert.equal(summary.okCount, 1);
  assert.equal(summary.errCount, 1);
});

test("replace applied to file[0] when it succeeds; file[1] then adds", async () => {
  const store = new FakeStore();
  store.contents = ["<old> <p> <o> ."];
  const files: FakeItem[] = [
    { name: "good-first.ttl", quads: ["<a> <p> <a> ."] },
    { name: "good-second.ttl", quads: ["<b> <p> <b> ."] },
  ];

  const { summary } = await runImportBatch(files, "replace", (f) => f.name, store.importOne);

  // file[0] replaced (old gone), file[1] added on top.
  assert.deepEqual(store.contents, ["<a> <p> <a> .", "<b> <p> <b> ."]);
  assert.equal(summary.replaceApplied, true);
});

test("all files fail under replace → store unchanged, replaceApplied=false", async () => {
  const store = new FakeStore();
  store.contents = ["<old> <p> <o> ."];
  const files: FakeItem[] = [
    { name: "BAD-1.ttl", quads: ["<a> <p> <a> ."] },
    { name: "BAD-2.ttl", quads: ["<b> <p> <b> ."] },
  ];

  const { summary } = await runImportBatch(files, "replace", (f) => f.name, store.importOne);

  // Nothing succeeded → old data is intact (runImport never mutates on a failed decode).
  assert.deepEqual(store.contents, ["<old> <p> <o> ."]);
  assert.equal(summary.replaceApplied, false);
  assert.equal(summary.replaceRequested, true);
  assert.equal(summary.okCount, 0);
  assert.equal(summary.errCount, 2);
});

test("add mode is unaffected: every file appends", async () => {
  const store = new FakeStore();
  store.contents = ["<old> <p> <o> ."];
  const files: FakeItem[] = [
    { name: "one.ttl", quads: ["<a> <p> <a> ."] },
    { name: "two.ttl", quads: ["<b> <p> <b> ."] },
  ];

  const { summary } = await runImportBatch(files, "add", (f) => f.name, store.importOne);

  assert.deepEqual(store.contents, ["<old> <p> <o> .", "<a> <p> <a> .", "<b> <p> <b> ."]);
  assert.equal(summary.replaceRequested, false);
  assert.equal(summary.replaceApplied, false);
});

test("a single failure does not abort the batch (sq-eydh9 invariant preserved)", async () => {
  const store = new FakeStore();
  const files: FakeItem[] = [
    { name: "good-1.ttl", quads: ["<a> <p> <a> ."] },
    { name: "BAD-mid.ttl", quads: ["<x> <p> <x> ."] },
    { name: "good-2.ttl", quads: ["<b> <p> <b> ."] },
  ];

  const { statuses, summary } = await runImportBatch(files, "add", (f) => f.name, store.importOne);

  assert.equal(statuses["good-1.ttl"].kind, "ok");
  assert.equal(statuses["BAD-mid.ttl"].kind, "error");
  assert.equal(statuses["good-2.ttl"].kind, "ok"); // batch continued past the failure
  assert.equal(summary.okCount, 2);
  assert.equal(summary.errCount, 1);
});

// ── #6257 — the web batch's workspace-history entry carries real bytes + format ──────────────

test("summariseImportedBatch: nothing imported records nothing", () => {
  assert.equal(summariseImportedBatch([]), null);
});

test("summariseImportedBatch: one file keeps its own label, format and bytes", () => {
  assert.deepEqual(summariseImportedBatch([{ label: "a.ttl", format: "turtle", bytes: 42 }]), {
    label: "a.ttl",
    format: "turtle",
    bytes: 42,
  });
});

test("summariseImportedBatch: sums real bytes; a shared format is kept, else 'mixed'", () => {
  assert.deepEqual(
    summariseImportedBatch([
      { label: "a.ttl", format: "turtle", bytes: 100 },
      { label: "b.ttl", format: "turtle", bytes: 23 },
    ]),
    { label: "2 files", format: "turtle", bytes: 123 },
  );
  assert.deepEqual(
    summariseImportedBatch([
      { label: "a.ttl", format: "turtle", bytes: 100 },
      { label: "b.nq", format: "nquads", bytes: 5 },
      { label: "c.ttl", format: "turtle", bytes: 1 },
    ]),
    { label: "3 files", format: "mixed", bytes: 106 },
  );
});
