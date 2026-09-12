"use client";

// [OPUS-4.8] sq-ixc3.13 (epic sq-ixc3) — the active-WORKSPACE context for the operational GUI.
//
// The persistent-workspace MODEL + persistence abstraction is the framework-agnostic
// `@sparq/client` `workspace.ts` (sq-atb0): one `WorkspaceStore` interface with three runtime-
// selected backends (Tauri on-disk when the fs capability is granted, browser localStorage on
// the web target, in-memory fallback). This context is the GUI's host glue around it: it holds
// the active workspace + its imported-source list (drives the left rail's Imports subgroup),
// persists a workspace SNAPSHOT (the save/open cache) after each import AND after each
// successful SPARQL UPDATE (sq-7gdfp — INSERT/DELETE-applied data is covered too, plus a
// best-effort beforeunload flush), and — the load-bearing fix (sq-lcd6e) — RESTORES that
// snapshot into the live engine on launch so a user's imported + updated data and editor state
// survive a reload instead of being silently replaced by the sample graph.
//
// [OPUS-4.8] sq-lcd6e — this context is the workspace⇄engine HYDRATION bridge. Because
// <WorkspaceProvider> is rendered UNDER <EngineProvider> (see app/layout.tsx), it can call
// useEngine() and push the restored snapshot into the engine (hydrateFromSnapshot) at each
// lifecycle point: initial restore, create, switch, delete-of-active. It also exposes the
// lifecycle APIs (list / create / rename / delete / switch) the switcher UI (sq-lmlch) consumes;
// this bead is the PLUMBING — the rail switcher UI is a separate, dependent bead. NO secret is
// ever persisted.

import * as React from "react";
import {
  createSerializedWorkspaceSaver,
  createWorkspaceStore,
  describeWorkspaceSaveError,
  newWorkspace,
  parseServiceAllowlist,
  type SerializedWorkspaceSaver,
  type Workspace,
  type WorkspaceBackend,
  type WorkspaceInferenceMode,
  type WorkspaceRulesDoc,
  type WorkspaceSourceMeta,
  type WorkspaceStore,
  type WorkspaceSummary,
} from "@sparq/client";

import { useEngine } from "@/lib/engine-context";
import { loadTauriFs } from "@/lib/tauri-fs";
import { upsertSource } from "@/lib/workspace-sources";
import { DEFAULT_QUERY } from "@/data/sample-graph";

// [OPUS-4.8] sq-lcd6e — the starting editor query a freshly-created workspace carries. It is the
// curated Query-tool default (the demonstrative foaf query over the sample graph) so a fresh
// workspace opens with the SAME editor content the Query tool shows before restore — the editor
// now round-trips through the workspace (sq-lcd6e), so this is the single source of that default.
const STARTER_QUERY = DEFAULT_QUERY;

export interface WorkspaceContextValue {
  /** The active workspace, or `null` until the store + workspace are restored on mount. */
  workspace: Workspace | null;
  /** Which concrete persistence backend resolved (for an honest "saved on device / in browser" label). */
  backend: WorkspaceBackend | null;
  /**
   * (sq-w3dmj) Human-readable description of the most recent FAILED workspace save — e.g. the
   * localStorage quota exhausted by a large snapshot — or `null` while saves succeed. Surfaced
   * in the status bar so a persistence failure is never silent (the old persist() swallowed it,
   * which meant silent data loss on the next restore). Cleared by the next successful save.
   */
  saveError: string | null;
  /** The imported-source metadata list for the active workspace (drives the rail's Imports subgroup). */
  sources: WorkspaceSourceMeta[];
  /**
   * [OPUS-4.8] sq-lcd6e — lightweight summaries of EVERY stored workspace, for the switcher
   * (sq-lmlch). Refreshed after any create / rename / delete / switch / import.
   */
  workspaces: WorkspaceSummary[];
  /**
   * Record an imported source + persist a fresh dataset SNAPSHOT of the live store. Called by the
   * Import drawer on a successful ingest, and by the rail's re-fetch of a recorded `url` source.
   * `snapshot` is the live store's whole-dataset N-Quads (the engine context's `snapshotStore()`),
   * the save/open cache. Best-effort persistence: a write failure does not throw (the import
   * itself already succeeded in-memory).
   *
   * The source list is UPSERTED by {@link upsertSource}: a `url` source re-fetched from the rail
   * updates its existing entry in place rather than appending a duplicate.
   */
  recordImport: (source: WorkspaceSourceMeta, snapshot: string | null) => Promise<void>;
  /**
   * [OPUS-4.8] sq-lcd6e — persist the SPARQL editor text for the active workspace (best-effort,
   * debounced by the Query tool) so the editor round-trips across a reload. Only the query text
   * is updated; the other editor fields are preserved.
   */
  setEditorQuery: (query: string) => Promise<void>;
  /**
   * [OPUS-4.8] sq-lcd6e — create a new, empty workspace and make it active. It seeds NO sample
   * data (an explicitly-created workspace stays empty); the engine store is hydrated empty.
   * Returns the new workspace.
   */
  createWorkspace: (name: string) => Promise<Workspace>;
  /** [OPUS-4.8] sq-lcd6e — rename a workspace by id (persisted; updates the active one in place). */
  renameWorkspace: (id: string, name: string) => Promise<void>;
  /**
   * [OPUS-4.8] sq-lcd6e — delete a workspace by id. If it was the ACTIVE one, switch to the most
   * recently-updated remaining workspace, or create a fresh default if none remain.
   */
  deleteWorkspace: (id: string) => Promise<void>;
  /**
   * [OPUS-4.8] sq-lcd6e — switch to another workspace: SAVE the current one's live snapshot first
   * (no data loss), then load the target, hydrate the engine from its snapshot, and re-apply its
   * inference regime. A no-op when the target is already active or missing.
   */
  switchWorkspace: (id: string) => Promise<void>;
  /**
   * [OPUS-4.8] sq-tp1m (#757) — the active workspace's persisted INFERENCE regime (query-time
   * RDFS / OWL 2 RL entailment), defaulting to `"off"`. The inference-mode bridge pushes this
   * into the engine so the two stay in lockstep.
   */
  inference: WorkspaceInferenceMode;
  /**
   * [OPUS-4.8] sq-tp1m — persist a new inference regime for the active workspace (best-effort,
   * mirroring {@link recordImport}: a write failure never breaks the in-memory selection).
   */
  setInference: (mode: WorkspaceInferenceMode) => Promise<void>;
  /**
   * [FABLE-5] sq-ixc3.14 — the active workspace's persisted FEDERATION egress allowlist: the
   * only SPARQL endpoints a SERVICE clause may dial when the desktop app evaluates the query
   * natively. FAIL-CLOSED: empty means every SERVICE endpoint is refused. The federation
   * bridge pushes this into the engine so the two stay in lockstep.
   */
  serviceAllowlist: string[];
  /**
   * [FABLE-5] sq-ixc3.14 — persist a new federation egress allowlist for the active workspace
   * (best-effort, mirroring {@link setInference}). Entries are trimmed; empties dropped.
   */
  setServiceAllowlist: (hosts: string[]) => Promise<void>;
  /**
   * [sq-glo5r] The active workspace's persisted N3 rule documents (empty when none). Drives the
   * Inference tool's N3 rules panel and the engine's N3 closure cache.
   */
  rulesDocs: WorkspaceRulesDoc[];
  /**
   * [sq-glo5r] Bulk-add N3 rule documents to the active workspace (each gets a unique `addedAt`
   * timestamp; enabled by default). Persists + pushes to the engine immediately.
   */
  addRulesDocs: (docs: Array<{ name: string; text: string }>) => Promise<void>;
  /**
   * [sq-glo5r] Remove an N3 rule document by its `addedAt` timestamp (the stable unique key).
   * Persists + pushes the updated list to the engine.
   */
  removeRulesDoc: (addedAt: number) => Promise<void>;
  /**
   * [sq-glo5r] Toggle the `enabled` state of an N3 rule document by `addedAt`. A disabled rule
   * is excluded from the N3 closure (its cache-key hash changes, triggering a rebuild).
   */
  setRulesDocEnabled: (addedAt: number, enabled: boolean) => Promise<void>;
  /**
   * (sq-7gdfp) Persist a snapshot of the live store into the active workspace after a
   * successful SPARQL UPDATE (INSERT/DELETE). Best-effort: a write failure never breaks the
   * in-memory state. Call this whenever engine.run() returns outcome.kind === "update".
   */
  recordUpdateSnapshot: () => Promise<void>;
}

const WorkspaceContext = React.createContext<WorkspaceContextValue | null>(null);

export function WorkspaceProvider({ children }: { children: React.ReactNode }) {
  const [workspace, setWorkspaceState] = React.useState<Workspace | null>(null);
  const [backend, setBackend] = React.useState<WorkspaceBackend | null>(null);
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [workspaces, setWorkspaces] = React.useState<WorkspaceSummary[]>([]);
  const storeRef = React.useRef<WorkspaceStore | null>(null);
  // (sq-9i1h9) EVERY save goes through this serializer (a per-workspace promise chain + a
  // monotonic revision counter) so a slow older save can never land after — and overwrite — a
  // newer one on the async Tauri fs backend (which sq-w3dmj makes reachable).
  const saverRef = React.useRef<SerializedWorkspaceSaver | null>(null);
  // A synchronous mirror of the active workspace so async mutators (switch/delete) can read it
  // without a stale-closure functional-setState dance.
  const workspaceRef = React.useRef<Workspace | null>(null);

  // [OPUS-4.8] sq-lcd6e — the engine hydration + snapshot surface. Held in a ref so the mount
  // restore effect (deps []) and the lifecycle callbacks reference the latest engine API without
  // re-running / re-creating. These are stable useCallbacks, but the ref keeps lint honest.
  const engine = useEngine();
  const engineRef = React.useRef(engine);
  engineRef.current = engine;

  /** Commit a workspace to both the render state and the synchronous ref. */
  const applyWorkspace = React.useCallback((ws: Workspace | null) => {
    workspaceRef.current = ws;
    setWorkspaceState(ws);
  }, []);

  /** Re-read the switcher list from the backend (best-effort; a failure leaves the last list). */
  const refreshList = React.useCallback(async () => {
    const store = storeRef.current;
    if (!store) return;
    try {
      setWorkspaces(await store.list());
    } catch {
      /* an unreadable index must not break the UI — keep the last list */
    }
  }, []);

  // Resolve the persistence backend + restore (or create) the active workspace once on mount,
  // then HYDRATE the engine store from the restored workspace's snapshot (sq-lcd6e).
  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      const store = await createWorkspaceStore({ loadTauriFs });
      if (cancelled) return;
      const saver = createSerializedWorkspaceSaver((w) => store.save(w));
      storeRef.current = store;
      saverRef.current = saver;
      setBackend(store.backend);
      // Restore the last-opened workspace if there is one, else create a fresh default.
      let ws: Workspace | null = null;
      let freshDefault = false;
      try {
        const lastId = await store.lastOpenedId();
        if (lastId) ws = await store.load(lastId);
      } catch {
        /* unreadable index — fall through to a fresh workspace */
      }
      if (!ws) {
        ws = newWorkspace("default workspace", STARTER_QUERY);
        freshDefault = true;
        try {
          await saver.save(ws);
          await store.setLastOpenedId(ws.id);
        } catch {
          /* in-memory / locked-down backend — keep the workspace for this session anyway */
        }
      }
      if (cancelled) return;
      applyWorkspace(ws);
      // The load-bearing fix: seed the engine from the RESTORED snapshot — the sample graph is
      // seeded ONLY for a genuinely-fresh default workspace, never over a user's restored data.
      engineRef.current.hydrateFromSnapshot(ws.dataSnapshot ?? null, {
        seedSampleWhenEmpty: freshDefault,
      });
      void refreshList();
    })();
    return () => {
      cancelled = true;
    };
  }, [applyWorkspace, refreshList]);

  // Persist a workspace + keep it as the last-opened, then refresh the switcher list.
  //
  // (sq-9i1h9) The save is SERIALIZED per workspace through `saverRef` (promise chain +
  // monotonic revision counter), so concurrent persists can never land out of order on the
  // async Tauri fs backend. (sq-w3dmj) A write failure no longer vanishes into a silent
  // `.catch(() => {})`: it is surfaced via `saveError` (the status bar renders it) — the
  // in-memory workspace stays intact either way. Returns the save promise so durability-
  // sensitive callers can await it; it NEVER rejects and resolves `true` iff this state (or a
  // newer one superseding it in the same chain) reached the backend.
  const persist = React.useCallback(
    (ws: Workspace): Promise<boolean> => {
      const store = storeRef.current;
      const saver = saverRef.current;
      if (!store || !saver) return Promise.resolve(false);
      return saver
        .save(ws)
        .then(() => store.setLastOpenedId(ws.id))
        .then(() => refreshList())
        .then(() => {
          setSaveError((prev) => (prev === null ? prev : null));
          return true;
        })
        .catch((err: unknown) => {
          // A write failure must not break the in-memory workspace — but it must be VISIBLE
          // (a swallowed QuotaExceededError meant the on-launch restore silently reloaded an
          // older, smaller snapshot).
          console.error("workspace save failed", err);
          setSaveError(describeWorkspaceSaveError(err));
          return false;
        });
    },
    [refreshList],
  );

  const recordImport = React.useCallback(
    async (source: WorkspaceSourceMeta, snapshot: string | null): Promise<void> => {
      const base = workspaceRef.current ?? newWorkspace("default workspace", STARTER_QUERY);
      const next: Workspace = {
        ...base,
        // [OPUS-5] sq-ixc3.13 — UPSERT, not append: re-fetching a recorded `url` source updates
        // that entry in place (fresher importedAt / bytes / format) instead of growing a duplicate
        // rail row. Local imports always append (they have no re-fetch identity).
        sources: upsertSource(base.sources, source),
        dataSnapshot: snapshot ?? base.dataSnapshot,
        updatedAt: Date.now(),
      };
      applyWorkspace(next);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  const setEditorQuery = React.useCallback(
    async (query: string): Promise<void> => {
      const base = workspaceRef.current;
      if (!base || base.editor.query === query) return;
      const next: Workspace = {
        ...base,
        editor: { ...base.editor, query },
        updatedAt: Date.now(),
      };
      applyWorkspace(next);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  // [OPUS-4.8] sq-tp1m (#757) — persist a new inference regime for the active workspace. Same
  // best-effort discipline as recordImport: update the in-memory workspace immediately and write
  // through to the backend without letting a persistence failure surface to the caller.
  const setInference = React.useCallback(
    async (mode: WorkspaceInferenceMode): Promise<void> => {
      const base = workspaceRef.current ?? newWorkspace("default workspace", STARTER_QUERY);
      if (base.inference === mode) return;
      const next: Workspace = { ...base, inference: mode, updatedAt: Date.now() };
      applyWorkspace(next);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  // [FABLE-5] sq-ixc3.14 — persist a new federation egress allowlist for the active workspace.
  // Same best-effort discipline as setInference; normalised through the fail-closed parser
  // (trim, drop empties) so the persisted record and the engine always see the same list.
  // Pushed to the engine immediately (like the N3 rules), not just via the bridge.
  const setServiceAllowlist = React.useCallback(
    async (hosts: string[]): Promise<void> => {
      const base = workspaceRef.current ?? newWorkspace("default workspace", STARTER_QUERY);
      const cleaned = parseServiceAllowlist(hosts);
      const next: Workspace = { ...base, serviceAllowlist: cleaned, updatedAt: Date.now() };
      applyWorkspace(next);
      engineRef.current.setServiceAllowlist(cleaned);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  // [sq-glo5r] — bulk-add N3 rule documents; each gets a unique addedAt (now + index offset so
  // rapid bulk adds never collide). Pushes the updated list to the engine immediately.
  const addRulesDocs = React.useCallback(
    async (docs: Array<{ name: string; text: string }>): Promise<void> => {
      const base = workspaceRef.current ?? newWorkspace("default workspace", STARTER_QUERY);
      const now = Date.now();
      const newDocs: WorkspaceRulesDoc[] = docs.map((d, i) => ({
        name: d.name,
        text: d.text,
        addedAt: now + i, // +i keeps addedAt unique within a single bulk add
      }));
      const nextDocs = [...(base.rulesDocs ?? []), ...newDocs];
      const next: Workspace = { ...base, rulesDocs: nextDocs, updatedAt: Date.now() };
      applyWorkspace(next);
      engineRef.current.setN3Rules(nextDocs);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  // [sq-glo5r] — remove an N3 rule document by addedAt (the stable unique key).
  const removeRulesDoc = React.useCallback(
    async (addedAt: number): Promise<void> => {
      const base = workspaceRef.current ?? newWorkspace("default workspace", STARTER_QUERY);
      const nextDocs = (base.rulesDocs ?? []).filter((d) => d.addedAt !== addedAt);
      const next: Workspace = { ...base, rulesDocs: nextDocs, updatedAt: Date.now() };
      applyWorkspace(next);
      engineRef.current.setN3Rules(nextDocs);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  // [sq-glo5r] — toggle the enabled state of an N3 rule document by addedAt.
  const setRulesDocEnabled = React.useCallback(
    async (addedAt: number, enabled: boolean): Promise<void> => {
      const base = workspaceRef.current ?? newWorkspace("default workspace", STARTER_QUERY);
      const nextDocs = (base.rulesDocs ?? []).map((d) =>
        d.addedAt === addedAt ? { ...d, enabled } : d,
      );
      const next: Workspace = { ...base, rulesDocs: nextDocs, updatedAt: Date.now() };
      applyWorkspace(next);
      engineRef.current.setN3Rules(nextDocs);
      await persist(next);
    },
    [applyWorkspace, persist],
  );

  // (sq-7gdfp) — snapshot the live store into the active workspace after a successful SPARQL UPDATE.
  // Best-effort: a write failure (persistence layer down) never breaks the in-memory state.
  const recordUpdateSnapshot = React.useCallback(async (): Promise<void> => {
    const base = workspaceRef.current;
    if (!base) return;
    const snapshot = engineRef.current.snapshotStore();
    if (snapshot === null) return; // engine not ready — skip; a failed UPDATE never reaches here anyway
    const next: Workspace = { ...base, dataSnapshot: snapshot, updatedAt: Date.now() };
    applyWorkspace(next);
    await persist(next);
  }, [applyWorkspace, persist]);

  // (sq-7gdfp) — belt-and-suspenders save: on page unload, snapshot the live engine store into
  // the active workspace one final time. This catches any in-flight state that was not yet
  // persisted (e.g. if the debounce had not fired yet). For the localStorage backend the write
  // lands in a microtask (the serializer chains it), which still flushes before the page is
  // torn down; for the Tauri FS backend it is async and may not complete, but the explicit
  // recordUpdateSnapshot after each UPDATE is the primary mechanism — this is a safety net only.
  React.useEffect(() => {
    const handleBeforeUnload = () => {
      const current = workspaceRef.current;
      const saver = saverRef.current;
      if (!current || !saver) return;
      const snapshot = engineRef.current.snapshotStore();
      if (snapshot === null) return;
      const next: Workspace = { ...current, dataSnapshot: snapshot, updatedAt: Date.now() };
      // (sq-9i1h9) through the serializer, so the final flush is ordered AFTER any in-flight
      // save (an older in-flight save must not overwrite it). On the web backend the write
      // still lands in a microtask before the page is torn down; nothing to surface here —
      // the page is going away.
      void saver.save(next).catch(() => {});
    };
    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => window.removeEventListener("beforeunload", handleBeforeUnload);
  }, []); // storeRef, workspaceRef, engineRef are all refs — stable, no deps needed

  const createWorkspace = React.useCallback(
    async (name: string): Promise<Workspace> => {
      // 1. SAVE the current workspace's live snapshot first — no data loss on leaving it.
      //    Mirrors switchWorkspace step 1: any SPARQL UPDATE (engine-context updateInPlace)
      //    applied since the last import/switch snapshot is otherwise silently discarded on create.
      const store = storeRef.current;
      const current = workspaceRef.current;
      if (store && current) {
        const snapshot = engineRef.current.snapshotStore();
        const saved: Workspace =
          snapshot !== null
            ? { ...current, dataSnapshot: snapshot, updatedAt: Date.now() }
            : current;
        try {
          // (sq-9i1h9) through the serializer, so this final flush of the OLD workspace can
          // never be overwritten by one of its still-in-flight earlier saves.
          await saverRef.current?.save(saved);
        } catch {
          /* best-effort — a save failure must not block the create */
        }
      }
      // 2. Create the new workspace and activate it.
      const ws = newWorkspace(name, STARTER_QUERY);
      applyWorkspace(ws);
      // A brand-new workspace is EMPTY (no sample) — an explicitly-created workspace stays empty.
      engineRef.current.hydrateFromSnapshot(null, { seedSampleWhenEmpty: false });
      await persist(ws);
      return ws;
    },
    [applyWorkspace, persist],
  );

  const renameWorkspace = React.useCallback(
    async (id: string, name: string): Promise<void> => {
      const store = storeRef.current;
      const active = workspaceRef.current;
      if (active && active.id === id) {
        const next: Workspace = { ...active, name, updatedAt: Date.now() };
        applyWorkspace(next);
        await persist(next);
        return;
      }
      if (!store) return;
      try {
        const ws = await store.load(id);
        if (!ws) return;
        // (sq-9i1h9) through the serializer: same per-workspace chain as every other save.
        await saverRef.current?.save({ ...ws, name, updatedAt: Date.now() });
        await refreshList();
      } catch {
        /* a rename of a stored (non-active) workspace is best-effort */
      }
    },
    [applyWorkspace, persist, refreshList],
  );

  // Load `id` and make it active: hydrate the engine from its snapshot + re-apply its inference
  // regime and N3 rules (belt-and-suspenders alongside the bridges — ensures lockstep even before
  // the bridge effects fire).
  const activate = React.useCallback((ws: Workspace) => {
    applyWorkspace(ws);
    engineRef.current.hydrateFromSnapshot(ws.dataSnapshot ?? null, {
      seedSampleWhenEmpty: false,
    });
    // Re-apply the target's inference regime.
    engineRef.current.setInferenceMode(ws.inference ?? "off");
    // [sq-glo5r] — push the target workspace's N3 rules so the engine rebuilds the N3 closure.
    engineRef.current.setN3Rules(ws.rulesDocs ?? []);
    // [FABLE-5] sq-ixc3.14 — push the target workspace's federation egress allowlist so a
    // SERVICE run never dials under a PREVIOUS workspace's policy (fail-closed lockstep).
    engineRef.current.setServiceAllowlist(ws.serviceAllowlist ?? []);
  }, [applyWorkspace]);

  const switchWorkspace = React.useCallback(
    async (id: string): Promise<void> => {
      const store = storeRef.current;
      if (!store) return;
      const current = workspaceRef.current;
      if (current && current.id === id) return;
      // 1. SAVE the current workspace's live snapshot first — no data loss on leaving it.
      if (current) {
        const snapshot = engineRef.current.snapshotStore();
        const saved: Workspace =
          snapshot !== null
            ? { ...current, dataSnapshot: snapshot, updatedAt: Date.now() }
            : current;
        try {
          // (sq-9i1h9) through the serializer — see createWorkspace step 1.
          await saverRef.current?.save(saved);
        } catch {
          /* best-effort — a save failure must not block the switch */
        }
      }
      // 2. Load the target + activate it (hydrate engine + inference).
      let target: Workspace | null = null;
      try {
        target = await store.load(id);
      } catch {
        target = null;
      }
      if (!target) return;
      try {
        await store.setLastOpenedId(target.id);
      } catch {
        /* best-effort */
      }
      activate(target);
      void refreshList();
    },
    [activate, refreshList],
  );

  const deleteWorkspace = React.useCallback(
    async (id: string): Promise<void> => {
      const store = storeRef.current;
      if (!store) return;
      try {
        await store.remove(id);
      } catch {
        /* best-effort idempotent remove */
      }
      const active = workspaceRef.current;
      if (active && active.id === id) {
        // The active workspace was deleted: switch to the most recent remaining, else fresh.
        let summaries: WorkspaceSummary[] = [];
        try {
          summaries = await store.list();
        } catch {
          summaries = [];
        }
        const nextId = summaries.find((s) => s.id !== id)?.id ?? null;
        let target: Workspace | null = null;
        if (nextId) {
          try {
            target = await store.load(nextId);
          } catch {
            target = null;
          }
        }
        if (target) {
          try {
            await store.setLastOpenedId(target.id);
          } catch {
            /* best-effort */
          }
          activate(target);
        } else {
          const fresh = newWorkspace("default workspace", STARTER_QUERY);
          applyWorkspace(fresh);
          // A fresh default AFTER an explicit delete seeds the sample so the app is never blank.
          engineRef.current.hydrateFromSnapshot(null, { seedSampleWhenEmpty: true });
          await persist(fresh);
        }
      }
      void refreshList();
    },
    [activate, applyWorkspace, persist, refreshList],
  );

  const value = React.useMemo<WorkspaceContextValue>(
    () => ({
      workspace,
      backend,
      saveError,
      sources: workspace?.sources ?? [],
      workspaces,
      recordImport,
      setEditorQuery,
      createWorkspace,
      renameWorkspace,
      deleteWorkspace,
      switchWorkspace,
      inference: workspace?.inference ?? "off",
      setInference,
      serviceAllowlist: workspace?.serviceAllowlist ?? [],
      setServiceAllowlist,
      rulesDocs: workspace?.rulesDocs ?? [],
      addRulesDocs,
      removeRulesDoc,
      setRulesDocEnabled,
      recordUpdateSnapshot,
    }),
    [
      workspace,
      backend,
      saveError,
      workspaces,
      recordImport,
      setEditorQuery,
      createWorkspace,
      renameWorkspace,
      deleteWorkspace,
      switchWorkspace,
      setInference,
      setServiceAllowlist,
      addRulesDocs,
      removeRulesDoc,
      setRulesDocEnabled,
      recordUpdateSnapshot,
    ],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}

export function useWorkspace(): WorkspaceContextValue {
  const ctx = React.useContext(WorkspaceContext);
  if (!ctx) throw new Error("useWorkspace must be used within a <WorkspaceProvider>");
  return ctx;
}
