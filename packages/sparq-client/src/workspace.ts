// [OPUS-4.8] sq-atb0 (epic sq-ixc3) — the persistent cross-session WORKSPACE model + the
// persistence ABSTRACTION that lets a user's work survive an app restart.
//
// WHAT A WORKSPACE IS. A named, self-contained piece of GUI work:
//   * its imported data sources — both local-file imports AND URL-loaded sources — captured
//     as IMPORT METADATA (kind, label, url, format, byte size) so the dataset list re-hydrates
//     and a URL source can be re-fetched;
//   * a SNAPSHOT of the loaded dataset as N-Quads text (the engine's whole default+named-graph
//     content), so re-opening a workspace restores the EXACT store it was saved with — a
//     save/open snapshot cache, NOT a re-ingest-from-source-on-open (maintainer decision on
//     sq-atb0 / #757, mirroring the engine's online-snapshot primitive sq-o5bi / #905);
//   * the SPARQL editor state — the query text plus a small amount of editor UI state (the
//     run mode, and whether endpoint mode was active with its endpoint config sans secrets).
//
// WHY HERE (framework-agnostic). This is pure data + a thin async persistence interface with
// NO React, NO Next.js, and NO DOM beyond the two storage globals it FEATURE-DETECTS at
// runtime. It transfers unchanged to any GUI host — the design's load-bearing "part (A)
// transfers" property — exactly like the results / endpoint / highlight cores already in this
// package. The site-specific glue (serialising the live wasm Store to a snapshot and back) is
// a thin layer the site owns; this module is only "the shape of a workspace and where it is
// stored".
//
// PERSISTENCE TIERS (one abstraction, runtime-selected — never assume Tauri exists):
//   * Tauri desktop (`tauri` backend) — persists each workspace to a JSON file in the app's
//     local data dir via the Tauri filesystem plugin, so it reloads on next launch. Selected
//     ONLY when the Tauri filesystem plugin is actually present at runtime (feature-detected);
//     the desktop shell must grant the `fs` capability for this path to activate.
//   * Browser / GitHub Pages static export (`web` backend) — degrades gracefully to
//     `localStorage`. Static-export-safe; works in any browser AND inside a Tauri webview that
//     has not (yet) granted the fs capability.
//   * No storage at all (SSR / Node build, or a locked-down webview) — an in-memory
//     (`memory`) backend so the model still functions for one session and the build never
//     hard-fails on a missing global.
//
// NO PERFORMANCE CLAIM is made anywhere here. NO secret (a bearer token) is ever persisted.

// ---------------------------------------------------------------------------
// The data model.
// ---------------------------------------------------------------------------

/**
 * Where one imported data source came from. We persist enough METADATA to (a) re-draw the
 * dataset list and (b) re-fetch a URL source on demand. We do NOT persist a file-system
 * handle: browsers cannot silently re-read a previously chosen local file across sessions
 * (no persistent handle without a fresh user gesture + the non-static File System Access
 * API), so a `local` source re-hydrates from the workspace SNAPSHOT, not from the original
 * file. This limitation is documented honestly in the UI, not papered over.
 */
export interface WorkspaceSourceMeta {
  /** A `local` file import (re-hydrates from the snapshot only) or a `url` load (re-fetchable). */
  kind: "local" | "url";
  /** Display label — a filename for `local`, a short URL tail for `url`. */
  label: string;
  /** The fetch URL for a `url` source (absent for `local`). */
  url?: string;
  /** The RDF serialisation the source was parsed as (`turtle` / `nquads` / `trig` / …). */
  format: string;
  /** UTF-8 byte length of the imported document, for an honest "≈N bytes" note (best-effort). */
  bytes?: number;
  /** Epoch milliseconds the source was imported (for ordering / display). */
  importedAt: number;
}

/** Which engine the editor was driving when the workspace was saved. */
export type WorkspaceRunMode = "run" | "explain" | "analyze";

/**
 * [OPUS-4.8] sq-tp1m (#757) — the per-workspace INFERENCE (entailment) regime applied to
 * QUERIES over the live store. `"off"` runs plain SPARQL over the asserted data; `"rdfs"` and
 * `"owl-rl"` forward-chain the deductive closure under that entailment profile (OWL 2 RL
 * includes RDFS) BEFORE the query runs, so an entailed triple can match. This is applied by the
 * engine's real forward-chaining reasoner (`sparq-reason`, via the tier-b W-reason wasm
 * bundle) — NOT a mock. It is a query-time regime: it never mutates the persisted store, only
 * what queries see. `"n3"` applies per-workspace N3 rules ({@link WorkspaceRulesDoc}) via the
 * same reasoner's `reasonN3` path — derived GROUND triples appear in query results; formula
 * conclusions are dropped (sq-glo5r).
 */
export type WorkspaceInferenceMode = "off" | "rdfs" | "owl-rl" | "n3";

/** The persisted inference modes, in selector order (the UI mirrors this). */
export const WORKSPACE_INFERENCE_MODES: readonly WorkspaceInferenceMode[] = [
  "off",
  "rdfs",
  "owl-rl",
  "n3",
] as const;

/** Normalise an arbitrary value to a {@link WorkspaceInferenceMode}, defaulting to `"off"`. */
export function parseInferenceMode(value: unknown): WorkspaceInferenceMode {
  return value === "rdfs" || value === "owl-rl" || value === "n3" ? value : "off";
}

/**
 * [sq-glo5r] One N3 rule document attached to a workspace. Each document is an N3 Turtle file
 * (prefix declarations + one or more `{ antecedent } => { consequent }` rules). Rules are
 * applied at query time in `"n3"` inference mode — derived GROUND triples appear in query
 * results; formula / quoted-graph conclusions are dropped and not visible in the triple store.
 * `enabled` defaults to `true` when absent (backward-compatible).
 */
export interface WorkspaceRulesDoc {
  /** Human-readable label for the rule document (e.g. a filename or authored name). */
  name: string;
  /** The N3 Turtle source text of the rule document. */
  text: string;
  /** Epoch milliseconds when the document was added (used as a stable unique key). */
  addedAt: number;
  /** Whether this document is applied; defaults to `true` when absent. */
  enabled?: boolean;
}

/**
 * The saved SPARQL editor state. `query` is the editor text; `mode` is the Run/EXPLAIN/ANALYZE
 * selector; `endpointActive` + `endpointUrl` capture whether the editor was pointed at a
 * running server (so the panel restores that view) — but the bearer TOKEN is NEVER persisted
 * (a secret), so a restored endpoint session re-prompts for it.
 */
export interface WorkspaceEditorState {
  query: string;
  mode: WorkspaceRunMode;
  endpointActive: boolean;
  /** The endpoint URL (no token) when endpoint mode was active. */
  endpointUrl?: string;
}

/**
 * One named workspace. `dataSnapshot` is the N-Quads serialisation of the whole dataset
 * (default graph + every named graph) at save time — the save/open cache. It MAY be absent
 * for a freshly-created empty workspace.
 */
export interface Workspace {
  /** Stable opaque id (also the storage key / filename stem). */
  id: string;
  /** User-facing name. */
  name: string;
  /** Epoch ms the workspace was created. */
  createdAt: number;
  /** Epoch ms of the last save. */
  updatedAt: number;
  /** The import metadata for every source the user loaded (local + url). */
  sources: WorkspaceSourceMeta[];
  /** The whole-dataset N-Quads snapshot at save time (the save/open cache); absent when empty. */
  dataSnapshot?: string;
  /** The saved SPARQL editor state. */
  editor: WorkspaceEditorState;
  /**
   * [OPUS-4.8] sq-tp1m (#757) — the per-workspace inference regime applied to queries (see
   * {@link WorkspaceInferenceMode}). Optional + backward-compatible: a workspace persisted
   * before this field existed simply omits it and loads as `"off"` (no schema bump needed).
   */
  inference?: WorkspaceInferenceMode;
  /**
   * [sq-glo5r] Per-workspace N3 rule documents. Applied when inference is `"n3"`. Optional +
   * backward-compatible: an older record without this field loads with an empty rules list.
   * Schema stays at 1 (the field is purely additive and defaults gracefully).
   */
  rulesDocs?: WorkspaceRulesDoc[];
  /**
   * [FABLE-5] sq-ixc3.14 — the per-workspace FEDERATION egress allowlist: the only SPARQL
   * endpoints a `SERVICE` clause may dial when the desktop GUI evaluates the query over the
   * native engine (`host` / `host:port` / `*.suffix` entries, the same grammar as
   * sparq-server's `--service-allow`). FAIL-CLOSED: absent or empty means every SERVICE
   * endpoint is refused. Optional + backward-compatible: an older record simply omits it
   * (no schema bump — purely additive, like `inference`).
   */
  serviceAllowlist?: string[];
  /**
   * The schema version of this persisted record. Bumped if the on-disk shape changes so a
   * loader can migrate or discard an incompatible old record rather than crash.
   */
  schema: number;
}

/** The on-disk schema version. Bump + migrate (or drop) on a breaking shape change. */
export const WORKSPACE_SCHEMA = 1;

/** A workspace stripped to its identity, for a lightweight switcher list. */
export interface WorkspaceSummary {
  id: string;
  name: string;
  updatedAt: number;
  /** Triple/quad lines in the snapshot, derived cheaply (newline count); 0 when empty. */
  approxTriples: number;
  sourceCount: number;
}

/** Reduce a full {@link Workspace} to its {@link WorkspaceSummary}. */
export function workspaceSummary(ws: Workspace): WorkspaceSummary {
  const snap = ws.dataSnapshot?.trim() ?? "";
  return {
    id: ws.id,
    name: ws.name,
    updatedAt: ws.updatedAt,
    approxTriples: snap === "" ? 0 : snap.split("\n").length,
    sourceCount: ws.sources.length,
  };
}

/** Create a new, empty workspace with a fresh id and the given starting editor text. */
export function newWorkspace(name: string, query: string): Workspace {
  const now = Date.now();
  return {
    id: workspaceId(),
    name,
    createdAt: now,
    updatedAt: now,
    sources: [],
    editor: { query, mode: "run", endpointActive: false },
    inference: "off",
    schema: WORKSPACE_SCHEMA,
  };
}

/**
 * A short, collision-resistant workspace id. Uses `crypto.randomUUID()` when available
 * (browsers, Tauri webview, modern Node), else a timestamp+random fallback so a build/SSR
 * environment without `crypto` still works.
 */
export function workspaceId(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  if (c?.randomUUID) return `ws_${c.randomUUID()}`;
  return `ws_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 10)}`;
}

/**
 * Validate + normalise a parsed JSON value into a {@link Workspace}, or return `null` if it is
 * not a recognisable, current-schema workspace. Used on load so a corrupt or stale record is
 * dropped rather than crashing the GUI. Defensive about every field (a hand-edited file or a
 * future-version record).
 */
export function parseWorkspace(value: unknown): Workspace | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  if (v.schema !== WORKSPACE_SCHEMA) return null;
  if (typeof v.id !== "string" || typeof v.name !== "string") return null;
  const editor = v.editor as Record<string, unknown> | undefined;
  if (!editor || typeof editor.query !== "string") return null;
  const mode: WorkspaceRunMode =
    editor.mode === "explain" || editor.mode === "analyze" ? editor.mode : "run";
  const sources: WorkspaceSourceMeta[] = Array.isArray(v.sources)
    ? v.sources.flatMap((s) => {
        const src = parseSource(s);
        return src ? [src] : [];
      })
    : [];
  return {
    id: v.id,
    name: v.name,
    createdAt: typeof v.createdAt === "number" ? v.createdAt : Date.now(),
    updatedAt: typeof v.updatedAt === "number" ? v.updatedAt : Date.now(),
    sources,
    dataSnapshot:
      typeof v.dataSnapshot === "string" ? v.dataSnapshot : undefined,
    editor: {
      query: editor.query,
      mode,
      endpointActive: editor.endpointActive === true,
      endpointUrl:
        typeof editor.endpointUrl === "string" ? editor.endpointUrl : undefined,
    },
    // [OPUS-4.8] sq-tp1m — backward-compatible: an older record without `inference` → "off".
    inference: parseInferenceMode(v.inference),
    // [sq-glo5r] — backward-compatible: an older record without `rulesDocs` → empty array.
    rulesDocs: parseRulesDocs(v.rulesDocs),
    // [FABLE-5] sq-ixc3.14 — backward-compatible: no `serviceAllowlist` → empty (fail-closed).
    serviceAllowlist: parseServiceAllowlist(v.serviceAllowlist),
    schema: WORKSPACE_SCHEMA,
  };
}

/**
 * [FABLE-5] sq-ixc3.14 — validate + normalise a persisted federation egress allowlist: keep
 * only non-empty trimmed strings, drop everything else (a hand-edited or corrupt record must
 * degrade toward FEWER reachable endpoints, never more — fail-closed).
 */
export function parseServiceAllowlist(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((e) => {
    if (typeof e !== "string") return [];
    const trimmed = e.trim();
    return trimmed === "" ? [] : [trimmed];
  });
}

function parseSource(value: unknown): WorkspaceSourceMeta | null {
  if (typeof value !== "object" || value === null) return null;
  const s = value as Record<string, unknown>;
  if (s.kind !== "local" && s.kind !== "url") return null;
  if (typeof s.label !== "string" || typeof s.format !== "string") return null;
  return {
    kind: s.kind,
    label: s.label,
    url: typeof s.url === "string" ? s.url : undefined,
    format: s.format,
    bytes: typeof s.bytes === "number" ? s.bytes : undefined,
    importedAt: typeof s.importedAt === "number" ? s.importedAt : Date.now(),
  };
}

/** [sq-glo5r] Validate + normalise one persisted {@link WorkspaceRulesDoc}, or `null` if malformed. */
function parseRulesDoc(value: unknown): WorkspaceRulesDoc | null {
  if (typeof value !== "object" || value === null) return null;
  const d = value as Record<string, unknown>;
  if (typeof d.name !== "string" || typeof d.text !== "string") return null;
  if (typeof d.addedAt !== "number") return null;
  return {
    name: d.name,
    text: d.text,
    addedAt: d.addedAt,
    // `enabled` is only persisted when false (absent = true by default).
    enabled: d.enabled === false ? false : undefined,
  };
}

/** [sq-glo5r] Parse an arbitrary value into a {@link WorkspaceRulesDoc} array (empty when absent / invalid). */
function parseRulesDocs(value: unknown): WorkspaceRulesDoc[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((d) => {
    const doc = parseRulesDoc(d);
    return doc ? [doc] : [];
  });
}

// ---------------------------------------------------------------------------
// The persistence abstraction.
// ---------------------------------------------------------------------------

/**
 * Which concrete backend a {@link WorkspaceStore} is using — surfaced so the UI can label the
 * persistence honestly ("saved to disk" on Tauri, "saved in this browser" on the web, "this
 * session only" for the in-memory fallback).
 */
export type WorkspaceBackend = "tauri" | "web" | "memory";

/**
 * The ONE persistence interface every backend implements. Async throughout so the Tauri
 * filesystem path (which is async) and the web `localStorage` path (sync, trivially wrapped)
 * share a signature. The set of stored workspaces plus a pointer to the LAST-OPENED one (for
 * startup re-hydration).
 */
export interface WorkspaceStore {
  /** Which concrete storage this resolved to. */
  readonly backend: WorkspaceBackend;
  /** Lightweight list of every stored workspace, for the switcher. */
  list(): Promise<WorkspaceSummary[]>;
  /** Load one workspace by id, or `null` if it is gone / unreadable. */
  load(id: string): Promise<Workspace | null>;
  /** Create or overwrite a workspace (keyed by `id`); bumps `updatedAt`. */
  save(ws: Workspace): Promise<void>;
  /** Delete a workspace by id (no-op if absent). */
  remove(id: string): Promise<void>;
  /** The id of the workspace to restore on startup, or `null` if none / it was deleted. */
  lastOpenedId(): Promise<string | null>;
  /** Record which workspace is open (so the next launch re-hydrates it). */
  setLastOpenedId(id: string | null): Promise<void>;
}

// --- localStorage key scheme (web backend) ---------------------------------

const WEB_PREFIX = "sparq.workspace.v1.";
const WEB_INDEX_KEY = "sparq.workspace.v1.__index__";
const WEB_LAST_KEY = "sparq.workspace.v1.__last__";

/**
 * Minimal synchronous key/value surface — exactly `localStorage`'s shape. Abstracted so the
 * web backend can be unit-tested against an in-memory polyfill with no `jsdom`.
 */
export interface KeyValueStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/**
 * The web persistence backend: each workspace is one `localStorage` entry (`sparq.workspace.v1.<id>`),
 * an `__index__` entry holds the id list, and `__last__` the last-opened id. Static-export-safe
 * (no Tauri, no Node). Used in any browser and inside a Tauri webview lacking the fs capability.
 */
export class WebWorkspaceStore implements WorkspaceStore {
  readonly backend: WorkspaceBackend;
  private readonly kv: KeyValueStorage;

  constructor(kv: KeyValueStorage, backend: WorkspaceBackend = "web") {
    this.kv = kv;
    this.backend = backend;
  }

  private ids(): string[] {
    const raw = this.kv.getItem(WEB_INDEX_KEY);
    if (!raw) return [];
    try {
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
    } catch {
      return [];
    }
  }

  private writeIds(ids: string[]): void {
    this.kv.setItem(WEB_INDEX_KEY, JSON.stringify(ids));
  }

  private readOne(id: string): Workspace | null {
    const raw = this.kv.getItem(WEB_PREFIX + id);
    if (!raw) return null;
    try {
      return parseWorkspace(JSON.parse(raw));
    } catch {
      return null;
    }
  }

  async list(): Promise<WorkspaceSummary[]> {
    return this.ids()
      .flatMap((id) => {
        const ws = this.readOne(id);
        return ws ? [workspaceSummary(ws)] : [];
      })
      .sort((a, b) => b.updatedAt - a.updatedAt);
  }

  async load(id: string): Promise<Workspace | null> {
    return this.readOne(id);
  }

  async save(ws: Workspace): Promise<void> {
    const record: Workspace = { ...ws, updatedAt: Date.now(), schema: WORKSPACE_SCHEMA };
    this.kv.setItem(WEB_PREFIX + ws.id, JSON.stringify(record));
    const ids = this.ids();
    if (!ids.includes(ws.id)) this.writeIds([...ids, ws.id]);
  }

  async remove(id: string): Promise<void> {
    this.kv.removeItem(WEB_PREFIX + id);
    this.writeIds(this.ids().filter((x) => x !== id));
    if (this.kv.getItem(WEB_LAST_KEY) === id) this.kv.removeItem(WEB_LAST_KEY);
  }

  async lastOpenedId(): Promise<string | null> {
    const last = this.kv.getItem(WEB_LAST_KEY);
    return last && this.ids().includes(last) ? last : null;
  }

  async setLastOpenedId(id: string | null): Promise<void> {
    if (id === null) this.kv.removeItem(WEB_LAST_KEY);
    else this.kv.setItem(WEB_LAST_KEY, id);
  }
}

/**
 * The in-memory fallback backend (`memory`): a `Map` that lives only for this session. Used
 * when no persistent storage exists at all (SSR / Node build, or a webview that exposes
 * neither `localStorage` nor the Tauri fs plugin). The model still works for one session; the
 * UI labels it as non-persistent so the user is not misled.
 */
export class MemoryWorkspaceStore implements WorkspaceStore {
  readonly backend: WorkspaceBackend = "memory";
  private readonly map = new Map<string, Workspace>();
  private last: string | null = null;

  async list(): Promise<WorkspaceSummary[]> {
    return [...this.map.values()]
      .map(workspaceSummary)
      .sort((a, b) => b.updatedAt - a.updatedAt);
  }
  async load(id: string): Promise<Workspace | null> {
    return this.map.get(id) ?? null;
  }
  async save(ws: Workspace): Promise<void> {
    this.map.set(ws.id, { ...ws, updatedAt: Date.now(), schema: WORKSPACE_SCHEMA });
  }
  async remove(id: string): Promise<void> {
    this.map.delete(id);
    if (this.last === id) this.last = null;
  }
  async lastOpenedId(): Promise<string | null> {
    return this.last && this.map.has(this.last) ? this.last : null;
  }
  async setLastOpenedId(id: string | null): Promise<void> {
    this.last = id;
  }
}

// ---------------------------------------------------------------------------
// The Tauri filesystem backend (feature-detected at runtime).
// ---------------------------------------------------------------------------

/**
 * The subset of the Tauri filesystem plugin (`@tauri-apps/plugin-fs`) this backend needs. We
 * type it locally (rather than importing the plugin as a hard dependency) so the site bundle
 * never depends on a Tauri package — the plugin is read off the runtime when present and the
 * web build neither imports nor ships it.
 */
export interface TauriFsApi {
  readonly baseDir: number;
  readTextFile(path: string, opts?: { baseDir?: number }): Promise<string>;
  writeTextFile(path: string, contents: string, opts?: { baseDir?: number }): Promise<void>;
  remove(path: string, opts?: { baseDir?: number }): Promise<void>;
  exists(path: string, opts?: { baseDir?: number }): Promise<boolean>;
  mkdir(path: string, opts?: { baseDir?: number; recursive?: boolean }): Promise<void>;
  readDir(path: string, opts?: { baseDir?: number }): Promise<{ name: string }[]>;
}

const TAURI_DIR = "workspaces";
const TAURI_INDEX = "workspaces/__index__.json";
const TAURI_LAST = "workspaces/__last__.json";

/**
 * The Tauri persistence backend: each workspace is a JSON file under
 * `<app-local-data>/workspaces/<id>.json`, with sibling `__index__.json` / `__last__.json`
 * pointers. Files are written through the injected {@link TauriFsApi} (the runtime
 * `@tauri-apps/plugin-fs` surface) scoped to the app's local data dir, so workspaces survive
 * an app restart. Selected by {@link createWorkspaceStore} ONLY when that plugin is actually
 * present + the shell granted the fs capability.
 */
export class TauriWorkspaceStore implements WorkspaceStore {
  readonly backend: WorkspaceBackend = "tauri";
  private readonly fs: TauriFsApi;
  private readonly opts: { baseDir: number };
  /** [SONNET-4.6] Shared tail for every `__index__.json` read-modify-write. */
  private indexMutationTail: Promise<void> = Promise.resolve();

  constructor(fs: TauriFsApi) {
    this.fs = fs;
    this.opts = { baseDir: fs.baseDir };
  }

  /** Ensure the workspaces dir exists (idempotent). */
  private async ensureDir(): Promise<void> {
    if (!(await this.fs.exists(TAURI_DIR, this.opts))) {
      await this.fs.mkdir(TAURI_DIR, { ...this.opts, recursive: true });
    }
  }

  private path(id: string): string {
    return `${TAURI_DIR}/${id}.json`;
  }

  private async readIds(): Promise<string[]> {
    try {
      const raw = await this.fs.readTextFile(TAURI_INDEX, this.opts);
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
    } catch {
      return [];
    }
  }

  private async writeIds(ids: string[]): Promise<void> {
    await this.ensureDir();
    await this.fs.writeTextFile(TAURI_INDEX, JSON.stringify(ids), this.opts);
  }

  /** Serialize index mutations across workspace ids without wedging after a failed write. */
  private mutateIds(update: (ids: string[]) => string[] | null): Promise<void> {
    const operation = this.indexMutationTail.then(async () => {
      const ids = await this.readIds();
      const next = update(ids);
      if (next) await this.writeIds(next);
    });
    this.indexMutationTail = operation.catch(() => {});
    return operation;
  }

  async list(): Promise<WorkspaceSummary[]> {
    const ids = await this.readIds();
    const summaries: WorkspaceSummary[] = [];
    for (const id of ids) {
      const ws = await this.load(id);
      if (ws) summaries.push(workspaceSummary(ws));
    }
    return summaries.sort((a, b) => b.updatedAt - a.updatedAt);
  }

  async load(id: string): Promise<Workspace | null> {
    try {
      const raw = await this.fs.readTextFile(this.path(id), this.opts);
      return parseWorkspace(JSON.parse(raw));
    } catch {
      return null;
    }
  }

  async save(ws: Workspace): Promise<void> {
    await this.ensureDir();
    const record: Workspace = { ...ws, updatedAt: Date.now(), schema: WORKSPACE_SCHEMA };
    await this.fs.writeTextFile(this.path(ws.id), JSON.stringify(record), this.opts);
    await this.mutateIds((ids) => (ids.includes(ws.id) ? null : [...ids, ws.id]));
  }

  async remove(id: string): Promise<void> {
    try {
      if (await this.fs.exists(this.path(id), this.opts)) {
        await this.fs.remove(this.path(id), this.opts);
      }
    } catch {
      // A missing file is fine — remove is best-effort idempotent.
    }
    await this.mutateIds((ids) => ids.filter((x) => x !== id));
    if ((await this.lastOpenedId()) === id) await this.setLastOpenedId(null);
  }

  async lastOpenedId(): Promise<string | null> {
    try {
      const raw = await this.fs.readTextFile(TAURI_LAST, this.opts);
      const id = JSON.parse(raw);
      if (typeof id !== "string") return null;
      return (await this.readIds()).includes(id) ? id : null;
    } catch {
      return null;
    }
  }

  async setLastOpenedId(id: string | null): Promise<void> {
    await this.ensureDir();
    await this.fs.writeTextFile(TAURI_LAST, JSON.stringify(id), this.opts);
  }
}

// ---------------------------------------------------------------------------
// Runtime backend selection (NEVER assume Tauri / localStorage exists).
// ---------------------------------------------------------------------------

/**
 * True when running inside a Tauri webview. Feature-detection only — reads the Tauri runtime
 * globals (`__TAURI_INTERNALS__` is set by Tauri 2; `__TAURI__` is the legacy/global-API
 * marker) WITHOUT importing any Tauri package, so a browser build that never ships Tauri code
 * simply sees `false`. Safe during SSR (guards `typeof window`).
 */
export function isTauriRuntime(): boolean {
  if (typeof window === "undefined") return false;
  const w = window as unknown as Record<string, unknown>;
  return "__TAURI_INTERNALS__" in w || "__TAURI__" in w;
}

/**
 * Resolve a usable {@link KeyValueStorage} (the browser `localStorage`) if present, else
 * `null`. Guards the access in a `try` because some embedded webviews / privacy modes throw on
 * `localStorage` access rather than simply omitting it.
 */
export function detectWebStorage(): KeyValueStorage | null {
  try {
    if (typeof localStorage === "undefined") return null;
    // A probe write/remove confirms it is actually usable (not a throwing stub).
    const probe = "sparq.workspace.v1.__probe__";
    localStorage.setItem(probe, "1");
    localStorage.removeItem(probe);
    return localStorage;
  } catch {
    return null;
  }
}

/**
 * How a host injects the Tauri filesystem plugin (when it has one) into the store factory. The
 * site does NOT depend on `@tauri-apps/plugin-fs`; a Tauri host can pass a loader that
 * dynamically imports it, and {@link createWorkspaceStore} only calls the loader after
 * {@link isTauriRuntime} confirms a Tauri webview. Returning `null` (e.g. the fs capability was
 * not granted) cleanly degrades to the web backend.
 */
export interface WorkspaceStoreOptions {
  /**
   * Resolve the Tauri filesystem API + its app-local-data baseDir, or `null` to skip the Tauri
   * backend. Only invoked inside a Tauri webview. The default is `null` (no Tauri fs), which is
   * correct for the browser/static-export build — that path uses the web backend.
   */
  loadTauriFs?: () => Promise<TauriFsApi | null>;
}

/**
 * Pick the best available persistence backend for the current runtime, in priority order:
 *   1. Tauri filesystem (durable on-disk) — only inside a Tauri webview AND only if the host's
 *      `loadTauriFs` resolves a usable fs API (the shell granted the `fs` capability);
 *   2. browser `localStorage` (durable per-browser) — the static-export / GitHub Pages path,
 *      and the Tauri-webview fallback when the fs capability is absent;
 *   3. in-memory (this session only) — when neither exists, so the GUI still runs.
 *
 * Never throws on a missing global: each tier is feature-detected and falls through.
 */
export async function createWorkspaceStore(
  options: WorkspaceStoreOptions = {},
): Promise<WorkspaceStore> {
  if (isTauriRuntime() && options.loadTauriFs) {
    try {
      const fs = await options.loadTauriFs();
      if (fs) return new TauriWorkspaceStore(fs);
    } catch {
      // Fall through to the web/memory tiers if the plugin import / capability fails.
    }
  }
  const kv = detectWebStorage();
  if (kv) return new WebWorkspaceStore(kv);
  return new MemoryWorkspaceStore();
}

// ---------------------------------------------------------------------------
// (sq-9i1h9) Serialized per-workspace saves — out-of-order overwrite protection.
// ---------------------------------------------------------------------------

/**
 * Serializes whole-workspace saves PER WORKSPACE so a slow OLDER save can never land after —
 * and overwrite — a NEWER one. Obtain one via {@link createSerializedWorkspaceSaver} and route
 * EVERY save for a given store through it.
 */
export interface SerializedWorkspaceSaver {
  /**
   * Enqueue a save of `ws`. Resolves when this state is durably written — or when it has been
   * SUPERSEDED by a newer save for the same workspace that follows it in the same chain (each
   * save writes the whole workspace, so the newer snapshot covers this one). Rejects only when
   * this revision's own write fails (e.g. the localStorage quota is exhausted).
   */
  save(ws: Workspace): Promise<void>;
}

/**
 * Build a {@link SerializedWorkspaceSaver} over an underlying whole-workspace `write` (normally
 * a bound {@link WorkspaceStore}.save).
 *
 * WHY (sq-9i1h9). The GUI persists by firing `store.save(ws)` from every mutator without
 * awaiting. On the async Tauri fs backend a save is a multi-await IPC sequence
 * (ensureDir → writeTextFile → readIds → writeIds), so two un-serialized saves can interleave
 * and an OLDER save can complete after a NEWER one — the on-disk file then holds the stale
 * snapshot. (The web/memory backends write synchronously, so the race is specific to the async
 * backend — which sq-w3dmj makes reachable.)
 *
 * HOW. Two cooperating mechanisms, per workspace id:
 *   1. a PROMISE CHAIN — at most one underlying write is in flight per workspace, in enqueue
 *      order, so writes can never interleave (a failed write does not wedge the chain);
 *   2. a MONOTONIC REVISION COUNTER — each enqueued save takes the next revision; when a
 *      queued save's turn arrives and a newer revision has since been requested, its write is
 *      DROPPED (the newer whole-workspace snapshot that follows in the chain covers it).
 */
export function createSerializedWorkspaceSaver(
  write: (ws: Workspace) => Promise<void>,
): SerializedWorkspaceSaver {
  /** Per-workspace tail of the save chain (the most recently enqueued step). */
  const tails = new Map<string, Promise<void>>();
  /** Per-workspace newest REQUESTED revision (assigned at enqueue time, monotonic). */
  const latest = new Map<string, number>();

  return {
    save(ws: Workspace): Promise<void> {
      const id = ws.id;
      const revision = (latest.get(id) ?? 0) + 1;
      latest.set(id, revision);
      // Write only if this is STILL the newest requested revision when its turn arrives — a
      // stale revision is dropped (see the revision-counter contract above).
      const runIfCurrent = (): Promise<void> | undefined =>
        latest.get(id) === revision ? write(ws) : undefined;
      const prev = tails.get(id) ?? Promise.resolve();
      // Chain after the previous save WHATEVER its outcome: an older save's failure must
      // neither wedge nor fail the newer saves queued behind it.
      const step = prev.then(runIfCurrent, runIfCurrent);
      tails.set(id, step);
      // Reset the per-id book-keeping once the chain fully drains (this step settled while
      // still the tail), so deleted workspaces do not accumulate entries forever. The handler
      // also keeps a fire-and-forget rejected tail from surfacing as an unhandled rejection;
      // the CALLER's copy of `step` still rejects normally.
      const gc = () => {
        if (tails.get(id) === step) {
          tails.delete(id);
          latest.delete(id);
        }
      };
      step.then(gc, gc);
      return step;
    },
  };
}

// ---------------------------------------------------------------------------
// (sq-w3dmj) Save-failure classification — surfacing, not swallowing.
// ---------------------------------------------------------------------------

/**
 * True when `err` is the browser's storage-quota-exhausted failure — the `QuotaExceededError`
 * `DOMException` thrown by `localStorage.setItem` when a large workspace snapshot no longer
 * fits (plus Firefox's legacy `NS_ERROR_DOM_QUOTA_REACHED` name). Checked structurally by
 * `name` (not `instanceof DOMException`) so it works across realms and in Node tests.
 */
export function isQuotaExceededError(err: unknown): boolean {
  if (typeof err !== "object" || err === null) return false;
  const name = (err as { name?: unknown }).name;
  return name === "QuotaExceededError" || name === "NS_ERROR_DOM_QUOTA_REACHED";
}

/**
 * A short, honest, user-facing description of a failed workspace save, for surfacing in the
 * GUI instead of swallowing the failure (sq-w3dmj): the quota case names the real limit (and
 * that the desktop on-disk backend does not share it); anything else carries the underlying
 * message when there is one.
 */
export function describeWorkspaceSaveError(err: unknown): string {
  if (isQuotaExceededError(err)) {
    return (
      "workspace save failed: browser storage quota exceeded — the snapshot is too large for " +
      "localStorage (the desktop app's on-disk backend has no such limit)"
    );
  }
  const message =
    typeof err === "object" &&
    err !== null &&
    typeof (err as { message?: unknown }).message === "string"
      ? (err as { message: string }).message
      : "";
  return message ? `workspace save failed: ${message}` : "workspace save failed";
}
