// Research grade and not externally audited.
import { Noir } from '@noir-lang/noir_js';
import { UltraHonkBackend } from '@aztec/bb.js';
import { createBackend, proofOptions, srsPointsForKey } from './internal/runtime.mjs';
export function proverFailure(error) {
  if (error instanceof WebAssembly.RuntimeError || error instanceof RangeError || /alloc|memory|unreachable/i.test(String(error))) {
    return new Error('WebAssembly prover allocation refused');
  }
  return error;
}
export async function prove(artifact, inputs, options) {
  let api;
  try {
    api = options.createBackend ? await options.createBackend() : await createBackend(options.crsDirectory, options.crsPins, 'prove', srsPointsForKey(options.expectedKey, options.crsPins));
    const prover = new UltraHonkBackend(artifact.program.bytecode, api);
    const key = await prover.getVerificationKey(proofOptions);
    if (!Buffer.from(key).equals(options.expectedKey)) throw new Error('WebAssembly prover key mismatch');
    const { witness } = await new Noir(artifact.program).execute(inputs.json);
    const result = await prover.generateProof(witness, proofOptions);
    if (result.publicInputs.length !== options.publicInputs.length || result.publicInputs.some((v, i) => BigInt(v) !== BigInt(options.publicInputs[i]))) {
      throw new Error('WebAssembly public-input mismatch');
    }
    return result;
  } catch (error) {
    throw proverFailure(error);
  } finally { api?.destroy(); }
}
