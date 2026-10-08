//! SAML 2.0 subject tokens (`lws10-authn-saml`): a `saml:Assertion`, base64url-encoded (RFC 8693
//! section 3), from an identity provider this server trusts out of band
//! (`SOLID_SERVER_LWS_SAML_IDPS_FILE`: entity id to a PEM certificate, PEM public key, or public
//! JWK).
//!
//! The assertion must carry an enveloped XML-DSig signature, the direct child of the assertion,
//! whose one reference is the assertion itself (`#` + its `ID`, which no other element shares), with
//! only the enveloped-signature and exclusive canonicalization transforms, a SHA-2 digest, and an
//! `rsa-sha256` or `ecdsa-sha256` signature under the issuer's configured key. Anything else is
//! refused rather than interpreted, which keeps signature wrapping out: the assertion that is read
//! is the document's root, and the signature covers all of it.
//!
//! Then, as the reference authorization server checks: `Conditions` with `NotBefore` and
//! `NotOnOrAfter` around now, every `AudienceRestriction` naming this authorization server and no
//! condition this server cannot evaluate, and a `Subject/NameID`, which is the LWS subject. Every
//! `SubjectConfirmation` must be bearer (holder-of-key and sender-vouches need a proof this
//! endpoint does not take), with an unexpired `SubjectConfirmationData` whose `Recipient`, which
//! is required, is the client. Subject and client must be URIs.
//!
//! The XML is parsed with quick-xml into a small tree; a DOCTYPE (and so any entity but the
//! predefined ones) or a processing instruction is refused, and exclusive XML canonicalization
//! 1.0 (without comments, with an optional `InclusiveNamespaces PrefixList`) is implemented here.

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use quick_xml::events::{BytesStart, Event};
use serde_json::Value;
use sha2::Digest;

use super::jose;
use super::subject_tokens::{parse_datetime, Verified, SKEW_SECS};
use super::{is_uri, LwsConfig};

pub const SAML_NS: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
pub const DS_NS: &str = "http://www.w3.org/2000/09/xmldsig#";
const EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
const ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
const ECDSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Largest decoded assertion accepted.
const MAX_ASSERTION: usize = 256 * 1024;
/// Deepest element nesting accepted.
const MAX_DEPTH: usize = 64;

/// How many bytes of namespace bindings a document may copy, summed over the elements that declare
/// one: an element that declares a namespace gets its own copy of every binding in scope, so
/// without a bound a few long or numerous declarations repeated on many small elements would hold
/// a quadratic amount of them. Each binding counts its prefix, its URI and [`BINDING_OVERHEAD`].
/// Elements that declare none share their parent's bindings.
const MAX_SCOPE_BYTES: usize = 4 << 20;

/// What a copied binding costs beside its prefix and URI: the map entry and two string headers.
const BINDING_OVERHEAD: usize = 64;

/// Verify a base64url-encoded SAML 2.0 assertion for an exchange at this authorization server.
pub fn verify(cfg: &LwsConfig, token: &str) -> Result<Verified, String> {
    verify_at(cfg, token, jose::now_secs())
}

pub fn verify_at(cfg: &LwsConfig, token: &str, now: i64) -> Result<Verified, String> {
    let compact: String = token.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let bytes = URL_SAFE_NO_PAD
        .decode(compact.trim_end_matches('='))
        .or_else(|_| STANDARD.decode(&compact))
        .map_err(|_| "the subject token is not a base64url-encoded XML document")?;
    if bytes.len() > MAX_ASSERTION {
        return Err("the assertion is too large".into());
    }
    let xml = std::str::from_utf8(&bytes).map_err(|_| "the assertion is not UTF-8")?;
    let root = parse(xml)?;
    if !root.is(SAML_NS, "Assertion") {
        return Err("the subject token is not a saml:Assertion".into());
    }
    if root.attr("Version") != Some("2.0") {
        return Err("the assertion is not SAML 2.0".into());
    }
    let issuer = root
        .child(SAML_NS, "Issuer")
        .map(|e| e.text().trim().to_string())
        .ok_or("the assertion names no Issuer")?;
    let trusted = cfg.saml_idps.get(&issuer).ok_or_else(|| {
        format!("the assertion's issuer {issuer} is not a trusted identity provider")
    })?;
    let key = TrustKey::parse(trusted)
        .map_err(|e| format!("the configured key of {issuer} is unusable: {e}"))?;
    verify_signature(&root, &key)?;

    let conditions = root
        .child(SAML_NS, "Conditions")
        .ok_or("the assertion has no Conditions")?;
    let time = |name: &str| -> Result<i64, String> {
        conditions
            .attr(name)
            .and_then(parse_datetime)
            .ok_or_else(|| format!("Conditions {name} is missing or not an xsd:dateTime"))
    };
    if time("NotBefore")? > now + SKEW_SECS || time("NotOnOrAfter")? <= now - SKEW_SECS {
        return Err("the assertion is outside its validity period".into());
    }
    check_conditions(conditions, cfg.issuer())?;
    let subject = root
        .child(SAML_NS, "Subject")
        .ok_or("the assertion names no subject")?;
    let name_id = subject
        .child(SAML_NS, "NameID")
        .map(|e| e.text().trim().to_string())
        .unwrap_or_default();
    if name_id.is_empty() {
        return Err("the assertion names no subject".into());
    }
    // The NameID is the LWS subject, which an access token's sub carries: it MUST be a URI.
    if !is_uri(&name_id) {
        return Err("the assertion's NameID is not a URI".into());
    }
    let client = bearer_recipient(subject, now)?;
    Ok(Verified {
        subject: name_id,
        client,
    })
}

/// SAML 2.0 core 2.4.1.1: the bearer confirmation method, the one an assertion presented by
/// whoever holds it may use. Holder-of-key needs proof of a key and sender-vouches an
/// authenticated attesting party; this endpoint has neither, so an assertion confirmed that way
/// is refused rather than accepted without its proof.
const BEARER: &str = "urn:oasis:names:tc:SAML:2.0:cm:bearer";

/// Check the assertion's `Conditions` beyond its validity period (SAML 2.0 core 2.5.1):
/// - each `AudienceRestriction` is evaluated on its own, and every one of them must name this
///   authorization server ("the assertion is addressed to [...] each AudienceRestriction" — the
///   restrictions are ANDed, the audiences within one ORed);
/// - `ProxyRestriction` limits the assertions a relying party may issue on the strength of this
///   one; this server issues access tokens, never assertions, so it is satisfied and ignored;
/// - `OneTimeUse` is refused: honouring it needs a replay cache of assertions this server does not
///   keep, and an assertion whose issuer asked for single use must not be accepted repeatedly;
/// - any other condition (a `Condition` extension, or anything this server does not know) is
///   refused, since a relying party that cannot evaluate a condition must treat the assertion as
///   indeterminate (core 2.5.1.1), never as valid.
fn check_conditions(conditions: &Element, us: &str) -> Result<(), String> {
    let mut restrictions = 0;
    for condition in conditions.elements() {
        if condition.is(SAML_NS, "AudienceRestriction") {
            restrictions += 1;
            let named = condition
                .elements()
                .filter(|a| a.is(SAML_NS, "Audience"))
                .map(|a| a.text())
                .any(|a| a.trim() == us || a.trim().strip_suffix('/') == Some(us));
            if !named {
                return Err(
                    "an AudienceRestriction of the assertion does not include this authorization server"
                        .into(),
                );
            }
        } else if condition.is(SAML_NS, "ProxyRestriction") {
            continue;
        } else if condition.is(SAML_NS, "OneTimeUse") {
            return Err("the assertion is OneTimeUse, which this server cannot honour".into());
        } else {
            return Err(format!(
                "the assertion has a condition this server does not understand: {}",
                condition.qname()
            ));
        }
    }
    if restrictions == 0 {
        return Err("the assertion's audience does not include this authorization server".into());
    }
    Ok(())
}

/// The client an assertion was issued to: the `Recipient` of its bearer subject confirmation.
///
/// Every `SubjectConfirmation` must use the bearer method (holder-of-key, sender-vouches and
/// anything else are refused, not skipped), and there must be at least one. Each must carry
/// `SubjectConfirmationData` with a `NotOnOrAfter` in the future, a `NotBefore` (if any) in the
/// past, and a `Recipient` that is a URI: the LWS client identifier ("The SAML token MUST use the
/// Recipient parameter within a saml:SubjectConfirmationData assertion for the LWS client
/// identifier"). When several confirmations are present they must all name the same client.
fn bearer_recipient(subject: &Element, now: i64) -> Result<String, String> {
    let mut client: Option<String> = None;
    for confirmation in subject
        .elements()
        .filter(|e| e.is(SAML_NS, "SubjectConfirmation"))
    {
        if confirmation.attr("Method") != Some(BEARER) {
            return Err(format!(
                "the assertion's subject confirmation method {} is not accepted; only bearer is",
                confirmation.attr("Method").unwrap_or("(none)")
            ));
        }
        let data = confirmation
            .child(SAML_NS, "SubjectConfirmationData")
            .ok_or("the bearer subject confirmation has no SubjectConfirmationData")?;
        let limit = data
            .attr("NotOnOrAfter")
            .ok_or("the bearer subject confirmation has no NotOnOrAfter")?;
        if parse_datetime(limit).is_none_or(|t| t <= now - SKEW_SECS) {
            return Err("the subject confirmation has expired".into());
        }
        if let Some(start) = data.attr("NotBefore") {
            if parse_datetime(start).is_none_or(|t| t > now + SKEW_SECS) {
                return Err("the subject confirmation is not yet valid".into());
            }
        }
        // Without a Recipient the client is unknown.
        let recipient = data
            .attr("Recipient")
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .ok_or("the assertion's SubjectConfirmationData names no Recipient (the client)")?;
        if !is_uri(recipient) {
            return Err("the assertion's Recipient is not a URI".into());
        }
        match &client {
            Some(c) if c != recipient => {
                return Err(
                    "the assertion's subject confirmations name different Recipients".into(),
                )
            }
            _ => client = Some(recipient.to_string()),
        }
    }
    client.ok_or_else(|| "the assertion has no bearer SubjectConfirmation".into())
}

// ---------------------------------------------------------------- signature

fn verify_signature(root: &Element, key: &TrustKey) -> Result<(), String> {
    let signatures: Vec<&Element> = root
        .elements()
        .filter(|e| e.is(DS_NS, "Signature"))
        .collect();
    let signature = match signatures.as_slice() {
        [] => return Err("the assertion is not signed".into()),
        [one] => *one,
        _ => return Err("the assertion carries more than one signature".into()),
    };
    let id = root
        .attr("ID")
        .filter(|i| !i.is_empty())
        .ok_or("the assertion has no ID")?;
    let mut same_id = 0;
    root.count_ids(id, &mut same_id);
    if same_id != 1 {
        return Err("the assertion's ID is not unique in the document".into());
    }
    let signed_info = signature
        .child(DS_NS, "SignedInfo")
        .ok_or("the signature has no SignedInfo")?;
    let c14n = signed_info
        .child(DS_NS, "CanonicalizationMethod")
        .ok_or("SignedInfo names no CanonicalizationMethod")?;
    if c14n.attr("Algorithm") != Some(EXC_C14N) {
        return Err("SignedInfo is not canonicalized with exclusive XML canonicalization".into());
    }
    let method = signed_info
        .child(DS_NS, "SignatureMethod")
        .and_then(|m| m.attr("Algorithm"))
        .ok_or("SignedInfo names no SignatureMethod")?;
    let references: Vec<&Element> = signed_info
        .elements()
        .filter(|e| e.is(DS_NS, "Reference"))
        .collect();
    let [reference] = references.as_slice() else {
        return Err("the signature must have exactly one reference".into());
    };
    if reference.attr("URI") != Some(format!("#{id}").as_str()) {
        return Err("the signature does not reference the assertion".into());
    }
    let mut enveloped = false;
    let mut exclusive = None;
    if let Some(transforms) = reference.child(DS_NS, "Transforms") {
        for t in transforms.elements() {
            match (t.is(DS_NS, "Transform"), t.attr("Algorithm")) {
                (true, Some(ENVELOPED)) if !enveloped && exclusive.is_none() => enveloped = true,
                (true, Some(EXC_C14N)) if exclusive.is_none() => {
                    exclusive = Some(inclusive_prefixes(t))
                }
                _ => return Err("the reference has a transform this server does not accept".into()),
            }
        }
    }
    let (true, Some(prefixes)) = (enveloped, exclusive) else {
        return Err("the reference must apply the enveloped-signature and exclusive canonicalization transforms".into());
    };
    let digest_alg = reference
        .child(DS_NS, "DigestMethod")
        .and_then(|d| d.attr("Algorithm"))
        .ok_or("the reference names no DigestMethod")?;
    let expected = reference
        .child(DS_NS, "DigestValue")
        .map(|d| d.text())
        .ok_or("the reference has no DigestValue")?;
    let expected = b64_decode(&expected).ok_or("the DigestValue is not base64")?;
    let mut canonical = String::new();
    canonicalize(
        root,
        Some(signature as *const Element),
        &prefixes,
        &BTreeMap::new(),
        &mut canonical,
    )?;
    let actual: Vec<u8> = match digest_alg {
        "http://www.w3.org/2001/04/xmlenc#sha256" => {
            sha2::Sha256::digest(canonical.as_bytes()).to_vec()
        }
        "http://www.w3.org/2001/04/xmldsig-more#sha384" => {
            sha2::Sha384::digest(canonical.as_bytes()).to_vec()
        }
        "http://www.w3.org/2001/04/xmlenc#sha512" => {
            sha2::Sha512::digest(canonical.as_bytes()).to_vec()
        }
        other => return Err(format!("unsupported digest {other}")),
    };
    if actual != expected {
        return Err("the assertion's digest does not match: it was altered after signing".into());
    }
    let mut signed = String::new();
    canonicalize(
        signed_info,
        None,
        &inclusive_prefixes(c14n),
        &BTreeMap::new(),
        &mut signed,
    )?;
    let value = signature
        .child(DS_NS, "SignatureValue")
        .map(|v| v.text())
        .ok_or("the signature has no SignatureValue")?;
    let value = b64_decode(&value).ok_or("the SignatureValue is not base64")?;
    if !key.verify(method, signed.as_bytes(), &value)? {
        return Err("the assertion's signature does not verify".into());
    }
    Ok(())
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let compact: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    STANDARD.decode(compact).ok()
}

/// The `InclusiveNamespaces PrefixList` of an exclusive canonicalization method or transform.
fn inclusive_prefixes(method: &Element) -> Vec<String> {
    method
        .elements()
        .find(|e| e.is(EXC_C14N, "InclusiveNamespaces"))
        .and_then(|e| e.attr("PrefixList"))
        .map(|l| {
            l.split_ascii_whitespace()
                .map(|p| {
                    if p == "#default" {
                        String::new()
                    } else {
                        p.to_string()
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A trusted identity provider's signing key.
#[derive(Debug, Clone)]
pub enum TrustKey {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    P256(p256::PublicKey),
}

impl TrustKey {
    /// From a PEM certificate, a PEM public key (SPKI or PKCS#1 RSA), or a public JWK.
    pub fn parse(config: &str) -> Result<Self, String> {
        let config = config.trim();
        if config.starts_with('{') {
            let jwk: Value = serde_json::from_str(config).map_err(|_| "not a JWK")?;
            return match jwk.get("kty").and_then(Value::as_str) {
                Some("RSA") => {
                    let get = |k: &str| {
                        jwk.get(k)
                            .and_then(Value::as_str)
                            .and_then(jose::b64url_decode)
                            .ok_or("an RSA JWK needs n and e")
                    };
                    Ok(TrustKey::Rsa {
                        n: get("n")?,
                        e: get("e")?,
                    })
                }
                Some("EC") => jose::ec_public_from_jwk(&jwk)
                    .map(TrustKey::P256)
                    .ok_or_else(|| "not a P-256 JWK".into()),
                _ => Err("unsupported JWK key type".into()),
            };
        }
        let (label, der) = pem(config).ok_or("not PEM")?;
        match label.as_str() {
            "CERTIFICATE" => spki_key(certificate_spki(&der).ok_or("not an X.509 certificate")?),
            "PUBLIC KEY" => spki_key(&der),
            "RSA PUBLIC KEY" => rsa_public_key(&der),
            other => Err(format!("unsupported PEM {other}")),
        }
    }

    fn verify(&self, method: &str, message: &[u8], signature: &[u8]) -> Result<bool, String> {
        match (method, self) {
            (RSA_SHA256, TrustKey::Rsa { n, e }) => {
                let key = aws_lc_rs::signature::RsaPublicKeyComponents {
                    n: n.as_slice(),
                    e: e.as_slice(),
                };
                Ok(key
                    .verify(
                        &aws_lc_rs::signature::RSA_PKCS1_2048_8192_SHA256,
                        message,
                        signature,
                    )
                    .is_ok())
            }
            (ECDSA_SHA256, TrustKey::P256(k)) => {
                use p256::ecdsa::signature::Verifier;
                let Ok(sig) = p256::ecdsa::Signature::from_slice(signature) else {
                    return Ok(false);
                };
                Ok(p256::ecdsa::VerifyingKey::from(k)
                    .verify(message, &sig)
                    .is_ok())
            }
            (m, _) => Err(format!(
                "signature method {m} does not fit the identity provider's key"
            )),
        }
    }
}

/// The label and DER of the first PEM block.
fn pem(text: &str) -> Option<(String, Vec<u8>)> {
    let start = text.find("-----BEGIN ")?;
    let rest = &text[start + 11..];
    let label_end = rest.find("-----")?;
    let label = rest[..label_end].to_string();
    let body_start = label_end + 5;
    let end = rest.find(&format!("-----END {label}-----"))?;
    let body: String = rest[body_start..end]
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    Some((label, STANDARD.decode(body).ok()?))
}

/// One DER TLV: tag, contents, and what follows.
fn der(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        (
            rest[..n]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize),
            &rest[n..],
        )
    };
    if rest.len() < len {
        return None;
    }
    Some((tag, &rest[..len], &rest[len..]))
}

/// The SubjectPublicKeyInfo of an X.509 certificate.
fn certificate_spki(cert: &[u8]) -> Option<&[u8]> {
    let (0x30, cert, _) = der(cert)? else {
        return None;
    };
    let (0x30, tbs, _) = der(cert)? else {
        return None;
    };
    let mut rest = tbs;
    let (tag, _, after) = der(rest)?;
    if tag == 0xa0 {
        rest = after; // explicit version
    }
    // serial, signature algorithm, issuer, validity, subject
    for _ in 0..5 {
        rest = der(rest)?.2;
    }
    let (0x30, _, after) = der(rest)? else {
        return None;
    };
    Some(&rest[..rest.len() - after.len()])
}

const RSA_OID: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];
const EC_OID: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const P256_OID: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];

fn spki_key(spki: &[u8]) -> Result<TrustKey, String> {
    let bad = || "not a SubjectPublicKeyInfo".to_string();
    let (0x30, body, _) = der(spki).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x30, alg, rest) = der(body).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x06, oid, params) = der(alg).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x03, bits, _) = der(rest).ok_or_else(bad)? else {
        return Err(bad());
    };
    let key = bits.strip_prefix(&[0u8]).ok_or_else(bad)?;
    if oid == RSA_OID {
        rsa_public_key(key)
    } else if oid == EC_OID {
        match der(params) {
            Some((0x06, curve, _)) if curve == P256_OID => jose::ec_public_from_sec1(key)
                .map(TrustKey::P256)
                .ok_or_else(|| "not a P-256 point".into()),
            _ => Err("only P-256 EC keys are supported".into()),
        }
    } else {
        Err("only RSA and P-256 keys are supported".into())
    }
}

fn rsa_public_key(pkcs1: &[u8]) -> Result<TrustKey, String> {
    let bad = || "not an RSA public key".to_string();
    let (0x30, body, _) = der(pkcs1).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x02, n, rest) = der(body).ok_or_else(bad)? else {
        return Err(bad());
    };
    let (0x02, e, _) = der(rest).ok_or_else(bad)? else {
        return Err(bad());
    };
    let strip = |v: &[u8]| {
        let i = v.iter().position(|b| *b != 0).unwrap_or(v.len());
        v[i..].to_vec()
    };
    Ok(TrustKey::Rsa {
        n: strip(n),
        e: strip(e),
    })
}

// ---------------------------------------------------------------- XML tree

/// An element: its qualified name, resolved namespace, attributes, the namespaces in scope, and
/// children.
#[derive(Debug, Clone)]
pub struct Element {
    prefix: String,
    local: String,
    ns: String,
    attrs: Vec<Attr>,
    /// Every namespace in scope here, prefix (`""` for the default) to URI; `""` URI undeclares.
    scope: Arc<BTreeMap<String, String>>,
    children: Vec<Node>,
}

#[derive(Debug, Clone)]
struct Attr {
    prefix: String,
    local: String,
    ns: String,
    value: String,
}

#[derive(Debug, Clone)]
enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    fn is(&self, ns: &str, local: &str) -> bool {
        self.ns == ns && self.local == local
    }

    fn qname(&self) -> String {
        if self.prefix.is_empty() {
            self.local.clone()
        } else {
            format!("{}:{}", self.prefix, self.local)
        }
    }

    /// An unqualified attribute's value.
    fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.is_empty() && a.local == local)
            .map(|a| a.value.as_str())
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first child element `{ns}local`.
    fn child(&self, ns: &str, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(ns, local))
    }

    /// How many elements in this subtree carry an `ID`, `Id` or `AssertionID` of `id`.
    fn count_ids(&self, id: &str, count: &mut usize) {
        if self.attrs.iter().any(|a| {
            a.ns.is_empty()
                && matches!(a.local.as_str(), "ID" | "Id" | "AssertionID")
                && a.value == id
        }) {
            *count += 1;
        }
        for e in self.elements() {
            e.count_ids(id, count);
        }
    }

    /// The concatenated text content.
    fn text(&self) -> String {
        let mut s = String::new();
        self.collect_text(&mut s);
        s
    }

    fn collect_text(&self, out: &mut String) {
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => e.collect_text(out),
            }
        }
    }
}

/// Resolve XML character and predefined entity references; normalize line ends. In an attribute
/// value, literal whitespace becomes a space (XML 1.0 section 3.3.3).
fn unescape(raw: &str, attribute: bool) -> Result<String, String> {
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw.as_str();
    while let Some(i) = rest.find(['&', '\t', '\n']) {
        let (before, after) = rest.split_at(i);
        out.push_str(before);
        if !after.starts_with('&') {
            out.push(if attribute {
                ' '
            } else {
                after.chars().next().unwrap_or(' ')
            });
            rest = &after[1..];
            continue;
        }
        let end = after.find(';').ok_or("an unterminated reference")?;
        let name = &after[1..end];
        match name {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            n if n.starts_with("#x") => out.push(
                u32::from_str_radix(&n[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("a bad character reference")?,
            ),
            n if n.starts_with('#') => out.push(
                n[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or("a bad character reference")?,
            ),
            _ => return Err(format!("the entity &{name}; is not allowed")),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn split_qname(q: &str) -> (String, String) {
    match q.split_once(':') {
        Some((p, l)) => (p.to_string(), l.to_string()),
        None => (String::new(), q.to_string()),
    }
}

fn start_element(
    e: &BytesStart<'_>,
    parent_scope: &Arc<BTreeMap<String, String>>,
    copies: &mut usize,
) -> Result<Element, String> {
    let qname = std::str::from_utf8(e.name().as_ref())
        .map_err(|_| "a non-UTF-8 name")?
        .to_string();
    let mut declared = Vec::new();
    let mut raw_attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|e| format!("malformed attribute: {e}"))?;
        let key = std::str::from_utf8(a.key.as_ref())
            .map_err(|_| "a non-UTF-8 attribute name")?
            .to_string();
        let raw = std::str::from_utf8(&a.value).map_err(|_| "a non-UTF-8 attribute value")?;
        let value = unescape(raw, true)?;
        if key == "xmlns" {
            declared.push((String::new(), value));
        } else if let Some(p) = key.strip_prefix("xmlns:") {
            if value.is_empty() {
                return Err("a prefixed namespace cannot be undeclared in XML 1.0".into());
            }
            declared.push((p.to_string(), value));
        } else {
            raw_attrs.push((key, value));
        }
    }
    let scope = if declared.is_empty() {
        parent_scope.clone()
    } else {
        let bytes = |(k, v): (&String, &String)| k.len() + v.len() + BINDING_OVERHEAD;
        let copied = parent_scope.iter().map(bytes).sum::<usize>()
            + declared.iter().map(|(k, v)| bytes((k, v))).sum::<usize>();
        *copies = copies.saturating_add(copied);
        if *copies > MAX_SCOPE_BYTES {
            return Err("too many namespace declarations".into());
        }
        let mut scope = BTreeMap::clone(parent_scope);
        scope.extend(declared);
        Arc::new(scope)
    };
    // Each element and attribute keeps its own copy of its namespace URI, charged to the same
    // budget before it is made: a long URI used by many elements costs what its copies do.
    let mut owned = |uri: &str| -> Result<String, String> {
        *copies = copies.saturating_add(uri.len());
        if *copies > MAX_SCOPE_BYTES {
            return Err("too many namespace declarations".into());
        }
        Ok(uri.to_string())
    };
    let (prefix, local) = split_qname(&qname);
    let ns = if prefix == "xml" {
        XML_NS.to_string()
    } else if prefix.is_empty() {
        owned(scope.get("").map(String::as_str).unwrap_or_default())?
    } else {
        let uri = scope
            .get(&prefix)
            .filter(|u| !u.is_empty())
            .ok_or_else(|| format!("unbound prefix {prefix}"))?;
        owned(uri)?
    };
    let mut attrs = Vec::new();
    for (key, value) in raw_attrs {
        let (p, l) = split_qname(&key);
        let ans = if p.is_empty() {
            String::new()
        } else if p == "xml" {
            XML_NS.to_string()
        } else {
            let uri = scope
                .get(&p)
                .filter(|u| !u.is_empty())
                .ok_or_else(|| format!("unbound prefix {p}"))?;
            owned(uri)?
        };
        if attrs.iter().any(|x: &Attr| x.ns == ans && x.local == l) {
            return Err("a repeated attribute".into());
        }
        attrs.push(Attr {
            prefix: p,
            local: l,
            ns: ans,
            value,
        });
    }
    Ok(Element {
        prefix,
        local,
        ns,
        attrs,
        scope,
        children: Vec::new(),
    })
}

/// Parse a document into its root element. Comments are dropped; a DOCTYPE, a processing
/// instruction, or anything after the root is refused.
pub fn parse(xml: &str) -> Result<Element, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    reader.config_mut().check_end_names = true;
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let empty = Arc::new(BTreeMap::new());
    let mut copies = 0;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("not well-formed XML: {e}"))?;
        match event {
            Event::Start(e) | Event::Empty(e) if root.is_some() => {
                let _ = e;
                return Err("content after the root element".into());
            }
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err("the XML nests too deeply".into());
                }
                let el = start_element(
                    &e,
                    stack.last().map(|p| &p.scope).unwrap_or(&empty),
                    &mut copies,
                )?;
                stack.push(el);
            }
            Event::Empty(e) => {
                let el = start_element(
                    &e,
                    stack.last().map(|p| &p.scope).unwrap_or(&empty),
                    &mut copies,
                )?;
                match stack.last_mut() {
                    Some(p) => p.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or("an unmatched end tag")?;
                match stack.last_mut() {
                    Some(p) => p.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => {
                let raw = std::str::from_utf8(t.as_ref()).map_err(|_| "non-UTF-8 text")?;
                match stack.last_mut() {
                    Some(p) => p.children.push(Node::Text(unescape(raw, false)?)),
                    None if raw.trim().is_empty() => {}
                    None => return Err("text outside the root element".into()),
                }
            }
            Event::CData(t) => {
                let raw = std::str::from_utf8(t.as_ref()).map_err(|_| "non-UTF-8 text")?;
                let p = stack.last_mut().ok_or("text outside the root element")?;
                p.children
                    .push(Node::Text(raw.replace("\r\n", "\n").replace('\r', "\n")));
            }
            Event::Comment(_) | Event::Decl(_) => {}
            Event::DocType(_) => return Err("a DOCTYPE is not allowed".into()),
            Event::PI(_) => return Err("processing instructions are not allowed".into()),
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err("an unclosed element".into());
    }
    root.ok_or_else(|| "no root element".into())
}

// ---------------------------------------------------------------- exclusive canonicalization

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c => out.push(c),
        }
    }
}

/// Exclusive XML Canonicalization 1.0 (without comments) of the subtree at `el`, leaving out the
/// element `exclude` (the enveloped signature). `rendered` holds the namespace declarations the
/// output ancestors rendered; `inclusive` the `InclusiveNamespaces` prefixes (`""` is the default
/// namespace), which are rendered like inclusive canonicalization does.
pub fn canonicalize(
    el: &Element,
    exclude: Option<*const Element>,
    inclusive: &[String],
    rendered: &BTreeMap<String, String>,
    out: &mut String,
) -> Result<(), String> {
    if exclude == Some(el as *const Element) {
        return Ok(());
    }
    // Namespaces this element visibly utilizes, plus the inclusive ones in scope.
    let mut wanted: Vec<String> = vec![el.prefix.clone()];
    for a in &el.attrs {
        if !a.prefix.is_empty() && a.prefix != "xml" {
            wanted.push(a.prefix.clone());
        }
    }
    // An inclusive prefix is handled as inclusive canonicalization handles it: rendered when in
    // scope and not already rendered by an output ancestor. For the default namespace that
    // includes an undeclaration (`xmlns=""`) when an output ancestor rendered a non-empty default
    // (Exclusive XML Canonicalization 1.0, section 3; Canonical XML 1.0, section 2.3).
    for p in inclusive {
        if p.is_empty() || el.scope.get(p).is_some_and(|u| !u.is_empty()) {
            wanted.push(p.clone());
        }
    }
    wanted.sort();
    wanted.dedup();
    let mut now_rendered = rendered.clone();
    let mut decls = Vec::new();
    for p in wanted {
        if p == "xml" {
            continue;
        }
        let uri = el.scope.get(&p).cloned().unwrap_or_default();
        if p.is_empty() {
            let before = rendered.get("").map(String::as_str).unwrap_or("");
            if uri != before {
                decls.push((p.clone(), uri.clone()));
                now_rendered.insert(p, uri);
            }
        } else if rendered.get(&p) != Some(&uri) {
            if uri.is_empty() {
                return Err(format!("unbound prefix {p}"));
            }
            decls.push((p.clone(), uri.clone()));
            now_rendered.insert(p, uri);
        }
    }
    let qname = el.qname();
    out.push('<');
    out.push_str(&qname);
    for (p, uri) in &decls {
        if p.is_empty() {
            out.push_str(" xmlns=\"");
        } else {
            out.push_str(" xmlns:");
            out.push_str(p);
            out.push_str("=\"");
        }
        escape_attr(uri, out);
        out.push('"');
    }
    let mut attrs: Vec<&Attr> = el.attrs.iter().collect();
    attrs.sort_by(|a, b| (a.ns.as_str(), a.local.as_str()).cmp(&(b.ns.as_str(), b.local.as_str())));
    for a in attrs {
        out.push(' ');
        if !a.prefix.is_empty() {
            out.push_str(&a.prefix);
            out.push(':');
        }
        out.push_str(&a.local);
        out.push_str("=\"");
        escape_attr(&a.value, out);
        out.push('"');
    }
    out.push('>');
    for c in &el.children {
        match c {
            Node::Text(t) => escape_text(t, out),
            Node::Element(e) => canonicalize(e, exclude, inclusive, &now_rendered, out)?,
        }
    }
    out.push_str("</");
    out.push_str(&qname);
    out.push('>');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c14n(xml: &str) -> String {
        let root = parse(xml).unwrap();
        let mut out = String::new();
        canonicalize(&root, None, &[], &BTreeMap::new(), &mut out).unwrap();
        out
    }

    /// Review finding: every element copied every namespace binding in scope. An unsigned
    /// assertion with thousands of declarations on its root and tens of thousands of small
    /// children, well inside the size limit, held over a hundred million bindings before any
    /// signature check. Elements that declare nothing now share their parent's bindings, and the
    /// copies the declaring ones make are bounded.
    #[test]
    fn namespace_scopes_are_shared_and_bounded() {
        let decls: String = (0..4096)
            .map(|i| format!(" xmlns:p{i}=\"urn:{i}\""))
            .collect();
        let shared = format!("<r{decls}>{}</r>", "<a/>".repeat(30_000));
        let root = parse(&shared).unwrap();
        assert_eq!(root.children.len(), 30_000);
        assert!(root.children.iter().all(|c| match c {
            Node::Element(e) => Arc::ptr_eq(&e.scope, &root.scope),
            Node::Text(_) => false,
        }));
        let copied = format!("<r{decls}>{}</r>", "<a xmlns:q=\"urn:q\"/>".repeat(30_000));
        assert_eq!(
            parse(&copied).unwrap_err(),
            "too many namespace declarations"
        );
        // A few bindings with long URIs cost what their bytes do.
        let long = format!("<r xmlns:p=\"urn:{}\">", "x".repeat(128 * 1024));
        let copied = format!("{long}{}</r>", "<a xmlns:q=\"urn:q\"/>".repeat(6_000));
        assert_eq!(
            parse(&copied).unwrap_err(),
            "too many namespace declarations"
        );
        // So does a long URI that undeclaring descendants resolve to.
        let long = format!("<r xmlns=\"urn:{}\">", "x".repeat(128 * 1024));
        let used = format!("{long}{}</r>", "<a/>".repeat(30_000));
        assert_eq!(parse(&used).unwrap_err(), "too many namespace declarations");
        let long = format!("<r xmlns:p=\"urn:{}\">", "x".repeat(128 * 1024));
        let used = format!("{long}{}</r>", "<a p:x=\"1\"/>".repeat(30_000));
        assert_eq!(parse(&used).unwrap_err(), "too many namespace declarations");
        // An ordinary assertion's declarations are far inside the budget.
        let some: String = (0..32)
            .map(|i| format!(" xmlns:p{i}=\"urn:{i}\""))
            .collect();
        let few = format!("<r{some}>{}</r>", "<a xmlns:q=\"urn:q\"/>".repeat(20));
        assert!(parse(&few).is_ok());
    }

    #[test]
    fn exclusive_c14n_basics() {
        // Empty elements expand, attributes sort by namespace then name, unused namespaces drop,
        // and redundant declarations are not repeated.
        assert_eq!(
            c14n(
                r#"<a:r xmlns:a="urn:a" xmlns:b="urn:b" z="1" b:y="2" a="3"><a:c xmlns:a="urn:a"/><d xmlns="urn:d">x &amp; &#x41;</d></a:r>"#
            ),
            r#"<a:r xmlns:a="urn:a" xmlns:b="urn:b" a="3" z="1" b:y="2"><a:c></a:c><d xmlns="urn:d">x &amp; A</d></a:r>"#
        );
        assert_eq!(
            c14n("<r xmlns:u=\"urn:u\" v=\"a\tb\">\r\n&gt;</r>"),
            "<r v=\"a b\">\n&gt;</r>"
        );
        assert_eq!(
            c14n(r#"<r xmlns="urn:x"><s xmlns=""/></r>"#),
            r#"<r xmlns="urn:x"><s xmlns=""></s></r>"#
        );
    }

    /// Review finding: with `#default` in the InclusiveNamespaces PrefixList, an element whose
    /// default namespace is undeclared under an output ancestor with a non-empty default must
    /// render `xmlns=""`, though it does not use the default namespace itself.
    #[test]
    fn inclusive_default_namespace_keeps_its_undeclaration() {
        let xml = r#"<r xmlns="urn:x"><p:s xmlns:p="urn:p" xmlns=""><p:t/></p:s></r>"#;
        let root = parse(xml).unwrap();
        let render = |inclusive: &[String]| {
            let mut out = String::new();
            canonicalize(&root, None, inclusive, &BTreeMap::new(), &mut out).unwrap();
            out
        };
        assert_eq!(
            render(&[String::new()]),
            r#"<r xmlns="urn:x"><p:s xmlns="" xmlns:p="urn:p"><p:t></p:t></p:s></r>"#
        );
        // Without it, the default namespace is not visibly utilized by p:s and is left out.
        assert_eq!(
            render(&[]),
            r#"<r xmlns="urn:x"><p:s xmlns:p="urn:p"><p:t></p:t></p:s></r>"#
        );
        // Nothing to undeclare when no output ancestor rendered a default.
        let root = parse(r#"<p:s xmlns:p="urn:p" xmlns=""/>"#).unwrap();
        let mut out = String::new();
        canonicalize(&root, None, &[String::new()], &BTreeMap::new(), &mut out).unwrap();
        assert_eq!(out, r#"<p:s xmlns:p="urn:p"></p:s>"#);
    }

    #[test]
    fn hostile_xml_is_refused() {
        assert!(parse(r#"<!DOCTYPE r [<!ENTITY x "y">]><r>&x;</r>"#).is_err());
        assert!(parse("<r><?pi x?></r>").is_err());
        assert!(parse("<r>&unknown;</r>").is_err());
        assert!(parse("<r></r><s/>").is_err());
        assert!(parse("<p:r/>").is_err());
    }

    /// Build and sign an assertion the way the Touchstone identity provider does, with an
    /// ecdsa-sha256 key.
    fn signed_assertion(
        key: &jose::EcKey,
        issuer: &str,
        audience: &str,
        name_id: &str,
        tamper: bool,
    ) -> String {
        signed_assertion_to(
            key,
            issuer,
            audience,
            name_id,
            Some("https://client.example/"),
            tamper,
        )
    }

    /// [`signed_assertion`] with a chosen `Recipient` (none when `None`).
    fn signed_assertion_to(
        key: &jose::EcKey,
        issuer: &str,
        audience: &str,
        name_id: &str,
        recipient: Option<&str>,
        tamper: bool,
    ) -> String {
        let recipient = recipient
            .map(|r| format!(" Recipient=\"{r}\""))
            .unwrap_or_default();
        let now = jose::now_secs();
        let confirmation = format!(
            "<saml:SubjectConfirmation Method=\"{BEARER}\"><saml:SubjectConfirmationData NotOnOrAfter=\"{}\"{recipient}/></saml:SubjectConfirmation>",
            instant(now + 300)
        );
        let restriction = format!(
            "<saml:AudienceRestriction><saml:Audience>{audience}</saml:Audience></saml:AudienceRestriction>"
        );
        signed_custom(key, issuer, name_id, &confirmation, &restriction, tamper)
    }

    /// An xsd:dateTime for `t` (Unix seconds).
    fn instant(t: i64) -> String {
        let days = t.div_euclid(86_400);
        let secs = t.rem_euclid(86_400);
        // civil from days (Hinnant)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    }

    /// A signed assertion with the given `SubjectConfirmation` elements and `Conditions`
    /// children (the conditions valid for five minutes from now).
    fn signed_custom(
        key: &jose::EcKey,
        issuer: &str,
        name_id: &str,
        confirmations: &str,
        conditions: &str,
        tamper: bool,
    ) -> String {
        let now = jose::now_secs();
        let id = "_abc123";
        let body = |name: &str| {
            format!(
                "<saml:Issuer>{issuer}</saml:Issuer>{{SIG}}<saml:Subject><saml:NameID Format=\"urn:oasis:names:tc:SAML:2.0:nameid-format:persistent\">{name}</saml:NameID>{confirmations}</saml:Subject><saml:Conditions NotBefore=\"{nb}\" NotOnOrAfter=\"{exp}\">{conditions}</saml:Conditions>",
                exp = instant(now + 300),
                nb = instant(now)
            )
        };
        let open = format!("<saml:Assertion xmlns:saml=\"{SAML_NS}\" ID=\"{id}\" IssueInstant=\"{}\" Version=\"2.0\">", instant(now));
        let unsigned = format!(
            "{open}{}</saml:Assertion>",
            body(name_id).replace("{SIG}", "")
        );
        let digest = STANDARD.encode(sha2::Sha256::digest(c14n(&unsigned).as_bytes()));
        let signed_info = format!(
            "<ds:SignedInfo><ds:CanonicalizationMethod Algorithm=\"{EXC_C14N}\"/><ds:SignatureMethod Algorithm=\"{ECDSA_SHA256}\"/><ds:Reference URI=\"#{id}\"><ds:Transforms><ds:Transform Algorithm=\"{ENVELOPED}\"/><ds:Transform Algorithm=\"{EXC_C14N}\"/></ds:Transforms><ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/><ds:DigestValue>{digest}</ds:DigestValue></ds:Reference></ds:SignedInfo>"
        );
        // SignedInfo canonicalizes with the ds namespace declared on it.
        let si_c14n = c14n(&signed_info.replacen(
            "<ds:SignedInfo>",
            &format!("<ds:SignedInfo xmlns:ds=\"{DS_NS}\">"),
            1,
        ));
        let sig = STANDARD.encode(key.sign(si_c14n.as_bytes()));
        let signature = format!("<ds:Signature xmlns:ds=\"{DS_NS}\">{signed_info}<ds:SignatureValue>{sig}</ds:SignatureValue></ds:Signature>");
        let final_name = if tamper {
            format!("{name_id}#altered")
        } else {
            name_id.to_string()
        };
        let xml = format!(
            "{open}{}</saml:Assertion>",
            body(&final_name).replace("{SIG}", &signature)
        );
        URL_SAFE_NO_PAD.encode(xml)
    }

    #[test]
    fn signed_assertions_verify() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let key = jose::EcKey::generate("idp");
        let idp = "https://idp.example/";
        cfg.saml_idps
            .insert(idp.into(), key.public_jwk().to_string());
        let ok = signed_assertion(&key, idp, cfg.issuer(), "https://alice.example/#me", false);
        let v = verify(&cfg, &ok).unwrap();
        assert_eq!(v.subject, "https://alice.example/#me");
        assert_eq!(v.client, "https://client.example/");
        // Altered after signing, foreign audience, untrusted issuer, wrong key.
        assert!(verify(
            &cfg,
            &signed_assertion(&key, idp, cfg.issuer(), "https://alice.example/#me", true)
        )
        .is_err());
        assert!(verify(
            &cfg,
            &signed_assertion(
                &key,
                idp,
                "https://other.example",
                "https://alice.example/#me",
                false
            )
        )
        .is_err());
        assert!(verify(
            &cfg,
            &signed_assertion(&key, "https://rogue.example/", cfg.issuer(), "a", false)
        )
        .is_err());
        let other = jose::EcKey::generate("x");
        assert!(verify(
            &cfg,
            &signed_assertion(&other, idp, cfg.issuer(), "a", false)
        )
        .is_err());
        // Expired.
        assert!(verify_at(&cfg, &ok, jose::now_secs() + 3600).is_err());
    }

    #[test]
    fn subject_and_recipient_must_be_uris() {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let key = jose::EcKey::generate("idp");
        let idp = "https://idp.example/";
        cfg.saml_idps
            .insert(idp.into(), key.public_jwk().to_string());
        let alice = "https://alice.example/#me";
        // A NameID that is not a URI.
        let err = verify(
            &cfg,
            &signed_assertion(&key, idp, cfg.issuer(), "alice", false),
        );
        assert!(err.unwrap_err().contains("NameID"));
        // No Recipient: refused, not replaced by the issuer.
        let none = signed_assertion_to(&key, idp, cfg.issuer(), alice, None, false);
        assert!(verify(&cfg, &none).unwrap_err().contains("Recipient"));
        // A Recipient that is not a URI.
        let bad = signed_assertion_to(&key, idp, cfg.issuer(), alice, Some("client"), false);
        assert!(verify(&cfg, &bad).unwrap_err().contains("Recipient"));
    }

    fn saml_cfg() -> (LwsConfig, jose::EcKey, &'static str) {
        let mut cfg = LwsConfig::new("http://localhost:3000");
        let key = jose::EcKey::generate("idp");
        let idp = "https://idp.example/";
        cfg.saml_idps
            .insert(idp.into(), key.public_jwk().to_string());
        (cfg, key, idp)
    }

    fn confirmation(method: &str, data: &str) -> String {
        format!("<saml:SubjectConfirmation Method=\"{method}\">{data}</saml:SubjectConfirmation>")
    }

    fn confirmation_data(attrs: &str) -> String {
        format!("<saml:SubjectConfirmationData{attrs}/>")
    }

    #[test]
    fn only_bearer_subject_confirmations_are_accepted() {
        let (cfg, key, idp) = saml_cfg();
        let alice = "https://alice.example/#me";
        let now = jose::now_secs();
        let exp = instant(now + 300);
        let ours = format!(
            "<saml:AudienceRestriction><saml:Audience>{}</saml:Audience></saml:AudienceRestriction>",
            cfg.issuer()
        );
        let good = confirmation_data(&format!(
            " NotOnOrAfter=\"{exp}\" Recipient=\"https://client.example/\""
        ));
        let verify_with = |confirmations: &str| {
            verify(
                &cfg,
                &signed_custom(&key, idp, alice, confirmations, &ours, false),
            )
        };
        assert_eq!(
            verify_with(&confirmation(BEARER, &good)).unwrap().client,
            "https://client.example/"
        );
        // Holder-of-key and sender-vouches carry no proof here: refused, alone or beside a bearer.
        for method in [
            "urn:oasis:names:tc:SAML:2.0:cm:holder-of-key",
            "urn:oasis:names:tc:SAML:2.0:cm:sender-vouches",
        ] {
            let err = verify_with(&confirmation(method, &good)).unwrap_err();
            assert!(err.contains("not accepted"), "{err}");
            let both = format!(
                "{}{}",
                confirmation(BEARER, &good),
                confirmation(method, &good)
            );
            assert!(verify_with(&both).is_err());
        }
        // No confirmation, or one without a method.
        assert!(verify_with("").unwrap_err().contains("no bearer"));
        assert!(verify_with(
            "<saml:SubjectConfirmation><saml:SubjectConfirmationData/></saml:SubjectConfirmation>"
        )
        .is_err());
        // Bearer without data, without NotOnOrAfter, expired, or not yet valid.
        assert!(verify_with(&confirmation(BEARER, "")).is_err());
        let no_limit = confirmation_data(" Recipient=\"https://client.example/\"");
        assert!(verify_with(&confirmation(BEARER, &no_limit))
            .unwrap_err()
            .contains("NotOnOrAfter"));
        let expired = confirmation_data(&format!(
            " NotOnOrAfter=\"{}\" Recipient=\"https://client.example/\"",
            instant(now - 3600)
        ));
        assert!(verify_with(&confirmation(BEARER, &expired))
            .unwrap_err()
            .contains("expired"));
        let early = confirmation_data(&format!(
            " NotBefore=\"{}\" NotOnOrAfter=\"{exp}\" Recipient=\"https://client.example/\"",
            instant(now + 3600)
        ));
        assert!(verify_with(&confirmation(BEARER, &early)).is_err());
        // Two bearer confirmations naming different clients.
        let other = confirmation_data(&format!(
            " NotOnOrAfter=\"{exp}\" Recipient=\"https://other.example/\""
        ));
        let two = format!(
            "{}{}",
            confirmation(BEARER, &good),
            confirmation(BEARER, &other)
        );
        assert!(verify_with(&two).unwrap_err().contains("different"));
    }

    #[test]
    fn every_audience_restriction_must_name_this_server() {
        let (cfg, key, idp) = saml_cfg();
        let alice = "https://alice.example/#me";
        let now = jose::now_secs();
        let bearer = confirmation(
            BEARER,
            &confirmation_data(&format!(
                " NotOnOrAfter=\"{}\" Recipient=\"https://client.example/\"",
                instant(now + 300)
            )),
        );
        let restriction = |audiences: &[&str]| {
            let inner: String = audiences
                .iter()
                .map(|a| format!("<saml:Audience>{a}</saml:Audience>"))
                .collect();
            format!("<saml:AudienceRestriction>{inner}</saml:AudienceRestriction>")
        };
        let us = cfg.issuer().to_string();
        let verify_with = |conditions: &str| {
            verify(
                &cfg,
                &signed_custom(&key, idp, alice, &bearer, conditions, false),
            )
        };
        // One restriction listing several audiences, ours among them: accepted.
        assert!(verify_with(&restriction(&["https://other.example", &us])).is_ok());
        // Two restrictions, both naming us: accepted.
        let both = format!("{}{}", restriction(&[&us]), restriction(&[&us, "x:y"]));
        assert!(verify_with(&both).is_ok());
        // Two restrictions, one of which does not name us: the assertion is not for us.
        let split = format!(
            "{}{}",
            restriction(&[&us]),
            restriction(&["https://other.example"])
        );
        assert!(verify_with(&split)
            .unwrap_err()
            .contains("AudienceRestriction"));
        // An Audience nested deeper than a direct AudienceRestriction does not count.
        let nested = format!(
            "{}<saml:ProxyRestriction><saml:Audience>{us}</saml:Audience></saml:ProxyRestriction>",
            restriction(&["https://other.example"])
        );
        assert!(verify_with(&nested).is_err());
        // No restriction at all.
        assert!(verify_with("").is_err());
        // ProxyRestriction is ignored; OneTimeUse and unknown conditions are refused.
        let proxy = format!(
            "{}<saml:ProxyRestriction Count=\"0\"/>",
            restriction(&[&us])
        );
        assert!(verify_with(&proxy).is_ok());
        let once = format!("{}<saml:OneTimeUse/>", restriction(&[&us]));
        assert!(verify_with(&once).unwrap_err().contains("OneTimeUse"));
        let unknown = format!(
            "{}<saml:Condition xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"x:Custom\"/>",
            restriction(&[&us])
        );
        assert!(verify_with(&unknown)
            .unwrap_err()
            .contains("does not understand"));
        let foreign = format!("{}<x:Whatever xmlns:x=\"urn:x\"/>", restriction(&[&us]));
        assert!(verify_with(&foreign).is_err());
    }

    #[test]
    fn trust_keys_parse() {
        let key = jose::EcKey::generate("k");
        assert!(matches!(
            TrustKey::parse(&key.public_jwk().to_string()).unwrap(),
            TrustKey::P256(_)
        ));
        // A P-256 SubjectPublicKeyInfo.
        let point =
            p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(&key.public_key(), false);
        let mut spki = vec![0x30, 0x59, 0x30, 0x13, 0x06, 0x07];
        spki.extend_from_slice(EC_OID);
        spki.extend_from_slice(&[0x06, 0x08]);
        spki.extend_from_slice(P256_OID);
        spki.extend_from_slice(&[0x03, 0x42, 0x00]);
        spki.extend_from_slice(point.as_bytes());
        let pem = format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            STANDARD.encode(&spki)
        );
        assert!(matches!(TrustKey::parse(&pem).unwrap(), TrustKey::P256(_)));
        assert!(TrustKey::parse("garbage").is_err());
    }
}
