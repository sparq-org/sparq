// Research grade and not externally audited.
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { chmod, copyFile, lstat, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { checkPinnedFiles } from './internal/runtime.mjs';
import pins from './assets/toolchain.pins.json' with { type: 'json' };

const outputLimit = 1 << 20;
export class ProverRefusal extends Error {}

async function digest(path) {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
}

function run(binary, args, { cwd, signal } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(binary, args, {
      cwd, env: { ...process.env, HARDWARE_CONCURRENCY: '1', RAYON_NUM_THREADS: '1' },
      stdio: ['ignore', 'pipe', 'pipe'], detached: process.platform !== 'win32',
      shell: false, windowsHide: true,
    });
    let settled = false, outputBytes = 0, stdout = '', failure;
    const stop = reason => {
      failure = reason;
      try {
        if (process.platform === 'win32') child.kill('SIGTERM');
        else process.kill(-child.pid, 'SIGTERM');
      } catch {}
    };
    const abort = () => stop(new ProverRefusal('native operation aborted'));
    const complete = error => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener('abort', abort);
      if (error) reject(error); else resolve(stdout);
    };
    child.stdout.on('data', bytes => {
      outputBytes += bytes.length;
      if (outputBytes > outputLimit) stop(new ProverRefusal('native output limit exceeded'));
      else stdout += bytes.toString('utf8');
    });
    child.stderr.on('data', bytes => {
      outputBytes += bytes.length;
      if (outputBytes > outputLimit) stop(new ProverRefusal('native output limit exceeded'));
    });
    child.once('error', () => complete(new ProverRefusal('native executable could not start')));
    child.once('close', (code, exitSignal) => complete(failure ||
      (code === 0 ? undefined : new ProverRefusal(`native operation failed (${exitSignal || code})`))));
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
  });
}

export async function copyPinnedBinary(name, source, destination, options = {}) {
  const expected = pins.binaries[name];
  if (!expected || process.platform !== pins.platform || process.arch !== pins.arch) {
    throw new ProverRefusal('native platform has no binary pin');
  }
  const requirement = `${name} expected ${expected.version}, SHA-256 ${expected.sha256}`;
  if (typeof source !== 'string' || !source) throw new ProverRefusal(`missing binary: ${requirement}`);
  try {
    if (!(await lstat(source)).isFile()) throw new Error();
    await copyFile(source, destination);
  } catch {
    throw new ProverRefusal(`missing or unreadable binary: ${requirement}`);
  }
  await chmod(destination, 0o400);
  if (await digest(destination) !== expected.sha256) throw new ProverRefusal(`binary pin mismatch: ${requirement}`);
  await chmod(destination, 0o500);
  let version;
  try { version = await run(destination, ['--version'], options); }
  catch { throw new ProverRefusal(`could not execute pinned binary: ${requirement}`); }
  if (version.trim().split('\n')[0] !== expected.version) throw new ProverRefusal(`binary version mismatch: ${requirement}`);
  return destination;
}

function sourcePath(root, name) {
  if (typeof name !== 'string' || isAbsolute(name) || name.split('/').some(p => !p || p === '.' || p === '..') ||
      !/^[a-zA-Z0-9_./-]+$/.test(name)) throw new ProverRefusal('catalogue source path rejected');
  return join(root, name);
}

async function output(path, limit) {
  const info = await lstat(path);
  if (!info.isFile() || info.size <= 0 || info.size > limit) throw new ProverRefusal('native artifact rejected');
  return readFile(path);
}

export async function preflight(options = {}) {
  const work = await mkdtemp(join(tmpdir(), 'sparq-query-binaries-'));
  try {
    await chmod(work, 0o700);
    await copyPinnedBinary('nargo', options.nargoPath ?? process.env.SPARQ_NARGO, join(work, 'nargo'));
    await copyPinnedBinary('bb', options.bbPath ?? process.env.SPARQ_BB, join(work, 'bb'));
  } finally { await rm(work, { recursive: true, force: true }); }
}

export async function prove(acirArtifact, witnessInputs, options) {
  const work = await mkdtemp(join(tmpdir(), 'sparq-query-proof-'));
  try {
    await chmod(work, 0o700);
    const nargo = await copyPinnedBinary('nargo', options.nargoPath ?? process.env.SPARQ_NARGO, join(work, 'nargo'), { signal: options.signal });
    const bb = await copyPinnedBinary('bb', options.bbPath ?? process.env.SPARQ_BB, join(work, 'bb'), { signal: options.signal });
    if (!/^query_[a-f0-9]{64}$/.test(options.packageName)) throw new ProverRefusal('catalogue package name rejected');
    for (const [name, text] of Object.entries(options.sources)) {
      const target = sourcePath(work, name);
      await mkdir(dirname(target), { recursive: true });
      await writeFile(target, text, { mode: 0o600, flag: 'wx' });
    }
    const target = join(work, 'target');
    await mkdir(target, { recursive: true });
    const artifactPath = join(target, `${options.packageName}.json`);
    await writeFile(artifactPath, acirArtifact.bytes, { mode: 0o600, flag: 'wx' });
    await writeFile(join(work, options.packageName, 'Prover.toml'), witnessInputs.toml, { mode: 0o600, flag: 'wx' });
    try {
      await run(nargo, ['execute', 'witness', '--package', options.packageName], { cwd: work, signal: options.signal });
    } catch {
      throw new ProverRefusal('native witness execution failed');
    }
    const witness = join(target, 'witness.gz');
    if (!(await lstat(witness)).isFile()) throw new ProverRefusal('native witness was not produced');
    const after = JSON.parse(await readFile(artifactPath, 'utf8'));
    if (after.bytecode !== acirArtifact.program.bytecode || JSON.stringify(after.abi) !== JSON.stringify(acirArtifact.program.abi)) {
      throw new ProverRefusal('native execution changed the catalogue program');
    }
    const privateCrs = join(work, 'crs');
    await mkdir(privateCrs, { mode: 0o700 });
    for (const name of ['bn254_g1.dat', 'bn254_g2.dat']) {
      const target = join(privateCrs, name);
      await copyFile(join(options.crsPath, name), target);
      await chmod(target, 0o400);
    }
    await checkPinnedFiles(options.crsPins, pathToFileURL(privateCrs + '/'));
    const out = join(work, 'proof');
    await mkdir(out, { mode: 0o700 });
    try {
      await run(bb, ['prove', '-b', artifactPath, '-w', witness, '-o', out, '--write_vk', '-t', 'noir-recursive', '--crs_path', privateCrs], { cwd: work, signal: options.signal });
    } catch {
      throw new ProverRefusal('native proof generation failed');
    }
    const key = await output(join(out, 'vk'), options.expectedKey.length);
    if (!key.equals(options.expectedKey)) throw new ProverRefusal('native prover key mismatch');
    const expected = Buffer.concat(options.publicInputs.map(word => Buffer.from(word.slice(2), 'hex')));
    const publicInputs = await output(join(out, 'public_inputs'), expected.length);
    if (!publicInputs.equals(expected)) throw new ProverRefusal('native public-input mismatch');
    const proof = await output(join(out, 'proof'), options.maxPresentationBytes);
    return { proof, publicInputs: options.publicInputs };
  } finally {
    await rm(work, { recursive: true, force: true });
  }
}
