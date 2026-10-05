"use client";

// [OPUS-4.8] sq-ixc3.13 (epic sq-ixc3) — the IMPORT DRAWER: real disk / URL / paste ingest into
// the active workspace's live store via the NATIVE loader (research/gui-design.md §A.4).
//
// (sq-eydh9) [SONNET-4.6] Browser-upload addition: the File tab now works in BOTH personas —
//   * WEB  — multi-file picker via `file-ingest.ts` + drag-and-drop via `dropzone.tsx`;
//            reads the file as binary, decompresses via `@sparq/client` when compressed
//            (.gz/.bz2/.zst/.zip), then parses in-tab. Native-only HDT still needs the
//            desktop app (stated honestly; no silent failure).
//   * TAURI — multi-file native dialog (multiple:true, sq-eydh9 change to tauri-ipc.ts);
//            each path loaded sequentially via the native loader.
//
// (sq-1y04h) [SONNET-4.6] Decompression wiring: the web pane now routes compressed uploads
//   through `maybeDecompressFile` (file-decompress.ts), which calls `decompressDatasetBytes`
//   from `@sparq/client` (sq-epbw4). Codec selection is magic-bytes first, extension second.
//   gzip/zip are native browser APIs; zstd/bzip2 load codec chunks on demand. The import
//   drawer no longer tells web users "compressed files need the desktop app" for these four
//   formats. Native-only HDT still requires the desktop (no JS/WASM bead for it).
//
// [GPT-5.6] sq-n18o5 — the URL tab now fetches response bytes with `arrayBuffer()` and sends
//   them through that same decompression path before RDF parsing. Format detection uses the
//   served RDF Content-Type when available, otherwise the decompressed archive's inner name.
//
// (sq-ljc12) [OPUS-5] The web FILE tab now derives its RDF format from the archive's INNER name
//   too, closing the gap the URL tab already handled. Previously it guessed from the OUTER
//   upload name, which `rdf-format.ts#stripCompression` only unwraps for `.gz`/`.bz2`/`.zst[d]`
//   — so a `bundle.zip` (member `data.nq`) or a `data.nt.gzip` fell back to Turtle and
//   mis-parsed. Each accepted upload now carries the `effectiveName` that `maybeDecompressFile`
//   already computed (the zip member name, or the suffix-stripped name).
//   NOT fixed here: tar-gzip (`.tgz`/`.tbz`). `decompressDatasetBytes` inflates the gzip layer
//   but there is no tar reader, so the result is tar bytes rather than the RDF member — and
//   `innerNameForSource` deliberately returns `null` for those extensions. A name alone cannot
//   fix that; `.tgz` still falls back to Turtle and fails to parse.
//
// Three tabs:
//   * FILE  — web: browser File upload (incl. compressed .gz/.zip/.zst/.bz2); desktop: native
//             loader (every format incl. HDT), off the main thread.
//   * URL   — fetch a remote document, then load it.
//   * PASTE — load a typed/pasted document.

import * as React from "react";
import { Dialog as DialogPrimitive } from "radix-ui";
import {
  Upload,
  FileUp,
  Link2,
  ClipboardPaste,
  X,
  Loader2,
  FolderOpen,
  CheckCircle2,
  AlertTriangle,
  FileWarning,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { useEngine, type ImportKind, type ImportMode } from "@/lib/engine-context";
import { runImportBatch, type BatchSummary } from "@/lib/import-batch";
import { useWorkspace } from "@/lib/workspace-context";
import {
  FORMAT_OPTIONS,
  fileLabel,
  formatFromContentType,
  guessFormat,
  isCompressed,
  isHdt,
  urlLabel,
} from "@/lib/rdf-format";
import { hasNativeLoader, pickRdfFile } from "@/lib/tauri-ipc";
import { hasDraggedFiles } from "@/lib/file-ingest";
import type { IngestedFile, RejectedFile } from "@/lib/file-ingest";
// [SONNET-4.6] sq-1y04h — decompression shim; codec chunks loaded lazily on demand.
import { fetchRdfDocument, maybeDecompressFile } from "@/lib/file-decompress";
import type { WorkspaceSourceMeta } from "@sparq/client";

// ── Open-state context (so the rail / top bar / Cmd-K can open the drawer) ─────────────────────

const ImportDrawerContext = React.createContext<{
  open: boolean;
  setOpen: (open: boolean) => void;
} | null>(null);

/** Open / close the Import drawer from anywhere inside <ImportDrawerProvider>. */
export function useImportDrawer() {
  const ctx = React.useContext(ImportDrawerContext);
  if (!ctx) throw new Error("useImportDrawer must be used within <ImportDrawerProvider>");
  return ctx;
}

export function ImportDrawerProvider({ children }: { children: React.ReactNode }) {
  const [open, setOpen] = React.useState(false);
  const value = React.useMemo(() => ({ open, setOpen }), [open]);
  return (
    <ImportDrawerContext.Provider value={value}>
      {children}
      <ImportDrawer open={open} onOpenChange={setOpen} />
    </ImportDrawerContext.Provider>
  );
}

// ── Per-file import result (shown in the FilePane while/after a batch runs) ───────────────────

type FileItemStatus =
  | { kind: "ok"; added: number }
  | { kind: "error"; message: string };

// ── Web-upload ingest contract (decompress-aware superset of `IngestResult`) ───────────────────
// (sq-ljc12) The shared `file-ingest.ts` result is TEXT-only and has no notion of an archive, so
// the web File tab carries its own shape: the same fields plus the `effectiveName` that
// `maybeDecompressFile` derived (zip member name / suffix-stripped name / the original name when
// the upload was not compressed). That name — NOT the outer upload name — drives `guessFormat`.

interface WebUploadFile extends IngestedFile {
  /** The inner document name to detect the RDF format from. */
  effectiveName: string;
}

interface WebUploadResult {
  accepted: WebUploadFile[];
  rejected: RejectedFile[];
}

// ── RDF extensions the browser file picker + drop filter allow ─────────────────────────────────
// [SONNET-4.6] sq-1y04h — compressed archives (.gz/.gzip/.zip/.zst/.zstd/.bz2/.bzip2) are now
// accepted on the web persona too; `maybeDecompressFile` handles the decode before RDF parse.
// Native-only HDT is NOT included — there is no JS/WASM decoder bead for it.

const WEB_RDF_ACCEPT = [
  ".ttl", ".nt", ".nq", ".trig", ".jsonld", ".json",
  ".gz", ".gzip", ".tgz", ".zip", ".zst", ".zstd", ".bz2", ".bzip2",
];

// ── The drawer body ────────────────────────────────────────────────────────────────────────────

type Tab = "file" | "url" | "paste";

interface FeedbackOk {
  kind: "ok";
  message: string;
}
interface FeedbackErr {
  kind: "error";
  message: string;
}
type Feedback = FeedbackOk | FeedbackErr | null;

/**
 * (sq-810a0) Build the batch-import completion feedback from a {@link BatchSummary}.
 *
 * Beyond the ok/failed counts, it now SIGNALS when a `replace` the user selected was NOT applied
 * because every file that could have replaced the store failed — the old dataset was left intact.
 * Before this fix that outcome was silent (the user could believe the store had been cleared).
 */
function batchFeedback(summary: BatchSummary): Feedback {
  const { okCount, errCount, totalAdded, replaceRequested, replaceApplied } = summary;
  const quads = `${totalAdded.toLocaleString()} quad${totalAdded === 1 ? "" : "s"}`;
  // Replace was asked for but never landed on a file (all replace-eligible files failed).
  const replaceNotApplied = replaceRequested && !replaceApplied && okCount > 0;

  if (errCount === 0) {
    return {
      kind: "ok",
      message: `Imported ${okCount} file${okCount === 1 ? "" : "s"} — ${quads} added.`,
    };
  }
  if (okCount > 0) {
    const replaceNote = replaceNotApplied
      ? " The replace was NOT applied (the file(s) meant to replace the store failed); existing data was kept and the imported file(s) were added."
      : "";
    return {
      kind: "error",
      message: `${okCount} file${okCount === 1 ? "" : "s"} imported (${quads}); ${errCount} file${errCount === 1 ? "" : "s"} failed — see errors above.${replaceNote}`,
    };
  }
  return {
    kind: "error",
    message: `All ${errCount} file${errCount === 1 ? "" : "s"} failed — see errors above.${replaceRequested ? " The store was left unchanged (replace not applied)." : ""}`,
  };
}

function ImportDrawer({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { importRdf, snapshotStore, refreshDiskUsage } = useEngine();
  const { recordImport } = useWorkspace();
  const native = hasNativeLoader();

  const [tab, setTab] = React.useState<Tab>(native ? "file" : "paste");
  const [busy, setBusy] = React.useState(false);
  const [feedback, setFeedback] = React.useState<Feedback>(null);

  // Shared controls.
  const [mode, setMode] = React.useState<ImportMode>("add");
  const [preserveGraphs, setPreserveGraphs] = React.useState(true);

  // ── File tab — per-persona state ─────────────────────────────────────────────────────────────
  // Desktop: list of paths from the multi-file native dialog.
  const [nativePaths, setNativePaths] = React.useState<string[]>([]);
  // Browser: files already read from disk (and decompressed) by `readFilesWithDecompress`.
  const [webFiles, setWebFiles] = React.useState<WebUploadFile[]>([]);
  const [webRejected, setWebRejected] = React.useState<RejectedFile[]>([]);
  // Per-file import status (populated while/after the batch runs).
  const [fileStatuses, setFileStatuses] = React.useState<Record<string, FileItemStatus>>({});

  // URL tab.
  const [url, setUrl] = React.useState<string>("");
  // Paste tab.
  const [pasteText, setPasteText] = React.useState<string>("");
  const [pasteFormat, setPasteFormat] = React.useState<string>("turtle");

  // Reset transient state whenever the drawer is (re)opened.
  React.useEffect(() => {
    if (open) {
      setFeedback(null);
      setBusy(false);
      setTab(native ? "file" : "paste");
      setNativePaths([]);
      setWebFiles([]);
      setWebRejected([]);
      setFileStatuses({});
    }
  }, [open, native]);

  // ── Single-file finish helper (URL / Paste) ────────────────────────────────────────────────

  const finishImport = React.useCallback(
    async (
      sourceKind: ImportKind,
      run: () => Promise<{ source: WorkspaceSourceMeta; storeSize: number; added: number }>,
    ) => {
      setBusy(true);
      setFeedback(null);
      try {
        const { source, storeSize, added } = await run();
        await recordImport(source, snapshotStore());
        refreshDiskUsage();
        setFeedback({
          kind: "ok",
          message: `Imported ${added.toLocaleString()} quad${added === 1 ? "" : "s"} (${sourceKind}). Store now holds ${storeSize.toLocaleString()}.`,
        });
      } catch (err) {
        setFeedback({
          kind: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setBusy(false);
      }
    },
    [recordImport, snapshotStore, refreshDiskUsage],
  );

  // ── FILE (WEB) — multi-file browser upload: per-file in-tab WASM parse ─────────────────────

  const onIngestWebFiles = React.useCallback((result: WebUploadResult) => {
    setWebFiles((prev) => {
      // De-duplicate by name; later picks win.
      const byName = new Map(prev.map((f) => [f.name, f]));
      for (const f of result.accepted) byName.set(f.name, f);
      return Array.from(byName.values());
    });
    setWebRejected((prev) => {
      const byName = new Map(prev.map((r) => [r.name, r]));
      for (const r of result.rejected) byName.set(r.name, r);
      return Array.from(byName.values());
    });
    setFileStatuses({});
    setFeedback(null);
  }, []);

  const onImportWebFiles = React.useCallback(() => {
    if (webFiles.length === 0) return;
    setBusy(true);
    setFeedback(null);
    setFileStatuses({});

    void (async () => {
      // Sequence the batch: the selected mode is applied to the FIRST file that imports
      // SUCCESSFULLY (not to array index 0), 'add' thereafter (sq-810a0). Update each row
      // incrementally so it lights up as it completes.
      const { summary } = await runImportBatch(
        webFiles,
        mode,
        (file) => file.name,
        (file, fileMode) =>
          importRdf({
            kind: "paste",
            mode: fileMode,
            preserveGraphs,
            label: file.name,
            // (sq-ljc12) The INNER document name — `data.nq` for a `bundle.zip` carrying it —
            // so a compressed upload is parsed as its real serialisation, not as the fallback.
            format: guessFormat(file.effectiveName),
            text: file.text,
          }),
        (_key, _status, statuses) => setFileStatuses({ ...statuses }),
      );

      setFeedback(batchFeedback(summary));

      // Record the batch in workspace history.
      if (summary.okCount > 0) {
        const label =
          webFiles.length === 1 ? fileLabel(webFiles[0].name) : `${webFiles.length} files`;
        await recordImport(
          { kind: "local", label, format: "mixed", bytes: 0, importedAt: Date.now() },
          snapshotStore(),
        );
        refreshDiskUsage();
      }
    })().finally(() => setBusy(false));
  }, [webFiles, mode, preserveGraphs, importRdf, recordImport, snapshotStore, refreshDiskUsage]);

  // ── FILE (NATIVE) — multi-path desktop import ────────────────────────────────────────────────

  const onPickNativeFiles = React.useCallback(async () => {
    const paths = await pickRdfFile();
    if (paths.length > 0) {
      setNativePaths(paths);
      setFileStatuses({});
      setFeedback(null);
    }
  }, []);

  const onImportNativeFiles = React.useCallback(() => {
    if (nativePaths.length === 0) return;
    setBusy(true);
    setFeedback(null);
    setFileStatuses({});

    void (async () => {
      // Sequence the batch: the selected mode is applied to the FIRST path that imports
      // SUCCESSFULLY (not to array index 0), 'add' thereafter (sq-810a0). Each successful path is
      // recorded in workspace history inside the importer callback, so a failed leading file that
      // does NOT clear the store is never recorded — mirroring the pre-fix per-success recording.
      const { summary } = await runImportBatch(
        nativePaths,
        mode,
        (path) => path,
        async (path, fileMode) => {
          const label = fileLabel(path);
          const result = await importRdf({
            kind: "file",
            mode: fileMode,
            preserveGraphs,
            label,
            format: guessFormat(path),
            path,
          });
          await recordImport(
            {
              kind: "local",
              label,
              format: result.format,
              bytes: result.bytes,
              importedAt: Date.now(),
            },
            snapshotStore(),
          );
          refreshDiskUsage();
          return { added: result.added };
        },
        (_key, _status, statuses) => setFileStatuses({ ...statuses }),
      );

      setFeedback(batchFeedback(summary));
    })().finally(() => setBusy(false));
  }, [nativePaths, mode, preserveGraphs, importRdf, recordImport, snapshotStore, refreshDiskUsage]);

  // ── URL ────────────────────────────────────────────────────────────────────────────────────────

  const onImportUrl = React.useCallback(() => {
    const target = url.trim();
    if (!target) return;
    void finishImport("url", async () => {
      // [GPT-5.6] sq-n18o5 — keep compressed response bytes intact until after codec detection.
      const document = await fetchRdfDocument(target);
      const format =
        formatFromContentType(document.contentType) ?? guessFormat(document.effectiveName);
      const result = await importRdf({
        kind: "url",
        mode,
        preserveGraphs,
        label: urlLabel(target),
        format,
        text: document.text,
        url: target,
      });
      const source: WorkspaceSourceMeta = {
        kind: "url",
        label: urlLabel(target),
        url: target,
        format: result.format,
        bytes: result.bytes,
        importedAt: Date.now(),
      };
      return { source, storeSize: result.storeSize, added: result.added };
    });
  }, [url, finishImport, importRdf, mode, preserveGraphs]);

  // ── PASTE ────────────────────────────────────────────────────────────────────────────────────

  const onImportPaste = React.useCallback(() => {
    const text = pasteText.trim();
    if (!text) return;
    void finishImport("paste", async () => {
      const result = await importRdf({
        kind: "paste",
        mode,
        preserveGraphs,
        label: "pasted document",
        format: pasteFormat,
        text,
      });
      const source: WorkspaceSourceMeta = {
        kind: "local",
        label: "pasted document",
        format: result.format,
        bytes: result.bytes,
        importedAt: Date.now(),
      };
      return { source, storeSize: result.storeSize, added: result.added };
    });
  }, [pasteText, pasteFormat, finishImport, importRdf, mode, preserveGraphs]);

  // ── Shared import button enablement ─────────────────────────────────────────────────────────

  const importDisabled =
    busy ||
    (tab === "file" && native && nativePaths.length === 0) ||
    (tab === "file" && !native && webFiles.length === 0) ||
    (tab === "url" && !url.trim()) ||
    (tab === "paste" && !pasteText.trim());

  const onImport =
    tab === "url"
      ? onImportUrl
      : tab === "paste"
        ? onImportPaste
        : native
          ? onImportNativeFiles
          : onImportWebFiles;

  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay
          className={cn(
            "fixed inset-0 z-50 bg-black/40 backdrop-blur-sm",
            "data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0",
          )}
        />
        <DialogPrimitive.Content
          data-import-drawer
          className={cn(
            "bg-card text-card-foreground fixed right-0 top-0 z-50 flex h-full w-[min(28rem,calc(100vw-2rem))] flex-col",
            "border-l shadow-2xl",
            "data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:slide-out-to-right data-[state=open]:slide-in-from-right",
          )}
        >
          <div className="flex items-center gap-2 border-b px-4 py-3">
            <Upload className="size-4 text-primary" aria-hidden />
            <DialogPrimitive.Title className="flex-1 text-sm font-semibold">
              Import RDF into the workspace
            </DialogPrimitive.Title>
            <DialogPrimitive.Close asChild>
              <Button variant="ghost" size="icon-sm" aria-label="Close import drawer">
                <X className="size-4" />
              </Button>
            </DialogPrimitive.Close>
          </div>
          <DialogPrimitive.Description className="sr-only">
            Load RDF from a disk file (compressed and HDT supported on the desktop app), from a
            URL, or by pasting a document, into the active workspace&rsquo;s live store.
          </DialogPrimitive.Description>

          {/* Tab strip. */}
          <div className="flex border-b text-xs" role="tablist">
            <DrawerTab id="file" current={tab} onSelect={setTab} icon={FileUp} label="File" />
            <DrawerTab id="url" current={tab} onSelect={setTab} icon={Link2} label="URL" />
            <DrawerTab
              id="paste"
              current={tab}
              onSelect={setTab}
              icon={ClipboardPaste}
              label="Paste"
            />
          </div>

          <div className="flex-1 overflow-y-auto p-4">
            {tab === "file" && native && (
              <NativeFilePane
                paths={nativePaths}
                onPick={onPickNativeFiles}
                onClear={(p) => setNativePaths((prev) => prev.filter((x) => x !== p))}
                fileStatuses={fileStatuses}
                busy={busy}
              />
            )}
            {tab === "file" && !native && (
              <WebFilePane
                files={webFiles}
                rejected={webRejected}
                onIngest={onIngestWebFiles}
                onRemove={(name) => {
                  setWebFiles((prev) => prev.filter((f) => f.name !== name));
                  setFileStatuses((prev) => {
                    const next = { ...prev };
                    delete next[name];
                    return next;
                  });
                }}
                fileStatuses={fileStatuses}
                busy={busy}
              />
            )}
            {tab === "url" && <UrlPane url={url} onUrl={setUrl} />}
            {tab === "paste" && (
              <PastePane
                text={pasteText}
                onText={setPasteText}
                format={pasteFormat}
                onFormat={setPasteFormat}
              />
            )}
          </div>

          {/* Shared options + actions. */}
          <div className="space-y-3 border-t p-4">
            <OptionsRow
              mode={mode}
              onMode={setMode}
              preserveGraphs={preserveGraphs}
              onPreserveGraphs={setPreserveGraphs}
            />

            {feedback && (
              <p
                role="status"
                data-import-feedback={feedback.kind}
                className={cn(
                  "flex items-start gap-2 rounded-md border px-3 py-2 text-xs",
                  feedback.kind === "ok"
                    ? "border-[var(--success)]/40 text-[var(--success)]"
                    : "border-destructive/40 text-destructive",
                )}
              >
                {feedback.kind === "ok" ? (
                  <CheckCircle2 className="mt-0.5 size-3.5 shrink-0" />
                ) : (
                  <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
                )}
                <span>{feedback.message}</span>
              </p>
            )}

            <Button className="w-full" disabled={importDisabled} onClick={onImport}>
              {busy ? (
                <>
                  <Loader2 className="size-4 animate-spin" /> Importing…
                </>
              ) : (
                <>
                  <Upload className="size-4" /> Import{" "}
                  {mode === "add" ? "(add to store)" : "(replace store)"}
                </>
              )}
            </Button>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

// ── Panes ────────────────────────────────────────────────────────────────────────────────────

function DrawerTab({
  id,
  current,
  onSelect,
  icon: Icon,
  label,
}: {
  id: Tab;
  current: Tab;
  onSelect: (t: Tab) => void;
  icon: typeof FileUp;
  label: string;
}) {
  const active = id === current;
  return (
    <button
      role="tab"
      aria-selected={active}
      data-import-tab={id}
      onClick={() => onSelect(id)}
      className={cn(
        "flex flex-1 items-center justify-center gap-1.5 border-b-2 px-3 py-2 font-medium",
        active
          ? "border-primary text-foreground"
          : "border-transparent text-muted-foreground hover:text-foreground",
      )}
    >
      <Icon className="size-3.5" /> {label}
    </button>
  );
}

// ── Web file read helper (decompress-aware) ────────────────────────────────────────────────────

/**
 * [SONNET-4.6] sq-1y04h — read a list of `File` objects into a {@link WebUploadResult}, routing
 * each through `maybeDecompressFile` so compressed archives (.gz / .zip / .zst / .bz2) are decoded
 * before the text is handed to the RDF parser. Replaces the `readDroppedFiles` / `File.text()`
 * call paths in the web-upload pane; the `file-ingest.ts` text-only paths are unchanged and
 * continue to serve SHACL shapes / N3 rules (no compression needed there).
 *
 * (sq-ljc12) Keeps `effectiveName` so the caller guesses the RDF format from the archive's INNER
 * document (`bundle.zip` → member `data.nq` → nquads), not the container's extension.
 */
async function readFilesWithDecompress(files: File[]): Promise<WebUploadResult> {
  const accepted: WebUploadFile[] = [];
  const rejected: RejectedFile[] = [];
  for (const file of files) {
    try {
      const decoded = await maybeDecompressFile(file);
      accepted.push({
        name: file.name,
        text: decoded.text,
        bytes: file.size,
        effectiveName: decoded.effectiveName,
      });
    } catch (err) {
      rejected.push({
        name: file.name,
        reason: err instanceof Error ? err.message : String(err),
      });
    }
  }
  return { accepted, rejected };
}

// ── Web FilePane — browser multi-file upload ──────────────────────────────────────────────────

function WebFilePane({
  files,
  rejected,
  onIngest,
  onRemove,
  fileStatuses,
  busy,
}: {
  files: WebUploadFile[];
  rejected: RejectedFile[];
  onIngest: (result: WebUploadResult) => void;
  onRemove: (name: string) => void;
  fileStatuses: Record<string, FileItemStatus>;
  busy: boolean;
}) {
  // Stable <input type="file"> in the DOM so Playwright setInputFiles() can target it.
  // The Dropzone "Browse files…" button uses pickTextFiles (a dynamic hidden input) for the
  // keyboard/mouse path. This input is an additional test-accessible hook and accessible label.
  const inputRef = React.useRef<HTMLInputElement>(null);
  // [SONNET-4.6] sq-1y04h — custom drop state: we handle the drop event ourselves (instead of
  // delegating to Dropzone's readDroppedFiles) so that compressed archives are decoded BEFORE
  // the RDF parser sees them. Dropzone's internal readDroppedFiles uses File.text() which
  // corrupts binary payloads; here we extract File objects synchronously from the drop event
  // and pass them through readFilesWithDecompress.
  const [dragActive, setDragActive] = React.useState(false);
  const dragDepth = React.useRef(0);

  const handleInputChange = React.useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const fileArr = Array.from(e.target.files ?? []);
      if (fileArr.length === 0) return;
      // Reset so the same filenames can be re-picked after a remove.
      if (inputRef.current) inputRef.current.value = "";
      // [SONNET-4.6] sq-1y04h — readFilesWithDecompress routes each file through
      // maybeDecompressFile (binary read + codec detection + decode) before handing text to
      // the RDF parser; uncompressed files fall through with File.arrayBuffer() semantics.
      const result = await readFilesWithDecompress(fileArr);
      onIngest(result);
    },
    [onIngest],
  );

  // [SONNET-4.6] sq-1y04h — custom drag handlers that bypass the text-read path.
  const handleDragEnter = React.useCallback((e: React.DragEvent<HTMLDivElement>) => {
    if (busy || !hasDraggedFiles(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth.current += 1;
    setDragActive(true);
  }, [busy]);

  const handleDragOver = React.useCallback((e: React.DragEvent<HTMLDivElement>) => {
    if (busy || !hasDraggedFiles(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  }, [busy]);

  const handleDragLeave = React.useCallback((e: React.DragEvent<HTMLDivElement>) => {
    if (busy || !hasDraggedFiles(e.dataTransfer)) return;
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setDragActive(false);
  }, [busy]);

  const handleDrop = React.useCallback((e: React.DragEvent<HTMLDivElement>) => {
    if (busy || !hasDraggedFiles(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth.current = 0;
    setDragActive(false);
    // Extract File objects synchronously before the first await — browsers neuter the
    // DataTransfer items list once this handler yields.
    const files: File[] = [];
    const rejected: RejectedFile[] = [];
    if (e.dataTransfer.items?.length) {
      for (const item of Array.from(e.dataTransfer.items)) {
        if (item.kind !== "file") continue;
        const entry = typeof item.webkitGetAsEntry === "function" ? item.webkitGetAsEntry() : null;
        const file = item.getAsFile();
        if (entry?.isDirectory) {
          rejected.push({ name: entry.name || file?.name || "(directory)", reason: "directories cannot be ingested — drop the files inside it instead" });
          continue;
        }
        if (!file) {
          rejected.push({ name: entry?.name ?? "(unreadable item)", reason: "the browser could not expose this dropped item as a file" });
          continue;
        }
        files.push(file);
      }
    } else {
      files.push(...Array.from(e.dataTransfer.files ?? []));
    }
    void readFilesWithDecompress(files).then((res) => {
      onIngest({ accepted: res.accepted, rejected: [...rejected, ...res.rejected] });
    });
  }, [busy, onIngest]);

  const handleBrowse = React.useCallback(async () => {
    // pickTextFiles opens a file picker; we then re-read the File objects as binary so
    // compressed archives are handled. We can't get File objects back from pickTextFiles,
    // so instead we open our own hidden input programmatically (same pattern as pickViaInput).
    if (inputRef.current) inputRef.current.click();
  }, []);

  return (
    <div className="space-y-3">
      {/* [SONNET-4.6] sq-1y04h — updated copy: compressed archives are now handled in-tab.
          Only native-only HDT still requires the desktop app (no JS decoder bead). */}
      <p className="text-xs text-muted-foreground">
        Upload RDF files from your computer — Turtle, N-Triples, N-Quads, TriG, JSON-LD, or a
        compressed archive (.gz / .zip / .zst / .bz2). Each file is read and decompressed
        in-tab by the WASM engine.{" "}
        <span className="font-medium text-foreground">
          Native-only HDT archives (.hdt) still require the desktop app.
        </span>
      </p>

      {/* Stable file input: Playwright setInputFiles() target + accessible fallback.
          Also the backing input for the "Browse files…" button below (handleBrowse → click). */}
      <input
        ref={inputRef}
        type="file"
        multiple
        accept={WEB_RDF_ACCEPT.join(",")}
        data-web-file-input
        aria-label="Select RDF files to import"
        className="sr-only"
        disabled={busy}
        onChange={handleInputChange}
      />

      {/* [SONNET-4.6] sq-1y04h — custom drop zone panel: we handle drag/drop ourselves so that
          Files are read as binary (maybeDecompressFile) rather than via readDroppedFiles/text(). */}
      <div
        role="region"
        aria-label="File drop zone"
        onDragEnter={handleDragEnter}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        className={cn(
          "relative rounded-md border border-dashed p-5 text-center transition-colors",
          dragActive ? "border-primary bg-primary/5" : "border-border",
          busy && "opacity-50",
        )}
      >
        <FileUp className="mx-auto size-6 text-muted-foreground" aria-hidden />
        <p className="mt-2 text-sm">Drag &amp; drop RDF files here</p>
        <p className="mt-1 text-xs text-muted-foreground">
          .ttl · .nt · .nq · .trig · .jsonld · .gz · .zip · .zst · .bz2
        </p>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="mt-3"
          onClick={handleBrowse}
          disabled={busy}
        >
          <Upload className="size-4" /> Browse files…
        </Button>
        <span className="sr-only" role="status">
          {dragActive ? "Release to drop the files" : ""}
        </span>
      </div>

      {/* Per-file list: queued + status (appears after first pick/drop). */}
      {(files.length > 0 || rejected.length > 0) && (
        <ul className="space-y-1.5" aria-label="Selected files">
          {files.map((f) => (
            <WebFileRow
              key={f.name}
              name={f.name}
              format={guessFormat(f.effectiveName)}
              bytes={f.bytes}
              status={fileStatuses[f.name] ?? null}
              onRemove={busy ? undefined : () => onRemove(f.name)}
            />
          ))}
          {rejected.map((r) => (
            <li
              key={r.name}
              className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-2.5 py-1.5 text-xs"
            >
              <FileWarning
                className="mt-0.5 size-3.5 shrink-0 text-destructive"
                aria-hidden
              />
              <span className="min-w-0 flex-1">
                <span className="break-all font-mono">{r.name}</span>
                <span className="block text-muted-foreground">{r.reason}</span>
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function WebFileRow({
  name,
  format,
  bytes,
  status,
  onRemove,
}: {
  name: string;
  /** (sq-ljc12) Guessed from the archive's inner document name, so the preview matches the parse. */
  format: string;
  bytes: number;
  status: FileItemStatus | null;
  onRemove?: () => void;
}) {
  const fmt = new Intl.NumberFormat("en", { notation: "compact" });
  return (
    <li
      data-file-row={name}
      className={cn(
        "flex items-start gap-2 rounded-md border px-2.5 py-1.5 text-xs",
        status?.kind === "ok" && "border-[var(--success)]/30 bg-[var(--success)]/5",
        status?.kind === "error" && "border-destructive/30 bg-destructive/5",
        !status && "border-border bg-background",
      )}
    >
      {status?.kind === "ok" ? (
        <CheckCircle2
          className="mt-0.5 size-3.5 shrink-0 text-[var(--success)]"
          aria-hidden
        />
      ) : status?.kind === "error" ? (
        <AlertTriangle className="mt-0.5 size-3.5 shrink-0 text-destructive" aria-hidden />
      ) : (
        <FileUp className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" aria-hidden />
      )}
      <span className="min-w-0 flex-1">
        <span className="break-all font-mono">{name}</span>
        {status?.kind === "ok" && (
          <span className="block text-[var(--success)]">
            {status.added.toLocaleString()} quad{status.added === 1 ? "" : "s"} imported
          </span>
        )}
        {status?.kind === "error" && (
          <span className="block text-destructive" data-file-error={name}>
            {status.message}
          </span>
        )}
        {!status && (
          <span className="block text-muted-foreground">
            {fmt.format(bytes)}&nbsp;B · {format}
          </span>
        )}
      </span>
      {onRemove && !status && (
        <button
          onClick={onRemove}
          aria-label={`Remove ${name}`}
          className="shrink-0 text-muted-foreground hover:text-foreground"
        >
          <X className="size-3.5" />
        </button>
      )}
    </li>
  );
}

// ── Native FilePane — desktop multi-file import ───────────────────────────────────────────────

function NativeFilePane({
  paths,
  onPick,
  onClear,
  fileStatuses,
  busy,
}: {
  paths: string[];
  onPick: () => void;
  onClear: (path: string) => void;
  fileStatuses: Record<string, FileItemStatus>;
  busy: boolean;
}) {
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        Load RDF files through the native engine — Turtle, N-Triples, N-Quads, TriG, JSON-LD,
        including <strong>compressed</strong> (.gz / .bz2 / .zst) streams and{" "}
        <strong>native-only HDT</strong> (.hdt / .hdt.gz) archives. Multiple files are imported
        sequentially. Named graphs are preserved.
      </p>
      <Button variant="outline" className="w-full" onClick={onPick} disabled={busy}>
        <FolderOpen className="size-4" /> Choose files…
      </Button>
      {paths.length > 0 && (
        <ul className="space-y-1.5" aria-label="Selected files">
          {paths.map((p) => {
            const status = fileStatuses[p] ?? null;
            return (
              <li
                key={p}
                className={cn(
                  "flex items-start gap-2 rounded-md border px-3 py-2 text-xs",
                  status?.kind === "ok" && "border-[var(--success)]/30 bg-[var(--success)]/5",
                  status?.kind === "error" && "border-destructive/30 bg-destructive/5",
                  !status && "border-border bg-background",
                )}
              >
                {status?.kind === "ok" ? (
                  <CheckCircle2
                    className="mt-0.5 size-3.5 shrink-0 text-[var(--success)]"
                    aria-hidden
                  />
                ) : status?.kind === "error" ? (
                  <AlertTriangle
                    className="mt-0.5 size-3.5 shrink-0 text-destructive"
                    aria-hidden
                  />
                ) : (
                  <FileUp
                    className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
                    aria-hidden
                  />
                )}
                <span className="min-w-0 flex-1">
                  <span className="break-all font-mono">{fileLabel(p)}</span>
                  {status?.kind === "ok" && (
                    <span className="block text-[var(--success)]">
                      {status.added.toLocaleString()} quads imported
                    </span>
                  )}
                  {status?.kind === "error" && (
                    <span className="block text-destructive">{status.message}</span>
                  )}
                  {!status && (
                    <span className="block text-muted-foreground">
                      {guessFormat(p)}
                      {isCompressed(p) && !isHdt(p) && " · compressed"}
                      {isHdt(p) && " · native HDT"}
                    </span>
                  )}
                </span>
                {!busy && !status && (
                  <button
                    onClick={() => onClear(p)}
                    aria-label={`Remove ${fileLabel(p)}`}
                    className="shrink-0 text-muted-foreground hover:text-foreground"
                  >
                    <X className="size-3.5" />
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function UrlPane({ url, onUrl }: { url: string; onUrl: (v: string) => void }) {
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted-foreground">
        Fetch an RDF document, including a compressed archive, from a URL and load it into the
        store. The source is recorded as a re-fetchable workspace source. The format is
        auto-detected from the served <code>Content-Type</code> (or the inner file extension).
      </p>
      <input
        type="url"
        inputMode="url"
        placeholder="https://example.org/data.ttl"
        value={url}
        onChange={(e) => onUrl(e.target.value)}
        className="w-full rounded-md border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      />
      {url.trim() && (
        <FormatNote
          label={urlLabel(url)}
          format={guessFormat(url)}
          compressed={isCompressed(url)}
          hdt={isHdt(url)}
        />
      )}
    </div>
  );
}

function PastePane({
  text,
  onText,
  format,
  onFormat,
}: {
  text: string;
  onText: (v: string) => void;
  format: string;
  onFormat: (v: string) => void;
}) {
  // HDT is a BINARY archive format — it cannot be pasted as text, so it is never a paste option.
  const options = FORMAT_OPTIONS.filter((f) => f.value !== "hdt");
  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <label
          htmlFor="import-paste-format"
          className="text-xs font-medium text-muted-foreground"
        >
          Format
        </label>
        <select
          id="import-paste-format"
          value={format}
          onChange={(e) => onFormat(e.target.value)}
          className="rounded-md border bg-background px-2 py-1 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          {options.map((f) => (
            <option key={f.value} value={f.value}>
              {f.label}
            </option>
          ))}
        </select>
      </div>
      <textarea
        value={text}
        onChange={(e) => onText(e.target.value)}
        placeholder={"<http://example.org/a> <http://example.org/p> <http://example.org/b> ."}
        spellCheck={false}
        className="h-48 w-full resize-y rounded-md border bg-background px-3 py-2 font-mono text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      />
    </div>
  );
}

function FormatNote({
  label,
  format,
  compressed,
  hdt,
}: {
  label: string;
  format: string;
  compressed: boolean;
  hdt: boolean;
}) {
  return (
    <p className="text-[11px] text-muted-foreground">
      <span className="font-mono">{label}</span> → parsed as{" "}
      <span className="font-medium text-foreground">{format}</span>
      {compressed && !hdt && " (decompressed natively)"}
      {hdt && " (native-only HDT)"}.
    </p>
  );
}

function OptionsRow({
  mode,
  onMode,
  preserveGraphs,
  onPreserveGraphs,
}: {
  mode: ImportMode;
  onMode: (m: ImportMode) => void;
  preserveGraphs: boolean;
  onPreserveGraphs: (v: boolean) => void;
}) {
  return (
    <div className="space-y-2 text-xs">
      <div className="flex items-center gap-2">
        <span className="font-medium text-muted-foreground">Mode</span>
        <div className="inline-flex overflow-hidden rounded-md border">
          <ModeButton current={mode} value="add" label="Add to store" onSelect={onMode} />
          <ModeButton current={mode} value="replace" label="Replace store" onSelect={onMode} />
        </div>
      </div>
      <label className="flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          checked={preserveGraphs}
          onChange={(e) => onPreserveGraphs(e.target.checked)}
          className="size-3.5 accent-[var(--primary)]"
        />
        <span className="text-muted-foreground">
          Preserve named graphs (route quad formats through the dataset loader)
        </span>
      </label>
    </div>
  );
}

function ModeButton({
  current,
  value,
  label,
  onSelect,
}: {
  current: ImportMode;
  value: ImportMode;
  label: string;
  onSelect: (m: ImportMode) => void;
}) {
  const active = current === value;
  return (
    <button
      onClick={() => onSelect(value)}
      className={cn(
        "px-2.5 py-1",
        active ? "bg-primary text-primary-foreground" : "bg-background hover:bg-accent",
      )}
    >
      {label}
    </button>
  );
}
