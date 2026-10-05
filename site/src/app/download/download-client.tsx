"use client";

// [SONNET-4.6] sq-vw3ax.11.2 — /download: prerelease fallback + fail-closed asset matching +
// combined CLI+server card + sparq-org/sparq repo URLs.
//
// CHANGES vs the prior implementation ([OPUS-4.8] sq-vw3ax.11):
//
// (1) REPO_URL / API_LATEST → sparq-org/sparq (repo renamed; GitHub redirects mask the old name
//     but we should reference the canonical home).
//
// (2) PRERELEASE FALLBACK. Only v0.1.0-dev.3 (prerelease=true) exists right now, so
//     `releases/latest` 404s and the old code degraded every control to "No release yet."
//     Fix: on a definitive latest-404, fetch GET /releases?per_page=1 (returns newest release
//     including prereleases) and render per-asset DIRECT browser_download_url buttons, explicitly
//     labelled as unsigned development pre-release builds (keeps the unsigned banner). A card whose
//     platform has NO matching asset in that release degrades per-card to the Releases-page link —
//     never a dead button.  Matching is by pinned per-card pattern (see PRERELEASE_PATTERNS below).
//
// (3) FAIL-CLOSED READY STATE. When a stable release IS ready, a direct latest/download/<alias>
//     button only renders if the alias exists in the release's asset map (checked via
//     assetIsPresent). On API *error* (network failure / rate-limit) buttons stay optimistically
//     live, as documented in the component — the static alias URL needs no API.
//
// (4) HONEST COMBINED CLI+SERVER CARD. The release pipeline ships ONE combined archive carrying
//     BOTH sparq-cli AND sparq-server binaries (build-matrix.yml archive mode). The old separate
//     `sparq-server-<token>.<ext>` alias had no source asset. Replaced with a single "CLI + server"
//     card whose copy honestly states the archive carries both binaries.
//
// INVARIANT: no capability inflation — every rendered download button resolves to an asset that
// actually exists in the release the page is describing. Prerelease builds are explicitly labelled
// unsigned dev builds.

import * as React from "react";
import { toast } from "sonner";
import {
  Apple,
  Bell,
  Check,
  Copy,
  Cpu,
  Download,
  ExternalLink,
  Loader2,
  MonitorSmartphone,
  ShieldCheck,
  Terminal,
  TriangleAlert,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { withBasePath } from "@/lib/base-path";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

// [SONNET-4.6] sq-vw3ax.11.2 — repo renamed to sparq-org/sparq; update all URL roots.
const REPO_URL = "https://github.com/sparq-org/sparq";
const RELEASES_URL = `${REPO_URL}/releases`;
// The version-STABLE download endpoint. GitHub 302s `<LATEST_DL>/<asset>` to the asset CDN.
const LATEST_DL = `${REPO_URL}/releases/latest/download`;
const API_LATEST =
  "https://api.github.com/repos/sparq-org/sparq/releases/latest";
// [SONNET-4.6] sq-vw3ax.11.2 — prerelease fallback: list endpoint (newest release incl. pre).
const API_LIST =
  "https://api.github.com/repos/sparq-org/sparq/releases?per_page=1";

// The platforms the release matrix produces a desktop GUI installer for.
// win-arm64 is deliberately absent (the Tauri bundler can't reliably cross-bundle it from x64).
type OsKey = "macos-arm" | "macos-intel" | "windows" | "linux";

// [SONNET-4.6] sq-vw3ax.11.2 — per-card prerelease asset matching patterns.
// Versioned release asset names embed <version> and arch tokens; these patterns match them.
// GUI patterns are per-platform; CLI patterns are per-token (matched per-card inside cliTarget).
const GUI_PRERELEASE_PATTERNS: Record<OsKey, RegExp> = {
  "macos-arm":   /^sparq-gui_.*_aarch64\.dmg$/,
  "macos-intel": /^sparq-gui_.*_x86_64\.dmg$/,
  windows:       /^sparq-gui_.*_x64\.msi$/,
  linux:         /^sparq-gui_.*_amd64\.(AppImage|deb)$/,
};

// Linux secondary (.deb) has its own specific pattern to distinguish from AppImage.
const LINUX_DEB_PRERELEASE_PATTERN = /^sparq-gui_.*_amd64\.deb$/;

// [SONNET-4.6] sq-vw3ax.11.2 — CLI/server combined archive patterns per platform token.
// Baseline-tier preference for portability.
const CLI_PRERELEASE_PATTERNS: Record<string, RegExp> = {
  "arm64-darwin": /^sparq-cli-.*-arm64-darwin\.tar\.gz$/,
  "x64-darwin":   /^sparq-cli-.*-x64-darwin\.tar\.gz$/,
  "win-x64":      /^sparq-cli-.*-win-x64-baseline\.zip$/,
  "x64-linux":    /^sparq-cli-.*-x64-baseline\.tar\.gz$/,
};

interface SecondaryAsset {
  /** Version-agnostic alias file name served at `<LATEST_DL>/<file>`. */
  file: string;
  /** Short button label, e.g. ".deb (Debian/Ubuntu)". */
  label: string;
}

interface GuiPlatform {
  os: OsKey;
  label: string;
  forYou: string;
  /** Version-agnostic alias file name, served at `<LATEST_DL>/<file>` for stable releases. */
  file: string;
  shortName: string;
  ext: string;
  secondary?: SecondaryAsset;
  icon: React.ReactNode;
  bypass: React.ReactNode;
}

const GUI_PLATFORMS: GuiPlatform[] = [
  {
    os: "macos-arm",
    label: "macOS · Apple Silicon",
    forYou: "For your Mac (Apple Silicon)",
    file: "sparq-gui-arm64-darwin.dmg",
    shortName: "sparq-gui.dmg",
    ext: ".dmg",
    icon: <Apple className="size-5" aria-hidden />,
    bypass: (
      <>
        Gatekeeper quarantines unsigned apps. After opening the{" "}
        <code className="font-mono text-xs">.dmg</code> and dragging sparq to
        Applications, <strong>right-click the app → Open</strong> (don&rsquo;t
        double-click) and confirm once. If it still refuses, clear the quarantine
        flag from a terminal:
        <pre className="mt-2 overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs">
          <code>xattr -dr com.apple.quarantine /Applications/sparq.app</code>
        </pre>
      </>
    ),
  },
  {
    os: "macos-intel",
    label: "macOS · Intel",
    forYou: "For your Mac (Intel)",
    file: "sparq-gui-x64-darwin.dmg",
    shortName: "sparq-gui.dmg",
    ext: ".dmg",
    icon: <Apple className="size-5" aria-hidden />,
    bypass: (
      <>
        Same as Apple Silicon — <strong>right-click the app → Open</strong> on
        first launch, or clear the quarantine flag:
        <pre className="mt-2 overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs">
          <code>xattr -dr com.apple.quarantine /Applications/sparq.app</code>
        </pre>
        Pick this build only on an Intel Mac; Apple Silicon Macs should use the
        arm64 build.
      </>
    ),
  },
  {
    os: "windows",
    label: "Windows · x64",
    forYou: "For your Windows PC",
    file: "sparq-gui-win-x64.msi",
    shortName: "sparq-gui.msi",
    ext: ".msi",
    icon: <MonitorSmartphone className="size-5" aria-hidden />,
    bypass: (
      <>
        The installer isn&rsquo;t Authenticode-signed, so Microsoft SmartScreen
        shows a blue &ldquo;Windows protected your PC&rdquo; dialog. Click{" "}
        <strong>More info → Run anyway</strong> to proceed. There is no Windows
        on Arm installer yet (the bundler can&rsquo;t cross-build it from the x64
        runner).
      </>
    ),
  },
  {
    os: "linux",
    label: "Linux · x86-64",
    forYou: "For your Linux machine",
    file: "sparq-gui-x64-linux.AppImage",
    shortName: "sparq-gui.AppImage",
    ext: ".AppImage",
    secondary: { file: "sparq-gui-x64-linux.deb", label: ".deb (Debian/Ubuntu)" },
    icon: <Cpu className="size-5" aria-hidden />,
    bypass: (
      <>
        The <code className="font-mono text-xs">.AppImage</code> is a portable,
        self-contained binary — no install needed. Mark it executable, then run
        it:
        <pre className="mt-2 overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs">
          <code>chmod +x sparq-gui.AppImage{"\n"}./sparq-gui.AppImage</code>
        </pre>
        A <code className="font-mono text-xs">.deb</code> is published for
        Debian/Ubuntu (button above). arm64 Linux builds are attached to each
        release on the Releases page.
      </>
    ),
  },
];

// ---- GitHub Releases API metadata (progressive enhancement) ------------------------------------

/** One release asset, trimmed to what the cards render. */
interface ReleaseAsset {
  name: string;
  size: number;
  /** e.g. "sha256:<hex>" — GitHub attaches this per asset (CORS-open on api.github.com). */
  digest: string | null;
  /** Direct CDN URL — used for prerelease assets (no stable alias for versioned names). */
  browser_download_url?: string;
}

/** The metadata fetch state machine. */
type ReleaseState =
  | { kind: "loading" }
  // A published (non-prerelease) stable release exists — enrich alias buttons with version/size/sha.
  | { kind: "ready"; version: string; assets: Map<string, ReleaseAsset> }
  // [SONNET-4.6] sq-vw3ax.11.2 — prerelease fallback: latest-404 but a prerelease exists.
  // Render direct browser_download_url buttons per card, labelled unsigned dev build.
  | { kind: "prerelease"; version: string; assets: ReleaseAsset[] }
  // HTTP 404 on /latest AND either no releases at all or empty list.
  | { kind: "none" }
  // Network/other failure: can't confirm, buttons stay live (static alias URL needs no API).
  | { kind: "error" };

interface ApiReleaseShape {
  tag_name?: unknown;
  assets?: unknown;
  prerelease?: unknown;
}

function parseRelease(
  json: unknown,
): { version: string; assets: Map<string, ReleaseAsset>; assetList: ReleaseAsset[]; isPrerelease: boolean } | null {
  if (typeof json !== "object" || json === null) return null;
  const { tag_name, assets, prerelease } = json as ApiReleaseShape;
  if (typeof tag_name !== "string" || !Array.isArray(assets)) return null;
  const map = new Map<string, ReleaseAsset>();
  const list: ReleaseAsset[] = [];
  for (const a of assets) {
    if (typeof a !== "object" || a === null) continue;
    const { name, size, digest, browser_download_url } = a as {
      name?: unknown;
      size?: unknown;
      digest?: unknown;
      browser_download_url?: unknown;
    };
    if (typeof name !== "string" || typeof size !== "number") continue;
    const asset: ReleaseAsset = {
      name,
      size,
      digest: typeof digest === "string" ? digest : null,
      browser_download_url:
        typeof browser_download_url === "string" ? browser_download_url : undefined,
    };
    map.set(name, asset);
    list.push(asset);
  }
  return { version: tag_name, assets: map, assetList: list, isPrerelease: prerelease === true };
}

/**
 * [SONNET-4.6] sq-vw3ax.11.2 — Fetch the latest published release with prerelease fallback.
 *
 * 1. Try GET /releases/latest (stable releases only).
 *    - 200 → ready state (enrich alias buttons with version/size/sha).
 *    - 404 → fall back to GET /releases?per_page=1 (newest including prereleases).
 *      - 200 + isPrerelease → prerelease state (direct browser_download_url per card).
 *      - 200 + empty list → none.
 *      - 200 + stable (shouldn't happen if /latest 404'd) → none.
 *      - error → none (conservative: /latest confirmed no stable).
 *    - other HTTP error (rate-limit, 5xx) → error state (optimistic: alias buttons stay live).
 */
function useLatestRelease(): ReleaseState {
  const [state, setState] = React.useState<ReleaseState>({ kind: "loading" });

  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const res = await fetch(API_LATEST, {
          headers: { Accept: "application/vnd.github+json" },
        });
        if (cancelled) return;

        if (res.ok) {
          const parsed = parseRelease(await res.json());
          if (cancelled) return;
          setState(
            parsed
              ? { kind: "ready", version: parsed.version, assets: parsed.assets }
              : { kind: "error" },
          );
          return;
        }

        if (res.status === 404) {
          // [SONNET-4.6] sq-vw3ax.11.2 — definitive no-stable-release: try list endpoint.
          try {
            const listRes = await fetch(API_LIST, {
              headers: { Accept: "application/vnd.github+json" },
            });
            if (cancelled) return;
            if (!listRes.ok) {
              setState({ kind: "none" });
              return;
            }
            const listJson = await listRes.json();
            if (cancelled) return;
            if (!Array.isArray(listJson) || listJson.length === 0) {
              setState({ kind: "none" });
              return;
            }
            const newest = parseRelease(listJson[0]);
            if (!newest || !newest.isPrerelease) {
              // Stable returned by list when /latest 404'd — shouldn't happen, treat as none.
              setState({ kind: "none" });
              return;
            }
            setState({
              kind: "prerelease",
              version: newest.version,
              assets: newest.assetList,
            });
          } catch {
            if (!cancelled) setState({ kind: "none" });
          }
          return;
        }

        // Other HTTP error — stay optimistic (alias buttons need no API).
        setState({ kind: "error" });
      } catch {
        if (!cancelled) setState({ kind: "error" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${(bytes / 1024).toFixed(0)} KB`;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

/** The asset metadata for a given alias, if the ready release carries it. */
function assetOf(state: ReleaseState, file: string): ReleaseAsset | undefined {
  return state.kind === "ready" ? state.assets.get(file) : undefined;
}

/**
 * [SONNET-4.6] sq-vw3ax.11.2 — Fail-closed presence check for stable (ready) state.
 * Returns true only if the alias appears in the release's asset map.
 * In error state, returns true (optimistic — static alias URL needs no API).
 */
function assetIsPresent(state: ReleaseState, file: string): boolean {
  if (state.kind === "ready") return state.assets.has(file);
  if (state.kind === "error") return true;
  return false;
}

/**
 * [SONNET-4.6] sq-vw3ax.11.2 — Find the first prerelease asset matching a pattern.
 * Returns null when none found — callers degrade to the Releases page link.
 */
function findPrereleaseAsset(
  assets: ReleaseAsset[],
  pattern: RegExp,
): ReleaseAsset | null {
  return assets.find((a) => pattern.test(a.name)) ?? null;
}

// ---- OS detection (progressive enhancement) ----------------------------------------------------

function detectOs(): OsKey | null {
  if (typeof navigator === "undefined") return null;
  const uaData = (
    navigator as Navigator & { userAgentData?: { platform?: string } }
  ).userAgentData;
  const platform = (uaData?.platform ?? "").toLowerCase();
  const ua = navigator.userAgent.toLowerCase();
  const hay = `${platform} ${ua}`;
  if (/mac|iphone|ipad|ipod/.test(hay)) return "macos-arm";
  if (/win/.test(hay)) return "windows";
  if (/linux|android|cros/.test(hay)) return "linux";
  return null;
}

// ---- CLI / server aliases ----------------------------------------------------------------------

function cliTarget(os: OsKey | null): { token: string; ext: string; label: string } {
  switch (os) {
    case "macos-arm":
      return { token: "arm64-darwin", ext: "tar.gz", label: "macOS · Apple Silicon" };
    case "macos-intel":
      return { token: "x64-darwin", ext: "tar.gz", label: "macOS · Intel" };
    case "windows":
      return { token: "win-x64", ext: "zip", label: "Windows · x64" };
    case "linux":
      return { token: "x64-linux", ext: "tar.gz", label: "Linux · x86-64" };
    default:
      return { token: "x64-linux", ext: "tar.gz", label: "Linux · x86-64" };
  }
}

// ---- small presentational pieces ---------------------------------------------------------------

/** A copyable sha256 line — click to copy the full hex to the clipboard. */
function Sha256Line({ digest }: { digest: string }) {
  const hex = digest.replace(/^sha256:/, "");
  const [copied, setCopied] = React.useState(false);
  // [OPUS-4.8] track the reset timer id so it can be cancelled on unmount to avoid leaks.
  const timerRef = React.useRef<number | null>(null);

  React.useEffect(() => {
    return () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    };
  }, []);

  const copy = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(hex);
      setCopied(true);
      toast.success("sha256 copied");
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast.error("Couldn't copy to the clipboard");
    }
  }, [hex]);

  return (
    <div className="mt-2 space-y-1">
      <div className="flex items-center gap-1.5 text-xs font-medium text-foreground">
        <ShieldCheck className="size-3.5 text-[var(--success)]" aria-hidden />
        Verify the download (sha256)
      </div>
      <div className="flex items-start gap-2">
        {/* [FABLE-5] sq-ymr2e.10 — data-vr-mask: the checksum hex is release-specific. */}
        <code
          data-vr-mask
          className="min-w-0 flex-1 overflow-x-auto rounded-md bg-muted px-2 py-1 font-mono text-[11px] leading-relaxed break-all"
        >
          {hex}
        </code>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="size-7 shrink-0"
          onClick={copy}
          aria-label="Copy the sha256 checksum"
        >
          {copied ? (
            <Check className="size-4 text-[var(--success)]" />
          ) : (
            <Copy className="size-4" />
          )}
        </Button>
      </div>
    </div>
  );
}

/**
 * A direct-download control for one version-stable alias.
 *
 * State handling:
 * - loading:    spinner button (static alias href, click downloads if stable exists)
 * - ready + alias present:  static latest/download/<alias> with version·size meta
 * - ready + alias MISSING:  per-card degradation to Releases link (fail-closed)
 * - none:       "No release yet — watch releases" link
 * - error:      optimistic static alias button
 * - prerelease: NOT rendered here; use PrereleaseDownloadButton instead
 */
function DownloadButton({
  file,
  children,
  state,
  variant = "outline",
  size = "sm",
  className,
}: {
  file: string;
  children: React.ReactNode;
  state: ReleaseState;
  variant?: React.ComponentProps<typeof Button>["variant"];
  size?: React.ComponentProps<typeof Button>["size"];
  className?: string;
}) {
  if (state.kind === "none") {
    return (
      <Button
        asChild
        variant="outline"
        size={size}
        className={cn("w-full text-muted-foreground", className)}
      >
        <a href={RELEASES_URL} target="_blank" rel="noopener noreferrer">
          <Bell className="size-4" />
          No release yet — watch releases
        </a>
      </Button>
    );
  }

  // [SONNET-4.6] sq-vw3ax.11.2 — fail-closed: alias not in asset map → Releases link.
  if (state.kind === "ready" && !assetIsPresent(state, file)) {
    return (
      <Button
        asChild
        variant="outline"
        size={size}
        className={cn("w-full text-muted-foreground", className)}
      >
        <a href={RELEASES_URL} target="_blank" rel="noopener noreferrer">
          <ExternalLink className="size-4" />
          See Releases page
        </a>
      </Button>
    );
  }

  const asset = assetOf(state, file);
  const meta =
    state.kind === "ready"
      ? [state.version, asset ? formatBytes(asset.size) : null]
          .filter(Boolean)
          .join(" · ")
      : null;
  return (
    <Button asChild variant={variant} size={size} className={cn("w-full", className)}>
      <a href={`${LATEST_DL}/${file}`} download>
        {state.kind === "loading" ? (
          <Loader2 className="size-4 animate-spin" aria-hidden />
        ) : (
          <Download className="size-4" aria-hidden />
        )}
        <span className="truncate">
          {children}
          {/* [FABLE-5] sq-ymr2e.10 — data-vr-mask: version + byte-size churn per release. */}
          {meta ? (
            <span data-vr-mask className="opacity-80">
              {" "}
              · {meta}
            </span>
          ) : null}
        </span>
      </a>
    </Button>
  );
}

/**
 * [SONNET-4.6] sq-vw3ax.11.2 — Prerelease direct download button.
 * Uses the asset's browser_download_url (versioned CDN URL, not a stable alias).
 * If no matching asset: degrade per-card to the Releases page link.
 */
function PrereleaseDownloadButton({
  asset,
  version,
  children,
  variant = "outline",
  size = "sm",
  className,
}: {
  asset: ReleaseAsset | null;
  version: string;
  children: React.ReactNode;
  variant?: React.ComponentProps<typeof Button>["variant"];
  size?: React.ComponentProps<typeof Button>["size"];
  className?: string;
}) {
  if (!asset || !asset.browser_download_url) {
    return (
      <Button
        asChild
        variant="outline"
        size={size}
        className={cn("w-full text-muted-foreground", className)}
      >
        <a href={RELEASES_URL} target="_blank" rel="noopener noreferrer">
          <ExternalLink className="size-4" />
          See Releases page
        </a>
      </Button>
    );
  }
  const meta = [version, formatBytes(asset.size)].filter(Boolean).join(" · ");
  return (
    <Button asChild variant={variant} size={size} className={cn("w-full", className)}>
      <a href={asset.browser_download_url} download>
        <Download className="size-4" aria-hidden />
        <span className="truncate">
          {children}
          {/* [FABLE-5] sq-ymr2e.10 — data-vr-mask: version + byte-size churn per release. */}
          {meta ? (
            <span data-vr-mask className="opacity-80">
              {" "}
              · {meta}
            </span>
          ) : null}
        </span>
      </a>
    </Button>
  );
}

/** The collapsed first-launch / verify details shared by every GUI card. */
function LaunchDetails({
  bypass,
  digest,
}: {
  bypass: React.ReactNode;
  digest: string | null;
}) {
  return (
    <details className="text-muted-foreground">
      <summary className="cursor-pointer select-none font-medium text-foreground">
        First launch &amp; verification
      </summary>
      <div className="measure pt-2 leading-relaxed">
        {bypass}
        {digest ? <Sha256Line digest={digest} /> : null}
      </div>
    </details>
  );
}

/**
 * [SONNET-4.6] sq-vw3ax.11.2 — All download controls for one GUI platform card.
 * Handles the three substantive states (ready, prerelease, none/error/loading).
 */
function GuiDownloadControls({
  platform,
  state,
  primaryVariant = "outline",
  primarySize = "sm",
  primaryClass,
}: {
  platform: GuiPlatform;
  state: ReleaseState;
  primaryVariant?: React.ComponentProps<typeof Button>["variant"];
  primarySize?: React.ComponentProps<typeof Button>["size"];
  primaryClass?: string;
}) {
  if (state.kind === "prerelease") {
    const primaryPattern = GUI_PRERELEASE_PATTERNS[platform.os];
    const preAsset = findPrereleaseAsset(state.assets, primaryPattern);
    const preSecondary = platform.secondary
      ? findPrereleaseAsset(state.assets, LINUX_DEB_PRERELEASE_PATTERN)
      : null;
    return (
      <>
        <PrereleaseDownloadButton
          asset={preAsset}
          version={state.version}
          variant={primaryVariant}
          size={primarySize}
          className={primaryClass}
        >
          Download {platform.shortName}
        </PrereleaseDownloadButton>
        {platform.secondary && (
          <PrereleaseDownloadButton
            asset={preSecondary}
            version={state.version}
          >
            {platform.secondary.label}
          </PrereleaseDownloadButton>
        )}
        <LaunchDetails
          bypass={platform.bypass}
          digest={preAsset?.digest ?? null}
        />
      </>
    );
  }

  // ready / error / loading / none — all handled by DownloadButton's internal state logic.
  const stableDigest =
    state.kind === "ready" ? (assetOf(state, platform.file)?.digest ?? null) : null;
  return (
    <>
      <DownloadButton
        file={platform.file}
        state={state}
        variant={primaryVariant}
        size={primarySize}
        className={primaryClass}
      >
        Download {platform.shortName}
      </DownloadButton>
      {platform.secondary && state.kind !== "none" && (
        <DownloadButton file={platform.secondary.file} state={state}>
          {platform.secondary.label}
        </DownloadButton>
      )}
      <LaunchDetails bypass={platform.bypass} digest={stableDigest} />
    </>
  );
}

/**
 * [SONNET-4.6] sq-vw3ax.11.2 — Combined CLI + server card.
 * The release pipeline ships ONE archive carrying BOTH sparq-cli and sparq-server binaries.
 * No separate sparq-server-<token>.<ext> alias exists; the old two-card design was dishonest.
 */
function CliServerCard({
  state,
  cliArchive,
  cliToken,
  cliLabel,
  cliExt,
}: {
  state: ReleaseState;
  cliArchive: string;
  cliToken: string;
  cliLabel: string;
  cliExt: string;
}) {
  const bypass = (
    <>
      Portable archive for <strong>{cliLabel}</strong> (
      <code className="font-mono text-xs">.{cliExt}</code>). Extract and run{" "}
      <code className="font-mono text-xs">sparq</code> (CLI) or{" "}
      <code className="font-mono text-xs">sparq-server</code> (HTTP server) —
      no installer, no signing dialog.
    </>
  );

  let downloadNode: React.ReactNode;
  let digest: string | null = null;

  if (state.kind === "prerelease") {
    const cliPattern =
      CLI_PRERELEASE_PATTERNS[cliToken] ?? /^sparq-cli-.*-x64-baseline\.tar\.gz$/;
    const asset = findPrereleaseAsset(state.assets, cliPattern);
    digest = asset?.digest ?? null;
    downloadNode = (
      <PrereleaseDownloadButton asset={asset} version={state.version}>
        Download CLI + server ({cliLabel})
      </PrereleaseDownloadButton>
    );
  } else {
    digest =
      state.kind === "ready" ? (assetOf(state, cliArchive)?.digest ?? null) : null;
    downloadNode = (
      <DownloadButton file={cliArchive} state={state}>
        Download CLI + server ({cliLabel})
      </DownloadButton>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Terminal className="size-5" aria-hidden />
          CLI + server
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3 text-sm text-muted-foreground">
        <p className="measure">
          The archive carries both the{" "}
          <code className="font-mono text-xs">sparq</code> CLI and the{" "}
          <code className="font-mono text-xs">sparq-server</code> HTTP server
          binary. Extract and run either — no installer, no signing dialog.
          Every platform&rsquo;s build is on the{" "}
          <a
            className="text-primary underline underline-offset-4"
            href={RELEASES_URL}
            target="_blank"
            rel="noopener noreferrer"
          >
            Releases page
          </a>
          .
        </p>
        {downloadNode}
        <LaunchDetails bypass={bypass} digest={digest} />
      </CardContent>
    </Card>
  );
}

// ---- page --------------------------------------------------------------------------------------

export function DownloadClient({ children }: { children?: React.ReactNode }) {
  // null until hydrated → SSR renders the full grid with nothing promoted (no hydration mismatch).
  const [detected, setDetected] = React.useState<OsKey | null>(null);
  const release = useLatestRelease();

  React.useEffect(() => {
    setDetected(detectOs());
  }, []);

  const primary = React.useMemo(
    () => (detected ? GUI_PLATFORMS.find((p) => p.os === detected) ?? null : null),
    [detected],
  );
  const rest = React.useMemo(
    () => (primary ? GUI_PLATFORMS.filter((p) => p !== primary) : GUI_PLATFORMS),
    [primary],
  );

  const cli = React.useMemo(() => cliTarget(detected), [detected]);
  const cliArchive = `sparq-cli-${cli.token}.${cli.ext}`;

  return (
    <div className="space-y-10">
      <header className="space-y-3">
        <h1 className="text-2xl font-semibold">Download sparq</h1>
        <p className="measure text-muted-foreground">
          The sparq desktop GUI bundles the engine and the workbench into a
          native app for macOS, Windows, and Linux. Prefer not to install
          anything? The full SPARQL workbench runs entirely in your browser at{" "}
          {/* [OPUS-4.8] sq-ymr2e.4 — inline prose links carry a PERSISTENT underline (WCAG 2.1
              §1.4.1 Use of Color / axe `link-in-text-block`). */}
          {/* [OPUS-4.8] sq-4hiqe — /app is the single workbench; hard anchor not next/link. */}
          <a
            className="text-primary underline underline-offset-4"
            href={withBasePath("/app/")}
          >
            /app
          </a>{" "}
          — no download required.
        </p>
      </header>

      {children}

      {/* Unsigned-build honesty banner — front and centre in BOTH states, never buried. */}
      <div
        role="note"
        className="flex items-start gap-3 rounded-xl bg-[color-mix(in_oklch,var(--warning)_12%,transparent)] px-4 py-3 ring-1 ring-[color-mix(in_oklch,var(--warning)_30%,transparent)]"
      >
        <TriangleAlert
          className="mt-0.5 size-5 shrink-0 text-[var(--warning)]"
          aria-hidden
        />
        <div className="space-y-1 text-sm">
          <p className="font-medium">Developer builds — unsigned.</p>
          <p className="measure text-muted-foreground">
            {/* [SONNET-4.6] sq-vw3ax.11.2 — prerelease state adds explicit dev-build callout. */}
            {release.kind === "prerelease" ? (
              <>
                These are <strong>unsigned development pre-release builds</strong>{" "}
                ({release.version}), not production releases. macOS Gatekeeper
                and Windows SmartScreen will warn on first open — see the per-OS
                instructions. Only install builds you fetched directly from the{" "}
                <a
                  className="text-primary underline underline-offset-4"
                  href={RELEASES_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  official Releases page
                </a>
                .
              </>
            ) : (
              <>
                These desktop bundles are{" "}
                <strong>not code-signed or notarized</strong>, so macOS
                Gatekeeper and Windows SmartScreen will warn the first time you
                open them. That&rsquo;s expected — see the per-OS instructions
                on each card to allow the app. Officially signed and notarized
                builds (Apple Developer ID + Windows Authenticode) are coming;
                until then, only install builds you fetched yourself from the{" "}
                <a
                  className="text-primary underline underline-offset-4"
                  href={RELEASES_URL}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  official Releases page
                </a>
                . Each release attaches a{" "}
                <code className="font-mono text-xs">SHA256SUMS</code> manifest
                and SLSA build-provenance attestations you can verify with{" "}
                <code className="font-mono text-xs">gh attestation verify</code>
                .
              </>
            )}
          </p>
        </div>
      </div>

      {/* Desktop GUI downloads */}
      <section className="space-y-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-xl font-semibold">Desktop app</h2>
          {(release.kind === "ready" || release.kind === "prerelease") && (
            // [FABLE-5] sq-ymr2e.10 — data-vr-mask: the version tag churns per release.
            <Badge variant="muted" className="h-6 font-mono" data-vr-mask>
              {release.version}
            </Badge>
          )}
        </div>

        {/* Detected platform → full-width primary row. */}
        {primary && (
          <Card className="ring-2 ring-primary/60">
            <CardContent className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
              <div className="min-w-0 space-y-1">
                <div className="flex items-center gap-2">
                  {primary.icon}
                  <span className="text-base font-semibold">{primary.forYou}</span>
                  <Badge variant="success" className="h-5">
                    Detected
                  </Badge>
                </div>
                <p className="text-sm text-muted-foreground">
                  Recommended installer for your platform — {primary.label},{" "}
                  <code className="font-mono text-xs">{primary.ext}</code>.
                </p>
              </div>
              <div className="w-full space-y-2 md:w-80">
                <GuiDownloadControls
                  platform={primary}
                  state={release}
                  primaryVariant="default"
                  primarySize="lg"
                />
              </div>
            </CardContent>
          </Card>
        )}

        {/* Remaining platforms → compact grid (3-up when a platform is promoted, 2×2 otherwise). */}
        <div
          className={cn(
            "grid gap-4",
            primary ? "md:grid-cols-3" : "sm:grid-cols-2",
          )}
        >
          {rest.map((p) => (
            <Card key={p.os}>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  {p.icon}
                  <span className="flex-1">{p.label}</span>
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-3 text-sm">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="muted">{p.ext}</Badge>
                  <Badge variant="warning">Unsigned</Badge>
                </div>
                <GuiDownloadControls platform={p} state={release} />
              </CardContent>
            </Card>
          ))}
        </div>
      </section>

      {/* [SONNET-4.6] sq-vw3ax.11.2 — ONE combined CLI + server section (one archive). */}
      <section className="space-y-4">
        <h2 className="text-xl font-semibold">Command-line &amp; server</h2>
        <p className="measure text-sm text-muted-foreground">
          The same releases carry the standalone{" "}
          <code className="font-mono text-xs">sparq</code> CLI and the{" "}
          <code className="font-mono text-xs">sparq-server</code> HTTP server
          in <strong>one combined archive</strong>, built across a broad OS &times;
          CPU matrix (macOS/Windows/Linux, x86-64 + arm64). Plain executables —
          no signing dialog. The button below fetches the build for{" "}
          <strong>{cli.label}</strong>; every other platform&rsquo;s archive is
          on the{" "}
          <a
            className="text-primary underline underline-offset-4"
            href={RELEASES_URL}
            target="_blank"
            rel="noopener noreferrer"
          >
            Releases page
          </a>
          .
        </p>
        <div className="grid gap-4 md:grid-cols-2">
          <CliServerCard
            state={release}
            cliArchive={cliArchive}
            cliToken={cli.token}
            cliLabel={cli.label}
            cliExt={cli.ext}
          />
        </div>
      </section>

      {/* No-install closer */}
      <section className="space-y-3">
        <h2 className="text-xl font-semibold">No install needed</h2>
        <p className="measure text-sm text-muted-foreground">
          The web playground runs the same engine compiled to WebAssembly,
          entirely in your browser tab — nothing is uploaded, nothing is
          installed.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <Button asChild variant="default" size="sm">
            <a href={withBasePath("/app/")}>
              Open the workbench
              <ExternalLink className="size-4" />
            </a>
          </Button>
          <Badge variant="success" className="h-6 gap-1.5">
            <ShieldCheck className="size-3.5" aria-hidden />
            Runs locally — nothing uploaded
          </Badge>
        </div>
      </section>
    </div>
  );
}
