// sq-tg6b — the NETWORK tier of the MPC benchmark matrix (Tier 2/3).
// New module: a REAL multi-process transport where each of N parties is its own
// OS process, exchanging the actual protocol messages over a loopback TCP socket
// (`127.0.0.1`). The codec + `Channel` + `Coordinator` are transport-AGNOSTIC
// (generic over any `Read + Write`, so a `std::os::unix::net::UnixStream` slots
// in unchanged), but the shipped runner uses TCP loopback. This is what makes
// wall-clock latency a meaningful MPC cost — unlike the in-process counting tier
// (`bench.rs`/`metrics.rs`, sq-sxm), where every "party" is a function call and
// there is no network.
//
// HONESTY (design record §5, the critical-honesty constraint): the in-process
// tier emits MODELLED communication (deterministic byte/round counts); this tier
// emits MEASURED wall-clock + MEASURED bytes-on-wire under a real transport. The
// two must agree on the protocol RESULT (the loopback validation test), which is
// the only correctness anchor — a faster-but-wrong network run is caught.
//
// What is privileged-gated: the netem LAN/WAN shaping (`netprofiles.rs` +
// `scripts/mpc-netem.sh`) needs CAP_NET_ADMIN (sudo `tc qdisc`), NOT available
// unprivileged on the dev box, so the LAN/WAN-shaped runs are documented +
// shipped but gated to a privileged host or the EC2 tier (bead sq-hoaj). The
// loopback transport itself needs no privilege and is validated here.
//! Real multi-process transport for the MPC benchmark network tier (Tier 2/3).
//!
//! ## The two tiers this enables
//!
//! - **Tier 2 (loopback, unprivileged):** N OS processes over `127.0.0.1`
//!   exchanging the actual protocol messages — real serialisation, real socket
//!   syscalls, ~0 RTT. Gives real bytes-on-wire and real round latency at
//!   LAN-ideal. Validated here ([`run_cell_networked`] vs the in-process result).
//!   (The transport types are generic over any `Read + Write`, so a Unix-domain
//!   stream works too; the shipped runner uses loopback TCP.)
//! - **Tier 3 (netem-shaped, privileged):** the same processes under a Linux
//!   `tc qdisc netem` LAN/WAN profile ([`crate::netprofiles`]). Needs
//!   CAP_NET_ADMIN, so it runs on a privileged host or the EC2 bead (sq-hoaj),
//!   driven by `scripts/mpc-netem.sh`. The transport is identical; only the
//!   kernel queue discipline changes.
//!
//! ## Protocol fidelity — the star-coordinator simulation harness
//!
//! Each **party process** holds ONLY its own share-points (its column of the
//! Shamir sharing) — never the cleartext inputs. A **coordinator** (the
//! benchmark driver) deals shares out to the parties and collects the per-party
//! contributions needed to open a result, exactly as the in-process dealer does,
//! but now over a socket. This star topology is the standard MPC-DB simulation
//! harness (MP-SPDZ's `Player` / Secrecy's coordinator); it faithfully exercises
//! serialisation + socket round-trips while keeping the harness small. The
//! *deployed* protocol would use a full party mesh; the star is the loopback
//! measurement vehicle, and the round count it pays matches the modelled
//! `round_count` of the in-process tier (asserted by the loopback test).
//!
//! ## Why this is native-only
//!
//! Sockets + child processes have no browser story; `sparq-mpc` is already
//! excluded from `sparq-wasm` (verified by `cargo tree -p sparq-wasm`), and this
//! module uses only `std::net` / `std::io` / `std::thread` (the example driver
//! adds `std::process`), so the exclusion is preserved.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::bench::QueryClass;
use crate::field::{Fp, P};
use crate::metrics::FIELD_BYTES;
use crate::partial::MpcError;
use crate::shamir::{self, ShamirBackend, ShamirDealer, Share};

// =============================================================================
// Wire codec — length-prefixed frames of field elements / control words.
// =============================================================================

/// One on-wire message in the network-tier protocol. Every variant serialises to
/// a self-describing length-prefixed frame; the codec is hand-written (no `serde`
/// — the crate stays dependency-minimal, mirroring `bench::MatrixResults::to_json`).
///
/// Field elements travel as their canonical `u64` representative in big-endian
/// (8 bytes each — [`FIELD_BYTES`]), so the measured bytes-on-wire equals the
/// modelled `bytes_per_party` of the in-process tier up to the small fixed frame
/// header.
///
/// PR #96: the harness reports a single RAW byte count
/// ([`NetworkCell::bytes_sent`] / [`NetworkCell::bytes_recv`]) — total bytes on
/// the wire, which includes the per-frame length+tag header and the 8-byte `x`
/// coordinate of every share, NOT a field-payload-only figure. When comparing to
/// the modelled `FIELD_BYTES` accounting, account for that fixed overhead rather
/// than expecting an exact field-only match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Coordinator → party: "here are your share-points for this batch of
    /// dealt secrets" — one [`Share`] value per dealt secret (all at this party's
    /// fixed evaluation point `x`).
    Shares(Vec<Share>),
    /// Party → coordinator: "here are my contributions to open these values" —
    /// one share value per value being opened (the party's broadcast for the
    /// recombine step). Carries the party's `x` so the coordinator can interpolate.
    Open(Vec<Share>),
    /// Coordinator → party: run this query-class step over the shares you hold.
    /// Carries the opcode so a party process is a thin protocol executor.
    Step(StepCode),
    /// Coordinator → party (network-tier oblivious shuffle / sort, sq-bdbv): apply
    /// ONE compare-exchange / permutation switch to the share-COLUMN you hold. The
    /// gate acts on wire indices `(a, b)` (row positions, `a < b`); `swap` is the
    /// public realised control bit / comparator outcome for THIS gate. A `swap`
    /// exchanges the party's two share-points at those rows (pure-local relabeling,
    /// degree `t` preserved — the same local swap the in-process
    /// [`crate::oblivious`] network does); a non-swap leaves them. This is the
    /// per-switch / per-comparison message the bead asks for — unlike the
    /// single-round aggregate/join, the shuffle/sort drive one `Swap` per network
    /// gate, so the round count is the network's gate count. The switch TOPOLOGY
    /// (`a`, `b`) is data-INDEPENDENT (a function of the row count only — the
    /// obliviousness substrate); only the boolean varies, and for the shuffle it is
    /// a fresh random permutation's routing bit, for the disclosed-key sort the
    /// public comparator outcome.
    Swap { a: u32, b: u32, swap: bool },
    /// Either direction: protocol is done for this cell; close cleanly.
    Done,
}

/// The per-cell protocol step a party executes (matches the in-process cells).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepCode {
    /// Cumulative-sum aggregate: locally ADD all share-points you hold (free,
    /// non-interactive) and send the single summed share back to open.
    SumAndOpen,
    /// Hidden-value equality: you hold `(d_share, r_share)` per candidate pair;
    /// locally multiply them (degree-`2t` product) and send each product share
    /// back to open. The match bit is `product == 0`.
    MulAndOpen,
    /// Oblivious shuffle / sort (sq-bdbv): you hold ONE share-point per row (a
    /// column of the secret-shared rows). Receive the dealt column, then a stream
    /// of [`Message::Swap`] gates (the permutation network's switches / the sort
    /// network's compare-exchanges, in application order), apply each locally to
    /// your column, and on [`Message::Done`] send the whole permuted column back to
    /// open. Multi-round: one `Swap` per network gate. The party never learns the
    /// permutation as a whole — only the per-gate boolean — and never reconstructs.
    SwapNetworkAndOpen,
}

impl StepCode {
    fn to_byte(self) -> u8 {
        match self {
            StepCode::SumAndOpen => 1,
            StepCode::MulAndOpen => 2,
            StepCode::SwapNetworkAndOpen => 3,
        }
    }
    fn from_byte(b: u8) -> Result<Self, MpcError> {
        match b {
            1 => Ok(StepCode::SumAndOpen),
            2 => Ok(StepCode::MulAndOpen),
            3 => Ok(StepCode::SwapNetworkAndOpen),
            other => Err(MpcError::Protocol(format!("unknown StepCode {other}"))),
        }
    }
}

// Frame tags.
const TAG_SHARES: u8 = 1;
const TAG_OPEN: u8 = 2;
const TAG_STEP: u8 = 3;
const TAG_DONE: u8 = 4;
const TAG_SWAP: u8 = 5;

fn write_share(buf: &mut Vec<u8>, s: &Share) {
    buf.extend_from_slice(&s.x.to_be_bytes());
    buf.extend_from_slice(&s.y.value().to_be_bytes());
}

/// Refuse a declared item count that the bytes still in the frame cannot possibly
/// hold, BEFORE anything is reserved for it.
///
/// A length-prefixed decode reads its count from the frame, so the count is
/// attacker-chosen while the frame is not: a `u32` count of `0xffffffff` at
/// `8 + FIELD_BYTES` bytes an item asks for about 64 GiB, and
/// `Vec::with_capacity` on that aborts the process rather than returning an error.
/// Reading item by item and letting the short-frame check fail is not enough,
/// because the reserve happens first.
///
/// The test is `declared <= remaining / per_item`. Dividing rather than
/// multiplying `declared` by `per_item` means there is no product to overflow, and
/// a `per_item` of zero — which no caller has, but which would make the division
/// meaningless — is refused rather than panicking.
fn checked_item_count(
    what: &str,
    declared: usize,
    remaining: usize,
    per_item: usize,
) -> Result<usize, MpcError> {
    if per_item == 0 {
        return Err(MpcError::Protocol(format!(
            "{}: zero-width items cannot be counted from a frame",
            what
        )));
    }
    let affordable = remaining / per_item;
    if declared > affordable {
        return Err(MpcError::Protocol(format!(
            "{}: frame declares {} item(s) but its remaining {} byte(s) hold at most \
             {} at {} byte(s) each — refusing before reserving",
            what, declared, remaining, affordable, per_item
        )));
    }
    Ok(declared)
}

/// Refuse a wire `u64` that is not a CANONICAL `Fp` representative (`>= `[`P`]).
///
/// Both halves of a wire `Share` are interpreted in `F_p`: the value through
/// `Fp::new`, and the evaluation point wherever a column is interpolated over it
/// (`Fp::new(share.x)`). `Fp::new` REDUCES, so a `>= P` word is a second encoding
/// of a value that already has one — two different frames would decode to the same
/// `Share`, which makes the encoding non-injective and a frame digest meaningless.
/// Refuse it instead.
///
/// The refusal names the FIELD (`value` or `point`) and the modulus, both public.
/// It does not print the word: a share's value is secret-shared data, and a
/// refusal is exactly where such a word would otherwise reach a log.
fn require_canonical(field: &str, word: u64) -> Result<(), MpcError> {
    if word >= P {
        return Err(MpcError::Protocol(format!(
            "non-canonical Share {} on the wire: the word is not below the field modulus \
             {}, and Fp reduces it, so two different frames would decode to one Share — \
             refusing",
            field, P
        )));
    }
    Ok(())
}

/// How much a receiver may reserve for a body of `len` declared bytes BEFORE any
/// of them have arrived: one [`RECV_CHUNK_BYTES`] chunk at most.
///
/// The declared length is the peer's own number, checked only against
/// [`MAX_FRAME_BYTES`], so reserving `len` would let a 9-byte header commit the
/// receiver to the whole cap. The body then grows as bytes actually arrive. This
/// is the decision itself rather than a hint, so it is a named function with its
/// own test: the reservation is not observable through the public API, and
/// `a_declared_length_commits_at_most_one_chunk` can only pin the READ sizes.
fn initial_body_capacity(len: usize) -> usize {
    len.min(RECV_CHUNK_BYTES)
}

// The widest capacity the LAST body buffer held while filling, recorded by
// `Channel::read_body_observed`. A finished body's own capacity says nothing about
// how far ahead of the data the buffer ran while filling, and `recv` returns only
// the message, so the chunk-step growth has no public observable. Tests read the
// peak through here.
#[cfg(test)]
thread_local! {
    static LAST_BODY_PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Add `bytes` to a running byte counter with CHECKED arithmetic.
///
/// The counters are reported as session totals, so a wrapped counter would
/// under-report traffic and go unnoticed; an overflow is a refusal instead. The
/// refusal names the counter and the public byte count.
fn add_bytes(total: u64, bytes: usize, what: &str) -> Result<u64, MpcError> {
    u64::try_from(bytes)
        .ok()
        .and_then(|b| total.checked_add(b))
        .ok_or_else(|| {
            MpcError::Protocol(format!(
                "transport {} counter overflows u64 at {} more byte(s) — refusing rather \
                 than wrapping to a smaller total",
                what, bytes
            ))
        })
}

fn read_share(bytes: &[u8], off: &mut usize) -> Result<Share, MpcError> {
    let need = 8 + FIELD_BYTES;
    if *off + need > bytes.len() {
        return Err(MpcError::Protocol("short Share frame".into()));
    }
    let x = u64::from_be_bytes(bytes[*off..*off + 8].try_into().unwrap());
    *off += 8;
    let y = u64::from_be_bytes(bytes[*off..*off + FIELD_BYTES].try_into().unwrap());
    *off += FIELD_BYTES;
    require_canonical("point", x)?;
    require_canonical("value", y)?;
    Ok(Share { x, y: Fp::new(y) })
}

impl Message {
    /// The message's KIND, as a fixed string — never its contents.
    ///
    /// A `Shares` or `Open` message carries share POINTS, so Debug-formatting an
    /// unexpected message into a protocol error would put those values in the
    /// error text. Every "expected X, got Y" refusal in this module names the
    /// kind through this accessor instead; the kind is what the reader needs to
    /// diagnose a desynchronised exchange.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Message::Shares(_) => "Shares",
            Message::Open(_) => "Open",
            Message::Step(_) => "Step",
            Message::Swap { .. } => "Swap",
            Message::Done => "Done",
        }
    }

    /// The exact BODY length [`Self::encode`] will produce, in closed form from the
    /// message's element count — no serialisation.
    ///
    /// `Shares` and `Open` carry `len` points of `8 + FIELD_BYTES` bytes each after
    /// a tag byte and a `u32` count, so the size is known from the count alone.
    /// Checked: a count whose byte size leaves `usize` is a refusal here rather
    /// than a wrapped length later.
    pub(crate) fn encoded_len(&self) -> Result<usize, MpcError> {
        let points = |n: usize| -> Result<usize, MpcError> {
            n.checked_mul(8 + FIELD_BYTES)
                .and_then(|v| v.checked_add(5))
                .ok_or_else(|| {
                    MpcError::Protocol(format!(
                        "outbound frame of {} share point(s) overflows a usize body length — \
                         refusing before anything is serialised",
                        n
                    ))
                })
        };
        match self {
            Message::Shares(v) | Message::Open(v) => points(v.len()),
            Message::Step(_) => Ok(2),
            Message::Swap { .. } => Ok(1 + 4 + 4 + 1),
            Message::Done => Ok(1),
        }
    }

    /// Serialise to a length-prefixed frame: `[u32 len][u8 tag][payload]`. `len`
    /// covers the tag + payload, so a reader frames on the prefix alone.
    ///
    /// Refuses a body that does not fit the framing: the same
    /// [`MAX_FRAME_BYTES`] cap the receiver applies, and the `u32` length prefix
    /// itself. The cast used to be unchecked, so a body of `2^32 + k` bytes would
    /// have been framed as `k` and the peer would have read a prefix of the body as
    /// a whole message. Both are public byte counts, so the refusal is closed form
    /// and names only sizes. At today's cap the `u32` conversion cannot fail once
    /// the cap has passed — 64 MiB fits a `u32` — so it is the belt the cap is the
    /// braces for, and only the cap's own refusal is reachable from a caller.
    ///
    /// The cap is charged from [`Self::encoded_len`], a closed form of the message's
    /// element count, BEFORE anything is serialised, so an over-cap `Shares` or
    /// `Open` refuses without building its body. It was charged on `body.len()`
    /// after the loop, which is a refusal that has already done the work.
    pub fn encode(&self) -> Result<Vec<u8>, MpcError> {
        let declared = self.encoded_len()?;
        if declared > MAX_FRAME_BYTES {
            return Err(MpcError::Protocol(format!(
                "outbound frame body of {} byte(s) exceeds the MAX_FRAME_BYTES cap of {} \
                 — refusing to send it before it is serialised (the receiver applies the \
                 same cap, so the frame could not be read either)",
                declared, MAX_FRAME_BYTES
            )));
        }
        let mut body = Vec::with_capacity(declared);
        match self {
            Message::Shares(v) => {
                body.push(TAG_SHARES);
                body.extend_from_slice(&(v.len() as u32).to_be_bytes());
                for s in v {
                    write_share(&mut body, s);
                }
            }
            Message::Open(v) => {
                body.push(TAG_OPEN);
                body.extend_from_slice(&(v.len() as u32).to_be_bytes());
                for s in v {
                    write_share(&mut body, s);
                }
            }
            Message::Step(code) => {
                body.push(TAG_STEP);
                body.push(code.to_byte());
            }
            Message::Swap { a, b, swap } => {
                body.push(TAG_SWAP);
                body.extend_from_slice(&a.to_be_bytes());
                body.extend_from_slice(&b.to_be_bytes());
                body.push(if *swap { 1 } else { 0 });
            }
            Message::Done => body.push(TAG_DONE),
        }
        debug_assert_eq!(body.len(), declared, "encoded_len must be exact");
        let len = u32::try_from(body.len()).map_err(|_| {
            MpcError::Protocol(format!(
                "outbound frame body of {} byte(s) does not fit the u32 length prefix — \
                 refusing (a wrapped prefix would frame a prefix of the body as a whole \
                 message)",
                body.len()
            ))
        })?;
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&len.to_be_bytes());
        frame.extend_from_slice(&body);
        Ok(frame)
    }

    /// Decode one frame BODY (`[u8 tag][payload]`), exactly and only.
    ///
    /// STRICT: the body must be exactly one message's encoding. A body with
    /// TRAILING bytes past the decoded message is refused, not ignored — accepting
    /// it would make the encoding non-injective (two different frames decoding to
    /// one `Message`), so a frame digest or a replay check over the bytes would not
    /// mean what it says. Every `Share` word must also be a canonical `Fp`
    /// representative (see `require_canonical`), for the same reason.
    ///
    /// The refusals name the tag, the byte COUNTS and the modulus — all public
    /// framing facts. No share value and no decoded payload appears in any of them.
    fn decode(body: &[u8]) -> Result<Message, MpcError> {
        let (msg, consumed) = Self::decode_prefix(body)?;
        if consumed != body.len() {
            return Err(MpcError::Protocol(format!(
                "frame has {} trailing byte(s) after a complete {} message — refusing \
                 (a frame must be exactly one message's encoding, or two different \
                 frames would decode to the same one)",
                body.len() - consumed,
                msg.kind()
            )));
        }
        Ok(msg)
    }

    /// [`Self::decode`] without the trailing-byte check: the message and how many
    /// bytes it consumed. Separate so the strictness lives in ONE place.
    fn decode_prefix(body: &[u8]) -> Result<(Message, usize), MpcError> {
        if body.is_empty() {
            return Err(MpcError::Protocol("empty frame".into()));
        }
        let tag = body[0];
        let mut off = 1;
        match tag {
            TAG_SHARES | TAG_OPEN => {
                if off + 4 > body.len() {
                    return Err(MpcError::Protocol("short share-vec header".into()));
                }
                let count = u32::from_be_bytes(body[off..off + 4].try_into().unwrap()) as usize;
                off += 4;
                // The count came from the frame; the frame's remaining length did
                // not. Refuse a count those bytes cannot hold BEFORE reserving.
                let count = checked_item_count(
                    "share-vec frame",
                    count,
                    body.len() - off,
                    8 + FIELD_BYTES,
                )?;
                let mut v = Vec::with_capacity(count);
                for _ in 0..count {
                    v.push(read_share(body, &mut off)?);
                }
                let msg = if tag == TAG_SHARES {
                    Message::Shares(v)
                } else {
                    Message::Open(v)
                };
                Ok((msg, off))
            }
            TAG_STEP => {
                if off >= body.len() {
                    return Err(MpcError::Protocol("short Step frame".into()));
                }
                let code = StepCode::from_byte(body[off])?;
                Ok((Message::Step(code), off + 1))
            }
            TAG_SWAP => {
                // 4 (a) + 4 (b) + 1 (swap flag).
                if off + 9 > body.len() {
                    return Err(MpcError::Protocol("short Swap frame".into()));
                }
                let a = u32::from_be_bytes(body[off..off + 4].try_into().unwrap());
                off += 4;
                let b = u32::from_be_bytes(body[off..off + 4].try_into().unwrap());
                off += 4;
                let swap = match body[off] {
                    0 => false,
                    1 => true,
                    other => {
                        return Err(MpcError::Protocol(format!(
                            "Swap flag must be 0 or 1, got {other}"
                        )))
                    }
                };
                Ok((Message::Swap { a, b, swap }, off + 1))
            }
            TAG_DONE => Ok((Message::Done, off)),
            other => Err(MpcError::Protocol(format!("unknown frame tag {other}"))),
        }
    }
}

/// Largest single frame body [`Channel::recv`] accepts before refusing with an
/// [`MpcError::Protocol`] error.
///
/// The length prefix is the peer's number, so it is capped on its own, in closed
/// form, before anything is reserved. A single frame over this harness carries a
/// batch of 8-byte shares and never approaches this; the cap is there so a garbled
/// or hostile header is a controlled refusal rather than an allocation request.
///
/// Capping the declared length is NOT on its own enough: see
/// [`RECV_CHUNK_BYTES`] for the other half.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// How many body bytes [`Channel::recv`] reserves ahead of the bytes that have
/// actually arrived.
///
/// [`MAX_FRAME_BYTES`] bounds what a frame may declare, but a four-byte header
/// declaring the cap used to reserve the whole thing before a single body byte was
/// read. The body is now read in chunks of this size with the buffer growing as
/// bytes arrive, so the header bounds the TOTAL and this bounds the COMMITMENT.
pub const RECV_CHUNK_BYTES: usize = 64 * 1024;

/// A framed, byte-counting message channel over any `Read + Write` stream
/// (TcpStream on `127.0.0.1`, or a Unix-domain stream). Tracks bytes sent /
/// received so the harness can report the MEASURED on-wire volume.
pub struct Channel<S: Read + Write> {
    stream: S,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

impl<S: Read + Write> Channel<S> {
    /// Wrap a connected stream.
    pub fn new(stream: S) -> Self {
        Channel {
            stream,
            bytes_sent: 0,
            bytes_recv: 0,
        }
    }

    /// Send one message as a length-prefixed frame; flush so the peer sees a full
    /// round-trip (matters under netem — we want the RTT, not a coalesced batch).
    /// I/O errors surface as [`MpcError::Protocol`] (the crate's error type) so
    /// the coordinator / party loops can `?` uniformly.
    pub fn send(&mut self, msg: &Message) -> Result<(), MpcError> {
        let frame = msg.encode()?;
        self.write_all(&frame)?;
        self.flush()?;
        // Checked: the counters are reported as totals, and a wrapped total would
        // under-report traffic rather than fail.
        self.bytes_sent = add_bytes(self.bytes_sent, frame.len(), "bytes_sent")?;
        Ok(())
    }

    fn write_all(&mut self, buf: &[u8]) -> Result<(), MpcError> {
        self.stream
            .write_all(buf)
            .map_err(|e| MpcError::Protocol(format!("transport write: {e}")))
    }

    fn flush(&mut self) -> Result<(), MpcError> {
        self.stream
            .flush()
            .map_err(|e| MpcError::Protocol(format!("transport flush: {e}")))
    }

    /// Block until one full frame is read and decoded.
    ///
    /// ## The declared length never commits the allocation
    ///
    /// The length prefix comes from the PEER. It is first refused above
    /// [`MAX_FRAME_BYTES`] — a closed-form check on the declared number, before
    /// anything is reserved — and then the body is read INCREMENTALLY, in
    /// [`RECV_CHUNK_BYTES`] steps, each chunk reserved exactly before it is read,
    /// so the buffer stays within one chunk of the bytes that have arrived. So a
    /// four-byte header can commit the receiver to at most one chunk, not to the
    /// whole declared frame: a header declaring the cap with nothing behind it
    /// fails on the read, having reserved one chunk. Reserving `len` up front, as
    /// this did, let the header alone decide the allocation.
    pub fn recv(&mut self) -> Result<Message, MpcError> {
        let mut len_buf = [0u8; 4];
        self.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;
        // The declared length's cap FIRST: closed form on a public number, before
        // a single byte is reserved.
        if len > MAX_FRAME_BYTES {
            return Err(MpcError::Protocol(format!(
                "frame length {} exceeds the MAX_FRAME_BYTES cap of {} — refusing \
                 before anything is reserved",
                len, MAX_FRAME_BYTES
            )));
        }
        let body = self.read_body(len)?;
        self.bytes_recv = add_bytes(self.bytes_recv, len, "bytes_recv")
            .and_then(|v| add_bytes(v, 4, "bytes_recv"))?;
        Message::decode(&body)
    }

    /// Read `len` body bytes, growing the buffer in CHUNK STEPS as they arrive.
    ///
    /// The buffer starts at `initial_body_capacity(len)` and every further chunk is
    /// reserved with `reserve_exact` just before it is read, so the allocation
    /// stays within one chunk of the bytes that have actually arrived. Plain
    /// `extend_from_slice` would grow geometrically instead — a `Vec` doubling from
    /// 64 KiB runs tens of MiB ahead of the data on a large frame — which is the
    /// commitment the header cap alone does not bound.
    ///
    /// Returns the body, and (in tests) the widest capacity the buffer ever held,
    /// which `read_body` has no other way to show a caller.
    fn read_body(&mut self, len: usize) -> Result<Vec<u8>, MpcError> {
        self.read_body_observed(len).map(|(body, _)| body)
    }

    /// [`Self::read_body`], also returning the buffer's PEAK capacity — the seam
    /// the growth test observes, since a finished body's capacity says nothing
    /// about how far ahead of the data the buffer ran while filling.
    fn read_body_observed(&mut self, len: usize) -> Result<(Vec<u8>, usize), MpcError> {
        let mut body: Vec<u8> = Vec::with_capacity(initial_body_capacity(len));
        let mut peak = body.capacity();
        let mut chunk = [0u8; RECV_CHUNK_BYTES];
        let mut remaining = len;
        while remaining > 0 {
            let take = remaining.min(RECV_CHUNK_BYTES);
            // Reserve exactly this chunk, so the buffer never runs more than one
            // chunk ahead of the bytes that have arrived.
            let needed = body.len() + take;
            if body.capacity() < needed {
                body.reserve_exact(needed - body.len());
            }
            peak = peak.max(body.capacity());
            #[cfg(test)]
            LAST_BODY_PEAK.with(|c| c.set(peak));
            self.read_exact(&mut chunk[..take])?;
            body.extend_from_slice(&chunk[..take]);
            peak = peak.max(body.capacity());
            #[cfg(test)]
            LAST_BODY_PEAK.with(|c| c.set(peak));
            remaining -= take;
        }
        Ok((body, peak))
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> Result<(), MpcError> {
        self.stream
            .read_exact(buf)
            .map_err(|e| MpcError::Protocol(format!("transport read: {e}")))
    }
}

// =============================================================================
// The PARTY side — a thin protocol executor over one channel.
// =============================================================================

/// Run one party's side of the network protocol over an already-connected
/// channel: receive a [`StepCode`] + the share batch, do the local computation,
/// and send the share contributions back to open. This is what each party PROCESS
/// runs (the child binary's body); it holds only its own share-points.
///
/// The party never sees cleartext, never reconstructs — it only does local
/// share-arithmetic (add for the sum; multiply for the equality product) and
/// broadcasts its result share. Returns when it receives [`Message::Done`].
pub fn run_party<S: Read + Write>(chan: &mut Channel<S>) -> Result<(), MpcError> {
    loop {
        match chan.recv()? {
            Message::Step(StepCode::SumAndOpen) => {
                // Next frame: the shares this party holds (one per dealt secret).
                let shares = match chan.recv()? {
                    Message::Shares(v) => v,
                    other => {
                        return Err(MpcError::Protocol(format!(
                            "expected Shares, got {}",
                            other.kind()
                        )))
                    }
                };
                // Local, non-interactive sum of all share-points (the aggregate's
                // free linear step). All shares are at this party's single point.
                // PR #96: an empty batch is a protocol error, NOT an
                // `x=0` placeholder share — interpolating a manufactured `x=0`
                // point (or any duplicate x) is undefined for Shamir and panics in
                // `Fp::inv(0)` during reconstruction. Fail closed, mirroring the
                // in-process `ShamirBackend::run_secure` ("no shared inputs").
                let x = match shares.first() {
                    Some(s) => s.x,
                    None => {
                        return Err(MpcError::Protocol(
                            "SumAndOpen: empty share batch (no shared inputs)".into(),
                        ))
                    }
                };
                // All share-points in this batch must share this party's single
                // evaluation point `x`; a mixed batch is a framing/protocol fault.
                if let Some(bad) = shares.iter().find(|s| s.x != x) {
                    return Err(MpcError::Protocol(format!(
                        "SumAndOpen: mixed evaluation points in batch (expected x={x}, saw x={})",
                        bad.x
                    )));
                }
                let mut acc = Fp::zero();
                for s in &shares {
                    acc = acc.add(s.y);
                }
                chan.send(&Message::Open(vec![Share { x, y: acc }]))?;
            }
            Message::Step(StepCode::MulAndOpen) => {
                // Next frame: interleaved (d_share, r_share) pairs, one pair per
                // candidate equality. Local degree-2t product per pair.
                let shares = match chan.recv()? {
                    Message::Shares(v) => v,
                    other => {
                        return Err(MpcError::Protocol(format!(
                            "expected Shares, got {}",
                            other.kind()
                        )))
                    }
                };
                if shares.len() % 2 != 0 {
                    return Err(MpcError::Protocol("MulAndOpen expects (d,r) pairs".into()));
                }
                let mut products = Vec::with_capacity(shares.len() / 2);
                for pair in shares.chunks_exact(2) {
                    let (d, r) = (pair[0], pair[1]);
                    if d.x != r.x {
                        return Err(MpcError::Protocol("MulAndOpen pair point mismatch".into()));
                    }
                    products.push(Share {
                        x: d.x,
                        y: d.y.mul(r.y),
                    });
                }
                chan.send(&Message::Open(products))?;
            }
            Message::Step(StepCode::SwapNetworkAndOpen) => {
                // Next frame: this party's column — ONE share-point per row, all at
                // this party's single evaluation point `x`. The party then receives
                // a stream of `Swap` gates (one per network switch / compare-
                // exchange) terminated by `Done`, applies each locally to its
                // column, and sends the whole permuted column back to open. The
                // party never reconstructs and never learns the permutation as a
                // whole — only the per-gate public boolean. sq-bdbv.
                let mut column = match chan.recv()? {
                    Message::Shares(v) => v,
                    other => {
                        return Err(MpcError::Protocol(format!(
                            "SwapNetworkAndOpen: expected Shares, got {}",
                            other.kind()
                        )))
                    }
                };
                // An empty column is a protocol fault (mirrors the SumAndOpen
                // guard): a zero-row shuffle/sort ships nothing to open and would
                // open an empty batch. Fail closed before applying gates.
                if column.is_empty() {
                    return Err(MpcError::Protocol(
                        "SwapNetworkAndOpen: empty share column (no rows)".into(),
                    ));
                }
                loop {
                    match chan.recv()? {
                        Message::Swap { a, b, swap } => {
                            let (a, b) = (a as usize, b as usize);
                            if a >= column.len() || b >= column.len() {
                                return Err(MpcError::Protocol(format!(
                                    "Swap gate ({a},{b}) out of range for column length {}",
                                    column.len()
                                )));
                            }
                            if a == b {
                                return Err(MpcError::Protocol(format!(
                                    "Swap gate touches one wire twice (a == b == {a})"
                                )));
                            }
                            if swap {
                                column.swap(a, b);
                            }
                        }
                        Message::Done => break,
                        other => {
                            return Err(MpcError::Protocol(format!(
                                "SwapNetworkAndOpen: expected Swap or Done, got {}",
                                other.kind()
                            )))
                        }
                    }
                }
                chan.send(&Message::Open(column))?;
            }
            Message::Done => return Ok(()),
            other => {
                return Err(MpcError::Protocol(format!(
                    "party got an unexpected {} message",
                    other.kind()
                )))
            }
        }
    }
}

// =============================================================================
// The COORDINATOR side — deals shares, drives the step, reconstructs.
// =============================================================================

/// One networked cell's measured result: the protocol RESULT (so it can be
/// checked against the in-process tier) plus the MEASURED wall-clock and
/// bytes-on-wire. Distinct from `bench::MatrixCell`, whose cost is MODELLED.
#[derive(Debug, Clone)]
pub struct NetworkCell {
    /// The query class this cell ran.
    pub query_class: QueryClass,
    /// Number of party processes.
    pub parties: usize,
    /// Threshold `t`.
    pub threshold: usize,
    /// Data scale (rows/holder for the join; party count for the aggregate).
    pub scale: usize,
    /// The protocol RESULT, type-erased to an integer the caller verifies against
    /// the in-process result: the reconstructed sum for the aggregate, or the
    /// match count for the hidden join.
    pub result: u64,
    /// MEASURED wall-clock for the networked run (the cross-`N` latency the
    /// in-process tier cannot produce). NOT a modelled figure.
    pub wall_clock: Duration,
    /// Total bytes the coordinator sent across all party channels.
    pub bytes_sent: u64,
    /// Total bytes the coordinator received across all party channels.
    pub bytes_recv: u64,
    /// Number of logical communication rounds the coordinator drove (one deal +
    /// one open per protocol step — the MEASURED round count, to cross-check the
    /// modelled `round_count`).
    pub rounds: u64,
}

impl NetworkCell {
    /// Total measured on-wire bytes (sent + received), coordinator's view.
    pub fn total_bytes(&self) -> Result<u64, MpcError> {
        self.bytes_sent.checked_add(self.bytes_recv).ok_or_else(|| {
            MpcError::Protocol(
                "measured byte total overflows u64 — refusing rather than reporting a \
                 wrapped total"
                    .into(),
            )
        })
    }
}

/// Acknowledgement that a shuffle's PERMUTATION IS PUBLIC, required by
/// [`Coordinator::shuffle_public_permutation`].
///
/// The transport tier drives the switch network by sending each switch's endpoints
/// and its control bit in the clear to every party, so every party's view carries
/// the composed permutation (see that method's docs). This marker exists so no
/// caller can reach that behaviour without naming it: the plain
/// `Coordinator::shuffle` is refused, and the only way through takes a value of
/// this type, which can be built only by
/// [`PublicPermutation::acknowledged`].
///
/// Pass it from a measurement or correctness harness. Do NOT pass it from a path
/// whose output is meant to hide which row went where — that needs secret control
/// bits, which this tier does not have.
///
/// [`run_cell_networked`] does not build one on a caller's behalf either: it takes
/// `Option<PublicPermutation>` and refuses [`QueryClass::ObliviousShuffle`] when it
/// is `None`.
#[derive(Debug, Clone, Copy)]
pub struct PublicPermutation {
    /// Private, so the struct cannot be built by a literal elsewhere.
    _acknowledged: (),
}

impl PublicPermutation {
    /// Acknowledge that the permutation will be public to every party.
    ///
    /// Naming this function IS the opt-in; read [`PublicPermutation`] first.
    pub fn acknowledged() -> Self {
        PublicPermutation { _acknowledged: () }
    }
}

/// The coordinator drives one cell over `n` already-connected party channels.
/// Generic over the stream type so the same logic runs over TCP loopback or a
/// Unix socket (and, under netem, over a shaped loopback — same code).
pub struct Coordinator<S: Read + Write> {
    channels: Vec<Channel<S>>,
    backend: ShamirBackend,
}

impl<S: Read + Write> Coordinator<S> {
    /// Build a coordinator over `channels` (one per party, in party-index order)
    /// and a backend whose `(n, t)` matches the party set. `channels.len()` must
    /// equal `backend.parties()`.
    pub fn new(channels: Vec<Channel<S>>, backend: ShamirBackend) -> Result<Self, MpcError> {
        if channels.len() != backend.parties() {
            return Err(MpcError::Protocol(format!(
                "coordinator: {} channels for {} parties",
                channels.len(),
                backend.parties()
            )));
        }
        Ok(Coordinator { channels, backend })
    }

    /// Total bytes the coordinator sent / received across all channels.
    pub fn byte_totals(&self) -> Result<(u64, u64), MpcError> {
        let overflow = || {
            MpcError::Protocol(
                "measured byte total over the channels overflows u64 — refusing rather \
                 than reporting a wrapped total"
                    .into(),
            )
        };
        let mut sent = 0u64;
        let mut recv = 0u64;
        for c in &self.channels {
            sent = sent.checked_add(c.bytes_sent).ok_or_else(overflow)?;
            recv = recv.checked_add(c.bytes_recv).ok_or_else(overflow)?;
        }
        Ok((sent, recv))
    }

    /// Run the **cumulative-sum aggregate** over the parties: deal each holder's
    /// private value, instruct each party to sum-and-open, collect the opened
    /// shares, reconstruct the sum at degree `t`. Mirrors the in-process
    /// `bench::check_aggregate` exactly, but over the wire.
    pub fn aggregate(
        &mut self,
        dealer: &mut ShamirDealer,
        values: &[u64],
    ) -> Result<u64, MpcError> {
        let n = self.channels.len();
        // PR #96: fail closed on an empty input set, exactly as the
        // in-process `ShamirBackend::run_secure` does ("run_secure: no shared
        // inputs"). An empty deal would otherwise ship empty share columns to the
        // parties, which manufacture an `x=0` open-share and panic in
        // reconstruction (`Fp::inv(0)`). Reject before any frame crosses the wire.
        if values.is_empty() {
            return Err(MpcError::Protocol(
                "aggregate: no shared inputs (empty values)".into(),
            ));
        }
        // Deal: each value → a full sharing; party `p` gets share at point `p+1`.
        // We build, per party, the column of all its share-points.
        let sharings: Vec<Vec<Share>> = values.iter().map(|&v| dealer.share(Fp::new(v))).collect();
        // Step opcode first, then this party's column of shares.
        for (p, chan) in self.channels.iter_mut().enumerate() {
            chan.send(&Message::Step(StepCode::SumAndOpen))?;
            let column: Vec<Share> = sharings.iter().map(|s| s[p]).collect();
            chan.send(&Message::Shares(column))?;
        }
        // Open: collect each party's single summed share, reconstruct at degree t.
        let opened = self.collect_opens(1)?;
        let sum = shamir::reconstruct_degree(&opened[0], self.backend.threshold())?;
        // Tell parties the cell is done.
        self.broadcast_done()?;
        debug_assert_eq!(n, self.channels.len());
        Ok(sum.value())
    }

    /// Run the **hidden-value equi-join** over two `scale`-row tables (keys
    /// `0..scale` on both sides ⇒ exactly `scale` matches). For each of the
    /// `scale²` candidate pairs the coordinator deals `(d, r)` shares (d = keyL −
    /// keyR done in cleartext-then-shared here — the dealer plays the holders, as
    /// the in-process `secure_equal` does), the parties multiply-and-open, and the
    /// match bit is `product == 0`. Returns the match count. Mirrors
    /// `bench::check_hidden_join`.
    pub fn hidden_join(
        &mut self,
        dealer: &mut ShamirDealer,
        scale: usize,
    ) -> Result<u64, MpcError> {
        let t = self.backend.threshold();
        if self.channels.len() < 2 * t + 1 {
            return Err(MpcError::Protocol(format!(
                "hidden_join needs n >= 2t+1 (n={}, t={})",
                self.channels.len(),
                t
            )));
        }
        // Build all candidate (d, r) sharings. d = keyL - keyR; r = fresh nonzero
        // mask. Keys 0..scale on both sides. Pair count = scale², which is a product
        // of a caller-chosen number with itself: counted with checked arithmetic and
        // capped before either vector is reserved or a single mask is drawn.
        let pairs = crate::oblivious_join::checked_candidate_pairs(scale, scale)?;
        let mut d_sharings: Vec<Vec<Share>> = Vec::with_capacity(pairs);
        let mut r_sharings: Vec<Vec<Share>> = Vec::with_capacity(pairs);
        for lkey in 0..scale {
            for rkey in 0..scale {
                let d = Fp::new(lkey as u64).sub(Fp::new(rkey as u64));
                let mask = dealer.draw_nonzero_fp();
                d_sharings.push(dealer.share(d));
                r_sharings.push(dealer.share(mask));
            }
        }
        let pairs = d_sharings.len();
        // Deal each party its interleaved (d_share, r_share) column, then mul-open.
        for (p, chan) in self.channels.iter_mut().enumerate() {
            chan.send(&Message::Step(StepCode::MulAndOpen))?;
            let mut column = Vec::with_capacity(pairs * 2);
            for k in 0..pairs {
                column.push(d_sharings[k][p]);
                column.push(r_sharings[k][p]);
            }
            chan.send(&Message::Shares(column))?;
        }
        // Open: each party returns `pairs` product shares; reconstruct each at 2t.
        let opened = self.collect_opens(pairs)?;
        let mut matches = 0u64;
        for col in &opened {
            let m = shamir::reconstruct_degree(col, 2 * t)?;
            if m == Fp::zero() {
                matches += 1;
            }
        }
        self.broadcast_done()?;
        Ok(matches)
    }

    /// Refused. Run the switch network over the wire with
    /// [`Self::shuffle_public_permutation`] instead, which requires the caller to
    /// acknowledge that **this shuffle's permutation is PUBLIC**.
    ///
    /// ## Why it is refused rather than kept
    ///
    /// The coordinator (playing the dealer, as the in-process
    /// [`crate::oblivious::shuffle`] does) shares the row values, samples a
    /// permutation, routes it through the AS-Waksman
    /// [`WaksmanNetwork`](crate::oblivious::WaksmanNetwork), and then drives the
    /// network one switch at a time — sending each switch's endpoints AND its
    /// control bit **in the clear** to EVERY party ([`Message::Swap`]), with each
    /// party applying the swap to the column it holds. So every party sees the
    /// whole control-bit sequence over a topology that is public by construction,
    /// and keeps the intermediate column it swapped: the composed permutation is in
    /// every party's view, and the output share vectors are the input ones
    /// reordered. Opening the column at the end of the same call does not undo
    /// that, because the views along the way are what carry it.
    ///
    /// That is fine for what this tier exists to do — MEASURE wall-clock, bytes on
    /// the wire and the multiset-correctness of a multi-round network protocol —
    /// and it is not a privacy shuffle. Hiding the permutation needs SECRET control
    /// bits, i.e. the arithmetic conditional swap `x' = x + b·(y − x)` with `b`
    /// secret-shared and one multiplication per switch. This tier does not have
    /// that, and building it is not in scope here.
    #[deprecated(
        since = "0.1.7",
        note = "the permutation is public; call shuffle_public_permutation with \
                PublicPermutation::acknowledged()"
    )]
    pub fn shuffle(
        &mut self,
        _dealer: &mut ShamirDealer,
        _values: &[u64],
    ) -> Result<(Vec<u64>, u64), MpcError> {
        Err(MpcError::Protocol(
            "Coordinator::shuffle: this transport shuffle's PERMUTATION IS PUBLIC - every \
             party is sent each switch's endpoints and its control bit in the clear and \
             keeps the column it swapped, so the composed permutation is in every party's \
             view. It is a measurement harness for wall-clock and bytes-on-wire, not a \
             privacy shuffle. A caller that accepts that calls \
             `shuffle_public_permutation` and passes `PublicPermutation::acknowledged()`; \
             a private-output path needs secret control bits, which this tier does not \
             have"
                .to_string(),
        ))
    }

    /// [`Self::shuffle`] with the public-permutation property acknowledged — the
    /// only way to run it.
    ///
    /// See [`PublicPermutation`] for what the caller is accepting. Use this for
    /// measurement (wall-clock, bytes-on-wire, multiset correctness over the wire),
    /// never to hide which row went where.
    pub fn shuffle_public_permutation(
        &mut self,
        dealer: &mut ShamirDealer,
        values: &[u64],
        _ack: PublicPermutation,
    ) -> Result<(Vec<u64>, u64), MpcError> {
        if values.is_empty() {
            return Err(MpcError::Protocol("shuffle: no rows (empty column)".into()));
        }
        let n_rows = values.len();
        let net = crate::oblivious::WaksmanNetwork::try_new(n_rows)?;
        // Sample the fresh secret permutation + route it to per-switch control bits.
        // Same unbiased sampler the in-process `shuffle` uses (no second RNG path).
        let perm = crate::oblivious::sample_random_permutation(dealer, n_rows)?;
        let bits = net.route(&perm)?;
        let gates: Vec<(usize, usize)> = net.switches().iter().map(|s| (s.a, s.b)).collect();
        let opened = self.run_swap_network(dealer, values, &gates, &bits)?;
        // Reconstruct each row at degree t (a swap is local relabeling — degree t
        // preserved, so the open degree is the deal degree).
        let t = self.backend.threshold();
        let result = reconstruct_rows(&opened, t)?;
        Ok((result, gates.len() as u64))
    }

    /// Run the **oblivious sort** (disclosed-key ORDER BY path) over a `scale`-row
    /// secret-shared column (sq-bdbv). The coordinator shares the `scale` row
    /// values, builds the Batcher odd-even-mergesort
    /// [`SortingNetwork`](crate::oblivious::SortingNetwork) (a data-independent
    /// compare-exchange sequence), decides each compare-exchange against the
    /// **disclosed** sort keys (the sound shippable regime — see
    /// [`crate::oblivious::sort_with_keys`]; the secret VALUES stay shared, only the
    /// public keys drive the comparisons), and drives one compare-exchange per round
    /// over the wire. Returns the opened sorted column (ascending) + the round
    /// count. Mirrors the in-process `bench::check_sort`, but over the transport.
    ///
    /// The comparator is over PUBLIC keys (here the keys equal the values, as the
    /// in-process gate does), so no secure comparator is needed; the access PATTERN
    /// over the sharings is still the fixed network (the obliviousness substrate).
    pub fn sort(
        &mut self,
        dealer: &mut ShamirDealer,
        values: &[u64],
    ) -> Result<(Vec<u64>, u64), MpcError> {
        if values.is_empty() {
            return Err(MpcError::Protocol("sort: no rows (empty column)".into()));
        }
        let n_rows = values.len();
        let net = crate::oblivious::SortingNetwork::try_new(n_rows)?;
        // Disclosed keys = the row values (matches the in-process `sort_with_keys`
        // gate, which swaps on strict greater-than → stable ascending). The keys are
        // tracked in lockstep with the swap decisions so each gate's bit is the
        // public comparator outcome AT THAT POINT in the network.
        let mut keys: Vec<u64> = values.to_vec();
        let gates: Vec<(usize, usize)> = net.compare_exchanges().to_vec();
        let mut bits = Vec::with_capacity(gates.len());
        for &(i, j) in &gates {
            let swap = keys[i] > keys[j];
            if swap {
                keys.swap(i, j);
            }
            bits.push(swap);
        }
        let opened = self.run_swap_network(dealer, values, &gates, &bits)?;
        let t = self.backend.threshold();
        let result = reconstruct_rows(&opened, t)?;
        Ok((result, gates.len() as u64))
    }

    /// Shared driver for the oblivious shuffle / sort network tier (sq-bdbv): deal
    /// each party its share-COLUMN (one share-point per row), drive the gate stream
    /// (`Step` → `Shares` → `Swap`* → `Done`), and collect each party's opened
    /// permuted column. Returns `opened[row]` = the `Vec<Share>` for that output row
    /// (one share per party), ready to reconstruct. The `gates`/`bits` are PUBLIC
    /// (the data-independent topology + the per-gate control bit); the parties apply
    /// them as local swaps — no field multiplication, degree `t` preserved.
    fn run_swap_network(
        &mut self,
        dealer: &mut ShamirDealer,
        values: &[u64],
        gates: &[(usize, usize)],
        bits: &[bool],
    ) -> Result<Vec<Vec<Share>>, MpcError> {
        if gates.len() != bits.len() {
            return Err(MpcError::Protocol(format!(
                "run_swap_network: {} gates != {} control bits",
                gates.len(),
                bits.len()
            )));
        }
        let n_rows = values.len();
        // Share each row value → a full sharing; party p gets the point at index p.
        let sharings: Vec<Vec<Share>> = values.iter().map(|&v| dealer.share(Fp::new(v))).collect();
        // Deal: Step opcode, then this party's column of one share-point per row.
        for (p, chan) in self.channels.iter_mut().enumerate() {
            chan.send(&Message::Step(StepCode::SwapNetworkAndOpen))?;
            let column: Vec<Share> = sharings.iter().map(|s| s[p]).collect();
            chan.send(&Message::Shares(column))?;
        }
        // Drive the gate stream: one `Swap` per network switch / compare-exchange,
        // to EVERY party, then a `Done` that ends the swap phase. Multi-round: the
        // round count is the gate count (unlike the single-round aggregate/join).
        for (&(i, j), &swap) in gates.iter().zip(bits) {
            for chan in self.channels.iter_mut() {
                chan.send(&Message::Swap {
                    a: i as u32,
                    b: j as u32,
                    swap,
                })?;
            }
        }
        for chan in self.channels.iter_mut() {
            chan.send(&Message::Done)?; // end of swap phase → party opens its column.
        }
        // Open: each party returns its whole permuted column (n_rows shares).
        let opened = self.collect_opens(n_rows)?;
        // End the cell (the outer Done the party's run loop returns on).
        self.broadcast_done()?;
        Ok(opened)
    }

    /// Collect one [`Message::Open`] frame from every party and transpose it into
    /// `count` columns (one per opened value), each a `Vec<Share>` ready to
    /// reconstruct. Errors if any party returns the wrong shape.
    fn collect_opens(&mut self, count: usize) -> Result<Vec<Vec<Share>>, MpcError> {
        let n = self.channels.len();
        let mut columns: Vec<Vec<Share>> = vec![Vec::with_capacity(n); count];
        for chan in self.channels.iter_mut() {
            match chan.recv()? {
                Message::Open(shares) => {
                    if shares.len() != count {
                        return Err(MpcError::Protocol(format!(
                            "open: party returned {} shares, expected {count}",
                            shares.len()
                        )));
                    }
                    for (i, s) in shares.into_iter().enumerate() {
                        columns[i].push(s);
                    }
                }
                other => {
                    return Err(MpcError::Protocol(format!(
                        "expected Open, got {}",
                        other.kind()
                    )))
                }
            }
        }
        Ok(columns)
    }

    fn broadcast_done(&mut self) -> Result<(), MpcError> {
        for chan in self.channels.iter_mut() {
            chan.send(&Message::Done)?;
        }
        Ok(())
    }
}

/// Reconstruct each opened row (one `Vec<Share>` per output row, transposed by
/// [`Coordinator::collect_opens`]) at degree `t` to its cleartext value. Used by
/// the oblivious shuffle / sort network drivers (sq-bdbv): a swap is local
/// relabeling, so the open degree equals the deal degree `t`.
fn reconstruct_rows(opened: &[Vec<Share>], t: usize) -> Result<Vec<u64>, MpcError> {
    opened
        .iter()
        .map(|row| shamir::reconstruct_degree(row, t).map(|v| v.value()))
        .collect()
}

// =============================================================================
// In-process LOOPBACK harness over real TCP sockets + OS threads-as-processes.
// =============================================================================
//
// The fully-separate-PROCESS runner needs a child binary entry point; the crate
// ships that as an example (`examples/mpc_party.rs`) + the driver
// (`examples/mpc_net_bench.rs`). For the in-crate TEST that the network result
// matches the in-process result, we run each party on its own OS THREAD with a
// real TCP `127.0.0.1` connection: this exercises the SAME transport (real
// serialisation + real socket syscalls + real round-trips) without needing to
// fork a child binary inside a unit test. A thread-per-party over loopback TCP is
// the same wire path a process-per-party uses; the process boundary adds only
// scheduling, not protocol behaviour, so the result-equality guarantee is
// identical. The example binaries provide the genuine process-per-party runner.

/// Spawn `n` party threads, each owning one accepted TCP connection on
/// `127.0.0.1`, and run `cell` over the coordinator side. Returns the measured
/// [`NetworkCell`]. The threads are the loopback stand-in for the party
/// processes (see the module note); the example binaries run real processes.
///
/// `values` is consulted for the aggregate; `scale` for the hidden join.
///
/// [`QueryClass::ObliviousShuffle`] runs this tier's PUBLIC-permutation shuffle, so
/// it is the caller that must acknowledge it: pass
/// `Some(PublicPermutation::acknowledged())`. With `None` that class is refused and
/// nothing is driven over the wire — the token is not manufactured here, or this
/// function would be a way around [`Coordinator::shuffle_public_permutation`]'s
/// opt-in. Every other class ignores the argument.
pub fn run_cell_networked(
    query_class: QueryClass,
    backend: &ShamirBackend,
    dealer: &mut ShamirDealer,
    values: &[u64],
    scale: usize,
    public_permutation: Option<PublicPermutation>,
) -> Result<NetworkCell, MpcError> {
    if query_class == QueryClass::ObliviousShuffle && public_permutation.is_none() {
        return Err(MpcError::Protocol(
            "run_cell_networked: QueryClass::ObliviousShuffle drives the transport shuffle, whose PERMUTATION IS PUBLIC - every party is sent each switch's endpoints and its control bit in the clear. A caller that accepts that passes `Some(PublicPermutation::acknowledged())`; a private-output path needs secret control bits, which this tier does not have"
                .to_string(),
        ));
    }
    // `scale` sizes the canonical input column each class builds, so cap it HERE —
    // before the listener is bound and before `n` party threads are spawned, let
    // alone before `(0..scale)` is collected. An uncapped `scale` is not a slow run
    // but a dead process: `usize::MAX` overflows the column build and `1 << 35`
    // exhausts memory. Each class is capped against the bound its own work is
    // proportional to; the two classes that do not read `scale` are capped by
    // nothing, which is stated rather than left to inference.
    match query_class {
        // Both build a `scale`-row column and route it through a network of that
        // width, so the network cap is the one that fits.
        QueryClass::ObliviousShuffle | QueryClass::ObliviousSort => {
            crate::oblivious::check_network_width("run_cell_networked", scale)?;
        }
        // All-pairs over `scale x scale` rows: the candidate cardinality is the
        // bound, and it is checked here as well as inside `Coordinator::hidden_join`.
        QueryClass::HiddenValueEquiJoin => {
            crate::oblivious_join::checked_candidate_pairs(scale, scale)?;
        }
        // `CumulativeSumAggregate` is sized by the caller's own `values` slice and
        // ignores `scale`; `HiddenBoundedPath` has no wire driver and refuses below.
        QueryClass::CumulativeSumAggregate | QueryClass::HiddenBoundedPath => {}
    }
    let n = backend.parties();
    // Bind a loopback listener; parties connect to it. Port 0 ⇒ OS-assigned.
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|e| MpcError::Protocol(format!("bind loopback: {e}")))?;
    let addr = listener
        .local_addr()
        .map_err(|e| MpcError::Protocol(format!("local_addr: {e}")))?;

    // Spawn the party threads. Each connects, runs `run_party`, exits.
    let mut party_handles = Vec::with_capacity(n);
    for _ in 0..n {
        party_handles.push(std::thread::spawn(move || -> Result<(), MpcError> {
            let stream = TcpStream::connect(addr)
                .map_err(|e| MpcError::Protocol(format!("party connect: {e}")))?;
            // Nagle off: we want each frame on the wire immediately so the round
            // latency is real (matters under netem; harmless on raw loopback).
            stream.set_nodelay(true).ok();
            let mut chan = Channel::new(stream);
            run_party(&mut chan)
        }));
    }

    // Accept `n` connections (coordinator side), build channels in connect order.
    let mut channels = Vec::with_capacity(n);
    for _ in 0..n {
        let (stream, _peer) = listener
            .accept()
            .map_err(|e| MpcError::Protocol(format!("accept: {e}")))?;
        stream.set_nodelay(true).ok();
        channels.push(Channel::new(stream));
    }

    let mut coord = Coordinator::new(channels, backend.clone())?;

    let start = Instant::now();
    let (result, rounds) = match query_class {
        QueryClass::CumulativeSumAggregate => {
            let sum = coord.aggregate(dealer, values)?;
            // One deal round + one open round = 2 logical rounds (matches the
            // in-process aggregate: 0 mult rounds + 1 open, plus the deal).
            (sum, 2u64)
        }
        QueryClass::HiddenValueEquiJoin => {
            let matches = coord.hidden_join(dealer, scale)?;
            // One deal round + one batched mul-open round (the |L|·|R| equalities
            // are independent ⇒ one network round; the batched lower bound).
            (matches, 2u64)
        }
        QueryClass::ObliviousShuffle => {
            // Canonical input column (same shape as the in-process `check_shuffle`:
            // `i*7+1`), shared + obliviously permuted over the wire. The reported
            // `result` is the multiset-invariant COLUMN SUM: it equals the input
            // sum iff every row survived the wire exactly once (no row lost,
            // duplicated, or corrupted), the load-bearing correctness anchor for a
            // shuffle (which must not reveal — or alter — the multiset). The full
            // permuted column + permutation-of-input check lives in the in-crate
            // tests that call `Coordinator::shuffle_public_permutation` directly.
            // sq-bdbv.
            let col_values: Vec<u64> = (0..scale as u64).map(|i| i * 7 + 1).collect();
            // The caller's own token, checked for presence above: this function
            // never builds one.
            let ack = public_permutation.ok_or_else(|| {
                MpcError::Protocol(
                    "run_cell_networked: missing public-permutation acknowledgement".to_string(),
                )
            })?;
            let (opened, gates) = coord.shuffle_public_permutation(dealer, &col_values, ack)?;
            (opened.iter().copied().sum(), gates)
        }
        QueryClass::ObliviousSort => {
            // Canonical reverse-sorted input (same as the in-process `check_sort`:
            // `scale-i`) so the sort has work to do. The reported `result` is the
            // multiset-invariant COLUMN SUM (same anchor as the shuffle); the
            // ascending-order + permutation-of-input check lives in the in-crate
            // tests that call `Coordinator::sort` directly. sq-bdbv.
            let col_values: Vec<u64> = (0..scale as u64).map(|i| scale as u64 - i).collect();
            let (opened, gates) = coord.sort(dealer, &col_values)?;
            (opened.iter().copied().sum(), gates)
        }
        QueryClass::HiddenBoundedPath => {
            // The hidden bounded property path (sq-py8h) has no live-wire
            // `Coordinator` driver: its cost is wired into the ANALYTIC matrix
            // runner via `bench::model_bounded_path` /
            // `hidden_path::planner::BoundedPathPlan::comm_counter` (sq-py8h.5), not
            // this loopback transport harness. Refuse honestly here rather than
            // fabricate a wire run — the modelled cost is the counted metric, and a
            // networked driver for the operator is separate, deferred work.
            //
            return Err(MpcError::not_yet(
                "hidden bounded property-path over the live-wire transport harness",
                "sq-py8h (operator cost is modelled analytically via bench::run_matrix / \
                 BoundedPathPlan::comm_counter, sq-py8h.5; a Coordinator wire driver is \
                 separate deferred work)",
            ));
        }
    };
    let wall_clock = start.elapsed();

    let (bytes_sent, bytes_recv) = coord.byte_totals()?;

    // Join the party threads (they exit on `Done`).
    for h in party_handles {
        h.join()
            .map_err(|_| MpcError::Protocol("party thread panicked".into()))??;
    }

    Ok(NetworkCell {
        query_class,
        parties: n,
        threshold: backend.threshold(),
        scale: if query_class == QueryClass::CumulativeSumAggregate {
            values.len()
        } else {
            scale
        },
        result,
        wall_clock,
        bytes_sent,
        bytes_recv,
        rounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The receiver's PRE-ARRIVAL reservation is one chunk at most, whatever the
    /// peer declared. `a_declared_length_commits_at_most_one_chunk` pins the read
    /// sizes; this pins the reservation, which no public call reveals — a body
    /// that is fully delivered ends up `len` bytes long either way, and one that
    /// is not delivered returns an error instead of a buffer.
    #[test]
    fn the_pre_arrival_reservation_is_one_chunk_at_most() {
        // A declared length at the frame cap reserves one chunk, not the cap.
        assert_eq!(initial_body_capacity(MAX_FRAME_BYTES), RECV_CHUNK_BYTES);
        assert_eq!(
            initial_body_capacity(RECV_CHUNK_BYTES + 1),
            RECV_CHUNK_BYTES
        );
        // A body that fits in one chunk is reserved exactly, so the common small
        // frame still takes one allocation.
        assert_eq!(initial_body_capacity(0), 0);
        assert_eq!(initial_body_capacity(9), 9);
        assert_eq!(initial_body_capacity(RECV_CHUNK_BYTES), RECV_CHUNK_BYTES);
    }

    /// The body buffer stays within ONE chunk of the bytes that have arrived, at
    /// the real `recv` call site.
    ///
    /// The declared-length cap and the one-chunk pre-arrival reservation bound
    /// what the HEADER alone commits; neither bounds how far the buffer runs
    /// ahead of the data once bytes are flowing. `extend_from_slice` on its own
    /// grows geometrically, so a buffer doubling from one chunk ends several
    /// chunks past the body it is holding. `read_body_observed` reserves exactly
    /// the next chunk instead, and the peak capacity it records is the only place
    /// that difference shows: `recv` returns the message, whose length is the
    /// same either way.
    #[test]
    fn the_body_buffer_stays_within_one_chunk_of_the_bytes_that_arrived() {
        // A body of several chunks, so geometric growth has room to overshoot.
        let shares: Vec<Share> = (0..20_480u64)
            .map(|i| Share {
                x: i + 1,
                y: Fp::new(i),
            })
            .collect();
        let msg = Message::Shares(shares);
        let frame = msg.encode().expect("a multi-chunk frame encodes");
        let body_len = frame.len() - 4;
        assert!(
            body_len > 4 * RECV_CHUNK_BYTES,
            "the fixture must span several chunks, got {} byte(s)",
            body_len
        );

        LAST_BODY_PEAK.with(|c| c.set(0));
        let mut chan = Channel::new(std::io::Cursor::new(frame));
        assert_eq!(chan.recv().expect("the frame decodes"), msg);

        let peak = LAST_BODY_PEAK.with(|c| c.get());
        assert!(peak > 0, "the body read must have been attempted");
        assert!(
            peak <= body_len + RECV_CHUNK_BYTES,
            "the buffer reached {} byte(s) of capacity for a {} byte body; chunk-step \
             growth must stay within one RECV_CHUNK_BYTES ({}) of what arrived",
            peak,
            body_len,
            RECV_CHUNK_BYTES
        );
    }

    // --- Codec round-trips every message variant. --------------------------
    #[test]
    fn codec_roundtrips_all_messages() {
        let msgs = vec![
            Message::Shares(vec![
                Share {
                    x: 1,
                    y: Fp::new(42),
                },
                Share {
                    x: 2,
                    y: Fp::new(99),
                },
            ]),
            Message::Open(vec![Share {
                x: 3,
                y: Fp::new(7),
            }]),
            Message::Step(StepCode::SumAndOpen),
            Message::Step(StepCode::MulAndOpen),
            Message::Step(StepCode::SwapNetworkAndOpen),
            Message::Swap {
                a: 0,
                b: 7,
                swap: true,
            },
            Message::Swap {
                a: 3,
                b: 4,
                swap: false,
            },
            Message::Done,
        ];
        for m in &msgs {
            let frame = m.encode().expect("a small message frames");
            // Strip the 4-byte length prefix the way Channel::recv does.
            let len = u32::from_be_bytes(frame[0..4].try_into().unwrap()) as usize;
            let body = &frame[4..4 + len];
            let decoded = Message::decode(body).unwrap();
            assert_eq!(&decoded, m, "round-trip {m:?}");
        }
    }

    #[test]
    fn decode_rejects_garbage_tag() {
        assert!(Message::decode(&[0xFF]).is_err());
        assert!(Message::decode(&[]).is_err());
    }

    /// A declared item count is refused when the frame's remaining bytes cannot
    /// hold it, BEFORE anything is reserved for it.
    ///
    /// The nine-byte frame `00 00 00 05 01 ff ff ff ff` is a share-vec header
    /// declaring `0xffffffff` items with no item bytes behind it. Reading item by
    /// item would have failed too, but only after `Vec::with_capacity` had asked
    /// for about 64 GiB, which aborts the process instead of returning an error —
    /// so "it returns `Err`" is the whole point, and a test that only checked the
    /// `Err` while the reserve still happened would have passed before this fix.
    #[test]
    fn a_declared_count_beyond_the_frame_is_refused_before_reserving() {
        let per_item = 8 + FIELD_BYTES;

        // The crafted frame's body, exactly as it arrives after the length prefix.
        for tag in [TAG_SHARES, TAG_OPEN] {
            let mut body = vec![tag];
            body.extend_from_slice(&u32::MAX.to_be_bytes());
            let err = Message::decode(&body).expect_err("an impossible count is refused");
            assert!(
                matches!(&err, MpcError::Protocol(m) if m.contains("refusing before reserving")),
                "tag={}: {:?}",
                tag,
                err
            );

            // One item's worth of bytes present, many more declared.
            let mut body = vec![tag];
            body.extend_from_slice(&1000u32.to_be_bytes());
            body.extend_from_slice(&1u64.to_be_bytes());
            body.extend_from_slice(&[0u8; FIELD_BYTES]);
            assert!(
                Message::decode(&body).is_err(),
                "tag={}: a count past the payload is refused",
                tag
            );

            // Exactly affordable decodes; one more than affordable does not. This
            // is the boundary, so an off-by-one in the check shows up here.
            let mut exact = vec![tag];
            exact.extend_from_slice(&2u32.to_be_bytes());
            for x in 1..=2u64 {
                exact.extend_from_slice(&x.to_be_bytes());
                exact.extend_from_slice(&[0u8; FIELD_BYTES]);
            }
            Message::decode(&exact).expect("a count the frame can hold decodes");
            let mut over = exact.clone();
            over[1..5].copy_from_slice(&3u32.to_be_bytes());
            assert!(
                Message::decode(&over).is_err(),
                "tag={}: one item more than the frame holds is refused",
                tag
            );
        }

        // The validator itself, including the boundary and the degenerate width.
        assert_eq!(
            checked_item_count("t", 4, 4 * per_item, per_item).unwrap(),
            4
        );
        assert!(checked_item_count("t", 5, 4 * per_item, per_item).is_err());
        assert!(checked_item_count("t", 1, 0, per_item).is_err());
        assert_eq!(checked_item_count("t", 0, 0, per_item).unwrap(), 0);
        assert!(
            checked_item_count("t", 1, 8, 0).is_err(),
            "a zero item width is refused, not divided by"
        );

        // Every other tag is fixed-width and already length-checked; a truncated
        // body of each must still be an Err and never a panic.
        for body in [
            vec![TAG_STEP],
            vec![TAG_SWAP],
            vec![TAG_SWAP, 0, 0, 0, 1, 0, 0, 0],
            vec![TAG_SHARES],
            vec![TAG_OPEN, 0, 0],
            vec![0xFE],
        ] {
            assert!(
                Message::decode(&body).is_err(),
                "a truncated or unknown frame must be an Err: {:?}",
                body
            );
        }
    }

    // --- THE load-bearing test: networked result == in-process result. -----
    #[test]
    fn networked_aggregate_matches_in_process() {
        use crate::bench::check_aggregate;
        for &n in &[2usize, 3, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0xA11CE).unwrap();
            let values: Vec<u64> = (0..n as u64).map(|i| 1000 * (i + 1)).collect();
            let expected_plain: u64 = values.iter().sum();

            // In-process reference.
            let in_proc = check_aggregate(&backend, &values).unwrap();
            assert_eq!(in_proc, expected_plain, "in-process sum");

            // Networked (real TCP loopback).
            let mut dealer = backend.dealer();
            let cell = run_cell_networked(
                QueryClass::CumulativeSumAggregate,
                &backend,
                &mut dealer,
                &values,
                1,
                None,
            )
            .unwrap();
            assert_eq!(
                cell.result, in_proc,
                "n={n}: networked aggregate must equal in-process result"
            );
            assert_eq!(cell.result, expected_plain);
            // The transport actually moved bytes and drove rounds.
            assert!(cell.total_bytes().unwrap() > 0, "n={n}: bytes must cross the wire");
            assert_eq!(cell.rounds, 2);
            assert_eq!(cell.parties, n);
        }
    }

    #[test]
    fn networked_hidden_join_matches_in_process() {
        use crate::bench::check_hidden_join;
        // n must satisfy n >= 2t+1 (always true at t = ⌊(n−1)/2⌋).
        for &n in &[3usize, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0xB0B).unwrap();
            let scale = 4; // 16 candidate pairs, 4 expected matches.

            let in_proc = check_hidden_join(&backend, scale).unwrap();
            assert_eq!(in_proc, scale, "in-process join finds all matches");

            let mut dealer = backend.dealer();
            let cell = run_cell_networked(
                QueryClass::HiddenValueEquiJoin,
                &backend,
                &mut dealer,
                &[],
                scale,
                None,
            )
            .unwrap();
            assert_eq!(
                cell.result as usize, in_proc,
                "n={n}: networked hidden-join match count must equal in-process"
            );
            assert_eq!(cell.result as usize, scale);
            assert!(cell.total_bytes().unwrap() > 0);
        }
    }

    #[test]
    fn networked_join_allows_n2_degenerate_t0() {
        // PR #96: renamed — this test asserts the join is ALLOWED at
        // n=2, not rejected. n=2 ⇒ t=0 ⇒ 2t+1 = 1 ≤ 2, so the `n >= 2t+1` guard
        // passes; assert the join runs at n=2 (degenerate t=0) and still matches.
        // This pins that the guard mirrors the in-process one rather than being
        // stricter.
        let backend = ShamirBackend::new_seeded(2, 0xC0FFEE).unwrap();
        let mut dealer = backend.dealer();
        let cell = run_cell_networked(
            QueryClass::HiddenValueEquiJoin,
            &backend,
            &mut dealer,
            &[],
            3,
            None,
        )
        .unwrap();
        assert_eq!(cell.result as usize, 3);
    }

    /// The transport shuffle is refused without the explicit opt-in, and the
    /// refusal says why: this tier's permutation is public to every party.
    ///
    /// No network is needed to assert it — the refusal lands before anything is
    /// dealt — so the test builds a coordinator over a pair of in-memory streams.
    #[test]
    fn the_transport_shuffle_is_refused_without_the_public_permutation_opt_in() {
        let backend = ShamirBackend::new_seeded(3, 0x7A11).unwrap();
        let mut dealer = backend.dealer();
        let (coord, handles) = loopback_coordinator(&backend);
        let mut coord = coord;
        #[allow(deprecated)]
        let refused = coord.shuffle(&mut dealer, &[1u64, 2, 3]);
        match refused {
            Err(MpcError::Protocol(m)) => {
                assert!(
                    m.contains("PERMUTATION IS PUBLIC"),
                    "the refusal must say why: {m}"
                );
                assert!(
                    m.contains("shuffle_public_permutation"),
                    "the refusal must name the opt-in: {m}"
                );
            }
            other => panic!("must fail closed without the opt-in, got {other:?}"),
        }
        // With the acknowledgement the same call runs and permutes the multiset.
        let values = vec![11u64, 22, 33];
        let (opened, gates) = coord
            .shuffle_public_permutation(&mut dealer, &values, PublicPermutation::acknowledged())
            .unwrap();
        assert!(gates > 0);
        let mut got = opened;
        got.sort_unstable();
        assert_eq!(got, values);
        for h in handles {
            h.join().unwrap().unwrap();
        }
    }

    // sq-bdbv: the oblivious shuffle + sort now run over the SAME
    // Coordinator/Channel transport, multi-round (one `Swap` per network gate). The
    // load-bearing anchor is the networked result == in-process result, exactly as
    // for the aggregate/join.

    /// Helper: spin up `n` party threads over loopback and return a coordinator (the
    /// in-test stand-in for the process-per-party runner, same as
    /// `run_cell_networked`). Returns the coordinator + the join handles.
    fn loopback_coordinator(
        backend: &ShamirBackend,
    ) -> (
        Coordinator<TcpStream>,
        Vec<std::thread::JoinHandle<Result<(), MpcError>>>,
    ) {
        let n = backend.parties();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            handles.push(std::thread::spawn(move || -> Result<(), MpcError> {
                let stream = TcpStream::connect(addr).unwrap();
                stream.set_nodelay(true).ok();
                let mut chan = Channel::new(stream);
                run_party(&mut chan)
            }));
        }
        let mut channels = Vec::with_capacity(n);
        for _ in 0..n {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).ok();
            channels.push(Channel::new(stream));
        }
        let coord = Coordinator::new(channels, backend.clone()).unwrap();
        (coord, handles)
    }

    /// `Coordinator::hidden_join`'s OWN cap, tested directly on a Coordinator
    /// rather than through `run_cell_networked`.
    ///
    /// Through the wire cell the two caps mask each other: the cell's own
    /// preflight refuses first, so disabling either one alone leaves the other to
    /// catch it and neither mutant shows. Driving the Coordinator directly isolates
    /// this one, so it is pinned on its own terms.
    #[test]
    fn the_coordinators_hidden_join_caps_its_own_scale() {
        let backend = ShamirBackend::new_seeded(3, 0xE9).unwrap();
        let (mut coord, handles) = loopback_coordinator(&backend);
        let mut dealer = backend.dealer();

        // `scale^2` over the cap with `scale` itself under it, and the overflowing
        // case. Both must be a typed refusal, not a wrapped length or an allocation.
        // `scale` itself is legal (1024 is far below MAX_OUTPUT_BOUND); it is the
        // SQUARE that is not, which is what distinguishes this cap from a width one.
        for scale in [1usize << 10, usize::MAX] {
            let err = coord
                .hidden_join(&mut dealer, scale)
                .expect_err("an over-cap pair count must be refused");
            assert!(
                matches!(&err, MpcError::Protocol(m)
                    if m.contains("MAX_OUTPUT_BOUND") || m.contains("overflows usize")),
                "scale={}: {:?}",
                scale,
                err
            );
        }
        // A legal scale still runs the real wire join over the same Coordinator, so
        // the cap refuses a size and not the operation.
        let matches = coord
            .hidden_join(&mut dealer, 4)
            .expect("a legal scale still runs");
        assert_eq!(matches, 4, "keys 0..4 on both sides match pairwise");

        for h in handles {
            h.join().unwrap().unwrap();
        }
    }

    #[test]
    fn networked_shuffle_preserves_multiset_over_the_wire() {
        // A networked oblivious shuffle must open to a PERMUTATION of the input
        // multiset — never lose, duplicate, or corrupt a row across the per-switch
        // exchange. (The permutation itself is secret; we assert only the multiset
        // invariant + that the network actually drove > 0 switch rounds.)
        for &n in &[2usize, 3, 5, 7] {
            let backend =
                ShamirBackend::new_seeded(n, 0x5_FF_1E_u64.wrapping_add(n as u64)).unwrap();
            let values: Vec<u64> = (0..8u64).map(|i| i * 7 + 1).collect();
            let (mut coord, handles) = loopback_coordinator(&backend);
            let mut dealer = backend.dealer();
            let (opened, gates) = coord
                .shuffle_public_permutation(&mut dealer, &values, PublicPermutation::acknowledged())
                .unwrap();
            for h in handles {
                h.join().unwrap().unwrap();
            }
            let mut got = opened.clone();
            let mut want = values.clone();
            got.sort_unstable();
            want.sort_unstable();
            assert_eq!(got, want, "n={n}: shuffle must preserve the multiset");
            assert!(gates > 0, "n={n}: shuffle must drive > 0 switch rounds");
        }
    }

    #[test]
    fn networked_sort_orders_ascending_and_preserves_multiset() {
        // A networked oblivious (disclosed-key) sort must open ASCENDING and be a
        // permutation of the input multiset.
        for &n in &[2usize, 3, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0xB47C4E).unwrap();
            // Reverse-sorted input so the network has real work.
            let values: Vec<u64> = (0..9u64).map(|i| 9 - i).collect();
            let (mut coord, handles) = loopback_coordinator(&backend);
            let mut dealer = backend.dealer();
            let (opened, gates) = coord.sort(&mut dealer, &values).unwrap();
            for h in handles {
                h.join().unwrap().unwrap();
            }
            assert!(
                opened.windows(2).all(|w| w[0] <= w[1]),
                "n={n}: sort must open ascending, got {opened:?}"
            );
            let mut got = opened.clone();
            let mut want = values.clone();
            got.sort_unstable();
            want.sort_unstable();
            assert_eq!(got, want, "n={n}: sort must preserve the multiset");
            assert!(
                gates > 0,
                "n={n}: sort must drive > 0 compare-exchange rounds"
            );
        }
    }

    #[test]
    fn networked_shuffle_matches_in_process_multiset() {
        // THE cross-tier anchor for the shuffle: the networked multiset (column sum
        // is multiset-invariant) equals the in-process shuffle's multiset on the
        // SAME canonical input the runner uses (`i*7+1`).
        use crate::bench::check_shuffle;
        for &n in &[2usize, 3, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0x511FF1E).unwrap();
            let mut dealer = backend.dealer();
            // In-process correctness gate must pass (multiset preserved).
            assert!(check_shuffle(&mut dealer, backend.threshold(), 8).unwrap());
            // Networked: the column-sum result equals the canonical input sum.
            let mut dealer2 = backend.dealer();
            let cell = run_cell_networked(
                QueryClass::ObliviousShuffle,
                &backend,
                &mut dealer2,
                &[],
                8,
                Some(PublicPermutation::acknowledged()),
            )
            .unwrap();
            let want: u64 = (0..8u64).map(|i| i * 7 + 1).sum();
            assert_eq!(cell.result, want, "n={n}: networked shuffle multiset sum");
            assert!(cell.total_bytes().unwrap() > 0);
            assert!(cell.rounds > 1, "shuffle is multi-round (per-switch)");
        }
    }

    #[test]
    fn networked_sort_matches_in_process() {
        use crate::bench::check_sort;
        for &n in &[2usize, 3, 5, 7] {
            let backend = ShamirBackend::new_seeded(n, 0x504C).unwrap();
            let mut dealer = backend.dealer();
            assert!(check_sort(&mut dealer, backend.threshold(), 9).unwrap());
            let mut dealer2 = backend.dealer();
            let cell = run_cell_networked(
                QueryClass::ObliviousSort,
                &backend,
                &mut dealer2,
                &[],
                9,
                None,
            )
            .unwrap();
            let want: u64 = (1..=9u64).sum();
            assert_eq!(cell.result, want, "n={n}: networked sort multiset sum");
            assert!(cell.total_bytes().unwrap() > 0);
            assert!(
                cell.rounds > 1,
                "sort is multi-round (per-compare-exchange)"
            );
        }
    }

    /// `run_cell_networked` is public and drives the public-permutation shuffle, so
    /// it must not manufacture the acknowledgement for its caller. Without the token
    /// the shuffle class is refused, and the refusal lands before any wire work.
    /// `scale` sizes the canonical column each class builds, so an uncapped one is
    /// not a slow cell but a dead process: `usize::MAX` overflows the `(0..scale)`
    /// build and a merely large value exhausts memory. The cap sits at the top of
    /// the function, ahead of the listener and the party threads, so the refusal
    /// costs no socket and no thread — which is also why this test can afford to
    /// try `usize::MAX` at all.
    #[test]
    fn run_cell_networked_refuses_an_over_cap_scale_without_panicking() {
        use crate::oblivious::MAX_NETWORK_WIDTH;
        use crate::oblivious_join::MAX_OUTPUT_BOUND;

        let backend = ShamirBackend::new_seeded(3, 0xE8).unwrap();

        for scale in [MAX_NETWORK_WIDTH + 1, usize::MAX] {
            // Both network classes cap on the width. The shuffle class is passed its
            // opt-in token, so the refusal under test is the size one and not the
            // acknowledgement one.
            for (class, token) in [
                (
                    QueryClass::ObliviousShuffle,
                    Some(PublicPermutation::acknowledged()),
                ),
                (QueryClass::ObliviousSort, None),
            ] {
                let mut dealer = backend.dealer();
                let err = run_cell_networked(class, &backend, &mut dealer, &[], scale, token)
                    .expect_err("an over-cap scale must be refused");
                assert!(
                    matches!(&err, MpcError::Protocol(m) if m.contains("MAX_NETWORK_WIDTH")),
                    "{:?} scale={}: {:?}",
                    class,
                    scale,
                    err
                );
            }

            // The all-pairs class caps on the candidate cardinality instead.
            let mut dealer = backend.dealer();
            let err = run_cell_networked(
                QueryClass::HiddenValueEquiJoin,
                &backend,
                &mut dealer,
                &[],
                scale,
                None,
            )
            .expect_err("an over-cap scale must be refused");
            // `usize::MAX` squared refuses on the overflow; a merely over-cap
            // scale refuses on the cap. Both are the same guard, reached by its
            // two arms, and both must be a typed refusal rather than a wrap.
            assert!(
                matches!(&err, MpcError::Protocol(m)
                    if m.contains("MAX_OUTPUT_BOUND") || m.contains("overflows usize")),
                "HiddenValueEquiJoin scale={}: {:?}",
                scale,
                err
            );
        }

        // `scale` below the cap but whose SQUARE is above it: the join class must
        // still refuse, which is what distinguishes its cap from the width one.
        let square_over = 1usize << 10;
        assert!(
            square_over <= MAX_NETWORK_WIDTH && square_over * square_over > MAX_OUTPUT_BOUND,
            "the fixture must be legal as a width but illegal as a pair count"
        );
        let mut dealer = backend.dealer();
        assert!(
            run_cell_networked(
                QueryClass::HiddenValueEquiJoin,
                &backend,
                &mut dealer,
                &[],
                square_over,
                None,
            )
            .is_err(),
            "scale^2 above the cap must be refused even though scale is not"
        );

        // And a legal scale still runs the real wire cell, so the caps refuse a
        // size rather than the class.
        let mut dealer = backend.dealer();
        run_cell_networked(
            QueryClass::ObliviousSort,
            &backend,
            &mut dealer,
            &[],
            8,
            None,
        )
        .expect("a legal scale still runs");
    }

    #[test]
    fn run_cell_networked_refuses_the_shuffle_class_without_the_opt_in() {
        let backend = ShamirBackend::new_seeded(3, 0xE7).unwrap();
        let mut dealer = backend.dealer();
        let err = run_cell_networked(
            QueryClass::ObliviousShuffle,
            &backend,
            &mut dealer,
            &[],
            8,
            None,
        )
        .unwrap_err();
        match err {
            MpcError::Protocol(msg) => assert!(
                msg.contains("PERMUTATION IS PUBLIC")
                    && msg.contains("PublicPermutation::acknowledged"),
                "got: {}",
                msg
            ),
            other => panic!("expected a Protocol refusal, got {:?}", other),
        }
        // And the same cell goes through once the caller names it.
        let mut dealer = backend.dealer();
        let cell = run_cell_networked(
            QueryClass::ObliviousShuffle,
            &backend,
            &mut dealer,
            &[],
            8,
            Some(PublicPermutation::acknowledged()),
        )
        .unwrap();
        assert_eq!(cell.result, (0..8u64).map(|i| i * 7 + 1).sum::<u64>());
    }

    #[test]
    fn networked_shuffle_rejects_empty_column() {
        // Empty input must FAIL CLOSED (mirrors the aggregate empty-input guard).
        let backend = ShamirBackend::new_seeded(3, 0xE3).unwrap();
        let mut dealer = backend.dealer();
        let err = run_cell_networked(
            QueryClass::ObliviousShuffle,
            &backend,
            &mut dealer,
            &[],
            0,
            Some(PublicPermutation::acknowledged()),
        )
        .unwrap_err();
        match err {
            MpcError::Protocol(msg) => assert!(msg.contains("no rows"), "got: {msg}"),
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    // PR #96: an empty aggregate input must FAIL CLOSED on the network
    // tier exactly as the in-process `ShamirBackend::run_secure` does, never
    // manufacture an `x=0` open-share that would panic in reconstruction.
    #[test]
    fn networked_aggregate_rejects_empty_inputs() {
        let backend = ShamirBackend::new_seeded(3, 0xDEAD).unwrap();
        let mut dealer = backend.dealer();
        let err = run_cell_networked(
            QueryClass::CumulativeSumAggregate,
            &backend,
            &mut dealer,
            &[],
            1,
            None,
        )
        .unwrap_err();
        match err {
            MpcError::Protocol(msg) => assert!(
                msg.contains("no shared inputs"),
                "expected a 'no shared inputs' protocol error, got: {msg}"
            ),
            other => panic!("expected Protocol error, got {other:?}"),
        }
    }

    /// A desynchronised exchange is refused by NAMING THE MESSAGE KIND, never by
    /// rendering the message.
    ///
    /// `Shares` and `Open` carry share points, so Debug-formatting an unexpected
    /// message into the refusal would put those points in the error text. Each
    /// case below sends a wrong-but-well-formed message whose share value is
    /// distinctive, and asserts the value appears in neither `Display` nor
    /// `Debug` of the resulting error while the kind does.
    #[test]
    fn a_wrong_message_is_refused_by_kind_without_rendering_its_shares() {
        let marked = |y: u64| {
            vec![Share {
                x: 1,
                y: Fp::new(y),
            }]
        };
        let shown = |e: &MpcError| format!("{} || {:?}", e, e);

        // (a) Party side, mid-step: a `Step` is acknowledged, then the party is
        // handed `Open` where it expects `Shares`.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let party = std::thread::spawn(move || -> Result<(), MpcError> {
            let stream = TcpStream::connect(addr).unwrap();
            stream.set_nodelay(true).ok();
            run_party(&mut Channel::new(stream))
        });
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).ok();
        let mut chan = Channel::new(stream);
        chan.send(&Message::Step(StepCode::SumAndOpen)).unwrap();
        chan.send(&Message::Open(marked(121_212))).unwrap();
        let err = party.join().unwrap().unwrap_err();
        let text = shown(&err);
        assert!(
            !text.contains("121212"),
            "the unexpected message's share value must not be rendered: {}",
            text
        );
        assert!(
            text.contains("expected Shares, got Open"),
            "the kind is named instead: {}",
            text
        );

        // (b) Party side, at the top of the loop: a `Shares` frame where only a
        // `Step` or `Done` is legal.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let party = std::thread::spawn(move || -> Result<(), MpcError> {
            let stream = TcpStream::connect(addr).unwrap();
            stream.set_nodelay(true).ok();
            run_party(&mut Channel::new(stream))
        });
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).ok();
        let mut chan = Channel::new(stream);
        chan.send(&Message::Shares(marked(343_434))).unwrap();
        let err = party.join().unwrap().unwrap_err();
        let text = shown(&err);
        assert!(!text.contains("343434"), "{}", text);
        assert!(
            text.contains("unexpected Shares message"),
            "the kind is named instead: {}",
            text
        );

        // (b2) The same refusal on the other two step opcodes, so each site is
        // pinned on its own: `MulAndOpen` expects `Shares`, and the swap network
        // expects first `Shares` and then `Swap`/`Done`.
        for (step, then, expected) in [
            (
                StepCode::MulAndOpen,
                Message::Open(marked(454_545)),
                "expected Shares, got Open",
            ),
            (
                StepCode::SwapNetworkAndOpen,
                Message::Open(marked(676_767)),
                "SwapNetworkAndOpen: expected Shares, got Open",
            ),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let party = std::thread::spawn(move || -> Result<(), MpcError> {
                let stream = TcpStream::connect(addr).unwrap();
                stream.set_nodelay(true).ok();
                run_party(&mut Channel::new(stream))
            });
            let (stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).ok();
            let mut chan = Channel::new(stream);
            chan.send(&Message::Step(step)).unwrap();
            let marker = match &then {
                Message::Open(v) | Message::Shares(v) => v[0].y.value().to_string(),
                _ => unreachable!("the wrong-kind messages above all carry shares"),
            };
            chan.send(&then).unwrap();
            let err = party.join().unwrap().unwrap_err();
            let text = shown(&err);
            assert!(!text.contains(&marker), "{}", text);
            assert!(text.contains(expected), "{}", text);
        }

        // (b3) Mid-swap-phase: a well-formed column, then a message that is
        // neither `Swap` nor `Done`.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let party = std::thread::spawn(move || -> Result<(), MpcError> {
            let stream = TcpStream::connect(addr).unwrap();
            stream.set_nodelay(true).ok();
            run_party(&mut Channel::new(stream))
        });
        let (stream, _) = listener.accept().unwrap();
        stream.set_nodelay(true).ok();
        let mut chan = Channel::new(stream);
        chan.send(&Message::Step(StepCode::SwapNetworkAndOpen))
            .unwrap();
        chan.send(&Message::Shares(marked(1))).unwrap();
        chan.send(&Message::Open(marked(898_989))).unwrap();
        let err = party.join().unwrap().unwrap_err();
        let text = shown(&err);
        assert!(!text.contains("898989"), "{}", text);
        assert!(
            text.contains("SwapNetworkAndOpen: expected Swap or Done, got Open"),
            "{}",
            text
        );

        // (c) Coordinator side: a party answers the open round with `Shares`.
        let backend = ShamirBackend::new_seeded(3, 0x0FEE).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut fakes = Vec::new();
        for _ in 0..backend.parties() {
            fakes.push(std::thread::spawn(move || {
                let stream = TcpStream::connect(addr).unwrap();
                stream.set_nodelay(true).ok();
                let mut chan = Channel::new(stream);
                // The wrong KIND, with a distinctive share value.
                chan.send(&Message::Shares(vec![Share {
                    x: 1,
                    y: Fp::new(565_656),
                }]))
                .unwrap();
            }));
        }
        let mut channels = Vec::new();
        for _ in 0..backend.parties() {
            let (stream, _) = listener.accept().unwrap();
            stream.set_nodelay(true).ok();
            channels.push(Channel::new(stream));
        }
        let mut coord = Coordinator::new(channels, backend).unwrap();
        let err = coord.collect_opens(1).unwrap_err();
        let text = shown(&err);
        assert!(
            !text.contains("565656"),
            "the share value must not be rendered: {}",
            text
        );
        assert!(text.contains("expected Open, got Shares"), "{}", text);
        for f in fakes {
            f.join().unwrap();
        }
    }
}
