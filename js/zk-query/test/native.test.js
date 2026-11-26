// Research grade and not externally audited.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { copyPinnedBinary, ProverRefusal } from '../native.mjs';

const versions = { nargo: 'nargo version = 1.0.0-beta.21', bb: '5.0.0-nightly.20260324' };
const hashes = {
  'linux-x64': {
    nargo: '0852cd5631ac5ca25f744f25c931e3cff16351d9bcdbfb03cebb48a4494a0fe3',
    bb: 'd7621dfb151f55c23fb04bf274f366a6a1e0516b85a3cf0ae9165a9952509793',
  },
  'darwin-arm64': {
    nargo: '8174924bda8d8ef3ed04d0d863495efdb12c5efefd2ba7827b17eabf1ee65866',
    bb: '6bf8ebe16175ba1e8ffb8ac83829d4b1ec1594a206c0a1470c05918e3eafdc01',
  },
  'darwin-x64': {
    nargo: 'a07f896b1f17614f843690b5fa31c43177db1a5ac8c540c0ff539bf2e22121ba',
    bb: 'bc15eaed2fb418b4f7c47b7ad8144428d68ac61379849751d1bb61715810aa0d',
  },
};

async function onPlatform(platform, arch, action) {
  const descriptors = Object.getOwnPropertyDescriptors(process);
  try {
    Object.defineProperty(process, 'platform', { value: platform });
    Object.defineProperty(process, 'arch', { value: arch });
    await action();
  } finally {
    Object.defineProperty(process, 'platform', descriptors.platform);
    Object.defineProperty(process, 'arch', descriptors.arch);
  }
}

for (const [platformKey, binaries] of Object.entries(hashes)) {
  test(`${platformKey} selects its binary pins and refuses mismatched executables before execution`, async () => {
    const directory = await mkdtemp(join(tmpdir(), 'sparq-query-native-test-'));
    try {
      const [platform, arch] = platformKey.split('-');
      await onPlatform(platform, arch, async () => {
        for (const [name, sha256] of Object.entries(binaries)) {
          const source = join(directory, `${name}-source`), marker = join(directory, `${name}-executed`);
          await writeFile(source, `#!/bin/sh\ntouch '${marker}'\necho '${versions[name]}'\n`, { mode: 0o700 });
          await assert.rejects(copyPinnedBinary(name, source, join(directory, name)), error => {
            assert.ok(error instanceof ProverRefusal);
            assert.equal(error.message, `binary pin mismatch: ${name} expected ${versions[name]}, SHA-256 ${sha256}`);
            assert.equal(error.name, 'ProverRefusal');
            return true;
          });
          assert.equal(existsSync(marker), false);
        }
      });
    } finally { await rm(directory, { recursive: true, force: true }); }
  });
}

for (const [platform, arch] of [['linux', 'arm64'], ['darwin', 'ia32'], ['win32', 'x64'], ['unknown', 'unknown']]) {
  test(`${platform}-${arch} refuses with a named error before reading a binary`, async () => {
    await onPlatform(platform, arch, async () => {
      for (const name of ['nargo', 'bb']) {
        await assert.rejects(copyPinnedBinary(name, undefined, undefined), error => {
          assert.ok(error instanceof ProverRefusal);
          assert.equal(error.name, 'ProverRefusal');
          assert.equal(error.message, 'native platform has no binary pin');
          return true;
        });
      }
    });
  });
}
