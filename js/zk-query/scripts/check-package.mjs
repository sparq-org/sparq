// Research grade and not externally audited. Packing is a maintainer operation.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { sha256, checkPinnedFiles } from '../internal/runtime.mjs';
const directory = new URL('../', import.meta.url);
const host = createRequire(import.meta.url)('../wasm/sparq_zk_query_wasm.cjs');
const read = name => readFileSync(new URL(name, directory));
const tableBytes = read('assets/catalogue.json'), table = JSON.parse(tableBytes);
if (sha256(tableBytes) !== host.catalogueHash() || sha256(read('assets/toolchain.pins.json')) !== host.toolchainPinsHash()) throw new Error('package host pins differ');
if (sha256(read('assets/sources.json')) !== table.sourceBundleSha256 || sha256(read('assets/crs.pins.json')) !== table.crsPinsSha256) throw new Error('package source or CRS pins differ');
for (const entry of table.entries) {
  if (entry.acir !== `${entry.planHash}.json` || entry.vk !== `${entry.planHash}.vk.hex`) throw new Error('package catalogue path rejected');
  if (sha256(read(`assets/${entry.acir}`)) !== entry.acirSha256 || sha256(Buffer.from(read(`assets/${entry.vk}`).toString().trim(), 'hex')) !== entry.vkSha256) throw new Error('package circuit or key pin differs');
}
const pins = JSON.parse(read('internal/package-pins.json'));
const manifest = JSON.parse(read('package.json'));
if (manifest.bundleDependencies.slice().sort().join() !== Object.keys(manifest.dependencies).sort().join()) throw new Error('package bundle set differs');
for (const [name, version] of Object.entries(manifest.dependencies)) {
  if (pins[name] !== version || JSON.parse(read(`node_modules/${name}/package.json`)).version !== version) throw new Error('package dependency pin differs');
}
await checkPinnedFiles(JSON.parse(read('assets/crs.pins.json')), new URL('assets/crs/', directory));
