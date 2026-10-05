//! SAML 2.0 subject tokens (`lws10-authn-saml`).

use super::subject_tokens::Verified;
use super::LwsConfig;

pub fn verify(_cfg: &LwsConfig, _token: &str) -> Result<Verified, String> {
    Err("SAML assertions are not supported yet".into())
}
