//! JSON wire types. Secret-bearing local state must stay with its contributor.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use std::fmt;

/// Bytes encoded as canonical, unpadded base64url in JSON.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);
impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Bytes(..)")
    }
}
impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&URL_SAFE_NO_PAD.encode(&self.0))
    }
}
impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        URL_SAFE_NO_PAD
            .decode(text)
            .map(Self)
            .map_err(D::Error::custom)
    }
}

/// Supported statistics; sum inputs are unsigned integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Statistic {
    CountAndSum,
    CountOnly,
}

/// The aggregate to compare without opening it to the recipient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThresholdStatistic {
    Count,
    Sum,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThresholdOp {
    #[serde(rename = ">=")]
    GreaterOrEqual,
    #[serde(rename = "<")]
    LessThan,
}
/// `value` is canonical unsigned decimal text, preserving integer precision in JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Threshold {
    pub statistic: ThresholdStatistic,
    pub op: ThresholdOp,
    pub value: String,
}

/// A roster identity. The application authenticates these key bindings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Party {
    pub party_iri: String,
    pub ed25519_public_key: Bytes,
    pub x25519_public_key: Bytes,
    /// Participate in computation with a provably zero input.
    #[serde(default, rename = "computeOnly")]
    pub compute_only: bool,
}

/// Public parameters agreed before any contribution.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameters {
    pub session_id: Bytes,
    pub statistic: Statistic,
    pub min_group: u32,
    #[serde(default = "default_contributors")]
    pub min_contributors: u32,
    #[serde(default = "default_bits")]
    pub max_value_bits: u8,
    /// When present, release only a comparison verdict after enforcing the group minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Threshold>,
    pub roster: Vec<Party>,
}
fn default_contributors() -> u32 {
    3
}
fn default_bits() -> u8 {
    48
}

/// Session contains a digest, recomputed at every public entry point.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub parameters: Parameters,
    pub rule_digest: Bytes,
}

/// Private keys. Never transmit these to the recipient or other parties.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretKeys {
    pub signing_secret: Bytes,
    pub encryption_secret: Bytes,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyPair {
    pub secret: SecretKeys,
    pub ed25519_public_key: Bytes,
    pub x25519_public_key: Bytes,
}

/// Sum uses decimal text so JSON runtimes preserve the full integer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sum: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commitments {
    pub count: Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sum: Option<Bytes>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ciphertext {
    pub ephemeral_key: Bytes,
    pub nonce: Bytes,
    pub ciphertext: Bytes,
}

/// Public, signed round-one message. Exactly one ordered share slot per party.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contribution {
    pub session_id: Bytes,
    pub rule_digest: Bytes,
    pub index: u32,
    pub commitment: Commitments,
    pub range_proof: Bytes,
    pub share_commitments: Vec<Commitments>,
    pub ciphertexts: Vec<Option<Ciphertext>>,
    pub signature: Bytes,
}

/// Secret scalar openings, always canonical little-endian scalars.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shares {
    pub count: Bytes,
    pub rho: Bytes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sum: Option<Bytes>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sigma: Option<Bytes>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalShare {
    pub session_id: Bytes,
    pub rule_digest: Bytes,
    pub index: u32,
    pub shares: Shares,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContributionState {
    pub contribution: Contribution,
    pub local: LocalShare,
    /// Original input opening retained privately for an explicitly authorized audit.
    pub audit_opening: Shares,
}

/// Signed partial plaintext; contributors only, never public output.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Partial {
    pub session_id: Bytes,
    pub rule_digest: Bytes,
    pub contributions_digest: Bytes,
    pub index: u32,
    pub shares: Shares,
    pub signature: Bytes,
}
/// An encrypted signed partial addressed to one contributor.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialMessage {
    pub session_id: Bytes,
    pub rule_digest: Bytes,
    pub contributions_digest: Bytes,
    pub index: u32,
    pub recipient: u32,
    pub packet: Ciphertext,
}
/// Send only messages to peers; retain the local plaintext privately.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialDispatch {
    pub messages: Vec<PartialMessage>,
    pub local: Partial,
}

/// Recipient-visible result. Refusals carry no aggregate openings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Outcome {
    Release {
        count: u64,
        rho: Bytes,
        #[serde(skip_serializing_if = "Option::is_none")]
        sum: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sigma: Option<Bytes>,
    },
    BelowMinimumGroup {
        range_proof: Bytes,
    },
    // A struct variant makes deny_unknown_fields apply to this refusal as well.
    TooFewContributors {},
    ThresholdVerdict {
        verdict: bool,
        /// Two hidden limbs per difference, optionally followed by the group-minimum difference.
        limb_commitments: Vec<Bytes>,
        range_proof: Bytes,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedOutcome {
    pub session_id: Bytes,
    pub rule_digest: Bytes,
    pub contributions_digest: Bytes,
    pub finisher: u32,
    pub outcome: Outcome,
    pub signature: Bytes,
}

/// Private input to the single-process simulation, in roster order.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartyInput {
    pub keys: SecretKeys,
    pub input: Input,
}
/// A simulation transcript contains contributor-private partials. Never send it to a recipient.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Simulation {
    pub session: Session,
    pub contributions: Vec<Contribution>,
    pub partials: Vec<PartialDispatch>,
    pub outcome: SignedOutcome,
}

/// Named failure, with the offending roster index when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub index: Option<u32>,
    pub check: String,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {:?}: {}", self.code, self.index, self.check)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
