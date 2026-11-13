// Research grade and not externally audited.
import { createHash } from 'node:crypto';
import { createReadStream, statSync, openSync, readSync, closeSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { BarretenbergSync, BackendType } from '@aztec/bb.js';

export const proofOptions = Object.freeze({ verifierTarget: 'noir-recursive' });
const settings = Object.freeze({ ipaAccumulation: false, oracleHashType: 'poseidon2', disableZk: false, optimizedSolidityVerifier: false });
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export async function checkPinnedFiles(pins, directory) {
  if (Object.keys(pins).sort().join() !== 'bn254_g1.dat,bn254_g2.dat') throw new Error('bundled CRS pin set rejected');
  for (const [name, pin] of Object.entries(pins)) {
    const path = new URL(name, directory);
    if (!Number.isSafeInteger(pin.bytes) || pin.bytes <= 0 || !/^[a-f0-9]{64}$/.test(pin.sha256) || statSync(path).size !== pin.bytes) throw new Error('bundled CRS length rejected');
    const hash = createHash('sha256');
    for await (const bytes of createReadStream(path)) hash.update(bytes);
    if (hash.digest('hex') !== pin.sha256) throw new Error('bundled CRS digest rejected');
  }
  if (pins['bn254_g1.dat'].bytes % 64 || pins['bn254_g2.dat'].bytes !== 128) throw new Error('bundled CRS encoding rejected');
}
function points(path, length) {
  const buffer = new Uint8Array(length), fd = openSync(path, 'r');
  try {
    let offset = 0;
    while (offset < length) {
      const read = readSync(fd, buffer, offset, Math.min(1 << 20, length - offset), offset);
      if (!read) throw new Error('bundled CRS truncated');
      offset += read;
    }
    return buffer;
  } finally { closeSync(fd); }
}
export async function createBackend(directory, pins, role = 'prove') {
  await checkPinnedFiles(pins, directory);
  const crsPath = fileURLToPath(directory);
  const count = role === 'verify' ? 1 : pins['bn254_g1.dat'].bytes / 64;
  const api = await BarretenbergSync.new({ backend: BackendType.Wasm, crsPath });
  try {
    api.srsInitSrs({ pointsBuf: points(new URL('bn254_g1.dat', directory), count * 64), numPoints: count, g2Point: points(new URL('bn254_g2.dat', directory), pins['bn254_g2.dat'].bytes) });
    return api;
  } catch (error) { api.destroy(); throw error; }
}
export function verifyProofSync(api, { proof, publicInputs, verificationKey }) {
  if (!proof.length || proof.length % 32) return false;
  const proofWords = [];
  for (let i = 0; i < proof.length; i += 32) proofWords.push(proof.slice(i, i + 32));
  const words = publicInputs.map(word => {
    const bytes = Buffer.from(BigInt(word).toString(16).padStart(64, '0'), 'hex');
    if (bytes.length !== 32) throw new Error('public-input encoding rejected');
    return bytes;
  });
  return api.circuitVerify({ proof: proofWords, publicInputs: words, verificationKey, settings }).verified;
}
