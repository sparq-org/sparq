// [OPUS-5] sq-ixc3.13 — the Imports list's source-identity rule.
//
// `WorkspaceSourceMeta` (packages/sparq-client) documents a `url` source as RE-FETCHABLE: the
// workspace persists the URL precisely so the rail can pull it again. Re-fetching the same URL is
// therefore an UPDATE of that source (a fresher `importedAt` / `bytes` / `format`), not a second
// source — appending would grow a duplicate rail row on every refresh.
//
// A `local` source has no such identity: the browser cannot re-read a previously chosen disk file
// across sessions (no persistent handle), and two picks of the same filename may be different
// files, so local imports always append.

import type { WorkspaceSourceMeta } from "@sparq/client";

/**
 * Add `next` to the workspace's source list, REPLACING an existing entry that denotes the same
 * source rather than appending a duplicate.
 *
 * Two entries denote the same source iff both are `kind: "url"` and carry the same (non-empty)
 * `url`; the replacement keeps the original list POSITION so the rail does not reorder under a
 * re-fetch. Everything else — every `local` import, and a `url` entry with no recorded URL —
 * appends.
 *
 * Pure: returns a NEW array and never mutates `prev` (safe as a React state updater).
 */
export function upsertSource(
  prev: readonly WorkspaceSourceMeta[],
  next: WorkspaceSourceMeta,
): WorkspaceSourceMeta[] {
  const key = sourceKey(next);
  if (key === null) return [...prev, next];
  const at = prev.findIndex((s) => sourceKey(s) === key);
  if (at < 0) return [...prev, next];
  const merged = [...prev];
  merged[at] = next;
  return merged;
}

/** The re-fetch identity of a source, or `null` when it has none (every `local` import). */
function sourceKey(source: WorkspaceSourceMeta): string | null {
  if (source.kind !== "url") return null;
  const url = source.url?.trim() ?? "";
  return url === "" ? null : url;
}
