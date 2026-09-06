"use client";

// [OPUS-4.8] sq-ixc3.9 — the bottom STATUS BAR (research/gui-design.md §A.2):
// MEASURED performance.now() latency of the last run · row count · target · persistence backend.
//
// [OPUS-4.8] sq-vw3ax (#820 "GUI: unintrusive stats") — the bold redesign adds the issue's two
// asked-for stats, unintrusively (no modal): a slim INGEST meter (the real file label + a live
// MEASURED elapsed while an import is in flight) and a tiny DISK gauge (the REAL on-device store
// footprint).
//
// [OPUS-4.8] sq-cno90 (#820 follow-up) — the disk gauge now PREFERS the OS-reported on-disk byte
// total (a recursive native `stat()` of the `$APPLOCALDATA/workspaces` tree via the `disk_usage`
// command) when running in the desktop shell, and FALLS BACK to the snapshot-bytes estimate on the
// web target (no native FS). The two are clearly distinguished in the label + tooltip — it is never
// dressed up as OS-reported when it is the estimate.
//
// HONESTY. Every figure here is real:
//   * latency  — wall-clock of the query the user JUST ran (performance.now), labelled per-run,
//                NOT a benchmark claim (this work box / CI runner is non-canonical).
//   * disk     — on the desktop shell, the OS-reported byte total of the workspaces dir (the
//                precise on-disk size, incl. the workspace index JSON); on the web target, the
//                snapshot-bytes ESTIMATE (the byte length of the persisted whole-dataset N-Quads
//                snapshot). There is no fixed capacity, so the gauge shows the live footprint; the
//                conic ring's fill is a log-scaled visual cue only, and the tooltip states the
//                exact bytes AND which of the two sources it is. Never a fabricated number.
//   * ingest   — the loaders are synchronous (no byte-level progress callback), so this is an
//                honest indeterminate "ingesting <file>…" with a real elapsed — never a fake %/ETA.

import * as React from "react";

import { useEngine, type IngestState } from "@/lib/engine-context";
import { useWorkspace } from "@/lib/workspace-context";
import { formatBytes } from "@/lib/utils";

/**
 * [OPUS-4.8] sq-ixc3.13 — an honest persistence-backend label from the workspace store's
 * RESOLVED backend (not just a Tauri-presence guess): on-device disk (Tauri fs capability
 * granted), this browser (localStorage), or this session only (in-memory fallback).
 */
function backendLabel(backend: "tauri" | "web" | "memory" | null): string {
  if (backend === "tauri") return "saved on device";
  if (backend === "web") return "saved in this browser";
  if (backend === "memory") return "this session only";
  return "resolving…";
}

/**
 * A log-scaled 0–100 fill for the disk gauge's conic ring. There is no fixed capacity, so this is
 * a purely VISUAL cue (empty store → a sliver, ~1 GB → nearly full); the tooltip + label carry the
 * exact real bytes. Kept monotonic so a bigger store always reads as a fuller ring.
 */
function diskRingPct(bytes: number): number {
  if (bytes <= 0) return 0;
  // 1 KB → ~0%, 1 GB → ~100%, log-spaced across the six orders of magnitude in between.
  const pct = (Math.log10(bytes) - 3) * (100 / 6);
  return Math.max(2, Math.min(100, pct));
}

/**
 * The unintrusive #820 ingest meter: an indeterminate sweep + the source label + a LIVE measured
 * elapsed (re-rendered ~4×/s from `performance.now()` against the ingest start). No fabricated %.
 */
function IngestMeter({ ingest }: { ingest: IngestState }) {
  const [elapsedMs, setElapsedMs] = React.useState(0);
  React.useEffect(() => {
    const tick = () => setElapsedMs(performance.now() - ingest.startedAt);
    tick();
    const id = window.setInterval(tick, 250);
    return () => window.clearInterval(id);
  }, [ingest.startedAt]);
  return (
    <span
      className="flex items-center gap-2 text-muted-foreground"
      title={`Ingesting ${ingest.label} — ${(elapsedMs / 1000).toFixed(1)} s elapsed (measured)`}
      data-status-ingest
    >
      <span className="max-w-[10rem] truncate">↧ {ingest.label}</span>
      <span className="sq-ingest-bar" aria-hidden>
        <span className="sq-ingest-fill" />
      </span>
      <span className="tabular">{(elapsedMs / 1000).toFixed(1)} s</span>
    </span>
  );
}

export function StatusBar() {
  const {
    lastLatencyMs,
    lastRowCount,
    storeSize,
    storeBytes,
    diskBytes,
    ingest,
    nativeLoaderAvailable,
  } = useEngine();
  const { backend, saveError } = useWorkspace();

  // [OPUS-4.8] sq-cno90 — PREFER the OS-reported on-disk figure when the desktop probe returned one;
  // otherwise fall back to the snapshot-bytes estimate (the web target). Labelled honestly either
  // way: "disk" for the OS figure, "≈disk" for the estimate. Never fabricated — `diskBytes` is null
  // unless the native probe actually reported real bytes.
  const osReported = diskBytes !== null;
  const displayBytes = osReported ? diskBytes : storeBytes;
  return (
    <footer className="sq-statusbar flex h-7 shrink-0 items-center gap-4 border-t px-3 font-mono text-[11px] text-muted-foreground">
      <span
        className="tabular text-primary"
        title="Wall-clock latency of the last query (performance.now) — measured, not a benchmark"
      >
        {lastLatencyMs === null ? "— ms" : `${lastLatencyMs.toFixed(1)} ms`}
      </span>
      <span className="tabular" title="Rows / triples returned by the last run">
        {lastRowCount === null ? "— rows" : `${lastRowCount.toLocaleString()} rows`}
      </span>
      <span title="Where queries run">target: local · in-tab WASM</span>
      <span
        title={
          nativeLoaderAvailable
            ? "Imports decode through the native engine (compressed + native-only HDT)"
            : "Imports parse in the in-tab WASM engine (no compressed-file / HDT path)"
        }
      >
        loader: <span className="text-[var(--success)]">{nativeLoaderAvailable ? "native" : "in-tab"}</span>
      </span>
      <span title="Where the workspace persists">backend: {backendLabel(backend)}</span>

      {/* (sq-w3dmj) A failed workspace save is SURFACED, not swallowed — e.g. the localStorage
          quota exhausted by a large snapshot. Without this the app keeps its in-memory state but
          the on-launch restore silently reloads an older snapshot. */}
      {saveError !== null && (
        <span className="text-destructive" title={saveError} data-workspace-save-error>
          save failed — workspace not persisted
        </span>
      )}

      {/* push the #820 stats to the right edge. */}
      <span className="ml-auto" />

      <span className="tabular" title="Quads in the live store (default + every named graph)">
        {storeSize.toLocaleString()} quads
      </span>

      {/* [OPUS-4.8] sq-vw3ax (#820) — the unintrusive ingest meter (only while an import runs). */}
      {ingest && <IngestMeter ingest={ingest} />}

      {/* [OPUS-4.8] sq-vw3ax (#820) + sq-cno90 (#820 follow-up) — the disk gauge: the OS-reported
          on-disk footprint in the desktop shell, else the snapshot-bytes estimate (≈) on the web. */}
      <span
        className="flex items-center gap-1.5"
        title={
          osReported
            ? `Workspace store on disk: ${displayBytes.toLocaleString()} bytes — OS-reported size of the app-data workspaces/ dir`
            : `Workspace store ≈ ${displayBytes.toLocaleString()} bytes — estimate (persisted whole-dataset snapshot); the OS-reported size shows in the desktop app`
        }
        data-status-disk
        data-disk-source={osReported ? "os" : "estimate"}
      >
        <span
          className="sq-disk-ring"
          style={{ ["--disk-pct" as string]: diskRingPct(displayBytes) }}
          aria-hidden
        />
        {osReported ? "disk" : "≈disk"} {formatBytes(displayBytes)}
      </span>
    </footer>
  );
}
