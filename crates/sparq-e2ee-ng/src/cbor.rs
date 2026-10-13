//! A minimal **deterministic** CBOR codec (RFC 8949 §4.2.1 core deterministic
//! encoding), scoped to exactly the subset the E2EE-NG wire objects need:
//! unsigned integers, byte strings, text strings, definite-length arrays, and
//! integer-keyed maps.
//!
//! Why hand-rolled rather than a general CBOR crate: the profile requires that
//! **the same logical object always encodes to the same bytes** (so a
//! [`crate::capability::Capability`] has a stable `cap_id`, a commit has a stable
//! `CommitId`, and test vectors are reproducible) and that the **parser is
//! fail-closed with explicit limits** (§8.1). A general serializer does not
//! guarantee canonical output on the read path, and canonical *validation* on
//! decode is the load-bearing property here, so the reader enforces it directly.
//!
//! Encoding rules enforced on **both** encode and decode:
//! * integers use the shortest form (`0..=23` inline, then `u8`/`u16`/`u32`/`u64`);
//! * only definite lengths (indefinite `0x_f` is rejected);
//! * map keys are strictly ascending by encoded key bytes, with no duplicates.
//!
//! The reader additionally enforces caller-supplied [`Limits`] and rejects any
//! trailing bytes at the top level.

use crate::error::{Error, Result};

// ---- major types -----------------------------------------------------------
pub(crate) const MAJOR_UINT: u8 = 0;
const MAJOR_NINT: u8 = 1;
pub(crate) const MAJOR_BYTES: u8 = 2;
const MAJOR_TEXT: u8 = 3;
const MAJOR_ARRAY: u8 = 4;
pub(crate) const MAJOR_MAP: u8 = 5;

/// Parser limits (§8.1 "deployment limits"). A block/message that declares more
/// than any of these is rejected before any allocation proportional to the
/// declared size is made.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Largest byte/text string length accepted, in bytes.
    pub max_str_len: usize,
    /// Largest array element count accepted.
    pub max_array_len: usize,
    /// Largest map entry count accepted.
    pub max_map_len: usize,
    /// Deepest nesting (array/map) accepted while skipping unknown extension
    /// values; guards against stack exhaustion from adversarial nesting.
    pub max_depth: usize,
}

impl Default for Limits {
    /// Conservative defaults suitable for capability / envelope-header decoding.
    /// A deployment advertises its real block/message ceiling via `Hello`
    /// (§8.4) and constructs its own [`Limits`] accordingly.
    fn default() -> Self {
        Limits {
            max_str_len: 1 << 20, // 1 MiB — a single AEAD ciphertext field ceiling
            max_array_len: 4096,
            max_map_len: 64,
            max_depth: 16,
        }
    }
}

// ===========================================================================
// Writer
// ===========================================================================

/// A deterministic CBOR encoder. Append items in order; for maps use
/// [`Writer::map_entries`], which sorts entries into canonical key order for you.
#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// New empty writer.
    pub fn new() -> Self {
        Writer { buf: Vec::new() }
    }

    /// Finish, returning the canonical bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    fn head(&mut self, major: u8, arg: u64) {
        push_head(&mut self.buf, major, arg);
    }

    /// Encode an unsigned integer.
    pub fn uint(&mut self, v: u64) -> &mut Self {
        self.head(MAJOR_UINT, v);
        self
    }

    /// Encode a byte string.
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.head(MAJOR_BYTES, b.len() as u64);
        self.buf.extend_from_slice(b);
        self
    }

    /// Encode a UTF-8 text string.
    pub fn text(&mut self, s: &str) -> &mut Self {
        self.head(MAJOR_TEXT, s.len() as u64);
        self.buf.extend_from_slice(s.as_bytes());
        self
    }

    /// Append already-canonical CBOR bytes verbatim (e.g. a pre-encoded value).
    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    /// Encode a definite-length array of pre-encoded items.
    pub fn array_of(&mut self, items: &[Vec<u8>]) -> &mut Self {
        self.head(MAJOR_ARRAY, items.len() as u64);
        for it in items {
            self.raw(it);
        }
        self
    }

    /// Encode a map from integer keys to pre-encoded value bytes. Entries are
    /// sorted into canonical ascending key order here, so callers may supply
    /// them in any order; duplicate keys are a programming error and panic.
    pub fn map_entries(&mut self, mut entries: Vec<(u64, Vec<u8>)>) -> &mut Self {
        // Canonical order for unsigned-int keys is numeric ascending, which
        // matches ascending encoded-key-byte order for major-0 items.
        entries.sort_by_key(|(k, _)| *k);
        for w in entries.windows(2) {
            assert_ne!(w[0].0, w[1].0, "duplicate map key {}", w[0].0);
        }
        self.head(MAJOR_MAP, entries.len() as u64);
        for (k, v) in entries {
            self.uint(k);
            self.raw(&v);
        }
        self
    }
}

/// Append a shortest-form CBOR head (`major` type, argument `arg`) to `buf`.
/// At most [`MAX_HEAD_LEN`] bytes. Shared by [`Writer`] and the secret-bearing
/// encoder in `capability`, which writes into its own pre-reserved buffer.
pub(crate) fn push_head(buf: &mut Vec<u8>, major: u8, arg: u64) {
    let mt = major << 5;
    if arg <= 23 {
        buf.push(mt | arg as u8);
    } else if arg <= u8::MAX as u64 {
        buf.push(mt | 24);
        buf.push(arg as u8);
    } else if arg <= u16::MAX as u64 {
        buf.push(mt | 25);
        buf.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg <= u32::MAX as u64 {
        buf.push(mt | 26);
        buf.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        buf.push(mt | 27);
        buf.extend_from_slice(&arg.to_be_bytes());
    }
}

/// Upper bound on the length of one CBOR head written by [`push_head`].
pub(crate) const MAX_HEAD_LEN: usize = 9;

/// Encode a single unsigned integer to canonical CBOR bytes.
pub fn enc_uint(v: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.uint(v);
    w.into_bytes()
}

/// Encode a single byte string to canonical CBOR bytes.
pub fn enc_bytes(b: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.bytes(b);
    w.into_bytes()
}

/// Encode a single text string to canonical CBOR bytes.
pub fn enc_text(s: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.text(s);
    w.into_bytes()
}

/// Encode an array of pre-encoded items to canonical CBOR bytes.
pub fn enc_array(items: &[Vec<u8>]) -> Vec<u8> {
    let mut w = Writer::new();
    w.array_of(items);
    w.into_bytes()
}

/// Encode a map (integer key -> pre-encoded value) to canonical CBOR bytes.
pub fn enc_map(entries: Vec<(u64, Vec<u8>)>) -> Vec<u8> {
    let mut w = Writer::new();
    w.map_entries(entries);
    w.into_bytes()
}

// ===========================================================================
// Reader
// ===========================================================================

/// A canonical, limit-checked CBOR decoder over a borrowed buffer.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    limits: Limits,
}

/// A decoded map key: either an unsigned integer (a known/mandatory field) or a
/// negative integer (an extension key, to be skipped per §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Positive/zero field key.
    Uint(u64),
    /// Negative extension key (its raw `-1 - n` argument `n` is carried).
    Nint(u64),
}

impl<'a> Reader<'a> {
    /// New reader over `data` with `limits`.
    pub fn new(data: &'a [u8], limits: Limits) -> Self {
        Reader { data, pos: 0, limits }
    }

    /// Assert the whole buffer was consumed (no trailing bytes). Trailing bytes
    /// after a top-level item are non-canonical and rejected.
    pub fn finish(&self) -> Result<()> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(Error::NonCanonical("trailing bytes after top-level item"))
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Malformed("length overflow"))?;
        if end > self.data.len() {
            return Err(Error::Malformed("truncated"));
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    /// Read one header, returning `(major, argument)` and enforcing shortest-form
    /// integer encoding and definite length.
    fn head(&mut self) -> Result<(u8, u64)> {
        let b = *self.take(1)?.first().unwrap();
        let major = b >> 5;
        let info = b & 0x1f;
        let arg = match info {
            0..=23 => info as u64,
            24 => {
                let v = *self.take(1)?.first().unwrap() as u64;
                if v <= 23 {
                    return Err(Error::NonCanonical("uint not shortest (1-byte)"));
                }
                v
            }
            25 => {
                let s = self.take(2)?;
                let v = u16::from_be_bytes([s[0], s[1]]) as u64;
                if v <= u8::MAX as u64 {
                    return Err(Error::NonCanonical("uint not shortest (2-byte)"));
                }
                v
            }
            26 => {
                let s = self.take(4)?;
                let v = u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64;
                if v <= u16::MAX as u64 {
                    return Err(Error::NonCanonical("uint not shortest (4-byte)"));
                }
                v
            }
            27 => {
                let s = self.take(8)?;
                let v = u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
                if v <= u32::MAX as u64 {
                    return Err(Error::NonCanonical("uint not shortest (8-byte)"));
                }
                v
            }
            28..=30 => return Err(Error::Malformed("reserved additional-info")),
            _ => return Err(Error::NonCanonical("indefinite length not allowed")),
        };
        Ok((major, arg))
    }

    fn expect(&mut self, want: u8, what: &'static str) -> Result<u64> {
        let (major, arg) = self.head()?;
        if major != want {
            return Err(Error::Schema(what));
        }
        Ok(arg)
    }

    /// Read an unsigned integer.
    pub fn uint(&mut self) -> Result<u64> {
        self.expect(MAJOR_UINT, "expected uint")
    }

    /// Read a byte string (length-limited).
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.expect(MAJOR_BYTES, "expected byte string")? as usize;
        if n > self.limits.max_str_len {
            return Err(Error::LimitExceeded("byte string too long"));
        }
        self.take(n)
    }

    /// Read a byte string of exactly `n` bytes into a fixed array (e.g. a 32-byte id).
    pub fn bytes_fixed<const N: usize>(&mut self) -> Result<[u8; N]> {
        let b = self.bytes()?;
        if b.len() != N {
            return Err(Error::Malformed("byte string wrong fixed length"));
        }
        let mut out = [0u8; N];
        out.copy_from_slice(b);
        Ok(out)
    }

    /// Read a UTF-8 text string (length-limited).
    pub fn text(&mut self) -> Result<&'a str> {
        let n = self.expect(MAJOR_TEXT, "expected text string")? as usize;
        if n > self.limits.max_str_len {
            return Err(Error::LimitExceeded("text string too long"));
        }
        let s = self.take(n)?;
        core::str::from_utf8(s).map_err(|_| Error::Malformed("invalid utf-8"))
    }

    /// Read an array header, returning its element count (count-limited).
    pub fn array_header(&mut self) -> Result<usize> {
        let n = self.expect(MAJOR_ARRAY, "expected array")? as usize;
        if n > self.limits.max_array_len {
            return Err(Error::LimitExceeded("array too long"));
        }
        Ok(n)
    }

    /// Read a map header, returning its entry count (count-limited).
    pub fn map_header(&mut self) -> Result<usize> {
        let n = self.expect(MAJOR_MAP, "expected map")? as usize;
        if n > self.limits.max_map_len {
            return Err(Error::LimitExceeded("map too many entries"));
        }
        Ok(n)
    }

    /// Read a map key, distinguishing a mandatory positive key from a negative
    /// extension key.
    pub fn map_key(&mut self) -> Result<Key> {
        let (major, arg) = self.head()?;
        match major {
            MAJOR_UINT => Ok(Key::Uint(arg)),
            MAJOR_NINT => Ok(Key::Nint(arg)),
            _ => Err(Error::Schema("map key must be an integer")),
        }
    }

    /// Skip exactly one CBOR item (used to discard an extension value), bounded
    /// by the depth limit. Only the supported subset is skippable; anything else
    /// is malformed. Skipped content is still held to the deterministic-encoding
    /// rules (`head` enforces shortest-form/definite lengths, and nested maps
    /// get the same integer-key + canonical-order + no-duplicate checks as
    /// decoded maps), so an ignored extension value cannot smuggle
    /// non-canonical bytes past a decoder that validates the full input.
    pub fn skip_value(&mut self) -> Result<()> {
        self.skip_value_depth(0)
    }

    fn skip_value_depth(&mut self, depth: usize) -> Result<()> {
        if depth > self.limits.max_depth {
            return Err(Error::LimitExceeded("nesting too deep"));
        }
        let (major, arg) = self.head()?;
        match major {
            MAJOR_UINT | MAJOR_NINT => Ok(()),
            MAJOR_BYTES | MAJOR_TEXT => {
                let n = arg as usize;
                if n > self.limits.max_str_len {
                    return Err(Error::LimitExceeded("string too long"));
                }
                self.take(n).map(|_| ())
            }
            MAJOR_ARRAY => {
                let n = arg as usize;
                if n > self.limits.max_array_len {
                    return Err(Error::LimitExceeded("array too long"));
                }
                for _ in 0..n {
                    self.skip_value_depth(depth + 1)?;
                }
                Ok(())
            }
            MAJOR_MAP => {
                let n = arg as usize;
                if n > self.limits.max_map_len {
                    return Err(Error::LimitExceeded("map too many entries"));
                }
                // Keys in a skipped map are held to the same rules as decoded
                // maps: integer-only, strictly ascending canonical rank
                // (encoded length first, then major type, then argument), no
                // duplicates.
                let mut last: Option<(usize, u8, u64)> = None;
                for _ in 0..n {
                    let (kmajor, karg) = self.head()?;
                    if kmajor != MAJOR_UINT && kmajor != MAJOR_NINT {
                        return Err(Error::Schema("map key must be an integer"));
                    }
                    let rank = (int_key_len(karg), kmajor, karg);
                    if let Some(prev) = last {
                        if rank <= prev {
                            return Err(Error::NonCanonical("map keys not strictly ascending"));
                        }
                    }
                    last = Some(rank);
                    self.skip_value_depth(depth + 1)?; // value
                }
                Ok(())
            }
            _ => Err(Error::Malformed("unsupported major type")),
        }
    }
}

/// Encoded byte length of a shortest-form integer key head with argument `arg`
/// (the initial byte plus 0/1/2/4/8 argument bytes).
fn int_key_len(arg: u64) -> usize {
    if arg <= 23 {
        1
    } else if arg <= u8::MAX as u64 {
        2
    } else if arg <= u16::MAX as u64 {
        3
    } else if arg <= u32::MAX as u64 {
        5
    } else {
        9
    }
}

/// Helper for decoding an integer-keyed struct-map: enforces the RFC 8949 core
/// deterministic key order over **all** keys (extension keys included), rejects
/// duplicates, skips negative extension keys, and rejects unknown mandatory
/// (positive) keys.
///
/// Canonical order compares encoded-key length first, then the encoded bytes
/// lexicographically. For shortest-form integer keys that is exactly the tuple
/// `(encoded length, major type, argument)` ascending — so a same-length
/// positive key (major type 0) sorts *before* a negative key (major type 1):
/// `0x01` (+1) precedes `0x20` (-1), but `0x20` (-1) precedes `0x18 0x18` (+24).
///
/// `handle` is called once per recognized positive key to decode its value; it
/// returns `true` if the key was recognized (and it consumed the value) and
/// `false` if the key is unknown — in which case decoding fails with
/// [`Error::Schema`] (unknown mandatory field).
pub fn read_struct_map<F>(r: &mut Reader<'_>, mut handle: F) -> Result<()>
where
    F: FnMut(&mut Reader<'_>, u64) -> Result<bool>,
{
    let n = r.map_header()?;
    let mut last: Option<(usize, u8, u64)> = None; // canonical rank of last key
    for _ in 0..n {
        let key = r.map_key()?;
        let rank = match key {
            Key::Uint(a) => (int_key_len(a), MAJOR_UINT, a),
            Key::Nint(a) => (int_key_len(a), MAJOR_NINT, a),
        };
        if let Some(prev) = last {
            if rank <= prev {
                return Err(Error::NonCanonical("map keys not strictly ascending"));
            }
        }
        last = Some(rank);
        match key {
            Key::Uint(k) => {
                if !handle(r, k)? {
                    return Err(Error::Schema("unknown mandatory field"));
                }
            }
            Key::Nint(_) => {
                // Extension key: its value is discarded per §8.1, but the key's
                // position/uniqueness are checked above and the skipped value
                // itself is still canonical-validated by `skip_value`.
                r.skip_value()?;
            }
        }
    }
    Ok(())
}
