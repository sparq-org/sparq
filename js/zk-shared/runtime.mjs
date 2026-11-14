// Research grade and not externally audited.
import { createHash } from 'node:crypto';
import { createReadStream, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { BarretenbergSync, BackendType } from '@aztec/bb.js';

export const proofOptions = Object.freeze({ verifierTarget: 'noir-recursive' });
const settings = Object.freeze({ ipaAccumulation: false, oracleHashType: 'poseidon2', disableZk: false, optimizedSolidityVerifier: false });
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
export async function checkPinnedFiles(pins, directory, capture = {}) {
  const verified = {};
  if (Object.keys(pins).sort().join() !== 'bn254_g1.dat,bn254_g2.dat') throw new Error('bundled CRS pin set rejected');
  for (const [name, pin] of Object.entries(pins)) {
    const path = new URL(name, directory);
    if (!Number.isSafeInteger(pin.bytes) || pin.bytes <= 0 || !/^[a-f0-9]{64}$/.test(pin.sha256) || statSync(path).size !== pin.bytes) throw new Error('bundled CRS length rejected');
    const hash = createHash('sha256');
    const wanted = capture[name] || 0, buffer = new Uint8Array(wanted);
    let copied = 0;
    for await (const bytes of createReadStream(path)) {
      hash.update(bytes);
      const count = Math.min(wanted - copied, bytes.length);
      if (count > 0) { buffer.set(bytes.subarray(0, count), copied); copied += count; }
    }
    if (copied !== wanted) throw new Error('bundled CRS truncated');
    verified[name] = buffer;
    if (hash.digest('hex') !== pin.sha256) throw new Error('bundled CRS digest rejected');
  }
  if (pins['bn254_g1.dat'].bytes % 64 || pins['bn254_g2.dat'].bytes !== 128) throw new Error('bundled CRS encoding rejected');
  return verified;
}
export async function createBackend(directory, pins, role = 'prove') {
  const crsPath = fileURLToPath(directory);
  const count = role === 'verify' ? 1 : pins['bn254_g1.dat'].bytes / 64;
  const verified = await checkPinnedFiles(pins, directory, { 'bn254_g1.dat': count * 64, 'bn254_g2.dat': pins['bn254_g2.dat'].bytes });
  const api = await BarretenbergSync.new({ backend: BackendType.Wasm, crsPath });
  try {
    api.srsInitSrs({ pointsBuf: verified['bn254_g1.dat'], numPoints: count, g2Point: verified['bn254_g2.dat'] });
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
