//! Authorization (LWS 1.0 core section 11): the storage owner may do anything, and the agent
//! that created a resource may do anything with it.

use super::{Agent, LwsState};
use crate::error::ServerError;
use crate::store::Store;

/// What a request does to a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Read,
    Modify,
    Create,
    Delete,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Read => "read",
            Action::Modify => "modify",
            Action::Create => "create",
            Action::Delete => "delete",
        }
    }
}

/// An RFC 3339 / xsd:dateTime instant, as seconds since the epoch.
///
/// Parsed over bytes with every field checked to be ASCII digits before it is read, so malformed
/// (including non-ASCII) input is `None`, never a slice-on-a-char-boundary panic.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    // YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)
    let b = s.trim().as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (
        ascii_num(b, 0, 4)?,
        ascii_num(b, 5, 2)?,
        ascii_num(b, 8, 2)?,
        ascii_num(b, 11, 2)?,
        ascii_num(b, 14, 2)?,
        ascii_num(b, 17, 2)?,
    );
    let mut rest = &b[19..];
    if let Some(frac) = rest.strip_prefix(b".") {
        let digits = frac.iter().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &frac[digits..];
    }
    let offset = match rest {
        b"Z" | b"z" => 0,
        [sign @ (b'+' | b'-'), _, _, b':', _, _] => {
            let (oh, om) = (ascii_num(rest, 1, 2)?, ascii_num(rest, 4, 2)?);
            if oh > 23 || om > 59 {
                return None;
            }
            (if *sign == b'-' { -1 } else { 1 }) * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // Days from civil (Howard Hinnant).
    let (y2, m2) = if mo <= 2 {
        (y - 1, mo + 9)
    } else {
        (y, mo - 3)
    };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// The `len` bytes at `b[at..]` as a number, if they exist and are all ASCII digits.
pub(crate) fn ascii_num(b: &[u8], at: usize, len: usize) -> Option<i64> {
    let digits = b.get(at..at.checked_add(len)?)?;
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    digits.iter().try_fold(0i64, |n, d| {
        n.checked_mul(10)?.checked_add(i64::from(d - b'0'))
    })
}

/// An RFC 3339 UTC timestamp for `secs` since the epoch.
pub fn format_rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Whether `agent` may perform `action` on the resource at `uri`; an error when the metadata the
/// decision rests on cannot be read.
pub async fn allowed<S: Store + 'static>(
    state: &LwsState<S>,
    _action: Action,
    uri: &str,
    agent: &Agent,
) -> Result<bool, ServerError> {
    if state.cfg.open || is_owner(state, agent) {
        return Ok(true);
    }
    // Past the owner, the creator may do anything with what it created.
    let meta = state.resource_meta(uri).await?;
    let subject = agent.subject.as_deref();
    Ok(subject.is_some() && subject == meta.creator.as_deref())
}

fn is_owner<S: Store + 'static>(state: &LwsState<S>, agent: &Agent) -> bool {
    let subject = agent.subject.as_deref();
    subject.is_some() && subject == state.cfg.owner.as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_round_trip() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_rfc3339("2026-10-05T16:20:08Z")
                .map(format_rfc3339)
                .as_deref(),
            Some("2026-10-05T16:20:08Z")
        );
        assert_eq!(
            parse_rfc3339("2026-10-05T18:20:08.123+02:00"),
            parse_rfc3339("2026-10-05T16:20:08Z")
        );
        assert_eq!(parse_rfc3339("yesterday"), None);
    }

    /// Review finding: a non-ASCII byte in the timezone used to reach `o[1..3]` and panic on a
    /// UTF-8 char boundary. Every malformed shape is now `None`.
    #[test]
    fn rfc3339_rejects_malformed_without_panicking() {
        for bad in [
            "2026-10-05T00:00:00+0\u{e9}00",
            "2026-10-05T00:00:00+\u{e9}:00",
            "2026-10-05T00:00:00\u{e9}0:00",
            "2026-10-05T00:00:00.\u{e9}Z",
            "2026-10-05T00:00:00.Z",
            "2026-10-05T00:00:00+0a:00",
            "2026-10-05T00:00:00+05-00",
            "2026-10-05T00:00:00+24:00",
            "+026-10-05T00:00:00Z",
            "2026-1\u{e9}5T00:00:00Z",
            "2026-10-05T00:00:\u{e9}Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad:?}");
        }
        assert_eq!(
            parse_rfc3339("2026-10-05T05:30:00+05:30"),
            parse_rfc3339("2026-10-05T00:00:00Z")
        );
    }
}
