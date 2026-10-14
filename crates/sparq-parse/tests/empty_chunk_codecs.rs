//! Deterministic empty-chunk member and decoded-equivalence coverage.

use sparq_parse::{
    compress_chunks, compress_member, decode_gzip_concat, decode_zstd_concat, gzip_single_member,
    Codec, Mode,
};

fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.iter().flatten().copied().collect()
}

#[test]
fn empty_members_are_nonempty_and_decode_to_empty() {
    let cases = [
        (
            Codec::Gzip { level: 6 },
            decode_gzip_concat as fn(&[u8]) -> std::io::Result<Vec<u8>>,
        ),
        (Codec::Zstd { level: 3 }, decode_zstd_concat),
    ];

    for (codec, decode) in cases {
        let member = compress_member(&codec, b"").expect("empty member must compress");
        assert!(!member.is_empty(), "empty input still emits a member");
        assert_eq!(decode(&member).expect("empty member must decode"), b"");
    }
}

#[test]
fn serial_chunks_keep_empty_members_and_decode_the_payload() {
    let chunks: [&[u8]; 3] = [b"", b"abc", b""];
    let cases = [
        (
            Codec::Gzip { level: 6 },
            decode_gzip_concat as fn(&[u8]) -> std::io::Result<Vec<u8>>,
        ),
        (Codec::Zstd { level: 3 }, decode_zstd_concat),
    ];

    for (codec, decode) in cases {
        let members = compress_chunks(&chunks, &codec, Mode::Serial).expect("chunks must compress");
        assert_eq!(members.len(), 3, "each chunk must emit one member");
        assert!(members.iter().all(|member| !member.is_empty()));
        assert_eq!(
            decode(&concat(&members)).expect("concatenated members must decode"),
            b"abc"
        );
    }
}

#[test]
fn serial_single_gzip_member_accepts_only_empty_chunks() {
    let chunks: [&[u8]; 2] = [b"", b""];
    let pieces = gzip_single_member(&chunks, 6, Mode::Serial)
        .expect("empty chunks must form one gzip member");
    let wire = concat(&pieces);

    assert!(!wire.is_empty(), "empty input still emits a gzip member");
    assert_eq!(
        decode_gzip_concat(&wire).expect("single gzip member must decode"),
        b""
    );
}

#[test]
fn parallel_and_serial_modes_decode_equivalently_with_empty_chunks() {
    let chunks: [&[u8]; 5] = [b"", b"alpha", b"", b"beta", b""];
    let cases = [
        (
            Codec::Gzip { level: 6 },
            decode_gzip_concat as fn(&[u8]) -> std::io::Result<Vec<u8>>,
        ),
        (Codec::Zstd { level: 3 }, decode_zstd_concat),
    ];

    for (codec, decode) in cases {
        let serial =
            compress_chunks(&chunks, &codec, Mode::Serial).expect("serial compression must work");
        let parallel = compress_chunks(&chunks, &codec, Mode::Parallel)
            .expect("parallel compression must work");
        assert_eq!(serial.len(), chunks.len());
        assert_eq!(parallel.len(), chunks.len());

        let serial_decoded =
            decode(&concat(&serial)).expect("serial members must decode successfully");
        let parallel_decoded =
            decode(&concat(&parallel)).expect("parallel members must decode successfully");
        assert_eq!(serial_decoded, b"alphabeta");
        assert_eq!(parallel_decoded, serial_decoded);
    }
}
