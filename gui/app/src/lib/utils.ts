import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Merge Tailwind class lists with conflict resolution (the shadcn `cn` convention). */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/**
 * Format a real byte count as a compact human size (B/KB/MB/GB/TB). Pure PRESENTATION of a value
 * the caller measured — it never estimates or fabricates one.
 *
 * [OPUS-5] sq-ixc3.13 — lifted out of status-bar.tsx so the rail's Imports subgroup can report
 * each source's recorded `bytes` with exactly the same rendering as the status bar's disk figure.
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}
