#!/usr/bin/env node
// Pin the native platform packages as exact optional dependencies of the host manifest.
// publish.yml runs this only in a dispatch that also publishes those packages. They are not
// pinned in the source manifest: the lockstep version reaches npm only when this release
// publishes it, so package-lock.json could never record a source pin and `npm ci` refuses the
// tagged tree.
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { NATIVE_PLATFORMS } from '../src/native-platforms.js';

const path = process.argv[2] ?? fileURLToPath(new URL('../package.json', import.meta.url));
const host = JSON.parse(readFileSync(path, 'utf8'));
host.optionalDependencies = Object.fromEntries(
  Object.keys(NATIVE_PLATFORMS).map((platform) => [`@sparq-org/solid-server-native-${platform}`, host.version]),
);
writeFileSync(path, `${JSON.stringify(host, null, 2)}\n`);
