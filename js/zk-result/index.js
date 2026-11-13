import { readFileSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { Noir } from '@noir-lang/noir_js';
import { BarretenbergSync, BackendType, Crs, UltraHonkBackend, UltraHonkVerifierBackend } from '@aztec/bb.js';

const require = createRequire(import.meta.url);
const host = require('./wasm/sparq_zk_result_wasm.cjs');
const assets = new URL('./assets/', import.meta.url);
const proofOptions = Object.freeze({ verifierTarget: 'noir-recursive' });
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const parse = text => JSON.parse(text);

// Read on each call so the canonical-key check precedes any untrusted statement.
function canonicalMembers() {
  const table = parse(readFileSync(new URL('members.json', assets), 'utf8'));
  const members = new Map();
  for (const entry of table) {
    if (members.has(entry.member) || !/^result_v5_[a-z0-9_]+$/.test(entry.member) ||
        !Number.isSafeInteger(entry.proofBytes) || entry.proofBytes <= 0 || entry.proofBytes % 32) {
      throw new Error('canonical member or VK rejected');
    }
    if (!/^[a-z0-9_]+\.vk$/.test(entry.vk) || !/^[a-z0-9_]+\.json$/.test(entry.acir)) {
      throw new Error('canonical member or VK rejected');
    }
    const vk = readFileSync(new URL(entry.vk, assets));
    if (sha256(vk) !== host.canonicalVkHash(entry.member)) {
      throw new Error('canonical member or VK rejected');
    }
    members.set(entry.member, { ...entry, vk });
  }
  if (!members.size) throw new Error('canonical member or VK rejected');
  return members;
}

function safePolicy(policy) {
  for (const v of [policy.minVersion, policy.maxVersion, ...policy.snapshots.map(s => s.version)]) {
    if (!Number.isSafeInteger(v) || v < 0) throw new Error('policy versions must be safe nonnegative integers');
  }
  return JSON.stringify(policy);
}

async function backend() {
  const crsPath = fileURLToPath(new URL('crs/', assets));
  const size = statSync(new URL('crs/bn254_g1.dat', assets)).size;
  if (!size || size % 64 || statSync(new URL('crs/bn254_g2.dat', assets)).size !== 128) {
    throw new Error('bundled CRS rejected');
  }
  // The synchronous WASM API permits an explicit SRS capacity without the
  // asynchronous constructor's unrelated default SRS allocation or native fallback.
  const crs = await Crs.new(size / 64, crsPath);
  const api = await BarretenbergSync.new({ backend: BackendType.Wasm, crsPath });
  try {
    api.srsInitSrs({ pointsBuf: crs.getG1Data(), numPoints: crs.numPoints, g2Point: crs.getG2Data() });
    return api;
  } catch (error) {
    api.destroy();
    throw error;
  }
}

export function issuerKeygen() {
  return parse(host.issuerKeygen());
}

export function testIssuerKeygen(seedHex) {
  return parse(host.testIssuerKeygen(seedHex));
}

export function issue(issuer, graphNQuads, statusRef) {
  if (![statusRef.version, statusRef.index].every(v => Number.isSafeInteger(v) && v >= 0)) {
    throw new Error('status version and index must be safe nonnegative integers');
  }
  return parse(host.issue(JSON.stringify(issuer), graphNQuads, JSON.stringify(statusRef)));
}

export async function prove({ query, credentials, policy, nonce }) {
  const members = canonicalMembers();
  const prepared = parse(host.prepare(query, credentials.map(c => JSON.stringify(c)), safePolicy(policy), nonce, '{}'));
  const member = members.get(prepared.member);
  if (!member) throw new Error('canonical member or VK rejected');
  const statement = parse(host.reconstructPublicInputs(query, JSON.stringify(prepared.presentationSkeleton), safePolicy(policy), nonce));
  if (statement.member !== prepared.member) throw new Error('canonical member or VK rejected');
  const circuit = parse(readFileSync(new URL(member.acir, assets), 'utf8'));
  const { witness } = await new Noir(circuit).execute(prepared.witnessInputs);
  const api = await backend();
  try {
    const prover = new UltraHonkBackend(circuit.bytecode, api);
    const vk = await prover.getVerificationKey(proofOptions);
    if (!Buffer.from(vk).equals(member.vk)) throw new Error('canonical member or VK rejected');
    const proof = await prover.generateProof(witness, proofOptions);
    if (proof.proof.length !== member.proofBytes) throw new Error('proof length differs from canonical member');
    if (proof.publicInputs.length !== statement.publicInputs.length ||
        proof.publicInputs.some((v, i) => BigInt(v) !== BigInt(statement.publicInputs[i]))) {
      throw new Error('public-input serialization differs from circuit ABI');
    }
    const presentation = { ...prepared.presentationSkeleton, proof: Array.from(proof.proof) };
    return { presentation, result: presentation.rows };
  } finally {
    api.destroy();
  }
}

export async function verify({ expectedQuery, presentation, policy, nonce, seenNonces }) {
  let members;
  try {
    members = canonicalMembers();
  } catch {
    return { verified: false, released: [], reason: 'canonical member or VK rejected' };
  }
  let statement;
  try {
    const serialized = JSON.stringify(presentation);
    presentation = JSON.parse(serialized);
    statement = parse(host.reconstructPublicInputs(expectedQuery, serialized, safePolicy(policy), nonce));
  } catch (error) {
    return { verified: false, released: [], reason: `statement rejected: ${String(error)}` };
  }
  const member = members.get(statement.member);
  if (!member) return { verified: false, released: [], reason: 'canonical member or VK rejected' };
  if (!presentation.proof.length) return { verified: false, released: [], reason: 'statement rejected: missing proof' };
  if (presentation.proof.length !== member.proofBytes) return { verified: false, released: [], reason: 'malformed proof encoding' };
  let valid;
  try {
    const api = await backend();
    try {
      valid = await new UltraHonkVerifierBackend(api).verifyProof({
        proof: Uint8Array.from(presentation.proof), publicInputs: statement.publicInputs, verificationKey: member.vk,
      }, proofOptions);
    } finally {
      api.destroy();
    }
  } catch {
    valid = false;
  }
  if (!valid) return { verified: false, released: [], reason: 'cryptographic proof rejected' };
  try {
    if (seenNonces.has(statement.nonce)) {
      return { verified: false, released: [], reason: 'challenge already consumed' };
    }
    seenNonces.add(statement.nonce);
  } catch {
    return { verified: false, released: [], reason: 'nonce store rejected' };
  }
  return { verified: true, released: statement.released };
}
