"use client";

// [OPUS-4.8] sq-ixc3.9 — the thin TOP BAR (h-10, research/gui-design.md §A.2):
// LOCAL⇄ENDPOINT target switch · store size · ⌘K · theme toggle · engine status LED.
//
// [OPUS-4.8] sq-ixc3.10 — the ⌘K hint is now a LIVE trigger that opens the keyboard-first command
// palette (the spine). The target switch remains a stub (the endpoint/Server tool is a later
// phase) and stays honestly labelled.

import * as React from "react";
import { useTheme } from "next-themes";
import { Moon, Sun, Command, ExternalLink, Upload } from "lucide-react";

import { Button } from "@/components/ui/button";
import { subscribeRuntime as subscribeNoop, useEngine } from "@/lib/engine-context";
import { useCommandPalette } from "@/components/workbench/command-palette";
import { useImportDrawer } from "@/components/workbench/import-drawer";

/** The honesty-website link target (the marketing site, opened in a NEW tab / system browser). */
const WEBSITE_URL = "https://sparq.jeswr.org/";

function StatusLed() {
  const { status } = useEngine();
  const color =
    status.kind === "ready"
      ? "bg-[var(--success)]"
      : status.kind === "error"
        ? "bg-destructive"
        : "bg-[var(--warning)] animate-pulse";
  const label =
    status.kind === "ready"
      ? "Engine ready"
      : status.kind === "error"
        ? "Engine error"
        : "Warming…";
  return (
    <span className="flex items-center gap-1.5 text-xs text-muted-foreground" title={label}>
      <span className={`size-2 rounded-full ${color}`} aria-hidden />
      {/* The literal "Engine ready" copy is the deterministic hook the tauri-driver e2e waits on. */}
      <span>{label}</span>
    </span>
  );
}

function ThemeToggle() {
  const { resolvedTheme, setTheme } = useTheme();
  const mounted = React.useSyncExternalStore(subscribeNoop, () => true, () => false);
  if (!mounted) {
    return <Button variant="ghost" size="icon-sm" aria-label="Toggle theme" />;
  }
  const isDark = resolvedTheme === "dark";
  return (
    <Button
      variant="ghost"
      size="icon-sm"
      aria-label="Toggle theme"
      onClick={() => setTheme(isDark ? "light" : "dark")}
    >
      {isDark ? <Sun /> : <Moon />}
    </Button>
  );
}

export function TopBar() {
  const { storeSize } = useEngine();
  const { setOpen } = useCommandPalette();
  // [OPUS-4.8] sq-ixc3.13 — the top-bar "+ Import" affordance (opens the Import drawer).
  const { setOpen: setImportOpen } = useImportDrawer();
  return (
    // [OPUS-4.8] sq-vw3ax (#820 redesign) — the teal-forward TOP BAR (the hero gradient makes the
    // brand the lead, not a faint sprinkle). Same controls, bolder identity.
    <header className="sq-topbar flex h-11 shrink-0 items-center gap-3 border-b px-3">
      <span className="font-mono text-[15px] font-bold tracking-tight">
        sparq<span className="sq-brand-dot">.</span>
      </span>
      <span className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">
        workbench
      </span>

      {/* LOCAL⇄ENDPOINT target switch — stub (the Server/endpoint tool is a later phase). */}
      <div className="ml-2 flex items-center overflow-hidden rounded-md border bg-background text-[11.5px] font-semibold">
        <button
          className="bg-primary px-2.5 py-1 text-primary-foreground shadow-[0_0_18px_-4px_var(--teal-glow)]"
          title="Run queries against the in-tab engine (this build)"
        >
          LOCAL
        </button>
        <button
          className="cursor-not-allowed px-2.5 py-1 text-muted-foreground/70"
          title="Endpoint mode (connect to a running sparq-server) — coming in a later phase"
          disabled
        >
          ENDPOINT
        </button>
      </div>

      <span className="tabular font-mono text-xs text-muted-foreground">
        {storeSize.toLocaleString()} quads
      </span>

      {/* [OPUS-4.8] sq-ixc3.13 — "+ Import": real disk/URL/paste ingest via the native loader.
          [OPUS-4.8] sq-vw3ax — promoted to the lit primary action in the bold chrome. */}
      <Button
        size="sm"
        className="sq-glow-btn ml-1 border-transparent"
        onClick={() => setImportOpen(true)}
        title="Import RDF from a disk file (compressed / HDT), a URL, or pasted text"
        data-import-trigger="topbar"
      >
        <Upload className="size-3.5" /> Import
      </Button>

      <div className="ml-auto flex items-center gap-1.5">
        {/* ⌘K — opens the keyboard-first command palette (the spine). */}
        <button
          type="button"
          onClick={() => setOpen(true)}
          aria-label="Open command palette (Command-K)"
          className="hidden items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground sm:flex"
          title="Command palette (⌘K / Ctrl-K)"
        >
          <Command className="size-3" />K
        </button>
        <StatusLed />
        <ThemeToggle />
        <Button
          variant="ghost"
          size="sm"
          asChild
          title="Open the sparq website (the marketing/docs site) in your browser"
        >
          <a href={WEBSITE_URL} target="_blank" rel="noreferrer noopener">
            Help <ExternalLink className="size-3" />
          </a>
        </Button>
      </div>
    </header>
  );
}
