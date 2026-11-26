// Research grade and not externally audited. Cache key for resumable live evidence.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { sha256 } from '../internal/runtime.mjs';
function rustSources(path) {
  return readdirSync(path, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name)).flatMap(entry => entry.isDirectory() ? rustSources(join(path, entry.name)) : entry.name.endsWith('.rs') ? [readFileSync(join(path, entry.name))] : []);
}
/** Digest of everything a cached live proof depends on besides its catalogue entry. */
export function liveRuntimeHash(root) {
  return sha256(Buffer.concat([
    ...['../index.js', '../native.mjs', '../wasm-prover.mjs', '../internal/runtime.mjs', '../package-lock.json', './solid-targets.mjs', './consumer-cases.mjs', './consumer-live.mjs'].map(name => readFileSync(new URL(name, import.meta.url))),
    ...['sparq-zk-query-wasm', 'sparq-zk-query', 'sparq-merkle-suite'].flatMap(name => rustSources(join(root, 'crates', name, 'src'))),
    readFileSync(join(root, 'Cargo.lock')),
  ]));
}
