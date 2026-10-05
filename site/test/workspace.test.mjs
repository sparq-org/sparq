// [OPUS-4.8] sq-atb0 — unit tests for the persistent cross-session WORKSPACE model + the
// persistence abstraction (`@sparq/client` `workspace.ts`): the data model + (de)serialisation
// guards, the web (localStorage) backend over an in-memory polyfill, the in-memory backend, the
// Tauri backend over an in-memory fake filesystem, and the runtime backend-selection factory.
// These cover the pure, framework-free logic the REPL workspace panel relies on. Run via
// `npm run test:unit`.
//
// Imported by RELATIVE path (not the `@sparq/client` alias) so the build-free ts-loader resolves
// it. In Node there is no `window`/`localStorage`, so the tests inject the storage surfaces
// explicitly — exactly the dependency-injection seams the abstraction is designed for.
import { test } from "node:test";
import assert from "node:assert/strict";

import {
  WORKSPACE_SCHEMA,
  WORKSPACE_INFERENCE_MODES,
  WebWorkspaceStore,
  MemoryWorkspaceStore,
  TauriWorkspaceStore,
  createSerializedWorkspaceSaver,
  createWorkspaceStore,
  describeWorkspaceSaveError,
  detectWebStorage,
  isQuotaExceededError,
  isTauriRuntime,
  newWorkspace,
  parseInferenceMode,
  parseWorkspace,
  workspaceId,
  workspaceSummary,
} from "../../packages/sparq-client/src/workspace.ts";

// A minimal in-memory localStorage polyfill (the `KeyValueStorage` shape).
function fakeKv() {
  const map = new Map();
  return {
    getItem: (k) => (map.has(k) ? map.get(k) : null),
    setItem: (k, v) => map.set(k, String(v)),
    removeItem: (k) => map.delete(k),
    _map: map,
  };
}

// A minimal in-memory TauriFsApi over a path→contents map.
function fakeFs() {
  const files = new Map();
  return {
    baseDir: 11, // AppLocalData placeholder
    _files: files,
    async readTextFile(path) {
      if (!files.has(path)) throw new Error("ENOENT");
      return files.get(path);
    },
    async writeTextFile(path, contents) {
      files.set(path, contents);
    },
    async remove(path) {
      files.delete(path);
    },
    async exists(path) {
      // Treat the dir marker generously: any path that is a known dir prefix "exists".
      if (files.has(path)) return true;
      return [...files.keys()].some((k) => k.startsWith(path + "/"));
    },
    async mkdir() {},
    async readDir() {
      return [];
    },
  };
}

// --- the data model ---------------------------------------------------------

test("newWorkspace seeds a current-schema record with a fresh id", () => {
  const ws = newWorkspace("Demo", "SELECT * WHERE { ?s ?p ?o }");
  assert.equal(ws.schema, WORKSPACE_SCHEMA);
  assert.equal(ws.name, "Demo");
  assert.equal(ws.editor.query, "SELECT * WHERE { ?s ?p ?o }");
  assert.equal(ws.editor.mode, "run");
  assert.equal(ws.editor.endpointActive, false);
  assert.deepEqual(ws.sources, []);
  assert.match(ws.id, /^ws_/);
});

test("workspaceId is unique across calls", () => {
  const a = workspaceId();
  const b = workspaceId();
  assert.notEqual(a, b);
});

test("workspaceSummary derives an approx triple count from the snapshot", () => {
  const ws = newWorkspace("S", "SELECT 1");
  ws.dataSnapshot = "<a> <b> <c> .\n<d> <e> <f> .";
  ws.sources = [{ kind: "url", label: "x", format: "turtle", importedAt: 1 }];
  const s = workspaceSummary(ws);
  assert.equal(s.approxTriples, 2);
  assert.equal(s.sourceCount, 1);
  assert.equal(workspaceSummary(newWorkspace("E", "")).approxTriples, 0);
});

test("parseWorkspace round-trips a valid record and rejects junk", () => {
  const ws = newWorkspace("RT", "ASK {}");
  ws.dataSnapshot = "<a> <b> <c> .";
  ws.sources = [
    { kind: "local", label: "f.ttl", format: "turtle", bytes: 12, importedAt: 5 },
    { kind: "url", label: "remote", url: "https://x/y.ttl", format: "turtle", importedAt: 6 },
  ];
  const parsed = parseWorkspace(JSON.parse(JSON.stringify(ws)));
  assert.ok(parsed);
  assert.equal(parsed.name, "RT");
  assert.equal(parsed.sources.length, 2);
  assert.equal(parsed.sources[1].url, "https://x/y.ttl");
  // junk / stale-schema / missing fields are rejected
  assert.equal(parseWorkspace(null), null);
  assert.equal(parseWorkspace({}), null);
  assert.equal(parseWorkspace({ ...ws, schema: 999 }), null);
  assert.equal(parseWorkspace({ ...ws, editor: undefined }), null);
});

test("parseWorkspace drops malformed sources but keeps valid ones", () => {
  const ws = newWorkspace("M", "SELECT 1");
  const raw = JSON.parse(JSON.stringify(ws));
  raw.sources = [
    { kind: "bogus" },
    { kind: "url", label: "ok", format: "turtle", importedAt: 1 },
    null,
  ];
  const parsed = parseWorkspace(raw);
  assert.equal(parsed.sources.length, 1);
  assert.equal(parsed.sources[0].label, "ok");
});

test("parseWorkspace never persists a token field (editor carries no secret)", () => {
  const ws = newWorkspace("T", "SELECT 1");
  ws.editor.endpointActive = true;
  ws.editor.endpointUrl = "https://srv/sparql";
  const parsed = parseWorkspace(JSON.parse(JSON.stringify(ws)));
  assert.equal(parsed.editor.endpointUrl, "https://srv/sparql");
  assert.ok(!("token" in parsed.editor));
});

// --- the web (localStorage) backend ----------------------------------------

test("WebWorkspaceStore: save → list → load → setLast → remove", async () => {
  const store = new WebWorkspaceStore(fakeKv());
  assert.equal(store.backend, "web");
  assert.deepEqual(await store.list(), []);

  const a = newWorkspace("A", "SELECT 1");
  const b = newWorkspace("B", "SELECT 2");
  await store.save(a);
  // A 2 ms gap so `b.updatedAt` is strictly later than `a.updatedAt` even on a coarse clock,
  // making the most-recently-updated-first ordering deterministic for the assertion below.
  await new Promise((r) => setTimeout(r, 2));
  await store.save(b);

  const list = await store.list();
  assert.equal(list.length, 2);
  // most-recently-updated first (b saved after a)
  assert.equal(list[0].id, b.id);

  const loaded = await store.load(a.id);
  assert.equal(loaded.name, "A");

  await store.setLastOpenedId(a.id);
  assert.equal(await store.lastOpenedId(), a.id);

  await store.remove(a.id);
  assert.equal(await store.load(a.id), null);
  assert.equal((await store.list()).length, 1);
  // last-opened pointer is cleared when the workspace it pointed at is deleted
  assert.equal(await store.lastOpenedId(), null);
});

test("WebWorkspaceStore: lastOpenedId is null when it points at a missing id", async () => {
  const kv = fakeKv();
  const store = new WebWorkspaceStore(kv);
  await store.setLastOpenedId("ws_ghost");
  assert.equal(await store.lastOpenedId(), null);
});

test("WebWorkspaceStore: a corrupt record is skipped, not thrown", async () => {
  const kv = fakeKv();
  const store = new WebWorkspaceStore(kv);
  const a = newWorkspace("Good", "SELECT 1");
  await store.save(a);
  // Hand-corrupt a second indexed entry.
  kv.setItem("sparq.workspace.v1.__index__", JSON.stringify([a.id, "ws_corrupt"]));
  kv.setItem("sparq.workspace.v1.ws_corrupt", "{not json");
  const list = await store.list();
  assert.equal(list.length, 1);
  assert.equal(list[0].name, "Good");
  assert.equal(await store.load("ws_corrupt"), null);
});

// --- the in-memory backend --------------------------------------------------

test("MemoryWorkspaceStore behaves like the web backend (session-only)", async () => {
  const store = new MemoryWorkspaceStore();
  assert.equal(store.backend, "memory");
  const a = newWorkspace("A", "SELECT 1");
  await store.save(a);
  await store.setLastOpenedId(a.id);
  assert.equal((await store.list()).length, 1);
  assert.equal(await store.lastOpenedId(), a.id);
  await store.remove(a.id);
  assert.equal(await store.lastOpenedId(), null);
});

// --- the Tauri backend (over a fake fs) ------------------------------------

test("TauriWorkspaceStore: save → list → load → remove over a fake fs", async () => {
  const fs = fakeFs();
  const store = new TauriWorkspaceStore(fs);
  assert.equal(store.backend, "tauri");

  const a = newWorkspace("Disk A", "SELECT 1");
  a.dataSnapshot = "<a> <b> <c> .";
  await store.save(a);
  await store.setLastOpenedId(a.id);

  // The record + index + last pointer are real files under workspaces/.
  assert.ok(fs._files.has(`workspaces/${a.id}.json`));
  assert.ok(fs._files.has("workspaces/__index__.json"));

  const list = await store.list();
  assert.equal(list.length, 1);
  assert.equal(list[0].approxTriples, 1);

  const loaded = await store.load(a.id);
  assert.equal(loaded.name, "Disk A");
  assert.equal(await store.lastOpenedId(), a.id);

  await store.remove(a.id);
  assert.equal(await store.load(a.id), null);
  assert.equal(await store.lastOpenedId(), null);
});

test("TauriWorkspaceStore: concurrent saves for DIFFERENT ids preserve both index entries", async () => {
  const fs = fakeFs();
  const rawWrite = fs.writeTextFile.bind(fs);
  let indexWrites = 0;
  let releaseFirstIndexWrite;
  const firstIndexWriteGate = new Promise((resolve) => (releaseFirstIndexWrite = resolve));
  let firstIndexWriteStarted;
  const firstIndexWriteReady = new Promise((resolve) => (firstIndexWriteStarted = resolve));
  fs.writeTextFile = async (path, contents) => {
    if (path === "workspaces/__index__.json" && ++indexWrites === 1) {
      firstIndexWriteStarted();
      await firstIndexWriteGate;
    }
    await rawWrite(path, contents);
  };

  const store = new TauriWorkspaceStore(fs);
  const saver = createSerializedWorkspaceSaver(store.save.bind(store));
  const a = newWorkspace("Concurrent A", "SELECT 1");
  const b = newWorkspace("Concurrent B", "SELECT 2");

  const saveA = saver.save(a);
  await firstIndexWriteReady;
  const saveB = saver.save(b);
  await new Promise((resolve) => setImmediate(resolve));
  releaseFirstIndexWrite();
  await Promise.all([saveA, saveB]);

  assert.deepEqual(
    new Set(JSON.parse(fs._files.get("workspaces/__index__.json"))),
    new Set([a.id, b.id]),
  );
});

// --- runtime backend selection ---------------------------------------------

test("isTauriRuntime is false in a headless Node (no window)", () => {
  assert.equal(isTauriRuntime(), false);
});

test("detectWebStorage returns null when localStorage is absent", () => {
  // Node has no localStorage global → null (the memory fallback path).
  assert.equal(detectWebStorage(), null);
});

test("createWorkspaceStore falls back to memory with no storage + no Tauri", async () => {
  const store = await createWorkspaceStore();
  assert.equal(store.backend, "memory");
});

test("createWorkspaceStore ignores a Tauri fs loader outside a Tauri runtime", async () => {
  let called = false;
  const store = await createWorkspaceStore({
    loadTauriFs: async () => {
      called = true;
      return fakeFs();
    },
  });
  // Not in a Tauri webview → the loader is never invoked, and it degrades to memory.
  assert.equal(called, false);
  assert.equal(store.backend, "memory");
});

// --- sq-glo5r: N3 inference mode + rulesDocs --------------------------------

test("parseInferenceMode recognises 'n3'", () => {
  assert.equal(parseInferenceMode("n3"), "n3");
});

test("WORKSPACE_INFERENCE_MODES includes 'n3'", () => {
  assert.ok(WORKSPACE_INFERENCE_MODES.includes("n3"));
});

test("parseWorkspace round-trips a workspace with rulesDocs", () => {
  const ws = newWorkspace("N3-test", "SELECT 1");
  ws.inference = "n3";
  ws.rulesDocs = [
    { name: "my-rule", text: "{ ?s a ex:A } => { ?s a ex:B . }", addedAt: 123456 },
    { name: "disabled-rule", text: "{ ?x a ex:C } => { ?x a ex:D . }", addedAt: 123457, enabled: false },
  ];
  const parsed = parseWorkspace(JSON.parse(JSON.stringify(ws)));
  assert.ok(parsed);
  assert.equal(parsed.inference, "n3");
  assert.equal(parsed.rulesDocs.length, 2);
  assert.equal(parsed.rulesDocs[0].name, "my-rule");
  assert.equal(parsed.rulesDocs[0].addedAt, 123456);
  assert.equal(parsed.rulesDocs[0].enabled, undefined); // enabled=true is stored as absent
  assert.equal(parsed.rulesDocs[1].enabled, false);
});

test("parseWorkspace defaults to empty rulesDocs when the field is absent (old record)", () => {
  const ws = newWorkspace("old-record", "SELECT 1");
  const raw = JSON.parse(JSON.stringify(ws));
  delete raw.rulesDocs; // simulate a record from before sq-glo5r
  const parsed = parseWorkspace(raw);
  assert.ok(parsed);
  assert.deepEqual(parsed.rulesDocs, []);
});

test("parseWorkspace skips malformed rulesDocs entries but keeps valid ones", () => {
  const ws = newWorkspace("M", "SELECT 1");
  const raw = JSON.parse(JSON.stringify(ws));
  raw.rulesDocs = [
    { name: "good", text: "...", addedAt: 1 },
    { name: "no-text", addedAt: 2 }, // missing text → skipped
    null, // null → skipped
    { name: "no-addedAt", text: "..." }, // missing addedAt → skipped
  ];
  const parsed = parseWorkspace(raw);
  assert.ok(parsed);
  assert.equal(parsed.rulesDocs.length, 1);
  assert.equal(parsed.rulesDocs[0].name, "good");
});

// --- (sq-9i1h9) serialized per-workspace saves -------------------------------

test("createSerializedWorkspaceSaver: a DELAYED older save cannot overwrite a NEWER one (Tauri backend)", async () => {
  const fs = fakeFs();
  const store = new TauriWorkspaceStore(fs);
  const ws = newWorkspace("Race", "SELECT 1");
  // Delay only the FIRST underlying write — the exact out-of-order-overwrite scenario: fired
  // un-serialized (plain store.save), the slow OLD write would land after — and clobber — the
  // fast NEW one, leaving the on-disk file at the stale snapshot.
  let firstDelay = 25;
  const write = async (w) => {
    const d = firstDelay;
    firstDelay = 0;
    if (d) await new Promise((r) => setTimeout(r, d));
    await store.save(w);
  };
  const saver = createSerializedWorkspaceSaver(write);
  const oldRev = { ...ws, editor: { ...ws.editor, query: "OLD" } };
  const newRev = { ...ws, editor: { ...ws.editor, query: "NEW" } };
  const p1 = saver.save(oldRev);
  const p2 = saver.save(newRev);
  await Promise.all([p1, p2]);
  const loaded = await store.load(ws.id);
  // Serialized + revision-countered, the file MUST end at the newest revision. (Mutation check:
  // swap `saver.save` for the raw un-serialized `write` above and this assertion goes red —
  // the delayed OLD write lands last.)
  assert.equal(loaded.editor.query, "NEW");
});

test("createSerializedWorkspaceSaver: stale queued revisions are DROPPED (monotonic revision counter)", async () => {
  const writes = [];
  let release;
  const gate = new Promise((r) => (release = r));
  const saver = createSerializedWorkspaceSaver(async (w) => {
    if (writes.length === 0) await gate; // hold the FIRST write in flight
    writes.push(w.editor.query);
  });
  const ws = newWorkspace("Coalesce", "SELECT 1");
  const p1 = saver.save({ ...ws, editor: { ...ws.editor, query: "v1" } });
  await new Promise((r) => setImmediate(r)); // let the v1 write actually START (held by the gate)
  const p2 = saver.save({ ...ws, editor: { ...ws.editor, query: "v2" } });
  const p3 = saver.save({ ...ws, editor: { ...ws.editor, query: "v3" } });
  release();
  await Promise.all([p1, p2, p3]);
  // v1 was already in flight; while it ran, v2 went stale (v3 arrived) and is dropped — its
  // state is covered by the newer whole-workspace write v3 in the same chain.
  assert.deepEqual(writes, ["v1", "v3"]);
});

test("createSerializedWorkspaceSaver: a failing save rejects its caller but does NOT wedge the chain", async () => {
  const writes = [];
  let failFirst = true;
  const saver = createSerializedWorkspaceSaver(async (w) => {
    if (failFirst) {
      failFirst = false;
      throw new Error("disk full");
    }
    writes.push(w.editor.query);
  });
  const ws = newWorkspace("Wedge", "SELECT 1");
  await assert.rejects(saver.save({ ...ws, editor: { ...ws.editor, query: "v1" } }), /disk full/);
  await saver.save({ ...ws, editor: { ...ws.editor, query: "v2" } });
  assert.deepEqual(writes, ["v2"]);
});

test("createSerializedWorkspaceSaver: chains are PER workspace — one slow workspace never blocks another", async () => {
  const done = [];
  let release;
  const gate = new Promise((r) => (release = r));
  const saver = createSerializedWorkspaceSaver(async (w) => {
    if (w.name === "slow") await gate;
    done.push(w.name);
  });
  const a = newWorkspace("slow", "SELECT 1");
  const b = newWorkspace("fast", "SELECT 1");
  const pa = saver.save(a);
  const pb = saver.save(b);
  await pb; // resolves while `a`'s write is still gated — independent chains
  assert.deepEqual(done, ["fast"]);
  release();
  await pa;
  assert.deepEqual(done, ["fast", "slow"]);
});

// --- (sq-w3dmj) quota-exceeded surfacing -------------------------------------

test("WebWorkspaceStore.save PROPAGATES QuotaExceededError — never swallowed at the store layer", async () => {
  const kv = fakeKv();
  kv.setItem = () => {
    const e = new Error("the quota has been exceeded");
    e.name = "QuotaExceededError";
    throw e;
  };
  const store = new WebWorkspaceStore(kv);
  await assert.rejects(store.save(newWorkspace("Big", "SELECT 1")), (e) => {
    assert.equal(e.name, "QuotaExceededError");
    return true;
  });
});

test("isQuotaExceededError + describeWorkspaceSaveError classify a quota failure honestly", () => {
  const quota = new Error("exceeded");
  quota.name = "QuotaExceededError";
  assert.equal(isQuotaExceededError(quota), true);
  assert.match(describeWorkspaceSaveError(quota), /quota/i);

  // Firefox's legacy name is covered too.
  const legacy = new Error("legacy");
  legacy.name = "NS_ERROR_DOM_QUOTA_REACHED";
  assert.equal(isQuotaExceededError(legacy), true);
  assert.match(describeWorkspaceSaveError(legacy), /quota/i);

  // A non-quota failure keeps its underlying message; junk degrades to a generic label.
  const other = new Error("boom");
  assert.equal(isQuotaExceededError(other), false);
  assert.match(describeWorkspaceSaveError(other), /boom/);
  assert.equal(isQuotaExceededError(null), false);
  assert.match(describeWorkspaceSaveError(null), /workspace save failed/);
  assert.equal(isQuotaExceededError("string"), false);
});
