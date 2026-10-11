//! The interface between the Linked Web Storage authorization server and its authentication
//! suites (LWS 1.0 core section 5.2.3).
//!
//! The authorization server exchanges a subject token for an access token to its storage (RFC
//! 8693). Each authentication suite says which subject token types it takes and checks a token of
//! those types, giving the agent and the client it speaks for ([`Verified`]); the server issues
//! the access token. A suite is a [`Suite`]: a server offers the suites it is given
//! ([`Suites`]), and holds no suite's code itself.
//!
//! Also here: what more than one suite needs, the JOSE primitives ([`jose`]) and the
//! `xsd:dateTime` reading and clock skew of subject token times ([`parse_datetime`],
//! [`SKEW_SECS`]).

#![forbid(unsafe_code)]

use std::sync::Arc;

pub mod fetch;
pub mod jose;

/// The subject token type of a JWT (RFC 8693 section 3): the self-issued credentials.
pub const JWT_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:jwt";
/// The subject token type of an OpenID Connect ID Token (RFC 8693 section 3).
pub const ID_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:id_token";

/// The LWS vocabulary namespace.
pub const LWS_NS: &str = "https://www.w3.org/ns/lws#";
/// The LWS JSON-LD context.
pub const LWS_CONTEXT: &str = "https://www.w3.org/ns/lws/v1";
/// The controlled identifier JSON-LD context.
pub const CID_CONTEXT: &str = "https://www.w3.org/ns/cid/v1";

/// How many bytes of expanded terms the triples parsed from `body_len` bytes of Turtle may hold
/// between them. A prefix is written once and expanded at every use, so a short document can
/// stand for an unbounded amount of text: parsing stops (and the document is refused) past this.
pub fn expansion_budget(body_len: usize) -> usize {
    body_len.saturating_mul(16).saturating_add(1 << 20)
}

/// The bytes a parsed triple's terms hold once expanded, as [`expansion_budget`] counts them.
pub fn triple_bytes(t: &oxrdf::Triple) -> usize {
    let node = |n: &oxrdf::NamedOrBlankNode| match n {
        oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().len(),
        oxrdf::NamedOrBlankNode::BlankNode(b) => b.as_str().len(),
    };
    #[allow(unreachable_patterns)]
    let object = match &t.object {
        oxrdf::Term::NamedNode(n) => n.as_str().len(),
        oxrdf::Term::BlankNode(b) => b.as_str().len(),
        oxrdf::Term::Literal(l) => {
            l.value().len() + l.datatype().as_str().len() + l.language().map_or(0, str::len)
        }
        other => other.to_string().len(),
    };
    node(&t.subject) + t.predicate.as_str().len() + object
}

/// Allowed clock skew, in seconds, for subject token times (the reference's).
pub const SKEW_SECS: i64 = 60;

/// Who a valid subject token authenticates (the access token's `sub`) and the client presenting it
/// (its `client_id`). The server checks both are URIs before it issues anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub subject: String,
    pub client: String,
    /// When the subject token stops being valid (seconds since the epoch). The server checks it
    /// again just before it issues, so a token that expired while a suite fetched documents is
    /// refused, whatever the suite checked when it began.
    pub expires: i64,
}

impl Verified {
    /// Whether the subject token is still valid at `now`, within [`SKEW_SECS`] (as
    /// [`check_times`] reads `exp`).
    pub fn still_valid(&self, now: i64) -> bool {
        self.expires > now.saturating_sub(SKEW_SECS)
    }
}

/// A JWT subject token's `exp`, for [`Verified::expires`]. A fractional `exp` is rounded up:
/// against whole-second clocks, `ceil(exp) > t` exactly when `exp > t`, so
/// [`Verified::still_valid`] agrees with [`check_times`].
pub fn jwt_expires(jws: &jose::Jws) -> Result<i64, String> {
    let exp = jws.claim_time("exp")?.ok_or("exp is required")?;
    Ok(exp.ceil() as i64)
}

/// One token exchange, as a suite sees it.
#[derive(Debug, Clone, Copy)]
pub struct Exchange<'a> {
    /// The subject token, as the request carried it.
    pub subject_token: &'a str,
    /// Its `subject_token_type`, one the suite named in [`Suite::token_types`].
    pub subject_token_type: &'a str,
    /// The authorization server's issuer identifier: the audience a subject token must be
    /// addressed to.
    pub authorization_server: &'a str,
    /// Its token endpoint, which the request was made to (a DPoP proof's `htu`).
    pub token_endpoint: &'a str,
    /// The request's one `DPoP` header, if it carried exactly one (RFC 9449 section 4.3).
    pub dpop: Option<&'a str>,
    /// Now, in seconds since the Unix epoch.
    pub now: i64,
    /// What a suite may fetch: identity documents, provider metadata, keys.
    pub fetch: fetch::Policy,
    /// The server's client to fetch with, built by [`fetch::fetch_client`] for [`Self::fetch`].
    pub http: &'a reqwest::Client,
}

/// An authentication suite: the subject tokens it takes, and how it checks one.
#[async_trait::async_trait]
pub trait Suite: Send + Sync + 'static {
    /// The suite's name, for configuration and logs (`saml`, say).
    fn name(&self) -> &'static str;

    /// The `subject_token_type`s the suite takes. No two suites a server offers share one.
    fn token_types(&self) -> &[&'static str];

    /// The subject identifier types its subjects take (RFC 8414 metadata), if any besides the
    /// server's own.
    fn identifier_types(&self) -> &[&'static str] {
        &[]
    }

    /// Check `exchange`'s subject token; the error is the reason, for `error_description`.
    async fn verify(&self, exchange: &Exchange<'_>) -> Result<Verified, String>;
}

/// The suites a server offers, each named by the subject token types it takes.
#[derive(Clone, Default)]
pub struct Suites(Vec<Arc<dyn Suite>>);

impl std::fmt::Debug for Suites {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|s| s.name()))
            .finish()
    }
}

impl Suites {
    /// Offer `suite` too. Refused when it takes a subject token type an offered suite already
    /// takes: which suite checks a token is never in doubt.
    pub fn register(&mut self, suite: Arc<dyn Suite>) -> Result<(), String> {
        for t in suite.token_types() {
            if self.find(t).is_some() {
                return Err(format!(
                    "suite {}: subject token type {t} is already taken",
                    suite.name()
                ));
            }
        }
        self.0.push(suite);
        Ok(())
    }

    /// The suite that takes `token_type`.
    pub fn find(&self, token_type: &str) -> Option<&dyn Suite> {
        self.0
            .iter()
            .find(|s| s.token_types().contains(&token_type))
            .map(|s| &**s)
    }

    /// Every offered suite, in the order they were registered.
    pub fn iter(&self) -> impl Iterator<Item = &dyn Suite> {
        self.0.iter().map(|s| &**s)
    }
}

/// A signed JWT: three parts, JSON header and claims, and an `alg` that is not `none`.
pub fn parse_jwt(token: &str) -> Result<jose::Jws, String> {
    let jws = jose::Jws::parse(token).ok_or("the subject token is not a compact JWS")?;
    match jws.alg() {
        None => Err("the subject token names no alg".into()),
        Some(a) if a.eq_ignore_ascii_case("none") => Err("alg none is refused".into()),
        Some(_) => Ok(jws),
    }
}

/// `exp` present and not past, `iat` present and not ahead of now, `nbf` (when present) reached;
/// each, when present, a time [`jose::Jws::claim_time`] reads.
pub fn check_times(jws: &jose::Jws, now: i64) -> Result<(), String> {
    let exp = jws.claim_time("exp")?.ok_or("exp is required")?;
    if exp <= now.saturating_sub(SKEW_SECS) as f64 {
        return Err("the credential has expired".into());
    }
    let iat = jws.claim_time("iat")?.ok_or("iat is required")?;
    if iat > now.saturating_add(SKEW_SECS) as f64 {
        return Err("iat lies in the future".into());
    }
    if jws
        .claim_time("nbf")?
        .is_some_and(|nbf| nbf > now.saturating_add(SKEW_SECS) as f64)
    {
        return Err("the credential is not valid yet".into());
    }
    Ok(())
}

/// Whether `aud` names the authorization server `issuer` (with or without the trailing slash).
pub fn addressed_to(issuer: &str, jws: &jose::Jws) -> bool {
    jws.audiences()
        .iter()
        .any(|a| a == issuer || a.strip_suffix('/') == Some(issuer))
}

/// Whether `v` is an absolute IRI (RFC 3987): a scheme, a colon and something after it, with
/// every character and percent-encoding the grammar allows. One parser checks every IRI a client
/// sends, so `%ZZ`, a space or a bare fragment is refused everywhere alike.
pub fn is_uri(v: &str) -> bool {
    v.split_once(':').is_some_and(|(_, rest)| !rest.is_empty()) && oxiri::Iri::parse(v).is_ok()
}

/// An xsd:dateTime / RFC 3339 timestamp (`YYYY-MM-DDThh:mm:ss[.frac](Z|±hh:mm)`; no zone is UTC)
/// as seconds since the epoch.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', 't'])?;
    // Every field must be ASCII digits of a bounded width (so no sign, no overflow on huge years,
    // and nothing for the arithmetic below to wrap on).
    let field = |f: &str, min: usize, max: usize| -> Option<i64> {
        if f.len() < min || f.len() > max || !f.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        f.parse().ok()
    };
    let mut d = date.splitn(3, '-');
    let (y, mo, da) = (
        field(d.next()?, 4, 4)?,
        field(d.next()?, 2, 2)?,
        field(d.next()?, 2, 2)?,
    );
    let zone_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(zone_at);
    let mut time = time.splitn(2, '.');
    let (time, frac) = (time.next()?, time.next());
    if frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let mut t = time.splitn(3, ':');
    let (h, mi, se) = (
        field(t.next()?, 2, 2)?,
        field(t.next()?, 2, 2)?,
        field(t.next()?, 2, 2)?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 24 || mi > 59 || se > 60 {
        return None;
    }
    let offset = match zone {
        "" | "Z" | "z" => 0,
        z => {
            let sign = match z.as_bytes().first() {
                Some(b'-') => -1,
                Some(b'+') => 1,
                _ => return None,
            };
            let (oh, om) = z.get(1..)?.split_once(':')?;
            let (oh, om) = (field(oh, 2, 2)?, field(om, 2, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    #[test]
    fn datetimes() {
        assert_eq!(parse_datetime("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_datetime("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(
            parse_datetime("2000-03-01T01:00:00.123+01:00"),
            Some(951_868_800)
        );
        assert_eq!(
            parse_datetime("2000-02-29T23:00:00-01:00"),
            Some(951_868_800)
        );
        assert_eq!(parse_datetime("yesterday"), None);
        // Malformed (non-ASCII, signed, oversized) fields are `None`, never a panic or overflow.
        for bad in [
            "2000-03-01T00:00:00+0\u{e9}:00",
            "2000-03-01T00:00:00Zjunk",
            "2000-03-01T00:00:00.Z",
            "2000-+3-01T00:00:00Z",
            "99999999999999999999-03-01T00:00:00Z",
            "9223372036854775807-03-01T00:00:00Z",
            "2000-03-01T00:00:00+05",
        ] {
            assert_eq!(parse_datetime(bad), None, "{bad:?}");
        }
    }

    /// Review finding: a time past the range a claim may hold was read as no claim at all, so a
    /// far-future `nbf` lifted the token's validity restriction. A claim present and out of
    /// range is refused, wherever it is read. Review finding: fractional times, which RFC 7519
    /// allows, were refused; they are read and compared as they are.
    #[test]
    fn the_last_check_reads_exp_as_check_times_does() {
        let key = jose::EcKey::generate("x");
        let now: i64 = 1_000;
        let verified = |exp: Value| {
            let jws = jose::Jws::parse(
                &key.sign_jws(serde_json::Map::new(), &json!({"iat": now, "exp": exp})),
            )
            .unwrap();
            let v = Verified {
                subject: String::new(),
                client: String::new(),
                expires: jwt_expires(&jws).unwrap(),
            };
            (check_times(&jws, now).is_ok(), v.still_valid(now))
        };
        // Review finding: a fractional exp was rounded down, so the last check refused a token
        // the first had just accepted. Either side of every boundary, the two agree.
        let edge = now - SKEW_SECS;
        for exp in [
            json!(edge as f64 + 0.5),
            json!(edge as f64 - 0.5),
            json!(edge),
            json!(edge + 1),
            json!(edge as f64 + 0.999),
        ] {
            let (first, last) = verified(exp.clone());
            assert_eq!(first, last, "exp {exp}");
        }
        assert_eq!(verified(json!(edge as f64 + 0.5)), (true, true));
        assert_eq!(verified(json!(edge)), (false, false));
    }

    #[test]
    fn out_of_range_times_are_refused_not_ignored() {
        let key = jose::EcKey::generate("x");
        let now = 1_000;
        let at = |extra: Value| {
            let mut claims = json!({"iat": now, "exp": now + 300});
            claims
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            jose::Jws::parse(&key.sign_jws(serde_json::Map::new(), &claims)).unwrap()
        };
        assert_eq!(check_times(&at(json!({})), now), Ok(()));
        assert_eq!(check_times(&at(json!({"nbf": now})), now), Ok(()));
        // Fractions are NumericDates too (RFC 7519 section 2), and are compared as they are.
        for good in [
            json!({"nbf": 1.5}),
            json!({"iat": now as f64 - 0.5}),
            json!({"exp": (now + 300) as f64 + 0.5}),
        ] {
            assert_eq!(check_times(&at(good.clone()), now), Ok(()), "{good}");
        }
        assert!(check_times(&at(json!({"exp": (now - SKEW_SECS) as f64 - 0.5})), now).is_err());
        assert!(check_times(&at(json!({"nbf": (now + SKEW_SECS) as f64 + 0.5})), now).is_err());
        for bad in [
            json!({"exp": (jose::MAX_TIME as f64) + 0.5}),
            json!({"iat": -0.5}),
            json!({"nbf": jose::MAX_TIME + 1}),
            json!({"nbf": -1}),
            json!({"nbf": 1e300}),
            json!({"nbf": "soon"}),
            json!({"exp": jose::MAX_TIME + 1}),
            json!({"iat": i64::MIN}),
        ] {
            assert!(check_times(&at(bad.clone()), now).is_err(), "{bad}");
        }
    }

    #[test]
    fn alg_none_is_refused() {
        let t = format!(
            "{}.{}.",
            jose::b64url(br#"{"alg":"none"}"#),
            jose::b64url(br#"{"sub":"a"}"#)
        );
        assert!(parse_jwt(&t).is_err());
        assert!(parse_jwt("not.a").is_err());
    }
}
