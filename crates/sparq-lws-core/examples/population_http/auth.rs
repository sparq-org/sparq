// [GPT-6] Separate benchmark issuer/client keys; never accept caller identity headers.
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::http::HeaderMap;
use base64::Engine as _;
use p256::ecdsa::{signature::Signer, Signature, SigningKey};
use rand_core::OsRng;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use solid_oidc_verifier::config::{StaticJwksProvider, VerifierConfig};
use solid_oidc_verifier::replay::InMemoryReplayStore;
use solid_oidc_verifier::verifier::Verifier;
use sparq_lws_core::auth::{AuthContext, VerifiedToken};
use sparq_lws_core::error::ServerError;

use super::{Result, Settings};

const ISSUER: &str = "https://issuer.benchmark.example";
const CLIENT: &str = "https://client.benchmark.example";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock predates Unix epoch")
        .as_secs()
}

fn encode(data: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn public(key: &SigningKey) -> Value {
    let point = key.verifying_key().to_encoded_point(false);
    json!({"kty":"EC", "crv":"P-256",
        "x":encode(point.x().expect("uncompressed P-256 x coordinate")),
        "y":encode(point.y().expect("uncompressed P-256 y coordinate"))})
}

fn sign(key: &SigningKey, header: Value, claims: Value) -> String {
    let input = format!(
        "{}.{}",
        encode(header.to_string().as_bytes()),
        encode(claims.to_string().as_bytes())
    );
    let signature: Signature = key.sign(input.as_bytes());
    format!("{input}.{}", encode(&signature.to_bytes()))
}

pub(super) fn provision(settings: &Settings) -> Result<()> {
    let directory = settings.path("auth-dir", "population-auth");
    fs::create_dir_all(&directory)?;
    let issuer = SigningKey::random(&mut OsRng);
    let client = SigningKey::random(&mut OsRng);
    let value = json!({"issuer":ISSUER,"client":CLIENT,
        "issuer_key":encode(&issuer.to_bytes()),"client_key":encode(&client.to_bytes())});
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut secret = options.open(directory.join("client-secrets.json"))?;
    secret.write_all(value.to_string().as_bytes())?;
    secret.sync_all()?;
    fs::write(
        directory.join("issuer-public.json"),
        json!({"issuer":ISSUER,"keys":[public(&issuer)]}).to_string(),
    )?;
    println!(
        "{}",
        json!({"record_type":"auth-provisioned", "directory":directory,
        "trust":"explicit benchmark issuer key; live WebID/OIDC discovery excluded"})
    );
    Ok(())
}

pub(super) struct Authentication {
    context: AuthContext<StaticJwksProvider, InMemoryReplayStore>,
}

impl Authentication {
    pub(super) fn load(settings: &Settings) -> Result<Self> {
        let document: Value = serde_json::from_slice(&fs::read(
            settings
                .path("auth-dir", "population-auth")
                .join("issuer-public.json"),
        )?)?;
        let issuer = document["issuer"].as_str().ok_or("missing issuer")?;
        let keys = serde_json::from_value(document["keys"].clone())?;
        let base = settings.text("base-url", "https://pods.example");
        let config =
            VerifierConfig::new(vec![issuer.into()], &base).authorized_parties(vec![CLIENT.into()]);
        let replay = InMemoryReplayStore::with_window(config.replay_ttl());
        let jwks = StaticJwksProvider::new().with_issuer(issuer, keys);
        let verifier = Verifier::new(config, jwks, replay)?;
        Ok(Self {
            context: AuthContext::new(verifier, base),
        })
    }

    pub(super) fn authenticate(
        &self,
        headers: &HeaderMap,
        path: &str,
    ) -> std::result::Result<VerifiedToken, ServerError> {
        let header = |name| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        self.context
            .authenticate(header("authorization"), header("dpop"), "POST", path)
    }
}

pub(super) struct Credentials {
    issuer: SigningKey,
    client: SigningKey,
    jwk: Value,
    thumbprint: String,
    nonce: String,
    base: String,
}

impl Credentials {
    pub(super) fn load(settings: &Settings) -> Result<Self> {
        let value: Value = serde_json::from_slice(&fs::read(
            settings
                .path("auth-dir", "population-auth")
                .join("client-secrets.json"),
        )?)?;
        let decode = |field: &str| -> Result<SigningKey> {
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(value[field].as_str().ok_or("missing signing key")?)?;
            Ok(SigningKey::from_slice(&bytes)?)
        };
        let issuer = decode("issuer_key")?;
        let client = decode("client_key")?;
        let jwk = public(&client);
        // RFC 7638 lexicographic order, independent of serde map representation.
        let canonical = format!(
            r#"{{"crv":"P-256","kty":"EC","x":{},"y":{}}}"#,
            jwk["x"], jwk["y"]
        );
        let thumbprint = encode(&Sha256::digest(canonical.as_bytes()));
        Ok(Self {
            issuer,
            client,
            jwk,
            thumbprint,
            nonce: uuid::Uuid::new_v4().to_string(),
            base: settings.text("base-url", "https://pods.example"),
        })
    }

    pub(super) fn headers(&self, webid: &str, path: &str) -> (String, String) {
        let timestamp = now();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let access = sign(
            &self.issuer,
            json!({"typ":"at+jwt","alg":"ES256"}),
            json!({
                "iss":ISSUER, "sub":webid, "webid":webid, "aud":self.base,
                "client_id":CLIENT, "iat":timestamp,"exp":timestamp+3600,
                "jti":format!("{}-at-{sequence}", self.nonce), "cnf":{"jkt":self.thumbprint}
            }),
        );
        let proof = sign(
            &self.client,
            json!({"typ":"dpop+jwt","alg":"ES256","jwk":self.jwk}),
            json!({
                "htm":"POST", "htu":format!("{}{path}",self.base), "iat":timestamp,
                "jti":format!("{}-proof-{sequence}",self.nonce), "ath":encode(&Sha256::digest(access.as_bytes()))
            }),
        );
        (format!("DPoP {access}"), proof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_forgery_replay_and_request_mismatch() -> Result<()> {
        let directory =
            std::env::temp_dir().join(format!("sparq-pop-auth-{}", uuid::Uuid::new_v4()));
        let settings = Settings(std::collections::HashMap::from([(
            "auth-dir".into(),
            directory.to_string_lossy().into_owned(),
        )]));
        provision(&settings)?;
        let auth = Authentication::load(&settings)?;
        let credentials = Credentials::load(&settings)?;
        let path = "/pods/0/sparql";
        let (access, proof) = credentials.headers("https://pods.example/p/0/profile/card#me", path);
        let mut headers = HeaderMap::new();
        headers.insert("authorization", access.parse()?);
        headers.insert("dpop", proof.parse()?);
        assert!(auth.authenticate(&headers, "/pods/1/sparql").is_err());
        assert!(auth.authenticate(&headers, path).is_ok());
        assert!(auth.authenticate(&headers, path).is_err());
        let (access, proof) = credentials.headers("https://pods.example/p/0/profile/card#me", path);
        headers.insert("authorization", format!("{access}forged").parse()?);
        headers.insert("dpop", proof.parse()?);
        assert!(auth.authenticate(&headers, path).is_err());
        for (audience, expiry) in [
            ("https://wrong-audience.example", now() + 300),
            ("https://pods.example", now() - 3600),
        ] {
            let access = sign(
                &credentials.issuer,
                json!({"typ":"at+jwt","alg":"ES256"}),
                json!({
                    "iss":ISSUER,"aud":audience,"sub":"https://pods.example/p/0/profile/card#me",
                    "webid":"https://pods.example/p/0/profile/card#me","client_id":CLIENT,
                    "cnf":{"jkt":credentials.thumbprint},"iat":now()-7200,"exp":expiry,"jti":uuid::Uuid::new_v4().to_string()
                }),
            );
            let proof = sign(
                &credentials.client,
                json!({"typ":"dpop+jwt","alg":"ES256","jwk":credentials.jwk}),
                json!({
                    "htm":"POST","htu":format!("https://pods.example{path}"),"iat":now(),
                    "jti":uuid::Uuid::new_v4().to_string(),"ath":encode(&Sha256::digest(access.as_bytes()))
                }),
            );
            headers.insert("authorization", format!("DPoP {access}").parse()?);
            headers.insert("dpop", proof.parse()?);
            assert!(auth.authenticate(&headers, path).is_err());
        }
        fs::remove_dir_all(directory)?;
        Ok(())
    }
}
