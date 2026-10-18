// sq-bif.15 — isolated codec glue tests for the MPC network-tier
// transport (`crate::transport`). TEST-COVERAGE bead: exercises the EXISTING
// hand-written wire codec (no production logic changed). The tests are
// transport-AGNOSTIC — they drive the real `Channel` (encode -> length-prefix
// frame -> decode) over an in-memory byte pipe, so NO sockets / threads / RNG
// are needed and the file compiles + runs in BOTH feature states (default and
// `insecure-test-rng`). 🤖 SPARQ agent.
//
// HONESTY: sparq-mpc is honest-majority, SEMI-HONEST only; nothing here claims
// soundness, malicious-security, or external audit. This is ordinary
// serialization round-trip + malformed-frame fail-closed coverage.

use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::rc::Rc;

use sparq_mpc::transport::{Channel, Message, StepCode, MAX_FRAME_BYTES, RECV_CHUNK_BYTES};
use sparq_mpc::{Fp, Share};

/// FIELD_BYTES — the on-wire size of one `Fp` value (the codec writes the
/// canonical `u64` representative big-endian). Mirrors the crate's re-exported
/// `metrics::FIELD_BYTES`; pinned here so a malformed-frame test that splices
/// raw bytes uses the same per-`Fp` width the decoder expects.
const FIELD_BYTES: usize = sparq_mpc::FIELD_BYTES;

/// An in-memory loopback byte pipe: `write` appends to the back of a queue,
/// `read` pops from the front. Single-threaded; a `send` followed by `recv`
/// drives the REAL codec path (`Message::encode` -> length-prefixed frame ->
/// `Channel::recv` -> internal decode) with no socket. `read` returns
/// `UnexpectedEof` when drained, exactly as a closed stream would, so the
/// truncated-frame cases surface as the codec's transport-read error.
#[derive(Default)]
struct MemPipe {
    buf: VecDeque<u8>,
}

impl MemPipe {
    /// Splice raw bytes straight into the read side, bypassing `Channel::send`,
    /// so a test can hand the decoder a deliberately MALFORMED frame.
    fn push_raw(&mut self, bytes: &[u8]) {
        self.buf.extend(bytes.iter().copied());
    }
}

impl Write for MemPipe {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend(b.iter().copied());
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Read for MemPipe {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        while n < out.len() {
            match self.buf.pop_front() {
                Some(b) => {
                    out[n] = b;
                    n += 1;
                }
                None => break,
            }
        }
        if n == 0 && !out.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "mem pipe drained",
            ));
        }
        Ok(n)
    }
}

/// Build a length-prefixed frame from a raw body: `[u32 BE body_len][body]` —
/// the exact framing `Channel::recv` expects. Lets a test feed an arbitrary
/// (possibly malformed) body to the decoder.
fn frame(body: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(4 + body.len());
    f.extend_from_slice(&(body.len() as u32).to_be_bytes());
    f.extend_from_slice(body);
    f
}

// =============================================================================
// Round-trip: every Message variant survives encode -> frame -> Channel::recv.
// =============================================================================

#[test]
fn every_message_variant_round_trips_through_a_channel() {
    // One of each variant, including a multi-share vector with a wide `x`, both
    // StepCodes, both Swap booleans, and the empty-payload Done — so a
    // regression in ANY arm's serialization fails this test.
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
            Share {
                x: 9_000_000_000, // a wide x exercising the full 8-byte field
                y: Fp::new(7),
            },
        ]),
        Message::Open(vec![Share {
            x: 3,
            y: Fp::new(0),
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

    // Send all, then receive all in order: FIFO over the pipe means the decoded
    // stream must equal the encoded stream element-for-element.
    let mut chan = Channel::new(MemPipe::default());
    for m in &msgs {
        chan.send(m).unwrap();
    }
    for expected in &msgs {
        let got = chan.recv().expect("frame must decode");
        assert_eq!(&got, expected, "round-trip changed {expected:?}");
    }
    // The codec actually moved bytes in both directions (a no-op stream would
    // make the round-trip vacuously pass).
    assert!(chan.bytes_sent > 0, "encode wrote no bytes");
    assert!(chan.bytes_recv > 0, "recv read no bytes");
    assert_eq!(
        chan.bytes_sent, chan.bytes_recv,
        "every sent frame was read back exactly once"
    );
}

#[test]
fn empty_share_vectors_round_trip() {
    // A zero-length Shares/Open vector is a legal frame (count prefix = 0); the
    // codec must round-trip it as the empty vector, not error.
    let mut chan = Channel::new(MemPipe::default());
    chan.send(&Message::Shares(vec![])).unwrap();
    chan.send(&Message::Open(vec![])).unwrap();
    assert_eq!(chan.recv().unwrap(), Message::Shares(vec![]));
    assert_eq!(chan.recv().unwrap(), Message::Open(vec![]));
}

#[test]
fn large_share_batch_round_trips() {
    // A wide batch (many shares) checks the count-prefix + per-share loop at
    // scale, well within the 64 MiB frame cap. The `y` values are distinct so
    // an off-by-one in the per-share offset arithmetic would corrupt the
    // decode and fail the equality.
    let shares: Vec<Share> = (1..=1024u64)
        .map(|x| Share {
            x,
            y: Fp::new(x.wrapping_mul(1_000_003)),
        })
        .collect();
    let msg = Message::Shares(shares);
    let mut chan = Channel::new(MemPipe::default());
    chan.send(&msg).unwrap();
    assert_eq!(chan.recv().unwrap(), msg);
}

// =============================================================================
// Malformed / adversarial frames must FAIL CLOSED (an error, never a panic,
// never a silent wrong decode). Each crafts a raw body and feeds it via recv.
// =============================================================================

#[test]
fn unknown_frame_tag_is_rejected() {
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&[0xFF])); // 0xFF is not a known tag (1..=5)
    let mut chan = Channel::new(pipe);
    assert!(chan.recv().is_err(), "an unknown tag must be rejected");
}

#[test]
fn empty_body_frame_is_rejected() {
    // A well-formed length prefix of 0 -> an empty body. The decoder must
    // reject "empty frame" rather than index a missing tag byte.
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&[]));
    let mut chan = Channel::new(pipe);
    assert!(chan.recv().is_err(), "an empty body must be rejected");
}

#[test]
fn oversized_frame_length_is_rejected_without_allocating() {
    // A garbled 4-byte length header claiming ~4 GiB must be refused on the
    // length cap alone — the decoder never tries to allocate the body. We
    // supply ONLY the 4-byte prefix; if the cap were not enforced, recv would
    // attempt to read gigabytes (here it errors on the cap first).
    let huge: u32 = u32::MAX; // ~4 GiB, far over the 64 MiB cap
    let mut pipe = MemPipe::default();
    pipe.push_raw(&huge.to_be_bytes());
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "an absurd frame length must be capped, not allocated"
    );
}

#[test]
fn truncated_length_prefix_is_rejected() {
    // Fewer than 4 bytes available for the length prefix -> the transport read
    // hits EOF and surfaces an error (no partial-length silent accept).
    let mut pipe = MemPipe::default();
    pipe.push_raw(&[0x00, 0x00]); // only 2 of the 4 length bytes
    let mut chan = Channel::new(pipe);
    assert!(chan.recv().is_err(), "a truncated length prefix must error");
}

#[test]
fn truncated_body_is_rejected() {
    // A length prefix promising 32 bytes but only 3 supplied -> the body read
    // hits EOF. The codec must error, not decode a short/garbage body.
    let mut pipe = MemPipe::default();
    pipe.push_raw(&32u32.to_be_bytes());
    pipe.push_raw(&[1, 2, 3]);
    let mut chan = Channel::new(pipe);
    assert!(chan.recv().is_err(), "a truncated body must error");
}

#[test]
fn short_share_vector_header_is_rejected() {
    // TAG_SHARES (1) with fewer than the 4 count-header bytes present.
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&[1, 0x00, 0x00])); // tag + only 2 of 4 count bytes
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "a short share-vector count header must be rejected"
    );
}

#[test]
fn share_count_exceeding_payload_is_rejected() {
    // TAG_SHARES claims a count of 3 but supplies only one share worth of
    // bytes -> the per-share read runs off the end of the body. Fail closed
    // ("short Share frame") rather than read garbage / wrong shares.
    let mut body = vec![1u8]; // TAG_SHARES
    body.extend_from_slice(&3u32.to_be_bytes()); // count = 3
                                                 // ...but only ONE share's worth of bytes (8 for x + FIELD_BYTES for y).
    body.extend_from_slice(&1u64.to_be_bytes());
    body.extend_from_slice(&[0u8; FIELD_BYTES]);
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&body));
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "a share count larger than the payload must be rejected"
    );
}

/// The exact crafted frame from the review, plus a variant per tag, driven through
/// the REAL `Channel::recv` path.
///
/// `00 00 00 05 01 ff ff ff ff` is a five-byte body: a share-vec tag and a declared
/// count of `0xffffffff`, with no item bytes behind it. At `8 + FIELD_BYTES` bytes
/// an item that count asks for roughly 64 GiB. Reading item by item would have
/// failed as well, but only after the reserve — and an aborted process is not an
/// `Err`. So what this test is really asserting is that the process is still here
/// to return one.
#[test]
fn a_declared_count_the_frame_cannot_hold_is_rejected() {
    // The frame verbatim, bytes and all.
    let crafted = [0x00, 0x00, 0x00, 0x05, 0x01, 0xff, 0xff, 0xff, 0xff];
    let mut pipe = MemPipe::default();
    pipe.push_raw(&crafted);
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "a count the frame cannot hold must be rejected"
    );

    // Both length-prefixed tags, with a huge count and with a count just past the
    // payload, and with a legal count for contrast.
    for tag in [1u8, 2] {
        for count in [u32::MAX, u32::MAX / 2, 1000, 2] {
            let mut body = vec![tag];
            body.extend_from_slice(&count.to_be_bytes());
            // One item's worth of bytes, whatever was declared.
            body.extend_from_slice(&7u64.to_be_bytes());
            body.extend_from_slice(&[0u8; FIELD_BYTES]);
            let mut pipe = MemPipe::default();
            pipe.push_raw(&frame(&body));
            let mut chan = Channel::new(pipe);
            let got = chan.recv();
            if count == 1 {
                assert!(got.is_ok(), "tag={tag}: a legal count must decode");
            } else {
                assert!(
                    got.is_err(),
                    "tag={tag}, count={count}: must be rejected, not reserved"
                );
            }
        }
    }

    // A legal one-item frame of each tag still round-trips, so the check above is
    // refusing impossible counts and not every count.
    for tag in [1u8, 2] {
        let mut body = vec![tag];
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&7u64.to_be_bytes());
        body.extend_from_slice(&[0u8; FIELD_BYTES]);
        let mut pipe = MemPipe::default();
        pipe.push_raw(&frame(&body));
        let mut chan = Channel::new(pipe);
        chan.recv().expect("a one-item frame decodes");
    }

    // Truncated bodies for every tag: each an Err, none a panic.
    for body in [
        vec![1u8],
        vec![2u8, 0, 0],
        vec![3u8],
        vec![5u8, 0, 0, 0, 1],
        vec![0xFEu8],
        vec![],
    ] {
        let mut pipe = MemPipe::default();
        pipe.push_raw(&frame(&body));
        let mut chan = Channel::new(pipe);
        assert!(
            chan.recv().is_err(),
            "a truncated or unknown frame must be an Err: {body:?}"
        );
    }
}

#[test]
fn step_frame_missing_opcode_is_rejected() {
    // TAG_STEP (3) with no following opcode byte.
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&[3]));
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "a Step frame without its opcode must be rejected"
    );
}

#[test]
fn step_frame_unknown_opcode_is_rejected() {
    // TAG_STEP with an opcode that is not 1/2/3.
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&[3, 0xAB]));
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "an unknown StepCode opcode must be rejected"
    );
}

#[test]
fn short_swap_frame_is_rejected() {
    // TAG_SWAP (5) needs 9 trailing bytes (u32 a + u32 b + u8 flag); supply 4.
    let mut body = vec![5u8];
    body.extend_from_slice(&7u32.to_be_bytes()); // only `a`, missing `b` + flag
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&body));
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "a Swap frame missing operand bytes must be rejected"
    );
}

#[test]
fn swap_flag_out_of_range_is_rejected() {
    // TAG_SWAP with a control flag byte that is neither 0 nor 1 — the codec
    // pins the boolean to {0,1} and refuses anything else (a garbled bit must
    // not silently become `true`).
    let mut body = vec![5u8];
    body.extend_from_slice(&0u32.to_be_bytes()); // a = 0
    body.extend_from_slice(&1u32.to_be_bytes()); // b = 1
    body.push(2u8); // flag = 2 (illegal)
    let mut pipe = MemPipe::default();
    pipe.push_raw(&frame(&body));
    let mut chan = Channel::new(pipe);
    assert!(
        chan.recv().is_err(),
        "a Swap flag outside {{0,1}} must be rejected"
    );
}

#[test]
fn valid_swap_flags_decode_to_the_right_boolean() {
    // The positive companion to the out-of-range test: flag 0 -> false, 1 ->
    // true, decoded through the real recv path (not just an encode round-trip).
    for (flag, want) in [(0u8, false), (1u8, true)] {
        let mut body = vec![5u8];
        body.extend_from_slice(&2u32.to_be_bytes()); // a = 2
        body.extend_from_slice(&5u32.to_be_bytes()); // b = 5
        body.push(flag);
        let mut pipe = MemPipe::default();
        pipe.push_raw(&frame(&body));
        let mut chan = Channel::new(pipe);
        assert_eq!(
            chan.recv().unwrap(),
            Message::Swap {
                a: 2,
                b: 5,
                swap: want
            },
            "Swap flag {flag} must decode to swap={want}"
        );
    }
}

// =============================================================================
// The declared length must not commit the allocation
// =============================================================================

/// A reader that serves a 4-byte length header and then answers every further
/// `read` with EOF, RECORDING the largest buffer it was asked to fill.
///
/// That maximum is the receiver's per-read commitment. With an up-front
/// `vec![0u8; len]` the decoder hands `read_exact` ONE buffer of the whole
/// declared length, so the four-byte header alone decides the allocation.
struct HeaderThenEof {
    header: [u8; 4],
    pos: usize,
    widest: Rc<Cell<usize>>,
}

impl Read for HeaderThenEof {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.header.len() {
            let n = out.len().min(self.header.len() - self.pos);
            out[..n].copy_from_slice(&self.header[self.pos..self.pos + n]);
            self.pos += n;
            return Ok(n);
        }
        self.widest.set(self.widest.get().max(out.len()));
        Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "no body",
        ))
    }
}

impl Write for HeaderThenEof {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A header declaring a body right up to [`MAX_FRAME_BYTES`], with NO body behind
/// it, must not commit the receiver to the declared length: the body is read
/// incrementally, so the widest single read is one [`RECV_CHUNK_BYTES`] chunk and
/// never the 64 MiB the header asked for.
#[test]
fn a_declared_length_commits_at_most_one_chunk() {
    let widest = Rc::new(Cell::new(0usize));
    let reader = HeaderThenEof {
        header: (MAX_FRAME_BYTES as u32).to_be_bytes(),
        pos: 0,
        widest: Rc::clone(&widest),
    };
    let mut chan = Channel::new(reader);
    assert!(chan.recv().is_err(), "a header with no body must error");

    let w = widest.get();
    assert!(w > 0, "the body read must have been attempted");
    assert!(
        w <= RECV_CHUNK_BYTES,
        "the receiver asked for {w} byte(s) in one read; a declared length must \
         commit at most RECV_CHUNK_BYTES ({RECV_CHUNK_BYTES})"
    );
    assert!(
        w < MAX_FRAME_BYTES,
        "the declared length must not become the allocation"
    );

    // And a body that IS fully supplied still round-trips across several chunks,
    // so the incremental read is a correct read and not just a smaller ceiling.
    let big = Message::Shares(
        (0..4096u64)
            .map(|i| Share {
                x: i + 1,
                y: Fp::new(i),
            })
            .collect(),
    );
    let mut chan = Channel::new(MemPipe::default());
    chan.send(&big).unwrap();
    assert_eq!(chan.recv().unwrap(), big);
}

// =============================================================================
// Strict decode: no trailing bytes, no non-canonical Fp words
// =============================================================================

/// A frame with TRAILING bytes past a complete message is refused, not ignored.
/// Accepting it would make the encoding non-injective — two different frames
/// decoding to the same `Message` — so a digest or replay check over the bytes
/// would not mean what it says. Checked on every variant, since each arm used to
/// return as soon as its own payload was read.
#[test]
fn a_frame_with_trailing_bytes_is_refused() {
    let msgs = vec![
        Message::Shares(vec![Share {
            x: 1,
            y: Fp::new(7),
        }]),
        Message::Open(vec![Share {
            x: 2,
            y: Fp::new(9),
        }]),
        Message::Step(StepCode::SumAndOpen),
        Message::Swap {
            a: 1,
            b: 2,
            swap: true,
        },
        Message::Done,
    ];
    for msg in &msgs {
        // The exact encoding still decodes.
        let exact = msg.encode().expect("a small message frames");
        let mut pipe = MemPipe::default();
        pipe.push_raw(&exact);
        let mut chan = Channel::new(pipe);
        assert_eq!(
            &chan.recv().unwrap(),
            msg,
            "the exact encoding must still round-trip"
        );

        // One extra byte in the body is refused, naming the trailing count.
        let body = &exact[4..];
        let mut extended = body.to_vec();
        extended.push(0xAB);
        let mut pipe = MemPipe::default();
        pipe.push_raw(&frame(&extended));
        let mut chan = Channel::new(pipe);
        let err = chan
            .recv()
            .expect_err("a trailing byte must be refused, not ignored");
        let text = format!("{}", err);
        assert!(
            text.contains("trailing"),
            "the refusal must name the trailing bytes: {text}"
        );
        // Value-free: the trailing byte itself is not echoed.
        assert!(!text.contains("171") && !text.contains("0xAB"), "{text}");
    }
}

/// A `Share` word at or above the field modulus is refused. `Fp::new` REDUCES, so
/// a non-canonical word is a second encoding of a value that already has one, and
/// accepting it would make the codec non-injective. Both halves of a wire share
/// are interpreted in `F_p`, so both are checked; the word exactly at `P - 1` is
/// canonical and still decodes.
#[test]
fn a_non_canonical_share_word_is_refused() {
    // `Fp` is over the Mersenne prime 2^61 - 1.
    const P: u64 = (1u64 << 61) - 1;

    // At P - 1: canonical, decodes.
    let ok = Message::Shares(vec![Share {
        x: P - 1,
        y: Fp::new(P - 1),
    }]);
    let mut pipe = MemPipe::default();
    pipe.push_raw(&ok.encode().expect("a small message frames"));
    let mut chan = Channel::new(pipe);
    assert_eq!(chan.recv().unwrap(), ok, "P - 1 is canonical");

    // At P and above: refused, on either half.
    for (field, x, y) in [
        ("point", P, 1u64),
        ("value", 1u64, P),
        ("value", 1, u64::MAX),
    ] {
        let mut body = vec![0x01u8]; // TAG_SHARES
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&x.to_be_bytes());
        body.extend_from_slice(&y.to_be_bytes()[8 - FIELD_BYTES..]);
        let mut pipe = MemPipe::default();
        pipe.push_raw(&frame(&body));
        let mut chan = Channel::new(pipe);
        let err = chan
            .recv()
            .expect_err("a non-canonical word must be refused");
        let text = format!("{}", err);
        assert!(
            text.contains("non-canonical") && text.contains(field),
            "the refusal must name the non-canonical field: {text}"
        );
        // Value-free: the refusal names the public modulus and the field, and NOT
        // the offending word. Asserted on a word that is distinguishable from the
        // modulus the message legitimately prints.
        assert!(
            !text.contains(&u64::MAX.to_string()),
            "the word must not be echoed: {text}"
        );
    }
}

// =============================================================================
// The OUTBOUND frame is capped too, in closed form
// =============================================================================

/// A message too large to frame is refused on the way OUT, from its element count,
/// before the body is serialised.
///
/// The receiver's cap was the only one: `send` serialised whatever it was handed
/// and wrote it, so the two ends disagreed about what a legal frame is — a body
/// the peer must refuse was built and written in full. `Message::encode` now
/// charges the same `MAX_FRAME_BYTES` cap on `encoded_len`, a closed form of the
/// share count, so the refusal precedes the serialisation. The boundary is
/// asserted from both sides, so the test pins the cap and not merely "large
/// refuses".
#[test]
fn an_over_cap_outbound_frame_is_refused_before_it_is_serialised() {
    // Body = 1 tag + 4 count + n·(8 + FIELD_BYTES). The largest n that fits:
    let per_point = 8 + FIELD_BYTES;
    let fits = (MAX_FRAME_BYTES - 5) / per_point;
    let share = |i: u64| Share {
        x: i + 1,
        y: Fp::new(i),
    };

    let at_cap = Message::Shares((0..fits as u64).map(share).collect());
    let frame = at_cap.encode().expect("the largest framable message encodes");
    assert!(frame.len() - 4 <= MAX_FRAME_BYTES);

    let over = Message::Shares((0..=fits as u64).map(share).collect());
    let err = over
        .encode()
        .expect_err("one point past the cap must be refused");
    let text = format!("{:?}", err);
    assert!(
        text.contains("MAX_FRAME_BYTES"),
        "the refusal must name the cap, got {}",
        text
    );
    assert!(
        text.contains("before it is serialised"),
        "the refusal must say it precedes serialisation, got {}",
        text
    );
}
