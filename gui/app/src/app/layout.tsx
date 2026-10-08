import type { Metadata, Viewport } from "next";
import { Inter } from "next/font/google";

import "./globals.css";
import { ThemeProvider } from "@/components/theme-provider";
import { EngineProvider } from "@/lib/engine-context";
import { WorkspaceProvider } from "@/lib/workspace-context";
import { basePath } from "@/lib/base-path";
import { enginePreloadHrefs, readWasmManifest } from "@/lib/wasm-preload";

const inter = Inter({
  subsets: ["latin"],
  variable: "--font-inter",
  display: "swap",
});

// [OPUS-4.8] sq-ixc3.8 / sq-ixc3.9 — the operational GUI is a DISTINCT app, not the marketing
// site. Its metadata names the workbench, never the showcase.
export const metadata: Metadata = {
  title: "sparq workbench",
  description:
    "The sparq operational workbench — query, validate, and inspect RDF over a live in-tab engine. A distinct desktop + web tool, not the marketing site.",
};

export const viewport: Viewport = {
  themeColor: [
    { media: "(prefers-color-scheme: light)", color: "#f7fbfc" },
    { media: "(prefers-color-scheme: dark)", color: "#13181c" },
  ],
};

export default function RootLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  // #2422 — resolve the hints through the content-hash manifest so they match the loader's
  // request exactly (an unhashed hint made a hosted build download the engine wasm twice).
  const preload = enginePreloadHrefs(readWasmManifest(), basePath());
  return (
    <html lang="en" suppressHydrationWarning>
      <body className={`${inter.variable} font-sans antialiased`}>
        {/* [FABLE-5] sq-qgkwy.2 — PARALLELISE the engine cold start: the in-tab wasm engine is
            loaded unconditionally by EngineProvider's mount effect (prewarmSparq), which means
            the ~3 MB engine fetch used to START only after the JS bundle downloaded + React
            hydrated. These two resource hints (React hoists rendered <link> elements into
            <head>) start the glue-module + wasm downloads IN PARALLEL with the bundle instead,
            so hydration finds them warm/in-flight. `as="fetch"` + `crossOrigin="anonymous"`
            matches the wasm-pack glue's plain same-origin fetch() (mode cors / credentials
            same-origin) so the preload cache is HIT, not doubled; the glue is a dynamic ESM
            import → `modulepreload`. Tier-b bundles (reason/text/rsp) are opt-in tools and are
            deliberately NOT preloaded. */}
        <link rel="modulepreload" href={preload.glue} />
        <link
          rel="preload"
          as="fetch"
          href={preload.wasm}
          crossOrigin="anonymous"
        />
        <ThemeProvider
          attribute="class"
          defaultTheme="dark"
          enableSystem
          disableTransitionOnChange
        >
          {/* The engine context provides the ONE live store the whole workbench shares; the
              workspace context tracks the active workspace's imported sources + persists the
              save/open snapshot (sq-atb0) the Import drawer (sq-ixc3.13) writes on each import. */}
          <EngineProvider>
            <WorkspaceProvider>{children}</WorkspaceProvider>
          </EngineProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
