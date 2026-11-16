import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';

function downloadCrs(pin, path) {
  execFileSync('curl', [
    '--fail', '--location', '--silent', '--show-error', '--retry', '3',
    '--connect-timeout', '30', '--max-time', '120', '--proto', '=https', '--tlsv1.2',
    '--range', `0-${pin.bytes - 1}`, '--max-filesize', String(pin.bytes),
    '--output', path, pin.url,
  ], { stdio: 'inherit' });
}

// Publish the complete set only after every downloaded file matches its pin.
export function installPinnedCrs(pins, destination, download = downloadCrs) {
  const names = ['bn254_g1.dat', 'bn254_g2.dat'];
  if (Object.keys(pins).sort().join() !== names.join()) throw new Error('invalid CRS pin set');
  for (const [i, name] of names.entries()) {
    const pin = pins[name];
    if (pin.url !== `https://crs.aztec-cdn.foundation/g${i + 1}.dat` ||
        !Number.isSafeInteger(pin.bytes) || pin.bytes <= 0 || !/^[a-f0-9]{64}$/.test(pin.sha256)) {
      throw new Error(`invalid CRS pin: ${name}`);
    }
  }
  mkdirSync(dirname(destination), { recursive: true });
  const staging = mkdtempSync(join(dirname(destination), '.crs-'));
  try {
    for (const name of names) {
      const pin = pins[name], path = join(staging, name);
      download(pin, path);
      if (statSync(path).size !== pin.bytes) throw new Error(`CRS length mismatch: ${name}`);
      const digest = createHash('sha256').update(readFileSync(path)).digest('hex');
      if (digest !== pin.sha256) throw new Error(`CRS SHA-256 mismatch: ${name}`);
    }
    rmSync(destination, { recursive: true, force: true });
    renameSync(staging, destination);
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}
