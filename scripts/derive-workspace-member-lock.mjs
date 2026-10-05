#!/usr/bin/env node
// [OPUS-4.8] sq-jpki.1 — derive a SELF-CONTAINED, standalone npm lockfile for ONE
// workspace member out of the repo-root `package-lock.json`, WITHOUT a network install.
//
// WHY: after the repo-root npm-workspaces migration there is a SINGLE root
// `package-lock.json` (the per-member locks were dropped as redundant). But
// `scripts/gen-js-sbom.sh` runs `cyclonedx-npm --package-lock-only` from `js/`, which
// requires a lockfile co-located with `js/package.json` AND must NOT hit the network (the
// supply-chain `js-sbom` lane is deliberately install-free + deterministic). This script
// projects the member's dependency CLOSURE out of the committed root lock into a temporary
// standalone lockfile whose root component is the member itself (e.g. `@sparq-org/sparq`), so
// the SBOM stays anchored at the published client — exactly as it did pre-migration with a
// committed per-package lock — while preserving the EXACT versions the root lock pins (no
// `^`-range re-resolution, no registry call). The emitted lock is TRANSIENT scratch (the
// caller removes it); it is never committed (the root lock is the single source of truth).
//
// Usage:  node scripts/derive-workspace-member-lock.mjs <member-dir> <out-lock-path>
//   e.g.  node scripts/derive-workspace-member-lock.mjs js js/package-lock.json
//
// Assumptions (asserted): the root lock is lockfileVersion 3 and every dependency in the
// member's closure is hoisted to the root `node_modules/<name>` (npm's default). If a dep
// were NESTED under `<member>/node_modules/<name>` (a version conflict), this script also
// looks there and re-homes it to top-level in the standalone lock; an UNRESOLVED dep is a
// hard error (so a future hoisting change fails LOUDLY rather than silently dropping a
// component from the SBOM).
//
// [OPUS-4.8] sq-f04e: a workspace-to-workspace dep edge (a `link: true` node, e.g. a member
// depending on a sibling `@sparq/client` in `packages/*`) is handled by MATERIALIZING the link
// target's real package node AND its own nested closure into the standalone lock (see
// `materializeLinkTarget`). Previously such a dep was re-homed VERBATIM as a dangling link stub,
// which made `npm ls` (and thus cyclonedx-npm, incl. its runtime `--omit=dev` view) crash with the
// cryptic "Cannot read properties of undefined (reading 'extraneous')" -> exit 254. Since a
// workspace link is a dev-graph edge, this materialization changes the FULL (dev) tree but never
// the RUNTIME (`--omit=dev`) set when the link is a devDependency of the member.
import fs from "node:fs";
import path from "node:path";

const [, , memberDir, outPath] = process.argv;
if (!memberDir || !outPath) {
  console.error("usage: derive-workspace-member-lock.mjs <member-dir> <out-lock-path>");
  process.exit(2);
}

const repoRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const rootLockPath = path.join(repoRoot, "package-lock.json");
if (!fs.existsSync(rootLockPath)) {
  console.error(`ERROR: root lockfile not found at ${rootLockPath}`);
  process.exit(1);
}
const root = JSON.parse(fs.readFileSync(rootLockPath, "utf8"));
if (root.lockfileVersion !== 3) {
  console.error(`ERROR: expected lockfileVersion 3, got ${root.lockfileVersion}`);
  process.exit(1);
}

const memberKey = memberDir.replace(/\/+$/, "");
const memberPkg = root.packages[memberKey];
if (!memberPkg) {
  console.error(`ERROR: workspace member "${memberKey}" not found in root lock packages map`);
  process.exit(1);
}

const declared = (pkg) =>
  Object.keys({
    ...(pkg.dependencies || {}),
    ...(pkg.devDependencies || {}),
    ...(pkg.optionalDependencies || {}),
  });

// Resolve a dependency name to its lock key, preferring a member-nested entry, else top-level.
const resolveKey = (name) => {
  const nested = `${memberKey}/node_modules/${name}`;
  if (root.packages[nested]) return { key: nested, node: root.packages[nested] };
  const top = `node_modules/${name}`;
  if (root.packages[top]) return { key: top, node: root.packages[top] };
  return null;
};

// [OPUS-4.8] sq-f04e: resolve <name> as REQUIRED BY the package living at lock-key `fromKey`,
// mirroring npm's nearest-ancestor `node_modules` walk: try `<fromKey>/node_modules/<name>`,
// then peel one trailing `node_modules/<seg>` segment and retry, down to top-level. Used to
// reconstruct a LINK TARGET's own (possibly version-conflicting, hence NESTED) closure at the
// exact keys npm pins it under in the root lock — so the materialized node stays valid and the
// target's `typescript@5.9.3` does NOT collide with a top-level `typescript@6`.
const resolveFrom = (fromKey, name) => {
  let base = fromKey;
  for (;;) {
    const k = `${base ? `${base}/` : ""}node_modules/${name}`;
    if (root.packages[k]) return { key: k, node: root.packages[k] };
    if (base === "") break;
    const i = base.lastIndexOf("/node_modules/");
    if (i < 0) {
      base = ""; // a member-root key like "packages/x" — next probe is top-level.
      continue;
    }
    base = base.slice(0, i);
  }
  return null;
};

const out = {
  name: memberPkg.name,
  version: memberPkg.version,
  lockfileVersion: 3,
  requires: true,
  packages: {},
};
// The standalone lock's root ("") IS the member — this anchors cyclonedx's root component.
out.packages[""] = { ...memberPkg };

let unresolved = 0;

// [OPUS-4.8] sq-f04e: materialize the FULL subtree of a workspace-link TARGET into the standalone
// lock at the keys the root lock uses (link node at `node_modules/<name>` PLUS the real package
// node at its `resolved` path PLUS the target's own nested closure). WHY: the derive script used
// to re-home a `link: true` dep VERBATIM (just the link stub) and drop its target, leaving an
// UNRESOLVED link node; `npm ls` (which cyclonedx-npm shells out to, incl. the runtime `--omit=dev`
// view) then crashed on the dangling node with "Cannot read properties of undefined (reading
// 'extraneous')" -> exit 254 — the cryptic failure that ANY future workspace-to-workspace dep edge
// on a published member (js/ or packages/*) would re-trip (sq-f04e; discovered #876/sq-jpki.2).
// Reconstructing the target node + its closure makes `npm ls` resolve the link cleanly in BOTH the
// runtime and full views. A workspace link is a DEV-graph/in-repo edge, so this changes the FULL
// (dev) tree but NEVER the runtime (`--omit=dev`) set when the link is a devDependency.
const materializeLinkTarget = (linkNode, seenKeys) => {
  const tgtKey = linkNode.resolved;
  const tgt = root.packages[tgtKey];
  if (!tgt) {
    console.error(`ERROR: workspace-link target "${tgtKey}" is not present in the root lock`);
    unresolved++;
    return;
  }
  if (seenKeys.has(tgtKey)) return;
  seenKeys.add(tgtKey);
  out.packages[tgtKey] = tgt;
  // BFS the target's own closure, resolving each edge FROM the target's directory (so a nested,
  // version-conflicting dep keeps its nested key and does not collide with a hoisted top-level one).
  const q = declared(tgt).map((n) => ({ from: tgtKey, name: n }));
  while (q.length) {
    const { from, name } = q.shift();
    const r = resolveFrom(from, name);
    if (!r) {
      console.error(`ERROR: dependency "${name}" of workspace-link target "${tgtKey}" is not present in the root lock`);
      unresolved++;
      continue;
    }
    if (seenKeys.has(r.key)) continue;
    seenKeys.add(r.key);
    out.packages[r.key] = r.node;
    if (r.node.link === true && r.node.resolved) {
      materializeLinkTarget(r.node, seenKeys); // chained workspace link.
    } else {
      for (const d of declared(r.node)) q.push({ from: r.key, name: d });
    }
  }
};

// Member closure: direct dependencies become top-level standalone nodes. Transitive dependencies
// retain their nearest-ancestor layout, including a version-conflicting node nested below its
// parent. The previous name-only queue re-resolved every transitive edge from the MEMBER root,
// incorrectly substituting a hoisted incompatible version for packages such as seek-bzip's nested
// commander. npm ls then rejected the derived runtime lock, blocking an honest client SBOM.
const queue = declared(memberPkg).map((name) => ({
  direct: true,
  from: memberKey,
  name,
  outFrom: "",
}));
const seenKeys = new Set();
while (queue.length) {
  const { direct, from, name, outFrom } = queue.shift();
  const r = direct ? resolveKey(name) : resolveFrom(from, name);
  if (!r) {
    console.error(`ERROR: dependency "${name}" in the ${memberKey} closure is not present in the root lock`);
    unresolved++;
    continue;
  }
  const nestedPrefix = `${from}/node_modules/`;
  let outKey = r.key;
  if (direct) {
    outKey = `node_modules/${name}`;
  } else if (r.key.startsWith(nestedPrefix)) {
    outKey = `${outFrom}${r.key.slice(from.length)}`;
  } else if (r.key.startsWith(`${memberKey}/node_modules/`)) {
    outKey = r.key.slice(memberKey.length + 1);
  }
  if (seenKeys.has(outKey)) continue;
  out.packages[outKey] = r.node;
  seenKeys.add(outKey);
  if (r.node.link === true && r.node.resolved) {
    // [OPUS-4.8] sq-f04e: a workspace-link dep — emit its real target node + closure so npm ls
    // resolves it instead of crashing on a dangling link (sq-f04e). Do NOT recurse into the link
    // node's (empty) own deps; the target carries the real dependency edges.
    materializeLinkTarget(r.node, seenKeys);
  } else {
    for (const dependency of declared(r.node)) {
      queue.push({ direct: false, from: r.key, name: dependency, outFrom: outKey });
    }
  }
}
if (unresolved > 0) {
  console.error(`ERROR: ${unresolved} dependency(ies) could not be resolved from the root lock`);
  process.exit(1);
}

fs.mkdirSync(path.dirname(path.resolve(outPath)), { recursive: true });
fs.writeFileSync(outPath, `${JSON.stringify(out, null, 2)}\n`);
console.error(
  `derived standalone lock for "${memberKey}" (${memberPkg.name}@${memberPkg.version}) -> ${outPath}: ${Object.keys(out.packages).length - 1} dependency package(s)`,
);
