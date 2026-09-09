"use client";

// [OPUS-4.8] sq-vw3ax.1 — the Cmd-K command palette.
//
// WHY IT IS LOAD-BEARING (research/website-redesign.md §2, §7). The redesign collapses a
// tripled navigation (full-tree sidebar + top-tab bar + 14-card landing grid) into one slim
// top bar. That is only safe once there is a fast path to every surface that loses a visible
// nav entry — a fuzzy command palette that jumps to any surface by name in 0 clicks. So this
// ships FIRST (this bead), BEFORE the sidebar is removed (sq-vw3ax.7).
//
// SINGLE SOURCE. Everything it indexes derives from the one canonical IA source
// `@/data/surfaces` — the 5 capability THEMES (`GROUPS`) and the flagship showcases
// (`FLAGSHIPS`) — plus the fixed top-level pages and a small set of actions. Re-grouping in
// surfaces.ts restructures this palette in one edit, exactly like the Home theme grid.
//
// POST-COLLAPSE (sq-vw3ax.7). With the sidebar removed, this palette is the 0-click fast path
// to every surface, so it routes to each surface's POST-redesign home (surfaceHref): the 5
// retained deep pages keep /surface/<slug>, every other surface jumps to /capabilities#<theme>.
//
// MOUNTED ONCE. It is a global overlay mounted once in the
// AppShell so the ⌘K / Ctrl-K binding is global.

import * as React from "react";
import { useRouter } from "next/navigation";
import { Command } from "cmdk";
import { Dialog as DialogPrimitive } from "radix-ui";
import {
  Search,
  Github,
  Sun,
  Moon,
  CornerDownLeft,
  type LucideIcon,
} from "lucide-react";
import { useTheme } from "next-themes";

import { cn } from "@/lib/utils";
import { withBasePath } from "@/lib/base-path";
import { Badge } from "@/components/ui/badge";
import {
  FLAGSHIPS,
  GROUPS,
  TIER_LABEL,
  TIER_VARIANT,
  type Surface,
} from "@/data/surfaces";
import {
  DEEP_PAGE_SLUGS,
  capabilityAnchor,
} from "@/data/capabilities";
// [OPUS-4.8] sq-ixc3.10 — the live OPERATIONAL command layer the workbench (the REPL)
// contributes: run / EXPLAIN / connect / export / import / switch-workspace / named-graph /
// recent-query. The website palette stays navigational; this is the GUI design's keyboard-first
// SPINE (research/gui-design.md §A.3) layered on top, rendered above the navigation so the live
// verbs lead. When no workbench is mounted the list is empty and the palette is pure navigation.
import {
  usePaletteCommands,
  type PaletteCommand,
} from "@/components/palette-commands";
import { PALETTE_COMMAND_GROUP_ORDER } from "@/lib/palette-commands";

const REPO_URL = "https://github.com/sparq-org/sparq";

// [OPUS-4.8] sq-vw3ax.3 — the palette is the 0-click fast path now the sidebar is gone, so it
// must jump to each surface's NEW home, not a removed /surface/* route that only redirects.
// The 5 retained deep pages keep their own /surface/<slug> route; every other surface lives as a
// row under /capabilities#<theme>. Resolving here (not at the data source) keeps surfaces.ts a
// pure structural source while the palette routes to the post-redesign IA.
// [FABLE-5] sq-vw3ax.11.1 — the dead `slug === "try"` special case was removed: the /try surface
// no longer exists in data/surfaces.ts (sq-4hiqe removed /try; it is now a hard-redirect stub to
// /app), so the branch could never fire. Every remaining surface is a same-build /surface or
// /capabilities route reachable by a soft push — none is the separate-build /app workbench.
function surfaceHref(surface: Surface, groupId: string): string {
  if (DEEP_PAGE_SLUGS.has(surface.slug)) return surface.href; // retained deep page
  return capabilityAnchor(groupId); // a demo/soon row lives in the gallery
}

// A tiny context so a header trigger button (or anything else in the shell) can open the
// palette without prop-drilling — the palette owns the state, exposes setOpen via context.
const CommandPaletteContext = React.createContext<{
  open: boolean;
  setOpen: (open: boolean) => void;
} | null>(null);

/** Open / toggle the global command palette from anywhere inside <CommandPalette>. */
export function useCommandPalette() {
  const ctx = React.useContext(CommandPaletteContext);
  if (!ctx) {
    throw new Error("useCommandPalette must be used within <CommandPalette>");
  }
  return ctx;
}

// The fixed top-level destinations of the slim top bar that are NOT capability surfaces (so
// they are not in GROUPS). Option-B (sq-rclb8): "Try" (the live REPL) lives in the surface data
// (the first Query & data surface), so it is not repeated here; "App" is the SEPARATE live
// operational GUI destination — it replaces the old "/gui · Try the GUI" entry, which now
// client-redirects to /app. /about is folded into Home #how-it-runs (it redirects), so the
// palette points there. /capabilities is the new gallery; /download + /app are the discovery
// gaps the maintainer flagged.
//
// [FABLE-5] sq-vw3ax.11.1 — `external: true` marks a destination served in production by a
// SEPARATE deployed Next.js app at the same origin (here: /app = the gui/app workbench overlaid
// at /app/ by pages.yml, sq-vnd0i). Such a slot MUST be a hard, full-page navigation (see `go`),
// mirroring app-shell.tsx's NavLink — a next/router soft push across the two distinct Next builds
// would fetch the foreign RSC Flight payload (/app/index.txt) and land on a raw .txt instead of
// the GUI. Selecting "App" via Cmd-K was the last live instance of that /app txt-redirect bug.
const TOP_PAGES: { href: string; title: string; blurb: string; external?: boolean }[] = [
  { href: "/", title: "Home", blurb: "Overview — what sparq is, the live REPL, the flagships." },
  // [OPUS-4.8] sq-vw3ax.6 — the curated "show me it working" gallery (3 flagships + Live REPL).
  // Discoverable here in Cmd-K (0 clicks) without bloating the slim top bar — the bar stays at 6.
  { href: "/examples", title: "Examples", blurb: "The curated 'show me it working' demo gallery." },
  { href: "/capabilities", title: "Capabilities", blurb: "Every surface in one gallery, by theme." },
  // [FABLE-5] sq-vw3ax.11.1 — the hosted GUI is LIVE at /app now (not "coming soon"); marked
  // external so `go` hard-navigates across the two Next builds (see TOP_PAGES comment above).
  { href: "/app", title: "App", blurb: "The live operational GUI — the in-browser workbench.", external: true },
  { href: "/benchmarks", title: "Benchmarks", blurb: "Per-commit, same-box benchmark dashboard." },
  // [OPUS-4.8] sq-1scgk — /papers is now a top-bar destination (maintainer 2026-07-04 item
  // 9b); it stays indexed here too, exactly like the other bar pages above.
  { href: "/papers", title: "Papers", blurb: "The research papers behind sparq." },
  // [OPUS-4.8] sq-rvgr2.1 — the /specs section: W3C ReSpec-style Unofficial Proposal Drafts.
  // Cmd-K only (unlike /papers, which is now in the bar) so the slim top bar stays lean.
  { href: "/specs", title: "Specs", blurb: "W3C ReSpec-style Unofficial Proposal Drafts (no W3C standing)." },
  // (sq-gk9lq) [SONNET-4.6] — how sparq stores and queries its own project knowledge as RDF.
  // Cmd-K only (like /papers, /specs) so the slim top bar stays at 6 destinations.
  { href: "/dogfooding", title: "Dogfooding sparq", blurb: "How sparq uses itself as a knowledge-management system — the PKG loop, ingest workflow, NL query tool, and measured cost benefit." },
  // (sq-0hv3o) [SONNET-4.6] — the web front door to the proof + testing estate (root
  // ASSURANCE.md, PR #1562). Cmd-K only (like /papers, /specs, /dogfooding) so the slim
  // top bar stays at 6 destinations; the maintainer asked to surface it, not to grow the bar.
  { href: "/assurance", title: "Assurance", blurb: "How to check that sparq works in 15 minutes — the 5-minute health check, the layer-by-layer proof + testing estate, and what green does not mean." },
  // (sq-549m0) [FABLE-5] — the honest GenAI-retrieval status ledger: implemented (crate +
  // feature) vs proposed (bead id). Cmd-K only (like /assurance, /dogfooding) — bar stays at 6.
  { href: "/genai-retrieval", title: "GenAI retrieval — implemented vs proposed", blurb: "The honest status ledger for the RDF-native retrieval composition — ANN-in-SPARQL, embedding provenance, NL→SPARQL, citations — implemented vs designed-only." },
  // (sq-vw3ax.16) [OPUS-4.8] — the honest competitive feature matrix: LEAD/PARITY/PARTIAL/GAP
  // vs RDFox/Stardog/GraphDB/TopBraid/Ent., GAP rows kept, bead links, zero perf numbers.
  // Cmd-K only (like /assurance, /genai-retrieval) — the slim top bar stays fixed.
  { href: "/compare", title: "How sparq compares", blurb: "The honest competitive feature matrix — sparq vs RDFox, Stardog, GraphDB, TopBraid and the enterprise cluster, with the GAP rows kept visible and zero performance numbers." },
  { href: "/download", title: "Download", blurb: "Desktop GUI + CLI/server binaries (latest release)." },
  { href: "/#how-it-runs", title: "How it runs", blurb: "The honest \"what runs where\" tier model." },
];

/** A tier badge a surface carries (flagship / surface rows render this in the right gutter). */
function TierBadge({ surface }: { surface: Surface }) {
  return (
    <Badge
      variant={TIER_VARIANT[surface.tier]}
      className="h-4 shrink-0 px-1.5 text-[10px]"
      title={TIER_LABEL[surface.tier]}
    >
      {TIER_LABEL[surface.tier]}
    </Badge>
  );
}

// [OPUS-4.8] sq-vw3ax.1 — a clean SUBSTRING/word scorer, used instead of cmdk's default
// fuzzy-subsequence one. The default scores almost every row > 0 for a short query (because
// each row's keyword soup contains the query's letters as a loose subsequence), and then DOM
// order breaks the near-ties — so "shacl" would surface a flagship rather than the SHACL row.
// This scorer only returns > 0 on a real substring hit, and weights a TITLE match far above a
// keyword/blurb match, so the obvious row wins. Case-insensitive; whitespace-normalised.
function scoreItem(value: string, search: string, keywords?: string[]): number {
  const q = search.trim().toLowerCase();
  if (!q) return 1; // empty query → show everything (cmdk renders all, DOM order)
  const title = value.toLowerCase();
  // Exact title, then title-prefix, then title-substring — the strongest signals.
  if (title === q) return 1;
  if (title.startsWith(q)) return 0.9;
  if (title.includes(q)) return 0.7;
  // Otherwise a keyword/blurb substring hit (theme label, blurb, action verbs).
  const hay = (keywords ?? []).join(" ").toLowerCase();
  if (hay.includes(q)) return 0.4;
  return 0; // no substring match → hidden
}

// [OPUS-4.8] sq-ixc3.10 — bucket the flat operational command list into its fixed-order
// groups, dropping empties. The groups always render ABOVE the navigation so the live verbs
// lead the spine. A `<Command.Group>` with no children would render a stray heading, so we
// filter those out here rather than in the JSX.
function groupOperationalCommands(
  commands: readonly PaletteCommand[],
): { group: string; commands: PaletteCommand[] }[] {
  return PALETTE_COMMAND_GROUP_ORDER.map((group) => ({
    group,
    commands: commands.filter((c) => c.group === group),
  })).filter((g) => g.commands.length > 0);
}

/** A single selectable row. `value` is the title; cmdk scores it via our custom filter. */
function PaletteItem({
  icon: Icon,
  title,
  blurb,
  keywords,
  onSelect,
  trailing,
  // [OPUS-4.8] sq-ixc3.10 — operational rows may be temporarily unavailable (e.g. "Save
  // workspace" with no open workspace, EXPLAIN on an Update form). They render greyed and
  // non-selectable rather than disappearing, so the spine is a stable, discoverable surface.
  disabled,
  // A stable disambiguator appended to cmdk's `value` so two rows with the SAME title (a
  // recent query and a graph could both be "default", say) do not collide in cmdk's value map.
  valueKey,
}: {
  icon: LucideIcon;
  title: string;
  blurb?: string;
  // Secondary match terms (theme label, blurb, action verbs). cmdk scores the `value`
  // (the title) FIRST, then falls back to keywords — so "shacl" ranks the SHACL surface
  // above an unrelated row whose blurb happens to fuzzy-match, rather than diluting the
  // score by concatenating everything into one `value` string.
  keywords?: string[];
  onSelect: () => void;
  trailing?: React.ReactNode;
  disabled?: boolean;
  valueKey?: string;
}) {
  return (
    <Command.Item
      // cmdk fuzzy-scores the `value`. We keep the human title as the leading token (so the
      // scorer still matches on it) and append an invisible disambiguator for uniqueness.
      value={valueKey ? `${title} ${valueKey}` : title}
      keywords={keywords}
      disabled={disabled}
      onSelect={onSelect}
      className={cn(
        "flex items-center gap-3 rounded-md px-3 py-2 text-sm",
        disabled
          ? "cursor-not-allowed opacity-50"
          : "cursor-pointer text-foreground/90 data-[selected=true]:bg-sidebar-accent data-[selected=true]:text-sidebar-accent-foreground",
      )}
    >
      <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
        <Icon className="size-4" />
      </span>
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{title}</span>
        {blurb && (
          <span className="truncate text-xs text-muted-foreground">{blurb}</span>
        )}
      </span>
      {trailing}
    </Command.Item>
  );
}

export function CommandPalette({ children }: { children: React.ReactNode }) {
  const [open, setOpenState] = React.useState(false);
  const router = useRouter();
  const { theme, setTheme, resolvedTheme } = useTheme();

  // [FABLE-5] sq-ymr2e.9 — ESC must RESTORE focus to the INVOKER (WAI-ARIA dialog focus
  // contract; asserted by e2e/a11y-keyboard.spec.ts). The palette has no Radix
  // <Dialog.Trigger> (it opens from a global ⌘K/Ctrl-K listener or the header button), and
  // without one the close left focus on <body> — invisible to a keyboard/AT user. So every
  // open path records document.activeElement and onCloseAutoFocus returns focus there.
  const invokerRef = React.useRef<HTMLElement | null>(null);
  const setOpen = React.useCallback((next: boolean | ((v: boolean) => boolean)) => {
    setOpenState((v) => {
      const n = typeof next === "function" ? next(v) : next;
      if (n && !v) {
        // Capturing inside the updater keeps the ⌘K toggle race-free; it is idempotent, so a
        // StrictMode double-invoke is harmless.
        invokerRef.current =
          document.activeElement instanceof HTMLElement ? document.activeElement : null;
      }
      return n;
    });
  }, []);

  // [OPUS-4.8] sq-ixc3.10 — the live operational commands the mounted workbench contributes,
  // bucketed into their fixed-order groups. Empty off the workbench (pure navigation then).
  const operationalCommands = usePaletteCommands();
  const operationalGroups = React.useMemo(
    () => groupOperationalCommands(operationalCommands),
    [operationalCommands],
  );

  const ctx = React.useMemo(() => ({ open, setOpen }), [open, setOpen]);

  // Global ⌘K (macOS) / Ctrl-K (Windows/Linux) toggles the palette. We also let the platform
  // browser-search shortcut stay free: only K with the platform modifier is intercepted.
  React.useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        setOpen((v) => !v);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [setOpen]);

  // Navigate (or run an action), then close.
  //
  // A same-build route is a SOFT next/navigation push: router.push respects the configured
  // basePath, so "/surface/zk" resolves under /sparq on Pages and at root under Tauri — no manual
  // basePath juggling, and the SPA transition keeps the site's own client alive.
  //
  // [FABLE-5] sq-vw3ax.11.1 — an `external` destination (only /app today) is served by a SEPARATE
  // Next.js build overlaid at /app/, so it MUST be a HARD full-page navigation: window.location
  // .assign to the base-path-prefixed, trailing-slashed URL, mirroring app-shell.tsx's NavLink
  // <a href={withBasePath(...)}>. A soft router.push across the two distinct Next builds would
  // fetch the foreign RSC Flight payload /app/index.txt and land on a raw .txt — the exact
  // txt-redirect bug. INVARIANT: every user-reachable navigation to /app from the site UI is a
  // hard navigation. Next does NOT auto-prefix a hand-written location.assign, hence withBasePath;
  // the trailing slash matches `trailingSlash: true` so the static export's directory index
  // resolves (identical to the NavLink's `href.endsWith("/") ? href : ${href}/`).
  const go = React.useCallback(
    (href: string, external?: boolean) => {
      setOpen(false);
      if (external) {
        window.location.assign(withBasePath(href.endsWith("/") ? href : `${href}/`));
        return;
      }
      router.push(href);
    },
    [router, setOpen],
  );

  const runAction = React.useCallback(
    (fn: () => void) => {
      setOpen(false);
      fn();
    },
    [setOpen],
  );

  const isDark = (resolvedTheme ?? theme) === "dark";

  return (
    <CommandPaletteContext.Provider value={ctx}>
      {children}
      <DialogPrimitive.Root open={open} onOpenChange={setOpen}>
        <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay
          className={cn(
            "fixed inset-0 z-50 bg-black/40 backdrop-blur-sm",
            "data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0",
          )}
        />
        <DialogPrimitive.Content
          // [FABLE-5] sq-ymr2e.9 — return focus to the recorded invoker (fall back to the
          // Radix default only when it is gone, e.g. after a palette navigation unmounted it).
          onCloseAutoFocus={(event) => {
            const invoker = invokerRef.current;
            invokerRef.current = null;
            if (invoker?.isConnected) {
              event.preventDefault();
              invoker.focus();
            }
          }}
          className={cn(
            "bg-card text-card-foreground fixed left-1/2 top-[12vh] z-50 w-[min(38rem,calc(100vw-2rem))] -translate-x-1/2",
            "overflow-hidden rounded-xl p-0 shadow-2xl ring-1 ring-foreground/10",
            "data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95",
          )}
        >
          {/* The Title is the dialog's accessible name (radix links aria-labelledby to it). */}
          <DialogPrimitive.Title className="sr-only">
            Command palette
          </DialogPrimitive.Title>
          <DialogPrimitive.Description className="sr-only">
            Type to fuzzy-search every sparq surface, page, and quick action, then press Enter
            to jump to it.
          </DialogPrimitive.Description>

          <Command
            label="Search surfaces, pages and actions"
            className="flex flex-col"
            filter={scoreItem}
          >
            <div className="flex items-center gap-2 border-b px-3">
              <Search className="size-4 shrink-0 text-muted-foreground" aria-hidden />
              <Command.Input
                autoFocus
                placeholder="Search surfaces, pages, actions…"
                className="h-12 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
              />
              <kbd className="hidden rounded border px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground sm:inline-block">
                ESC
              </kbd>
            </div>

            <Command.List className="max-h-[min(60vh,28rem)] overflow-y-auto p-2">
              <Command.Empty className="px-3 py-6 text-center text-sm text-muted-foreground">
                No matches. Try a surface name, &ldquo;benchmarks&rdquo;, or &ldquo;theme&rdquo;.
              </Command.Empty>

              {/* [OPUS-4.8] sq-ixc3.10 — the live OPERATIONAL spine FIRST: whatever the mounted
                  workbench contributes (run / EXPLAIN / connect / export / import / switch-
                  workspace / named-graph / recent-query). Empty off the workbench. */}
              {operationalGroups.map(({ group, commands }) => (
                <Command.Group
                  key={group}
                  heading={group}
                  className="px-1 pb-1 text-xs font-medium text-muted-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5"
                >
                  {commands.map((c) => (
                    <PaletteItem
                      key={c.id}
                      valueKey={c.id}
                      icon={c.icon}
                      title={c.title}
                      blurb={c.blurb}
                      keywords={c.keywords}
                      disabled={c.disabled}
                      onSelect={() => runAction(c.run)}
                    />
                  ))}
                </Command.Group>
              ))}

              {/* Flagship showcases first — the strongest proof artifacts. */}
              <Command.Group
                heading="Flagship showcases"
                className="px-1 pb-1 text-xs font-medium text-muted-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5"
              >
                {FLAGSHIPS.map((f) => (
                  <PaletteItem
                    key={f.href}
                    icon={f.icon}
                    title={f.title}
                    blurb={f.blurb}
                    keywords={["showcase", "flagship", f.blurb]}
                    onSelect={() => go(f.href)}
                    trailing={<TierBadge surface={f} />}
                  />
                ))}
              </Command.Group>

              {/* The 5 capability THEMES — every surface, derived from the single GROUPS source. */}
              {GROUPS.map((group) => (
                <Command.Group
                  key={group.id}
                  heading={group.label}
                  className="px-1 pb-1 text-xs font-medium text-muted-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5"
                >
                  {group.surfaces.map((s) => (
                    <PaletteItem
                      key={s.href}
                      icon={s.icon}
                      title={s.title}
                      blurb={s.blurb}
                      // The theme label + blurb are keywords so a theme search surfaces its
                      // members, without diluting the title-driven primary score.
                      keywords={[group.label, s.blurb]}
                      // Jump to the surface's POST-redesign home: a retained deep page keeps
                      // /surface/<slug>; every other surface lives at /capabilities#<theme>.
                      onSelect={() => go(surfaceHref(s, group.id))}
                      trailing={<TierBadge surface={s} />}
                    />
                  ))}
                </Command.Group>
              ))}

              {/* Fixed top-level pages (the slim top bar). */}
              <Command.Group
                heading="Pages"
                className="px-1 pb-1 text-xs font-medium text-muted-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5"
              >
                {TOP_PAGES.map((p) => (
                  <PaletteItem
                    key={p.href}
                    icon={Search}
                    title={p.title}
                    blurb={p.blurb}
                    keywords={["page", p.blurb]}
                    // [FABLE-5] sq-vw3ax.11.1 — an external page (/app) hard-navigates; every
                    // other same-build page keeps the soft router.push.
                    onSelect={() => go(p.href, p.external)}
                  />
                ))}
              </Command.Group>

              {/* Quick actions. */}
              <Command.Group
                heading="Actions"
                className="px-1 pb-1 text-xs font-medium text-muted-foreground [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5"
              >
                <PaletteItem
                  icon={isDark ? Sun : Moon}
                  title={isDark ? "Switch to light theme" : "Switch to dark theme"}
                  keywords={["action", "toggle", "theme", "dark", "light", "mode"]}
                  onSelect={() => runAction(() => setTheme(isDark ? "light" : "dark"))}
                />
                <PaletteItem
                  icon={Github}
                  title="Open sparq on GitHub"
                  blurb={REPO_URL}
                  keywords={["action", "github", "source", "repository", "code"]}
                  onSelect={() =>
                    runAction(() =>
                      window.open(REPO_URL, "_blank", "noopener,noreferrer"),
                    )
                  }
                />
              </Command.Group>
            </Command.List>

            <div className="flex items-center justify-between gap-2 border-t px-3 py-2 text-[11px] text-muted-foreground">
              <span className="inline-flex items-center gap-1">
                <CornerDownLeft className="size-3" aria-hidden /> to open
              </span>
              <span>
                Open with{" "}
                <kbd className="rounded border px-1 py-0.5 font-medium">⌘K</kbd> /{" "}
                <kbd className="rounded border px-1 py-0.5 font-medium">Ctrl K</kbd>
              </span>
            </div>
          </Command>
        </DialogPrimitive.Content>
        </DialogPrimitive.Portal>
      </DialogPrimitive.Root>
    </CommandPaletteContext.Provider>
  );
}

/**
 * The header trigger — a slim "search" affordance showing the ⌘K hint, plus an icon-only
 * variant for the mobile bar. Clicking it opens the same palette the keyboard shortcut does.
 */
export function CommandPaletteTrigger({ className }: { className?: string }) {
  const { setOpen } = useCommandPalette();
  return (
    <button
      type="button"
      onClick={() => setOpen(true)}
      aria-label="Search (Command-K)"
      className={cn(
        "inline-flex h-9 items-center gap-2 rounded-md border bg-background px-3 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring/50",
        className,
      )}
    >
      <Search className="size-4" aria-hidden />
      <span className="hidden sm:inline">Search…</span>
      <kbd className="ml-1 hidden rounded border px-1.5 py-0.5 text-[10px] font-medium sm:inline-block">
        ⌘K
      </kbd>
    </button>
  );
}
