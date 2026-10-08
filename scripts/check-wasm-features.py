#!/usr/bin/env python3
# sq-3ul2n.2 — assert a built .wasm artifact carries a required WASM feature.
#
# Parses the WebAssembly "target_features" custom section (emitted by LLVM/rustc) and checks
# that a named feature (e.g. simd128) is present with a "used"/"required" (`+`) prefix. Used by
# scripts/ci-bench.sh next to the wasm_bundle_bytes measurement so the root-invoked gate build
# is proven to actually contain +simd128 — closing the gate-vs-shipped parity gap where the
# feature previously lived only in a crate-level .cargo/config.toml that cargo discovered by CWD.
#
# Custom section layout we parse (WebAssembly binary format + the tool-conventions
# target_features section):
#   module   = magic(4) version(4) { section }*
#   section  = id:u8 size:u32(uleb) payload[size]
#   custom   = name_len:u32(uleb) name[name_len] contents...   (section id 0)
#   target_features contents = count:u32(uleb) { prefix:u8 name_len:u32(uleb) name[name_len] }*
#     prefix 0x2B '+' = feature used (required to run), 0x2D '-' = feature disallowed,
#            0x3D '=' = feature required-and-used. We treat '+' and '=' as "present".
#
# stdlib only, no third-party deps.

import argparse
import sys


def read_uleb128(buf, pos):
    """Decode an unsigned LEB128 varint at buf[pos:]. Returns (value, new_pos)."""
    result = 0
    shift = 0
    while True:
        if pos >= len(buf):
            raise ValueError("unexpected end of buffer while reading LEB128")
        byte = buf[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if (byte & 0x80) == 0:
            break
        shift += 7
        if shift > 63:
            raise ValueError("LEB128 varint too long")
    return result, pos


def iter_sections(buf):
    """Yield (section_id, payload_bytes) for each top-level section."""
    if len(buf) < 8:
        raise ValueError("file too short to be a wasm module")
    if buf[0:4] != b"\x00asm":
        raise ValueError("not a wasm module (bad magic %r)" % buf[0:4])
    # buf[4:8] is the version (little-endian u32); we don't need to validate it.
    pos = 8
    while pos < len(buf):
        section_id = buf[pos]
        pos += 1
        size, pos = read_uleb128(buf, pos)
        payload = buf[pos:pos + size]
        if len(payload) != size:
            raise ValueError("truncated section (id=%d)" % section_id)
        yield section_id, payload
        pos += size


def find_target_features(buf):
    """Return the target_features section contents (bytes) or None if absent."""
    for section_id, payload in iter_sections(buf):
        if section_id != 0:  # 0 == custom section
            continue
        name_len, p = read_uleb128(payload, 0)
        name = payload[p:p + name_len]
        if name == b"target_features":
            return payload[p + name_len:]
    return None


def parse_features(contents):
    """Parse target_features contents into a list of (prefix_char, feature_name)."""
    count, pos = read_uleb128(contents, 0)
    features = []
    for _ in range(count):
        if pos >= len(contents):
            raise ValueError("truncated target_features entry list")
        prefix = chr(contents[pos])
        pos += 1
        name_len, pos = read_uleb128(contents, pos)
        name = contents[pos:pos + name_len].decode("utf-8", "replace")
        pos += name_len
        features.append((prefix, name))
    return features


def main(argv):
    ap = argparse.ArgumentParser(
        description="Assert a built .wasm artifact carries a required target feature."
    )
    ap.add_argument("wasm", help="path to the .wasm artifact")
    ap.add_argument("--require", metavar="FEATURE",
                    help="feature name that must be present (e.g. simd128); exit 1 if absent")
    ap.add_argument("--list", action="store_true",
                    help="print the parsed target_features section and exit 0")
    args = ap.parse_args(argv)

    try:
        with open(args.wasm, "rb") as fh:
            buf = fh.read()
    except OSError as exc:
        print("check-wasm-features: cannot read %s: %s" % (args.wasm, exc), file=sys.stderr)
        return 1

    try:
        contents = find_target_features(buf)
    except ValueError as exc:
        print("check-wasm-features: %s: %s" % (args.wasm, exc), file=sys.stderr)
        return 1

    if contents is None:
        features = []
    else:
        try:
            features = parse_features(contents)
        except ValueError as exc:
            print("check-wasm-features: %s: malformed target_features: %s"
                  % (args.wasm, exc), file=sys.stderr)
            return 1

    if args.list:
        if not features:
            print("(no target_features custom section)")
        for prefix, name in features:
            print("%s%s" % (prefix, name))
        return 0

    if not args.require:
        ap.error("one of --require or --list is required")

    # A feature counts as present when prefixed '+' (used) or '=' (required+used).
    present = {name for prefix, name in features if prefix in ("+", "=")}
    if args.require in present:
        print("check-wasm-features: OK — %s carries +%s" % (args.wasm, args.require))
        return 0

    have = ", ".join("%s%s" % (p, n) for p, n in features) or "(none)"
    print("check-wasm-features: FAIL — %s does NOT carry +%s (target_features: %s)"
          % (args.wasm, args.require, have), file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
