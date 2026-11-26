#!/usr/bin/env node
// Research grade and not externally audited. Deterministic compiler and key fixture.
import { createHash } from 'node:crypto';
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';

const name = basename(process.argv[1]);
const [command, ...args] = process.argv.slice(2);
if (command === '--version') {
  process.stdout.write(name + '\n');
} else if (name === 'nargo' && command === 'compile') {
  const plan = readFileSync('Nargo.toml', 'utf8');
  appendFileSync(process.env.CATALOGUE_TEST_CALLS, `compile ${plan}\n`);
  if (plan === process.env.CATALOGUE_TEST_FAIL_PLAN) process.exit(1);
  const abi = JSON.parse(readFileSync('src/main.nr', 'utf8'));
  const bytecode = plan + readFileSync('zk/noir-query/core/src/lib.nr', 'utf8').trim();
  mkdirSync('target', { recursive: true });
  writeFileSync(join('target', `${plan}.json`), JSON.stringify({ abi, bytecode, file_map: {} }) + '\n');
} else if (name === 'bb' && command === 'write_vk') {
  const artifact = args[args.indexOf('-b') + 1];
  const plan = basename(artifact, '.json');
  appendFileSync(process.env.CATALOGUE_TEST_CALLS, `write_vk ${plan}\n`);
  const key = createHash('sha256').update(readFileSync(artifact)).digest();
  const keyPath = join(args[args.indexOf('-o') + 1], 'vk');
  const keyWrite = process.env.CATALOGUE_TEST_KEY_WRITE;
  if (keyWrite === 'partial') {
    writeFileSync(keyPath, key.subarray(0, 5));
    process.exit(1);
  }
  writeFileSync(keyPath, keyWrite === 'empty' ? Buffer.alloc(0) : key);
} else {
  throw new Error(`unexpected fixture command ${name} ${command}`);
}
