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
/**
 * The largest dyadic circuit size the 32-bit WebAssembly prover can key and prove within
 * its four-gibibyte memory (measured: a 2^21 program keys, a 2^22 program aborts).
 */
export const WASM_PROVER_DYADIC_LIMIT = 1 << 21;
/** Whether a program of this dyadic size fits the WebAssembly prover. */
export const wasmProverFits = dyadicSize => dyadicSize > 0 && dyadicSize <= WASM_PROVER_DYADIC_LIMIT;
/** The dyadic circuit size a verification key's header declares. */
export function keyDyadicSize(key) {
  if (key.length < 32) throw new Error('verification key header rejected');
  return 2 ** Number(BigInt('0x' + Buffer.from(key.subarray(0, 32)).toString('hex')));
}
export async function prove(artifact, inputs, options) {
  // Refuse up front: past this size the prover aborts rather than refusing.
  if (!wasmProverFits(keyDyadicSize(options.expectedKey))) throw new Error('program exceeds the WebAssembly prover memory bound');
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
