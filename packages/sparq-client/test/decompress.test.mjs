// CI coverage (#3006): this suite runs in the GATING gui.yml `shared TS client typecheck`
// job (`npm test` in packages/sparq-client), NOT in js.yml — js.yml runs the js/,
// rdfjs-conformance, eyereasoner-compat and solid-server suites only, so its verdict says
// nothing about the codec paths below. A green js.yml is not evidence that these pass.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { deflateRawSync } from "node:zlib";
import ts from "typescript";

import { decompressDatasetBytes } from "../src/index.ts";

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const SAMPLE =
  '<http://example.org/s> <http://example.org/p> "object value" .\n' +
  "<http://example.org/s> <http://example.org/q> <http://example.org/o2> .\n";

const fromBase64 = (value) => Uint8Array.from(Buffer.from(value, "base64"));

// Reference-codec fixtures, generated once from SAMPLE with zstd and bzip2.
const ZSTD_SAMPLE = fromBase64(
  "KLUv/SSHDQIAJAM8aHR0cDovL2V4YW1wbGUub3JnL3M+IHA+ICJvYmplY3QgdmFsdWUiIC4KcW8yPiAuCgQAOooIuAaWolmlZxOc62uy",
);
const BZIP2_SAMPLE = fromBase64(
  "QlpoOTFBWSZTWY3OZasAABVZgAAQUAGQFTrW/0AgAEhJSNNPU0xAGTQSUjQND1ANqPUxGHuVZwTkaIZFEddogtTCXlT66GleyLURCPEOyHFdV0VRbCJMiR9NEIZ+Efi7kinChIRucy1Y",
);
// A reference bzip2 stream with a large decoded output. It is a SINGLE huffman block
// (one 0x314159265359 block header): 1.1 MB of one repeated byte collapses to ~22 kB
// under bzip2's initial run-length pass, far inside the 900 kB block size. It covers
// output-buffer growth, not block iteration — BZIP2_MULTIBLOCK_SAMPLE covers that (#3006).
const BZIP2_LARGE_SAMPLE = fromBase64(
  "QlpoOTFBWSZTWQiXvEUACGyBCKAAAgAACCAAMMwFKaaKUGwlKDxdyRThQkAiXvEU",
);
// A genuinely MULTI-block reference stream: 250 kB of the repeating lowercase alphabet
// compressed at level 1 (100 kB blocks), so the decoder must iterate three blocks and
// concatenate them in order. Generated with a reference bzip2 encoder; the decoded bytes
// are reproduced independently below rather than being asserted against a stored digest.
const BZIP2_MULTIBLOCK_ALPHABET = "abcdefghijklmnopqrstuvwxyz";
const BZIP2_MULTIBLOCK_LENGTH = 250_000;
const BZIP2_MULTIBLOCK_SAMPLE = fromBase64(
  "QlpoMTFBWSZTWc5xShkAB4KBgD////AwAPgCgAADJkFAAAGTIClVDQMmgDRprIFWW1QKtygVYKBV" +
    "vUCrgoFXFQKuSgVc1AqxUCrJQKsZAq6SBVnIFXWQKu0gVd5Aq8SBV5kCr1IFXuQKvkgVaSBV9kCr" +
    "9IFWsgVfzFBWSZTWa8Ub8MAWh4BgD////AwAPgCgAADJkFAAAGTIClVDQMmgDRprIFWyQKtsgVbp" +
    "AqwkCrfIFXCQKuMgVcpAq5yBVjIFWUgVaZqBV1UCrsoFXdQKvCgVeVAq9KBV7UCr4oFWigVYyBV9" +
    "kCr9IFWsgVfzFBWSZTWaul8S4AUrEBgD////AwANtEUAAAZMgTVVADTIaDIBSqgaDTIBpo/SCrZI" +
    "Ktsgq3SCrfIocEqhxSqHJKoc0qh0kFXQgqxIKupBVkQVZkFXYgq7kFXggq8kFXogq9kFXwgqx0IQ" +
    "1SqH1Kofkqh/F3JFOFCQzEgGzg==",
);

async function compressNative(text, format) {
  const stream = new Blob([text])
    .stream()
    .pipeThrough(new CompressionStream(format));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

function concat(parts) {
  const output = new Uint8Array(
    parts.reduce((total, part) => total + part.length, 0),
  );
  let offset = 0;
  for (const part of parts) {
    output.set(part, offset);
    offset += part.length;
  }
  return output;
}

function crc32(bytes) {
  let crc = ~0;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return ~crc >>> 0;
}

function makeZip(name, text) {
  const nameBytes = encoder.encode(name);
  const raw = encoder.encode(text);
  const compressed = new Uint8Array(deflateRawSync(raw));
  const checksum = crc32(raw);

  const local = new DataView(new ArrayBuffer(30));
  local.setUint32(0, 0x04034b50, true);
  local.setUint16(4, 20, true);
  local.setUint16(8, 8, true);
  local.setUint32(14, checksum, true);
  local.setUint32(18, compressed.length, true);
  local.setUint32(22, raw.length, true);
  local.setUint16(26, nameBytes.length, true);

  const central = new DataView(new ArrayBuffer(46));
  central.setUint32(0, 0x02014b50, true);
  central.setUint16(4, 20, true);
  central.setUint16(6, 20, true);
  central.setUint16(10, 8, true);
  central.setUint32(16, checksum, true);
  central.setUint32(20, compressed.length, true);
  central.setUint32(24, raw.length, true);
  central.setUint16(28, nameBytes.length, true);

  const localBytes = concat([
    new Uint8Array(local.buffer),
    nameBytes,
    compressed,
  ]);
  const centralBytes = concat([new Uint8Array(central.buffer), nameBytes]);
  const eocd = new DataView(new ArrayBuffer(22));
  eocd.setUint32(0, 0x06054b50, true);
  eocd.setUint16(8, 1, true);
  eocd.setUint16(10, 1, true);
  eocd.setUint32(12, centralBytes.length, true);
  eocd.setUint32(16, localBytes.length, true);
  return concat([localBytes, centralBytes, new Uint8Array(eocd.buffer)]);
}

test("gzip decodes through the native browser stream and strips its suffix", async () => {
  const compressed = await compressNative(SAMPLE, "gzip");
  const result = await decompressDatasetBytes(compressed, "dataset.nt.gz");
  assert.equal(result.codec, "gzip");
  assert.equal(result.innerName, "dataset.nt");
  assert.equal(decoder.decode(result.bytes), SAMPLE);
});

test("ZIP selects and inflates an RDF member", async () => {
  const result = await decompressDatasetBytes(
    makeZip("graph.ttl", SAMPLE),
    "bundle.zip",
  );
  assert.equal(result.codec, "zip");
  assert.equal(result.innerName, "graph.ttl");
  assert.equal(decoder.decode(result.bytes), SAMPLE);
});

test("zstd decodes a reference frame through fzstd", async () => {
  const result = await decompressDatasetBytes(ZSTD_SAMPLE, "dataset.nt.zst");
  assert.equal(result.codec, "zstd");
  assert.equal(result.innerName, "dataset.nt");
  assert.equal(decoder.decode(result.bytes), SAMPLE);
});

test("bzip2 decodes a reference stream through the lazy browser codec", async () => {
  const result = await decompressDatasetBytes(BZIP2_SAMPLE, "dataset.nt.bz2");
  assert.equal(result.codec, "bzip2");
  assert.equal(result.innerName, "dataset.nt");
  assert.equal(decoder.decode(result.bytes), SAMPLE);
});

test("bzip2 decodes a large stream without truncation", async () => {
  const result = await decompressDatasetBytes(
    BZIP2_LARGE_SAMPLE,
    "large.nt.bz2",
  );
  assert.equal(result.bytes.length, 1_100_000);
  assert.equal(result.bytes[0], 0x61);
  assert.equal(result.bytes.at(-1), 0x61);
});

test("bzip2 concatenates every block of a multi-block stream in order", async () => {
  // Round-trip against an independently reconstructed expectation: the decoded stream must
  // equal the alphabet cycle byte for byte, so a decoder that dropped, reordered or
  // truncated any of the three blocks fails on content and not merely on length.
  const expected = encoder.encode(
    BZIP2_MULTIBLOCK_ALPHABET.repeat(
      Math.ceil(BZIP2_MULTIBLOCK_LENGTH / BZIP2_MULTIBLOCK_ALPHABET.length),
    ).slice(0, BZIP2_MULTIBLOCK_LENGTH),
  );
  const result = await decompressDatasetBytes(
    BZIP2_MULTIBLOCK_SAMPLE,
    "alphabet.nt.bz2",
  );
  assert.equal(result.codec, "bzip2");
  assert.equal(result.innerName, "alphabet.nt");
  assert.equal(result.bytes.length, BZIP2_MULTIBLOCK_LENGTH);
  assert.deepEqual(result.bytes, expected);
});

test("the filename extension selects a codec when magic is unavailable", async () => {
  await assert.rejects(
    decompressDatasetBytes(encoder.encode("not bzip2"), "dataset.nt.bz2"),
    (error) => {
      // The extension must route these bytes to the bzip2 decoder rather than the
      // magic-sniffing fallback...
      assert.doesNotMatch(String(error), /Unrecognised compressed payload/);
      // ...and the rejection must come FROM that decoder. Accepting any error at all let a
      // codec that could not even be loaded pass this test, so a missing seek-bzip install
      // read as a decode regression in the two tests above rather than as an install
      // problem visible here too (#3006).
      assert.notEqual(error.code, "ERR_MODULE_NOT_FOUND");
      return true;
    },
  );
});

test("zstd and bzip2 remain literal dynamic imports with no static codec import", async () => {
  const path = new URL("../src/decompress.ts", import.meta.url);
  const source = await readFile(path, "utf8");
  const file = ts.createSourceFile(
    path.pathname,
    source,
    ts.ScriptTarget.Latest,
    true,
  );
  const dynamicImports = new Set();
  const staticImports = new Set();

  function visit(node) {
    if (
      ts.isImportDeclaration(node) &&
      ts.isStringLiteral(node.moduleSpecifier)
    ) {
      staticImports.add(node.moduleSpecifier.text);
    }
    if (
      ts.isCallExpression(node) &&
      node.expression.kind === ts.SyntaxKind.ImportKeyword &&
      node.arguments.length === 1 &&
      ts.isStringLiteral(node.arguments[0])
    ) {
      dynamicImports.add(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  }
  visit(file);

  assert.deepEqual(dynamicImports, new Set(["fzstd", "seek-bzip", "buffer"]));
  assert.equal(staticImports.has("fzstd"), false);
  assert.equal(staticImports.has("seek-bzip"), false);
  assert.equal(staticImports.has("buffer"), false);
});

test("unknown bytes fail instead of being returned as decoded RDF", async () => {
  await assert.rejects(
    decompressDatasetBytes(encoder.encode("plain RDF?"), "dataset.bin"),
    /Unrecognised compressed payload/,
  );
});
