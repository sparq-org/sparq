// Issue #5274: the raw wasm-pack `--target web` glue is documented (skills/javascript-wasm/
// SKILL.md) as `import init, { Store } from '@sparq-org/sparq/wasm/sparq_wasm.js'`. An
// `exports` field makes every undeclared subpath unreachable (ERR_PACKAGE_PATH_NOT_EXPORTED),
// so `./wasm/*` must be declared. This test runs Node's real resolver against a fixture
// install that carries the package's actual manifest plus stub files — no wasm build needed.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const pkgDir = resolve(dirname(fileURLToPath(import.meta.url)), '..');

test('the documented ./wasm/sparq_wasm.js subpath resolves through the exports map', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'sparq-exports-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const installed = join(root, 'node_modules', '@sparq-org', 'sparq');
  mkdirSync(join(installed, 'wasm'), { recursive: true });
  copyFileSync(resolve(pkgDir, 'package.json'), join(installed, 'package.json'));
  for (const f of ['sparq_wasm.js', 'sparq_wasm.d.ts', 'sparq_wasm_bg.wasm']) {
    writeFileSync(join(installed, 'wasm', f), '');
  }

  const specifier = '@sparq-org/sparq/wasm/sparq_wasm.js';
  const run = spawnSync(
    process.execPath,
    ['--input-type=module', '-e', `process.stdout.write(import.meta.resolve(${JSON.stringify(specifier)}))`],
    { cwd: root, encoding: 'utf8' },
  );
  assert.equal(run.status, 0, `resolving ${specifier} failed:\n${run.stderr}`);
  assert.equal(fileURLToPath(run.stdout), join(installed, 'wasm', 'sparq_wasm.js'));
});
