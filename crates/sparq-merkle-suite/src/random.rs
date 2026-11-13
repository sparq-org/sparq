//! Optional entropy acquisition; the deterministic suite core performs no I/O.
use crate::{poseidon, Error};
use ark_ff::PrimeField;
use sparq_zk::sig;

/// Issue using a fresh uniform field salt from the platform entropy source.
/// Native builds use OS entropy; WebAssembly uses the selected JS backend.
pub fn issue(
    document: &str,
    proof_config: &str,
    key: &sig::SecretKey,
) -> Result<poseidon::Credential, Error> {
    loop {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| Error("entropy source unavailable"))?;
        let salt = crate::Fr::from_be_bytes_mod_order(&bytes);
        if poseidon::field_bytes(salt) == bytes {
            return poseidon::issue(document, proof_config, key, salt);
        }
    }
}
