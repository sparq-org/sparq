// Research grade and not externally audited. Dependency-free compiler control.
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { compile_program, createFileManager } from '@noir-lang/noir_wasm';
import { UltraHonkBackend } from '@aztec/bb.js';
import { createBackend, proofOptions, srsPointsForKey } from '../internal/runtime.mjs';
export async function compilerControl({ directory, nargo, bb, equalArtifacts, equalKeys }) {
  const work = join(directory, 'compiler-control'), packageDir = join(work, 'control');
  mkdirSync(join(packageDir, 'src'), { recursive: true });
  writeFileSync(join(work, 'Nargo.toml'), '[workspace]\nmembers = ["control"]\n');
  writeFileSync(join(packageDir, 'Nargo.toml'), '[package]\nname = "control"\ntype = "bin"\nauthors = ["sparq"]\ncompiler_version = ">=1.0.0"\n');
  writeFileSync(join(packageDir, 'src/main.nr'), 'fn main(x: Field) -> pub Field { x * x }\n');
  const { program } = await compile_program(createFileManager(work), packageDir, () => {}, () => {});
  execFileSync(nargo, ['compile'], { cwd: work, stdio: 'pipe' });
  const artifact = join(work, 'target/control.json');
  equalArtifacts(program, JSON.parse(readFileSync(artifact)));
  const crsDirectory = new URL('../assets/crs/', import.meta.url);
  execFileSync(bb, ['write_vk', '-b', artifact, '-o', work, '-t', 'noir-recursive', '--crs_path', crsDirectory.pathname], { stdio: 'pipe', env: { ...process.env, HARDWARE_CONCURRENCY: '1' } });
  const pins = JSON.parse(readFileSync(new URL('../assets/crs.pins.json', import.meta.url)));
  const nativeKey = readFileSync(join(work, 'vk'));
  const api = await createBackend(crsDirectory, pins, 'prove', srsPointsForKey(nativeKey, pins));
  try { equalKeys(await new UltraHonkBackend(program.bytecode, api).getVerificationKey(proofOptions), readFileSync(join(work, 'vk'))); }
  finally { api.destroy(); }
  writeFileSync(join(directory, 'compiler-control.json'), JSON.stringify({ bytecode: 'agreement', abi: 'agreement', key: 'agreement' }));
}
