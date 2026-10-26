#!/usr/bin/env node
// Build platform packages from the existing release matrix's versioned archives.
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { NATIVE_PLATFORMS, nativePackageName } from '../src/native-platforms.js';

const here = dirname(fileURLToPath(import.meta.url));
const host = JSON.parse(readFileSync(join(here, '../package.json')));
const [artifacts, destination, onlyPlatform] = process.argv.slice(2);
if (!artifacts || !destination) throw new Error('usage: assemble.mjs <release-archive-dir> <output-dir> [platform]');
const platforms = onlyPlatform ? [onlyPlatform] : Object.keys(NATIVE_PLATFORMS);
const archives = readdirSync(artifacts);
for (const platform of platforms) {
  const name = nativePackageName(...platform.split('-'));
  const { os, cpu, tier } = NATIVE_PLATFORMS[platform];
  const executable = `sparq-lws-core${os === 'win32' ? '.exe' : ''}`;
  const archive = `sparq-cli-v${host.version}-${tier}.${os === 'win32' ? 'zip' : 'tar.gz'}`;
  if (!archives.includes(archive)) throw new Error(`missing release artifact ${archive}`);
  const scratch = mkdtempSync(join(tmpdir(), 'solid-native-'));
  try {
    const path = resolve(artifacts, archive);
    execFileSync(os === 'win32' ? 'unzip' : 'tar', os === 'win32' ? ['-q', path, '-d', scratch] : ['-xzf', path, '-C', scratch]);
    const root = join(scratch, `sparq-cli-v${host.version}-${tier}`);
    const output = resolve(destination, platform);
    rmSync(output, { recursive: true, force: true });
    mkdirSync(join(output, 'bin'), { recursive: true });
    copyFileSync(join(root, executable), join(output, 'bin', executable));
    chmodSync(join(output, 'bin', executable), 0o755);
    // The license is taken from the same source archive as the executable.
    copyFileSync(join(root, 'LICENSE'), join(output, 'LICENSE'));
    const manifest = {
      name, version: host.version, description: `Native Solid/LWS development server for ${platform}`,
      license: host.license, os: [os], cpu: [cpu], files: ['bin', 'README.md', 'LICENSE'],
      repository: host.repository, homepage: host.homepage, bugs: host.bugs,
      publishConfig: host.publishConfig,
    };
    writeFileSync(join(output, 'package.json'), `${JSON.stringify(manifest, null, 2)}\n`);
    writeFileSync(join(output, 'README.md'), `# ${name}\n\nOptional native executable for \`@sparq-org/solid-server\` at the same version.\nUse \`solid-server --protocol lws\` or \`solid-server --native\`.\nInstalled by npm for ${os}/${cpu}; no installation scripts are required.\nServer configuration: [Solid/LWS skill](https://github.com/sparq-org/sparq/blob/main/skills/solid-lws-server/SKILL.md).\n`);
    console.log(output);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}
