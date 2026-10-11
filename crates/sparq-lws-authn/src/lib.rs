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

pub mod jose;

/// Allowed clock skew, in seconds, for subject token times (the reference's).
pub const SKEW_SECS: i64 = 60;

/// Who a valid subject token authenticates (the access token's `sub`) and the client presenting it
/// (its `client_id`). The server checks both are URIs before it issues anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub subject: String,
    pub client: String,
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
    /// The request's one `DPoP` header, if it carried exactly one (RFC 9449 section 4.3).
    pub dpop: Option<&'a str>,
    /// Now, in seconds since the Unix epoch.
    pub now: i64,
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
    /// takes, or one of `reserved` (the types the server checks itself): which suite checks a
    /// token is never in doubt. A server registers through its own method, which passes its
    /// reserved types, so no caller can leave them out.
    pub fn register(&mut self, suite: Arc<dyn Suite>, reserved: &[&str]) -> Result<(), String> {
        for t in suite.token_types() {
            if reserved.contains(t) || self.find(t).is_some() {
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
}
