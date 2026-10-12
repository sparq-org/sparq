// Merkle-root RDF credential suite eddsa-sha256-merkle-2026; spec in
// site/specs/zk-merkle-cryptosuite.typ.
//! Leaf, tree and signed-message functions of the `eddsa-sha256-merkle-2026`
//! credential suite.
//!
//! The issuer signs one 32-byte message that commits to a salted binary SHA-256
//! Merkle tree over the credential's quads. Each leaf hashes a typed byte encoding
//! of the quad's four terms. A literal's encoding carries a 31-byte comparison key,
//! derived only from its lexical form and datatype. Within one key class, the
//! keys' big-endian byte order is the order that the SPARQL relational operators
//! give the literals, so a proof can compare numbers, date-times and
//! `xsd:string` prefixes without parsing lexical forms. Equal keys do not make
//! two literals the same RDF term; term identity uses the whole encoding.
//!
//! The functions here are the normative computations of the spec. Issuance and
//! holder-side RDFC-1.0 canonicalization happen outside the proof; inside it,
//! [`crate::authenticated_rdf`] recomputes every leaf from the parsed terms, so
//! comparison keys always agree with lexical forms.

use oxrdf::{GraphName, Literal, Quad, Term};
use sha2::{Digest, Sha256};

/// The suite identifier, the `cryptosuite` value of its Data Integrity proofs.
pub const EDDSA_SHA256_MERKLE_2026: &str = "eddsa-sha256-merkle-2026";
/// Bytes in a comparison key: one class byte and thirty payload bytes.
pub const KEY_BYTES: usize = 31;
/// Bytes of the issuer's secret tree salt, appended to the signature in `proofValue`.
pub const SALT_BYTES: usize = 32;

/// Comparison-key classes. Keys compare in value order only within one class.
pub mod key_class {
    /// No comparison key: `rdf:langString`, other datatypes and ill-typed literals.
    pub const NONE: u8 = 0x00;
    /// `xsd:integer` or `xsd:decimal`, scaled by 10^18 and offset by 2^239.
    pub const DECIMAL: u8 = 0x01;
    /// `xsd:double` or `xsd:float`, as the IEEE 754 total-order key of the f64 value.
    pub const DOUBLE: u8 = 0x02;
    /// `xsd:dateTime` with a timezone: UTC milliseconds offset by 2^63.
    pub const DATE_TIME: u8 = 0x03;
    /// `xsd:dateTime` without a timezone, encoded as if it were UTC.
    pub const LOCAL_DATE_TIME: u8 = 0x04;
    /// `xsd:boolean`: 0 or 1.
    pub const BOOLEAN: u8 = 0x05;
    /// `xsd:string` of at most 30 bytes and no U+0000.
    pub const STRING: u8 = 0x06;
    /// A longer `xsd:string`, or one containing U+0000: its first 30 bytes.
    pub const STRING_PREFIX: u8 = 0x07;
    /// A valid value outside the key's range or precision, or NaN.
    pub const UNREPRESENTABLE: u8 = 0x7f;
}

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const PAYLOAD: usize = KEY_BYTES - 1;
const DECIMAL_SCALE: usize = 18;

/// The 31-byte comparison key of `literal`.
#[must_use]
pub fn comparison_key(literal: &Literal) -> [u8; KEY_BYTES] {
    let datatype = literal.datatype().as_str();
    let lexical = literal.value();
    let mut out = [0; KEY_BYTES];
    let local = datatype.strip_prefix(XSD);
    let class = match local {
        Some("integer") if is_integer(lexical) => decimal_key(lexical, &mut out),
        Some("decimal") if is_decimal(lexical) => decimal_key(lexical, &mut out),
        Some("double") => double_key(lexical, false, &mut out),
        Some("float") => double_key(lexical, true, &mut out),
        Some("dateTime") => date_time_key(lexical, &mut out),
        Some("boolean") => match lexical {
            "true" | "1" => {
                out[1] = 1;
                key_class::BOOLEAN
            }
            "false" | "0" => key_class::BOOLEAN,
            _ => key_class::NONE,
        },
        // SPARQL orders only simple literals and `xsd:string`; language-tagged
        // strings get no key.
        Some("string") => string_key(lexical, &mut out),
        _ => key_class::NONE,
    };
    if class == key_class::NONE || class == key_class::UNREPRESENTABLE {
        out = [0; KEY_BYTES];
    }
    out[0] = class;
    out
}

/// Appends the typed byte encoding of one term (or graph name) to `out`.
pub fn encode_term(term: &Term, out: &mut Vec<u8>) {
    match term {
        Term::NamedNode(node) => tagged(out, 0x01, node.as_str()),
        Term::BlankNode(node) => tagged(out, 0x02, node.as_str()),
        Term::Literal(literal) => {
            tagged(out, 0x03, literal.value());
            framed(out, literal.datatype().as_str());
            framed(out, literal.language().unwrap_or(""));
            out.extend_from_slice(&comparison_key(literal));
        }
        Term::Triple(_) => unreachable!("triple terms are rejected before encoding"),
    }
}

/// `SHA-256(0x00 || enc(s) || enc(p) || enc(o) || enc(g))` for one quad.
///
/// Returns `None` for a quad with an RDF 1.2 triple term, which the suite excludes.
#[must_use]
pub fn leaf(quad: &Quad) -> Option<[u8; 32]> {
    if matches!(quad.object, Term::Triple(_)) {
        return None;
    }
    let mut bytes = vec![0x00];
    encode_term(&Term::from(quad.subject.clone()), &mut bytes);
    tagged(&mut bytes, 0x01, quad.predicate.as_str());
    encode_term(&quad.object, &mut bytes);
    match &quad.graph_name {
        GraphName::DefaultGraph => bytes.push(0x00),
        GraphName::NamedNode(node) => tagged(&mut bytes, 0x01, node.as_str()),
        GraphName::BlankNode(node) => tagged(&mut bytes, 0x02, node.as_str()),
    }
    Some(Sha256::digest(&bytes).into())
}

/// The Merkle root over leaves in strictly increasing byte order.
///
/// The leaf row is padded with all-zero leaves to the next power of two; an inner
/// node is `SHA-256(0x01 || left || right)`. One leaf is its own root. Returns
/// `None` for no leaves or leaves that are not strictly increasing.
#[must_use]
pub fn root(leaves: &[[u8; 32]]) -> Option<[u8; 32]> {
    if leaves.is_empty() || leaves.windows(2).any(|pair| pair[0] >= pair[1]) {
        return None;
    }
    let mut row = leaves.to_vec();
    row.resize(leaves.len().next_power_of_two(), [0; 32]);
    while row.len() > 1 {
        row = row
            .chunks_exact(2)
            .map(|pair| {
                let mut hash = Sha256::new();
                hash.update([0x01]);
                hash.update(pair[0]);
                hash.update(pair[1]);
                hash.finalize().into()
            })
            .collect();
    }
    Some(row[0])
}

/// The 32-byte message the issuer signs with Ed25519.
///
/// `SHA-256("eddsa-sha256-merkle-2026" || 0x00 || salt || u32be(n) || root || config_digest)`,
/// where `n` is the number of quads and `config_digest` is SHA-256 of the
/// RDFC-1.0 canonical proof configuration.
#[must_use]
pub fn signed_message(salt: &[u8; SALT_BYTES], quads: u32, root: &[u8; 32], config_digest: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(EDDSA_SHA256_MERKLE_2026.as_bytes());
    hash.update([0x00]);
    hash.update(salt);
    hash.update(quads.to_be_bytes());
    hash.update(root);
    hash.update(config_digest);
    hash.finalize().into()
}

fn framed(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn tagged(out: &mut Vec<u8>, tag: u8, text: &str) {
    out.push(tag);
    framed(out, text);
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

fn unsigned(text: &str) -> (bool, &str) {
    match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    }
}

fn is_integer(lexical: &str) -> bool {
    digits(unsigned(lexical).1)
}

fn is_decimal(lexical: &str) -> bool {
    match unsigned(lexical).1.split_once('.') {
        None => is_integer(lexical),
        Some((whole, fraction)) => {
            (whole.is_empty() || digits(whole))
                && (fraction.is_empty() || digits(fraction))
                && !(whole.is_empty() && fraction.is_empty())
        }
    }
}

// Big-endian base-256 multiply-add; false on overflow of the payload width.
fn mul_add(acc: &mut [u8; PAYLOAD], mul: u32, add: u32) -> bool {
    let mut carry = add;
    for byte in acc.iter_mut().rev() {
        let value = u32::from(*byte) * mul + carry;
        *byte = value as u8;
        carry = value >> 8;
    }
    carry == 0
}

fn decimal_key(lexical: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    let (negative, body) = unsigned(lexical);
    let (whole, fraction) = body.split_once('.').unwrap_or((body, ""));
    let fraction = fraction.trim_end_matches('0');
    if fraction.len() > DECIMAL_SCALE {
        return key_class::UNREPRESENTABLE;
    }
    let mut magnitude = [0u8; PAYLOAD];
    let scaled = whole
        .bytes()
        .chain(fraction.bytes())
        .chain(std::iter::repeat_n(b'0', DECIMAL_SCALE - fraction.len()));
    for digit in scaled {
        if !mul_add(&mut magnitude, 10, u32::from(digit - b'0')) {
            return key_class::UNREPRESENTABLE;
        }
    }
    // The magnitude must stay below the 2^239 offset.
    if magnitude[0] & 0x80 != 0 {
        return key_class::UNREPRESENTABLE;
    }
    let zero = magnitude.iter().all(|b| *b == 0);
    let mut offset = [0u8; PAYLOAD];
    offset[0] = 0x80;
    let payload = if negative && !zero {
        subtract(&offset, &magnitude)
    } else {
        add(&offset, &magnitude)
    };
    out[1..].copy_from_slice(&payload);
    key_class::DECIMAL
}

fn add(a: &[u8; PAYLOAD], b: &[u8; PAYLOAD]) -> [u8; PAYLOAD] {
    let mut out = [0; PAYLOAD];
    let mut carry = 0u16;
    for i in (0..PAYLOAD).rev() {
        let sum = u16::from(a[i]) + u16::from(b[i]) + carry;
        out[i] = sum as u8;
        carry = sum >> 8;
    }
    out
}

fn subtract(a: &[u8; PAYLOAD], b: &[u8; PAYLOAD]) -> [u8; PAYLOAD] {
    let mut out = [0; PAYLOAD];
    let mut borrow = 0i16;
    for i in (0..PAYLOAD).rev() {
        let mut diff = i16::from(a[i]) - i16::from(b[i]) - borrow;
        borrow = i16::from(diff < 0);
        if diff < 0 {
            diff += 256;
        }
        out[i] = diff as u8;
    }
    out
}

fn is_xsd_double(lexical: &str) -> bool {
    let body = unsigned(lexical).1;
    if lexical == "NaN" || body == "INF" {
        return true;
    }
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        None => digits(mantissa),
        Some((whole, fraction)) => {
            (whole.is_empty() || digits(whole))
                && (fraction.is_empty() || digits(fraction))
                && !(whole.is_empty() && fraction.is_empty())
        }
    };
    mantissa_ok && exponent.is_none_or(|exponent| digits(unsigned(exponent).1))
}

fn double_key(lexical: &str, single: bool, out: &mut [u8; KEY_BYTES]) -> u8 {
    if !is_xsd_double(lexical) {
        return key_class::NONE;
    }
    let text = match lexical {
        "INF" | "+INF" => "inf",
        "-INF" => "-inf",
        "NaN" => return key_class::UNREPRESENTABLE,
        text => text,
    };
    let value = if single {
        text.parse::<f32>().map(f64::from)
    } else {
        text.parse::<f64>()
    };
    let Ok(value) = value else { return key_class::NONE };
    // Negative zero compares equal to zero.
    let bits = if value == 0.0 { 0 } else { value.to_bits() };
    let key = if bits >> 63 == 1 { !bits } else { bits ^ (1 << 63) };
    out[1..9].copy_from_slice(&key.to_be_bytes());
    key_class::DOUBLE
}

fn number(text: &str, len: usize) -> Option<i64> {
    (text.len() == len && digits(text)).then(|| text.parse().ok()).flatten()
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn date_time_key(lexical: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    match date_time_millis(lexical) {
        Ok((class, millis)) => {
            let key = (millis as u64) ^ (1 << 63);
            out[1..9].copy_from_slice(&key.to_be_bytes());
            class
        }
        Err(class) => class,
    }
}

// Parses an XSD 1.1 dateTime. Ill-typed input is `Err(NONE)`; a valid value
// outside the four-digit-year, millisecond, before-24:00 profile is
// `Err(UNREPRESENTABLE)`.
fn date_time_millis(lexical: &str) -> Result<(u8, i64), u8> {
    let invalid = key_class::NONE;
    let (date, time) = lexical.split_once('T').ok_or(invalid)?;
    let (negative, date) = match date.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, date),
    };
    let mut parts = date.split('-');
    let (year_text, month_text, day_text) = (
        parts.next().ok_or(invalid)?,
        parts.next().ok_or(invalid)?,
        parts.next().ok_or(invalid)?,
    );
    if parts.next().is_some()
        || year_text.len() < 4
        || !digits(year_text)
        || (year_text.len() > 4 && year_text.starts_with('0'))
    {
        return Err(invalid);
    }
    let (month, day) = (number(month_text, 2).ok_or(invalid)?, number(day_text, 2).ok_or(invalid)?);
    // Leap years follow the proleptic Gregorian calendar of the actual year, of any
    // length; only years of four digits can be represented.
    let modulo = |m: u32| year_text.bytes().fold(0, |acc, digit| (acc * 10 + u32::from(digit - b'0')) % m);
    let leap = modulo(4) == 0 && (modulo(100) != 0 || modulo(400) == 0);
    let year = if year_text.len() == 4 { year_text.parse::<i64>().map_err(|_| invalid)? } else { 0 };
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=12).contains(&month) || day < 1 || day > month_days[(month - 1) as usize] {
        return Err(invalid);
    }
    let (clock, zone) = match time.find(['Z', '+', '-']) {
        Some(at) => (&time[..at], Some(&time[at..])),
        None => (time, None),
    };
    let (hms, fraction) = match clock.split_once('.') {
        Some((hms, fraction)) if digits(fraction) => (hms, fraction),
        Some(_) => return Err(invalid),
        None => (clock, ""),
    };
    let mut fields = hms.split(':');
    let (hour, minute, second) = (
        number(fields.next().ok_or(invalid)?, 2).ok_or(invalid)?,
        number(fields.next().ok_or(invalid)?, 2).ok_or(invalid)?,
        number(fields.next().ok_or(invalid)?, 2).ok_or(invalid)?,
    );
    let fraction = fraction.trim_end_matches('0');
    let end_of_day = hour == 24 && minute == 0 && second == 0 && fraction.is_empty();
    if fields.next().is_some() || (hour > 23 && !end_of_day) || minute > 59 || second > 59 {
        return Err(invalid);
    }
    let offset_minutes = match zone {
        None | Some("Z") => 0,
        Some(zone) => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (h, m) = zone[1..].split_once(':').ok_or(invalid)?;
            let (h, m) = (number(h, 2).ok_or(invalid)?, number(m, 2).ok_or(invalid)?);
            if m > 59 || h * 60 + m > 14 * 60 {
                return Err(invalid);
            }
            sign * (h * 60 + m)
        }
    };
    if negative || year == 0 || end_of_day || fraction.len() > 3 {
        return Err(key_class::UNREPRESENTABLE);
    }
    let millis_part = format!("{fraction:0<3}").parse::<i64>().map_err(|_| invalid)?;
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset_minutes * 60;
    let class = if zone.is_some() { key_class::DATE_TIME } else { key_class::LOCAL_DATE_TIME };
    Ok((class, seconds * 1_000 + millis_part))
}

fn string_key(lexical: &str, out: &mut [u8; KEY_BYTES]) -> u8 {
    let bytes = lexical.as_bytes();
    let take = bytes.len().min(PAYLOAD);
    out[1..=take].copy_from_slice(&bytes[..take]);
    if bytes.len() <= PAYLOAD && !bytes.contains(&0) {
        key_class::STRING
    } else {
        key_class::STRING_PREFIX
    }
}

/// Issues a credential under this suite, outside any proof.
///
/// Canonicalizes `document` and `proof_config` with RDFC-1.0, orders the
/// document quads by leaf, signs the salted root message with `key`, and returns
/// the witness form: the leaf-ordered document, the canonical configuration, and
/// `signature || salt` as the `proofValue` bytes. `proof_config` must name this
/// suite; the relation checks that, not this function.
///
/// # Errors
/// Rejects input that does not parse or canonicalize, or that has a triple term.
#[cfg(not(target_os = "zkvm"))]
pub fn issue(
    document: &str,
    proof_config: &str,
    key: &ed25519_dalek::SigningKey,
    salt: [u8; SALT_BYTES],
) -> Result<crate::authenticated_rdf::SignedCredential, crate::Rejected> {
    use ed25519_dalek::Signer;
    let canonical = |text: &str| {
        sparq_canon::canonicalize_nquads(text).map_err(|_| crate::Rejected("issuer canonicalization rejected"))
    };
    let document = canonical(document)?;
    let proof_config = canonical(proof_config)?;
    let mut leaves = Vec::new();
    for (quad, line) in oxttl::NQuadsParser::new().for_slice(document.as_bytes()).zip(document.lines()) {
        let quad = quad.map_err(|_| crate::Rejected("issuer N-Quads parse rejected"))?;
        let leaf = leaf(&quad).ok_or(crate::Rejected("RDF 1.2 triple terms are not admitted"))?;
        leaves.push((leaf, line));
    }
    leaves.sort_unstable();
    let row: Vec<[u8; 32]> = leaves.iter().map(|(leaf, _)| *leaf).collect();
    let root = root(&row).ok_or(crate::Rejected("Merkle leaves must be non-empty and strictly increasing"))?;
    let config_digest: [u8; 32] = Sha256::digest(proof_config.as_bytes()).into();
    let message = signed_message(&salt, row.len() as u32, &root, &config_digest);
    let mut proof_value = key.sign(&message).to_bytes().to_vec();
    proof_value.extend_from_slice(&salt);
    Ok(crate::authenticated_rdf::SignedCredential {
        document: leaves.iter().map(|(_, line)| format!("{line}\n")).collect(),
        proof_config,
        signature: proof_value,
    })
}
