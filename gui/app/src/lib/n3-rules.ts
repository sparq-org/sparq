import type { WorkspaceRulesDoc } from "@sparq/client";

/** Workspace saves can replace the array without changing any rule document. */
export function sameN3Rules(
  previous: readonly WorkspaceRulesDoc[],
  next: readonly WorkspaceRulesDoc[],
): boolean {
  return previous.length === next.length && previous.every((doc, index) => {
    const other = next[index];
    return doc.name === other.name && doc.text === other.text &&
      doc.addedAt === other.addedAt && (doc.enabled !== false) === (other.enabled !== false);
  });
}
