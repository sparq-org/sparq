// [GPT-5.6] #1046 — import-boundary coverage for the same .nt.zst fixture through the
// Upload and From URL call shapes. The helper returns parser-ready RDF text + syntax; the
// wasm parser is covered separately, so these tests stay fast and framework-free.
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import assert from "node:assert/strict";
import { gzipSync } from "node:zlib";

import { archiveCodecForFile, bytesToRdf } from "../src/lib/rdf-import.ts";

const SAMPLE_NT =
  '<http://example.org/s> <http://example.org/p> "object value" .\n' +
  "<http://example.org/s> <http://example.org/q> <http://example.org/o2> .\n";

// Reference-codec fixture generated once from SAMPLE_NT with zstd. Node 20 has no zstd
// compressor, so retaining the real RFC 8878 frame as base64 keeps the test portable.
const NT_ZST_FIXTURE = Uint8Array.from(
  Buffer.from(
    "KLUv/SSHDQIAJAM8aHR0cDovL2V4YW1wbGUub3JnL3M+IHA+ICJvYmplY3QgdmFsdWUiIC4KcW8yPiAuCgQAOooIuAaWolmlZxOc62uy",
    "base64",
  ),
);

test("Upload imports a .nt.zst fixture as parser-ready N-Triples", async () => {
  const result = await bytesToRdf(NT_ZST_FIXTURE, "fixture.nt.zst");
  assert.deepEqual(result, { text: SAMPLE_NT, format: "ntriples" });
});

test("From URL imports the same .nt.zst fixture from binary response metadata", async () => {
  const result = await bytesToRdf(
    NT_ZST_FIXTURE,
    "https://example.org/fixture.nt.zst?download=1",
    { explicitFormat: "__auto__", contentType: "application/zstd" },
  );
  assert.deepEqual(result, { text: SAMPLE_NT, format: "ntriples" });
});

test("bzip2 stays rejected with the existing unsupported archive error", async () => {
  const bzip2Magic = new Uint8Array([0x42, 0x5a, 0x68, 0x39, 0, 0, 0, 0]);
  await assert.rejects(
    bytesToRdf(bzip2Magic, "fixture.nt.bz2"),
    /Unrecognised compressed payload: expected a gzip.*zip.*zstd/,
  );
});

test("bzip2 Content-Type is rejected even when the name and bytes say nothing", async () => {
  await assert.rejects(
    bytesToRdf(new TextEncoder().encode("x"), "https://example.org/dump", {
      contentType: "application/x-bzip2",
    }),
    /Unrecognised compressed payload: expected a gzip.*zip.*zstd/,
  );
});

test("Upload decodes gzip to parser-ready text using the inner name's format", async () => {
  const gz = new Uint8Array(gzipSync(Buffer.from(SAMPLE_NT)));
  const result = await bytesToRdf(gz, "umbc.nt.gz");
  assert.deepEqual(result, { text: SAMPLE_NT, format: "ntriples" });
});

test("archiveCodecForFile offers gzip/zip/zstd and never bzip2", () => {
  const bzip2Magic = new Uint8Array([0x42, 0x5a, 0x68, 0x39]);
  assert.equal(archiveCodecForFile(new Uint8Array([0x1f, 0x8b]), "x"), "gzip");
  assert.equal(archiveCodecForFile(NT_ZST_FIXTURE, "x"), "zstd");
  assert.equal(archiveCodecForFile(new Uint8Array(), "dump.zip?x=1"), "zip");
  assert.equal(archiveCodecForFile(bzip2Magic, "x.nt.bz2"), undefined);
  assert.equal(archiveCodecForFile(bzip2Magic, "x.nt.gz"), "gzip");
  assert.equal(archiveCodecForFile(new Uint8Array(), "data.ttl"), undefined);
});

test("site decoding goes through @sparq/client, not a site-local codec copy (#5114)", async () => {
  const source = await readFile(
    new URL("../src/lib/rdf-import.ts", import.meta.url),
    "utf8",
  );
  assert.match(source, /decompressDatasetBytes,[\s\S]*from "@sparq\/client"/);
  assert.doesNotMatch(source, /fzstd|DecompressionStream/);
});
