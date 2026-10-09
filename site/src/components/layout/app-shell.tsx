"use client";

// [OPUS-4.8] sq-vw3ax.7 — the COLLAPSED navigation.
//
// Before: a persistent w-64 sidebar rendering the full 6-group/16-surface tree (sidebar-nav.tsx)
// AND a duplicate top-tab bar — the same surfaces in two places, the literal "too much" the
// maintainer flagged (research/website-redesign.md §2, §7). After: ONE slim top bar of content
// destinations + a utility cluster. The sidebar is gone; the 0-click fast path to every surface
// is the Cmd-K palette (sq-vw3ax.1), which is WHY removing the sidebar is now safe — Cmd-K
// shipped first. A mobile Sheet drawer mirrors the same slim links (no full tree).
//
// OPTION-B (the maintainer's decision after #1004 opened, sq-rclb8 / sq-vnd0i). The single
// operational destination is "App" → /app, the LIVE GUI. That GUI is a SEPARATE Next.js app
// (gui/app), overlaid at /app/ by the Pages deploy (pages.yml) — it is NOT a route of this site.
// The old single "Try the GUI" → /gui slot, and the in-tab /try REPL playground, both redirect
// to /app (sq-4hiqe, maintainer directive 2026-07-05: /try was "very broken" and the app has
// everything you need — /app is now the ONE workbench).
//
// [OPUS-4.8] sq-vw3ax.11 — the "App" slot is therefore a HARD (full-page) link, not a next/link
// soft navigation: soft-navigating across two distinct Next builds fetches the WRONG app's RSC
// Flight payload (/app/index.txt) and lands on a raw .txt instead of the GUI. See NavLink.
//
// Destinations (research §2 + the maintainer's discoverability gaps):
//   Home · Capabilities · App · Benchmarks · Papers · Download
//   utility cluster: { Cmd-K · GitHub · theme }
// [OPUS-4.8] sq-1scgk (maintainer 2026-07-04 item 9b) — "Papers" is PROMOTED into the slim
// top bar so the paper-factory output is prominently findable, not buried in Cmd-K overflow.
// It stays in Cmd-K too (the palette indexes every top-bar page). The bar stays lean at 6 short
// labels. (/examples and the deep-page rebuild are sequenced in sq-vw3ax.6 / .4 — not this PR.)

import * as React from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";
import { Menu } from "lucide-react";

import { cn } from "@/lib/utils";
import { withBasePath } from "@/lib/base-path";
import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import { Logo } from "@/components/logo";
import { Github } from "@/components/github-icon";
import { ThemeToggle } from "@/components/theme-toggle";
import {
  CommandPalette,
  CommandPaletteTrigger,
} from "@/components/command-palette";
// [OPUS-4.8] sq-ixc3.10 — the operational-command registry. Mounted once inside the palette so
// interactive surfaces can contribute live commands to the keyboard-first spine. On this
// marketing site the registry now degrades to pure navigation (the in-tab REPL that used to
// register run / EXPLAIN / connect verbs was removed with /try, sq-4hiqe — /app owns them).
import { PaletteCommandsProvider } from "@/components/palette-commands";

const REPO_URL = "https://github.com/sparq-org/sparq";
// #2549 — Docs points at the human-facing mdBook guide, which pages.yml builds and overlays at
// /guide/ on the same origin (issue #5022; gated by scripts/check-guide-publish-wiring.py). It is
// a separate static build, so it is an `external` (hard-navigation) slot like /app.
const DOCS_URL = "/guide";

// The slim top bar's content destinations. Capabilities is the single gallery that replaces
// the collapsed /surface/* tree; App is the live operational GUI destination (the single
// workbench — /try redirects here); Download surfaces the desktop GUI + CLI binaries (the
// maintainer's discoverability ask). Each is a real, built route. [OPUS-4.8] sq-1scgk — Papers
// is now a first-class bar destination (maintainer 2026-07-04 item 9b: make the paper-factory
// output prominently findable), sitting between Benchmarks and Download; it remains reachable
// via Cmd-K too. `external: true` marks a destination that is served by a SEPARATE deployed app
// at the same origin (here: /app = the gui/app workbench overlaid at /app/). Such a slot must be
// a hard, full-page navigation — see NavLink — not a next/link soft nav (sq-vw3ax.11).
const NAV_ITEMS: { href: string; label: string; external?: boolean }[] = [
  { href: "/", label: "Home" },
  { href: DOCS_URL, label: "Docs", external: true },
  { href: "/capabilities", label: "Capabilities" },
  { href: "/app", label: "App", external: true },
  { href: "/benchmarks", label: "Benchmarks" },
  { href: "/papers", label: "Papers" },
  { href: "/download", label: "Download" },
];

function useIsActive() {
  const pathname = usePathname();
  return (href: string) =>
    href.startsWith("http")
      ? false
      : href === "/"
        ? pathname === "/"
        : pathname.startsWith(href);
}

function NavLink({
  href,
  children,
  active,
  external,
  onNavigate,
}: {
  href: string;
  children: React.ReactNode;
  active: boolean;
  external?: boolean;
  onNavigate?: () => void;
}) {
  const className = cn(
    "rounded-md px-3 py-1.5 text-sm transition-colors",
    active
      ? "bg-sidebar-accent text-sidebar-accent-foreground font-medium"
      : "text-foreground/70 hover:bg-muted hover:text-foreground",
  );

  // [OPUS-4.8] sq-vw3ax.11 — an `external` destination (e.g. /app) is served in production by a
  // DIFFERENT Next.js app overlaid at /app/ (gui/app, sq-vnd0i), so it must be a hard,
  // full-page navigation. A next/link soft nav across two distinct Next builds would fetch the
  // foreign RSC Flight payload (/app/index.txt) and render a raw .txt instead of the GUI —
  // exactly the bug this fixes. withBasePath prefixes the hand-written absolute href for the
  // /sparq (Pages) vs "" (Tauri) hosts — Next does NOT auto-prefix a plain <a>. The trailing
  // slash matches `trailingSlash: true` so the static export's directory index resolves.
  if (external) {
    const resolvedHref = href.startsWith("http")
      ? href
      : withBasePath(href.endsWith("/") ? href : `${href}/`);
    return (
      <a
        href={resolvedHref}
        onClick={onNavigate}
        aria-current={active ? "page" : undefined}
        className={className}
      >
        {children}
      </a>
    );
  }

  return (
    <Link
      href={href}
      onClick={onNavigate}
      aria-current={active ? "page" : undefined}
      className={className}
    >
      {children}
    </Link>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const [mobileOpen, setMobileOpen] = React.useState(false);
  // [OPUS-4.8] sq-ymr2e.1 — post-hydration readiness marker for E2E. Set in a mount effect, so
  // `[data-app-ready]` appears only after React has hydrated the shell — and, because the attribute
  // is applied on the RE-RENDER after the passive-effect batch, strictly after the wrapping
  // <CommandPalette>'s global ⌘K keydown effect has been registered. The E2E navigation barrier
  // waits on this instead of a fixed sleep (site/e2e/support/app-ready.ts); no user-facing effect.
  const [appReady, setAppReady] = React.useState(false);
  React.useEffect(() => setAppReady(true), []);
  const isActive = useIsActive();

  return (
    // The whole shell is wrapped in <CommandPalette> so the global ⌘K / Ctrl-K binding and the
    // header trigger share one palette instance, mounted once. With the sidebar removed, this
    // palette is the discoverability backstop for every surface (sq-vw3ax.1 → .7).
    // [OPUS-4.8] sq-ixc3.10 — <PaletteCommandsProvider> nests inside so the palette (read side)
    // and the workbench (register side) share one operational-command registry.
    <CommandPalette>
      <PaletteCommandsProvider>
      <div
        className="flex min-h-svh flex-col"
        data-app-ready={appReady ? "true" : undefined}
      >
        {/* Sticky slim top bar (h-16, backdrop blur) — the ONE navigation. */}
        <header className="sticky top-0 z-30 flex h-16 items-center gap-2 border-b bg-background/90 px-4 backdrop-blur md:px-6">
          {/* Mobile hamburger → left Sheet drawer with the SAME slim links (no full tree). */}
          <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
            <SheetTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="md:hidden"
                aria-label="Open navigation"
              >
                <Menu className="size-5" />
              </Button>
            </SheetTrigger>
            <SheetContent side="left" className="overflow-y-auto">
              <SheetTitle className="sr-only">Navigation</SheetTitle>
              <Link
                href="/"
                className="mb-4 flex items-center"
                aria-label="sparq home"
                onClick={() => setMobileOpen(false)}
              >
                <Logo className="h-7 w-auto" />
              </Link>
              <nav className="flex flex-col gap-1" aria-label="Primary">
                {NAV_ITEMS.map((item) => (
                  <NavLink
                    key={item.href}
                    href={item.href}
                    active={isActive(item.href)}
                    external={item.external}
                    onNavigate={() => setMobileOpen(false)}
                  >
                    {item.label}
                  </NavLink>
                ))}
              </nav>
            </SheetContent>
          </Sheet>

          <Link href="/" className="flex items-center" aria-label="sparq home">
            <Logo className="h-7 w-auto" />
          </Link>

          {/* The slim top-bar destinations (desktop). */}
          <nav
            className="ml-3 hidden items-center gap-1 md:flex"
            aria-label="Primary"
          >
            {NAV_ITEMS.map((item) => (
              <NavLink
                key={item.href}
                href={item.href}
                active={isActive(item.href)}
                external={item.external}
              >
                {item.label}
              </NavLink>
            ))}
          </nav>

          <div className="ml-auto flex items-center gap-1">
            {/* Try (/try, the REPL) and App (/app, the live GUI) are both first-class slim-bar
                destinations now (Option-B), so the utility cluster is just Cmd-K · GitHub · theme. */}
            {/* The Cmd-K affordance — the fast path to every surface now the sidebar is gone. */}
            <CommandPaletteTrigger className="mr-1 hidden lg:inline-flex" />
            <Button variant="ghost" size="icon" asChild>
              <a
                href={REPO_URL}
                target="_blank"
                rel="noopener noreferrer"
                aria-label="sparq on GitHub"
              >
                <Github className="size-4" />
              </a>
            </Button>
            <ThemeToggle />
          </div>
        </header>

        <main className="flex-1">
          <div className="mx-auto w-full max-w-6xl px-4 py-8 md:px-6 md:py-10">
            {children}
          </div>
        </main>
      </div>
      </PaletteCommandsProvider>
    </CommandPalette>
  );
}
