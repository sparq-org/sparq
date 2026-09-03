import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const component = readFileSync(
  new URL("../src/components/papers/paper-provenance.tsx", import.meta.url),
  "utf8",
);
const route = readFileSync(
  new URL("../src/app/papers/[slug]/page.tsx", import.meta.url),
  "utf8",
);

test("the paper UI distinguishes factory-checked container bytes from publisher-recorded members", () => {
  assert.match(component, /sync\/build independently re-hashes the archive container/);
  assert.match(component, /publisher-recorded attestations/);
  assert.match(component, /does not independently decompress and repeat/);
  assert.match(component, /descriptor-only and must be digest-checked after download/);
});

test("the run stamp renders source and analysis commits plus publisher and manifest identities", () => {
  assert.match(component, /source \{shortCommit\(run\.sourceCommit\)\}/);
  assert.match(component, /analysis \{shortCommit\(run\.analysisCommit\)\}/);
  assert.match(component, /run\.publisherPath/);
  assert.match(component, /run\.publisherSha256/);
  assert.match(component, /run\.rawArchiveManifestSha256/);
  assert.match(component, /run\.rawArchiveZstdExecutableSha256/);
});

test("the route maps every member-verification provenance field into the UI model", () => {
  for (const field of [
    "publisher_path",
    "publisher_sha256",
    "input_file_count",
    "raw_archive_member_verification_authority",
    "raw_archive_manifest_member",
    "raw_archive_manifest_sha256",
    "raw_archive_manifest_bytes",
    "raw_archive_manifest_entry_count",
    "raw_archive_regular_members",
    "raw_archive_all_members_rehashed",
    "raw_archive_exact_member_set",
    "raw_archive_sanitization_scan_passed",
    "raw_archive_deterministic_tar_headers",
    "raw_archive_zstd_version",
    "raw_archive_zstd_executable_sha256",
  ]) {
    assert.ok(route.includes(`run.${field}`), `missing UI mapping for ${field}`);
  }
});

test("superseded pure-overhead terminology is absent from the user-facing provenance UI", () => {
  const superseded = /authorization overhead|pure access-control overhead|guarded-interface premium|same-data counterfactual/i;
  assert.doesNotMatch(component, superseded);
  assert.doesNotMatch(route, superseded);
});
