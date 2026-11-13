use crate::wire::*;
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_POINT,
    ristretto::{CompressedRistretto, RistrettoPoint},
    scalar::Scalar,
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use merlin::Transcript;
use rand_core::{CryptoRng, OsRng, RngCore};
use sha2::{Digest, Sha256, Sha512};
use x25519_dalek::{PublicKey, StaticSecret};

pub(crate) const DOMAIN: &[u8] = b"sparq-verifiable-aggregate/v1";
pub(crate) fn error(index: Option<u32>, check: &str) -> Error {
    Error {
        code: "protocol-error".into(),
        index,
        check: check.into(),
    }
}
pub(crate) fn guard(ok: bool, index: Option<u32>, check: &str) -> Result<()> {
    if ok { Ok(()) } else { Err(error(index, check)) }
}
pub(crate) fn fixed<const N: usize>(b: &Bytes, i: Option<u32>, check: &str) -> Result<[u8; N]> {
    b.0.as_slice().try_into().map_err(|_| error(i, check))
}
pub(crate) fn scalar(b: &Bytes, i: u32) -> Result<Scalar> {
    Option::from(Scalar::from_canonical_bytes(fixed(
        b,
        Some(i),
        "scalar-encoding",
    )?))
    .ok_or_else(|| error(Some(i), "scalar-encoding"))
}
pub(crate) fn point(b: &Bytes, i: u32) -> Result<RistrettoPoint> {
    CompressedRistretto(fixed(b, Some(i), "point-encoding")?)
        .decompress()
        .ok_or_else(|| error(Some(i), "point-encoding"))
}
pub(crate) fn sb(s: Scalar) -> Bytes {
    Bytes(s.to_bytes().to_vec())
}
pub(crate) fn pb(p: RistrettoPoint) -> Bytes {
    Bytes(p.compress().to_bytes().to_vec())
}
pub(crate) fn pedersen() -> PedersenGens {
    PedersenGens {
        B: RISTRETTO_BASEPOINT_POINT,
        B_blinding: RistrettoPoint::hash_from_bytes::<Sha512>(b"sparq-verifiable-aggregate/v1/H"),
    }
}
pub(crate) fn commit(v: Scalar, r: Scalar) -> RistrettoPoint {
    pedersen().commit(v, r)
}

/// Fixed field order; byte strings and arrays have unsigned little-endian length prefixes.
pub(crate) struct Encoder(pub Vec<u8>);
impl Encoder {
    pub fn new(label: &[u8]) -> Self {
        let mut e = Self(Vec::new());
        e.bytes(DOMAIN);
        e.bytes(label);
        e
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(&(b.len() as u64).to_le_bytes());
        self.0.extend_from_slice(b);
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn context(&mut self, sid: &Bytes, rule: &Bytes, i: u32) {
        self.bytes(&sid.0);
        self.bytes(&rule.0);
        self.u32(i);
    }
    pub fn optional(&mut self, b: &Option<Bytes>) {
        self.0.push(u8::from(b.is_some()));
        if let Some(b) = b {
            self.bytes(&b.0);
        }
    }
    pub fn commitments(&mut self, c: &Commitments) {
        self.bytes(&c.count.0);
        self.optional(&c.sum);
    }
    pub fn shares(&mut self, s: &Shares) {
        self.bytes(&s.count.0);
        self.bytes(&s.rho.0);
        self.optional(&s.sum);
        self.optional(&s.sigma);
    }
}
pub(crate) fn rule_digest(p: &Parameters) -> Bytes {
    let mut e = Encoder::new(b"rules");
    e.bytes(&p.session_id.0);
    e.0.push(match p.statistic {
        Statistic::CountAndSum => 0,
        Statistic::CountOnly => 1,
    });
    e.u32(p.min_group);
    e.u32(p.min_contributors);
    e.0.push(p.max_value_bits);
    e.0.push(u8::from(p.threshold.is_some()));
    if let Some(t) = &p.threshold {
        e.0.push(match t.statistic {
            ThresholdStatistic::Count => 0,
            ThresholdStatistic::Sum => 1,
        });
        e.0.push(match t.op {
            ThresholdOp::GreaterOrEqual => 0,
            ThresholdOp::LessThan => 1,
        });
        e.bytes(t.value.as_bytes());
    }
    e.u32(p.roster.len() as u32);
    for party in &p.roster {
        e.bytes(party.party_iri.as_bytes());
        e.bytes(&party.ed25519_public_key.0);
        e.bytes(&party.x25519_public_key.0);
        e.0.push(u8::from(party.compute_only));
    }
    hash(&e.0)
}
pub(crate) fn hash(b: &[u8]) -> Bytes {
    Bytes(Sha256::digest(b).to_vec())
}
pub(crate) fn contribution_bytes(c: &Contribution) -> Vec<u8> {
    let mut e = Encoder::new(b"contribution");
    e.context(&c.session_id, &c.rule_digest, c.index);
    e.commitments(&c.commitment);
    e.bytes(&c.range_proof.0);
    e.u32(c.share_commitments.len() as u32);
    for s in &c.share_commitments {
        e.commitments(s);
    }
    e.u32(c.ciphertexts.len() as u32);
    for packet in &c.ciphertexts {
        e.0.push(u8::from(packet.is_some()));
        if let Some(p) = packet {
            e.bytes(&p.ephemeral_key.0);
            e.bytes(&p.nonce.0);
            e.bytes(&p.ciphertext.0);
        }
    }
    e.0
}
pub(crate) fn contributions_digest(cs: &[&Contribution]) -> Bytes {
    let mut e = Encoder::new(b"contributions");
    e.u32(cs.len() as u32);
    for c in cs {
        e.bytes(&contribution_bytes(c));
        e.bytes(&c.signature.0);
    }
    hash(&e.0)
}
pub(crate) fn partial_bytes(p: &Partial) -> Vec<u8> {
    let mut e = Encoder::new(b"partial");
    e.context(&p.session_id, &p.rule_digest, p.index);
    e.bytes(&p.contributions_digest.0);
    e.shares(&p.shares);
    e.0
}
pub(crate) fn outcome_bytes(o: &SignedOutcome) -> Vec<u8> {
    let mut e = Encoder::new(b"outcome");
    e.context(&o.session_id, &o.rule_digest, o.finisher);
    e.bytes(&o.contributions_digest.0);
    match &o.outcome {
        Outcome::Release {
            count,
            rho,
            sum,
            sigma,
        } => {
            e.0.push(0);
            e.0.extend_from_slice(&count.to_le_bytes());
            e.bytes(&rho.0);
            e.0.push(u8::from(sum.is_some()));
            if let Some(s) = sum {
                e.bytes(s.as_bytes());
            }
            e.optional(sigma);
        }
        Outcome::BelowMinimumGroup { range_proof } => {
            e.0.push(1);
            e.bytes(&range_proof.0);
        }
        Outcome::TooFewContributors => e.0.push(2),
        Outcome::ThresholdVerdict {
            verdict,
            limb_commitments,
            range_proof,
        } => {
            e.0.push(3);
            e.0.push(u8::from(*verdict));
            e.u32(limb_commitments.len() as u32);
            for limb in limb_commitments {
                e.bytes(&limb.0);
            }
            e.bytes(&range_proof.0);
        }
    }
    e.0
}
pub(crate) fn sign(keys: &SecretKeys, bytes: &[u8]) -> Result<Bytes> {
    Ok(Bytes(
        SigningKey::from_bytes(&fixed(&keys.signing_secret, None, "signing-key")?)
            .sign(bytes)
            .to_bytes()
            .to_vec(),
    ))
}
pub(crate) fn verify_signature(
    party: &Party,
    bytes: &[u8],
    signature: &Bytes,
    i: u32,
) -> Result<()> {
    let key = VerifyingKey::from_bytes(&fixed(&party.ed25519_public_key, Some(i), "signing-key")?)
        .map_err(|_| error(Some(i), "signing-key"))?;
    key.verify_strict(
        bytes,
        &Signature::from_bytes(&fixed(signature, Some(i), "signature")?),
    )
    .map_err(|_| error(Some(i), "signature"))
}
pub(crate) fn transcript(s: &Session, label: &'static [u8], i: u32) -> Transcript {
    let mut t = Transcript::new(DOMAIN);
    t.append_message(b"purpose", label);
    t.append_message(b"session_id", &s.parameters.session_id.0);
    t.append_message(b"rule_digest", &s.rule_digest.0);
    t.append_u64(b"index", i as u64);
    t
}
pub(crate) fn prove(
    s: &Session,
    label: &'static [u8],
    i: u32,
    values: &[u64],
    blindings: &[Scalar],
    bits: usize,
    rng: &mut (impl RngCore + CryptoRng),
) -> Result<Bytes> {
    let (proof, _) = RangeProof::prove_multiple_with_rng(
        &BulletproofGens::new(bits, values.len()),
        &pedersen(),
        &mut transcript(s, label, i),
        values,
        blindings,
        bits,
        rng,
    )
    .map_err(|_| error(Some(i), "range-proof"))?;
    Ok(Bytes(proof.to_bytes()))
}
pub(crate) fn verify_proof(
    s: &Session,
    label: &'static [u8],
    i: u32,
    proof: &Bytes,
    points: &[RistrettoPoint],
    bits: usize,
) -> Result<()> {
    let proof = RangeProof::from_bytes(&proof.0).map_err(|_| error(Some(i), "range-proof"))?;
    proof
        .verify_multiple_with_rng(
            &BulletproofGens::new(bits, points.len()),
            &pedersen(),
            &mut transcript(s, label, i),
            &points
                .iter()
                .map(RistrettoPoint::compress)
                .collect::<Vec<_>>(),
            bits,
            &mut OsRng,
        )
        .map_err(|_| error(Some(i), "range-proof"))
}
fn aad(s: &Session, i: u32, j: u32) -> Vec<u8> {
    let mut bytes = s.parameters.session_id.0.clone();
    bytes.extend_from_slice(&s.rule_digest.0);
    bytes.extend_from_slice(&i.to_le_bytes());
    bytes.extend_from_slice(&j.to_le_bytes());
    bytes
}
fn encryption_info(label: &[u8], aad: &[u8], digest: &Bytes) -> Vec<u8> {
    let mut e = Encoder::new(b"encryption");
    e.bytes(label);
    e.bytes(aad);
    e.bytes(&digest.0);
    e.0
}
fn cipher(shared: &x25519_dalek::SharedSecret, aad: &[u8], i: u32) -> Result<ChaCha20Poly1305> {
    guard(shared.was_contributory(), Some(i), "non-contributory-key")?;
    let mut key = [0; 32];
    Hkdf::<Sha256>::new(Some(DOMAIN), shared.as_bytes())
        .expand(aad, &mut key)
        .map_err(|_| error(Some(i), "key-derivation"))?;
    Ok(ChaCha20Poly1305::new((&key).into()))
}
pub(crate) fn seal(
    s: &Session,
    label: &[u8],
    i: u32,
    j: u32,
    digest: &Bytes,
    plaintext: &[u8],
    rng: &mut (impl RngCore + CryptoRng),
) -> Result<Ciphertext> {
    let secret = StaticSecret::random_from_rng(&mut *rng);
    let public = PublicKey::from(&secret);
    let recipient = PublicKey::from(fixed(
        &s.parameters.roster[j as usize].x25519_public_key,
        Some(j),
        "encryption-key",
    )?);
    let ad = aad(s, i, j);
    let info = encryption_info(label, &ad, digest);
    let cipher = cipher(&secret.diffie_hellman(&recipient), &info, i)?;
    let mut nonce = [0; 12];
    rng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &ad,
            },
        )
        .map_err(|_| error(Some(i), "aead"))?;
    Ok(Ciphertext {
        ephemeral_key: Bytes(public.as_bytes().to_vec()),
        nonce: Bytes(nonce.to_vec()),
        ciphertext: Bytes(ciphertext),
    })
}
pub(crate) fn open(
    s: &Session,
    label: &[u8],
    i: u32,
    j: u32,
    digest: &Bytes,
    packet: &Ciphertext,
    keys: &SecretKeys,
) -> Result<Vec<u8>> {
    let secret = StaticSecret::from(fixed(&keys.encryption_secret, Some(j), "encryption-key")?);
    let public = PublicKey::from(fixed(&packet.ephemeral_key, Some(i), "ephemeral-key")?);
    let ad = aad(s, i, j);
    let info = encryption_info(label, &ad, digest);
    let cipher = cipher(&secret.diffie_hellman(&public), &info, i)?;
    let nonce: [u8; 12] = fixed(&packet.nonce, Some(i), "nonce")?;
    cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &packet.ciphertext.0,
                aad: &ad,
            },
        )
        .map_err(|_| error(Some(i), "aead"))
}

/// Production protocol randomness always comes from the operating system.
/// A supplied seed derives fixture keys only, never protocol randomness.
pub fn generate_keys(seed: Option<[u8; 32]>) -> KeyPair {
    let mut seed = seed.unwrap_or_else(|| {
        let mut s = [0; 32];
        OsRng.fill_bytes(&mut s);
        s
    });
    let mut e = Encoder::new(b"signing-key");
    e.bytes(&seed);
    let signing = SigningKey::from_bytes(&Sha256::digest(&e.0).into());
    let mut e = Encoder::new(b"encryption-key");
    e.bytes(&seed);
    let encryption = StaticSecret::from(<[u8; 32]>::from(Sha256::digest(&e.0)));
    seed.fill(0);
    KeyPair {
        secret: SecretKeys {
            signing_secret: Bytes(signing.to_bytes().to_vec()),
            encryption_secret: Bytes(encryption.to_bytes().to_vec()),
        },
        ed25519_public_key: Bytes(signing.verifying_key().to_bytes().to_vec()),
        x25519_public_key: Bytes(PublicKey::from(&encryption).as_bytes().to_vec()),
    }
}
