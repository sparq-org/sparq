// Research grade and not externally audited. One compiler-agreement case in its own
// process: the WebAssembly compiler and prover keep their memory for the process
// lifetime, so each program gets a fresh instance.
// Usage: node compiler-agreement-case.mjs <workspace> <package> <native vk> <output dir> [compile-only]
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createFileManager, compile_program } from '@noir-lang/noir_wasm';
import { UltraHonkBackend } from '@aztec/bb.js';
import { createBackend, proofOptions, srsPointsForKey } from '../internal/runtime.mjs';
const [work, name, nativeKey, out, mode] = process.argv.slice(2);
let step = 'WebAssembly compile';
try {
  const { program } = await compile_program(createFileManager(work), join(work, name), () => {}, () => {});
  writeFileSync(join(out, 'wasm-program.json'), JSON.stringify(program));
  if (mode === 'compile-only') process.exit(0);
  step = 'WebAssembly verification key';
  const crs = new URL('../assets/crs/', import.meta.url);
  const pins = JSON.parse(readFileSync(new URL('../assets/crs.pins.json', import.meta.url)));
  const api = await createBackend(crs, pins, 'prove', srsPointsForKey(readFileSync(nativeKey), pins));
  try { writeFileSync(join(out, 'wasm.vk'), await new UltraHonkBackend(program.bytecode, api).getVerificationKey(proofOptions)); }
  finally { api.destroy(); }
} catch (error) {
  process.stderr.write(`${step}: ${error?.stack ?? error}\n`);
  process.exit(1);
}
