#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  closeSync,
  existsSync,
  lstatSync,
  openSync,
  readSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const GENERATED_BY = "site/scripts/sync-canonical-timing.mjs";
export const TIMING_ENVIRONMENT = "canonical-timing";

export class CanonicalTimingError extends Error {
  constructor(message, options = {}) {
    super(message, options);
    this.name = "CanonicalTimingError";
  }
}

function fail(message) {
  throw new CanonicalTimingError(message);
}

function readJson(path, label) {
  let raw;
  try {
    raw = readFileSync(path, "utf8");
  } catch (error) {
    throw new CanonicalTimingError(`${label} cannot be read: ${path}`, { cause: error });
  }
  try {
    return JSON.parse(raw);
  } catch (error) {
    throw new CanonicalTimingError(`${label} is not valid JSON: ${path}`, { cause: error });
  }
}

function requireObject(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${label} must be an object`);
  }
  return value;
}

function requireOnlyKeys(value, allowed, label) {
  const extras = Object.keys(value).filter((key) => !allowed.includes(key));
  if (extras.length > 0) {
    fail(`${label} contains unsupported field(s): ${extras.sort().join(", ")}`);
  }
}

function requireString(value, label) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${label} must be a non-empty string`);
  }
  return value;
}

function requireInteger(value, label, minimum = 0) {
  if (!Number.isInteger(value) || value < minimum) {
    fail(`${label} must be an integer >= ${minimum}`);
  }
  return value;
}

function requireArchiveMember(value, label) {
  const member = requireString(value, label);
  const segments = member.split("/");
  if (
    member.startsWith("/") ||
    member.includes("\\") ||
    /[\u0000-\u001f\u007f]/.test(member) ||
    segments.some((segment) => segment === "" || segment === "." || segment === "..")
  ) {
    fail(`${label} must be a safe relative archive member path`);
  }
  return member;
}

function resolveInside(root, rel, label) {
  requireString(rel, label);
  if (isAbsolute(rel)) fail(`${label} must be repository-relative: ${rel}`);
  const absolute = resolve(root, rel);
  const fromRoot = relative(root, absolute);
  if (fromRoot === ".." || fromRoot.startsWith(`..${sep}`) || isAbsolute(fromRoot)) {
    fail(`${label} escapes the repository root: ${rel}`);
  }
  return absolute;
}

function resolveUnder(root, rel, requiredPrefix, label) {
  const absolute = resolveInside(root, rel, label);
  const requiredRoot = resolve(root, requiredPrefix);
  const fromRequiredRoot = relative(requiredRoot, absolute);
  if (
    fromRequiredRoot === ".." ||
    fromRequiredRoot.startsWith(`..${sep}`) ||
    isAbsolute(fromRequiredRoot)
  ) {
    fail(`${label} must remain below ${requiredPrefix}: ${rel}`);
  }
  return absolute;
}

function sha256(path) {
  // Raw archives may be much larger than their JSON envelope. Hash incrementally so an
  // outcome-verification build has bounded memory overhead rather than duplicating the entire
  // compressed bundle in a Node buffer.
  const hash = createHash("sha256");
  const fd = openSync(path, "r");
  const chunk = Buffer.allocUnsafe(1024 * 1024);
  try {
    let bytesRead;
    do {
      bytesRead = readSync(fd, chunk, 0, chunk.length, null);
      if (bytesRead > 0) hash.update(chunk.subarray(0, bytesRead));
    } while (bytesRead > 0);
  } finally {
    closeSync(fd);
  }
  return hash.digest("hex");
}

function verifyRegularFileWithin(root, path, label) {
  if (!existsSync(path)) fail(`${label} is missing: ${path}`);
  const metadata = lstatSync(path);
  if (metadata.isSymbolicLink()) fail(`${label} must be a regular file, not a symbolic link`);
  if (!metadata.isFile()) fail(`${label} must be a regular file`);

  // Lexical containment alone is insufficient because any intermediate directory may be a
  // symlink. Resolve both sides and also require the target to occupy the expected real path;
  // canonical evidence must not smuggle bytes from elsewhere through a repository-local name.
  const lexicalRoot = resolve(root);
  const realRoot = realpathSync(lexicalRoot);
  const realPath = realpathSync(path);
  const fromRealRoot = relative(realRoot, realPath);
  if (
    fromRealRoot === ".." ||
    fromRealRoot.startsWith(`..${sep}`) ||
    isAbsolute(fromRealRoot)
  ) {
    fail(`${label} resolves outside the repository root`);
  }
  const expectedRealPath = resolve(realRoot, relative(lexicalRoot, path));
  if (realPath !== expectedRealPath) {
    fail(`${label} must not traverse a symbolic-link path component`);
  }
}

function verifyHashedFile(root, descriptor, label) {
  requireObject(descriptor, label);
  const path = resolveInside(root, descriptor.path, `${label}.path`);
  verifyRegularFileWithin(root, path, `${label}.path`);
  if (!/^[0-9a-f]{64}$/.test(descriptor.sha256 ?? "")) {
    fail(`${label}.sha256 must be lowercase 64-hex`);
  }
  const actual = sha256(path);
  if (actual !== descriptor.sha256) {
    fail(`${label} hash drift: expected ${descriptor.sha256}, got ${actual} (${descriptor.path})`);
  }
  return path;
}

function fileBytes(path) {
  return statSync(path).size;
}

function pointerToken(value) {
  return value.replaceAll("~", "~0").replaceAll("/", "~1");
}

function finiteResultPointers(value, pointer = "/results", pointers = []) {
  if (typeof value === "number") {
    if (!Number.isFinite(value)) fail(`${pointer}: result number must be finite`);
    pointers.push(pointer);
  } else if (typeof value === "boolean") {
    pointers.push(pointer);
  } else if (Array.isArray(value)) {
    value.forEach((item, index) => finiteResultPointers(item, `${pointer}/${index}`, pointers));
  } else if (value !== null && typeof value === "object") {
    for (const [key, item] of Object.entries(value)) {
      finiteResultPointers(item, `${pointer}/${pointerToken(key)}`, pointers);
    }
  } else if (value === null) {
    fail(`${pointer}: canonical results must not contain unresolved null placeholders`);
  }
  return pointers;
}

function validatePaperList(value, label) {
  if (!Array.isArray(value) || value.length === 0) {
    fail(`${label} must contain at least one paper slug`);
  }
  const papers = value.map((paper, index) => requireString(paper, `${label}[${index}]`));
  if (new Set(papers).size !== papers.length) fail(`${label} must not contain duplicates`);
  return papers;
}

function timingProvenance(envelope) {
  const rawArchive = envelope.analysis.raw_archive;
  const archiveVerification = envelope.analysis.raw_archive_verification;
  const localArchive = Object.hasOwn(rawArchive, "path");
  return {
    study_id: envelope.study_id,
    run_id: envelope.run_id,
    collected_at_utc: envelope.collected_at_utc,
    source_git_commit: envelope.source.git_commit,
    analysis_git_commit: envelope.analysis.git_commit,
    host_class: envelope.environment.class,
    host_label: envelope.environment.host_label,
    tenancy: envelope.environment.tenancy,
    noise_limitation: envelope.environment.noise_limitation,
    workload: envelope.workload.label,
    dataset: envelope.workload.dataset,
    query_scope: envelope.workload.query_scope,
    protocol: `${envelope.workload.protocol.path} v${envelope.workload.protocol.version}`,
    bootstrap_draws: envelope.analysis.bootstrap_draws,
    bootstrap_seed: envelope.analysis.bootstrap_seed,
    publisher_path: envelope.analysis.publisher.path,
    publisher_sha256: envelope.analysis.publisher.sha256,
    input_file_count: envelope.analysis.input_files.length,
    raw_archive_kind: localArchive ? "committed" : "external",
    raw_archive_location: localArchive ? rawArchive.path : rawArchive.url,
    raw_archive_public_url: localArchive ? (rawArchive.public_url ?? "") : rawArchive.url,
    raw_archive_sha256: rawArchive.sha256,
    raw_archive_bytes: rawArchive.bytes,
    raw_archive_build_verification: localArchive ? "local-rehash" : "descriptor-only",
    raw_archive_member_verification_authority: "publisher-recorded",
    raw_archive_manifest_member: archiveVerification.manifest_member,
    raw_archive_manifest_sha256: archiveVerification.manifest.sha256,
    raw_archive_manifest_bytes: archiveVerification.manifest.bytes,
    raw_archive_manifest_entry_count: archiveVerification.manifest.entry_count,
    raw_archive_regular_members: archiveVerification.regular_members,
    raw_archive_all_members_rehashed: archiveVerification.all_members_rehashed,
    raw_archive_exact_member_set: archiveVerification.exact_member_set,
    raw_archive_sanitization_scan_passed: archiveVerification.sanitization_scan_passed,
    raw_archive_deterministic_tar_headers: archiveVerification.deterministic_tar_headers,
    raw_archive_zstd_version: archiveVerification.zstd.version,
    raw_archive_zstd_executable_sha256: archiveVerification.zstd.executable_sha256,
  };
}

export function resolveJsonPointer(document, pointer) {
  if (pointer === "") return document;
  if (typeof pointer !== "string" || !pointer.startsWith("/")) {
    fail(`JSON pointer must start with '/': ${String(pointer)}`);
  }
  let current = document;
  for (const rawToken of pointer.slice(1).split("/")) {
    const token = rawToken.replaceAll("~1", "/").replaceAll("~0", "~");
    if (Array.isArray(current)) {
      if (!/^(0|[1-9][0-9]*)$/.test(token)) fail(`invalid JSON array index '${token}' in ${pointer}`);
      const index = Number(token);
      if (index >= current.length) fail(`JSON array index out of range in ${pointer}`);
      current = current[index];
    } else if (current !== null && typeof current === "object") {
      if (!Object.hasOwn(current, token)) fail(`JSON pointer key '${token}' not found in ${pointer}`);
      current = current[token];
    } else {
      fail(`JSON pointer descends through a scalar in ${pointer}`);
    }
  }
  return current;
}

function validateEnvelope(root, envelopeRel, envelope) {
  requireObject(envelope, `envelope ${envelopeRel}`);
  requireOnlyKeys(
    envelope,
    [
      "schema_version",
      "canonical",
      "study_id",
      "run_id",
      "collected_at_utc",
      "source",
      "environment",
      "workload",
      "analysis",
      "correctness",
      "results",
      "figures",
      "paper_evidence",
      "paper_figures",
    ],
    `envelope ${envelopeRel}`,
  );
  if (envelope.schema_version !== 1) fail(`${envelopeRel}: schema_version must be 1`);
  if (envelope.canonical !== true) fail(`${envelopeRel}: canonical must be true`);
  requireString(envelope.study_id, `${envelopeRel}.study_id`);
  requireString(envelope.run_id, `${envelopeRel}.run_id`);
  const canonicalRoot = resolve(root, "bench/canonical-competitor-results");
  const envelopePath = resolveInside(root, envelopeRel, `${envelopeRel} path`);
  const canonicalParts = relative(canonicalRoot, envelopePath).split(sep);
  if (canonicalParts.length !== 3 || canonicalParts[2] !== "paper-summary.json") {
    fail(
      `${envelopeRel}: canonical envelope path must be ` +
        "bench/canonical-competitor-results/<study-key>/<run-id>/paper-summary.json",
    );
  }
  if (canonicalParts[1] !== envelope.run_id) {
    fail(
      `${envelopeRel}: run_id '${envelope.run_id}' must equal its envelope directory ` +
        `'${canonicalParts[1]}'`,
    );
  }
  const collected = requireString(envelope.collected_at_utc, `${envelopeRel}.collected_at_utc`);
  if (!collected.endsWith("Z") || Number.isNaN(Date.parse(collected))) {
    fail(`${envelopeRel}.collected_at_utc must be an ISO-8601 UTC timestamp`);
  }

  const source = requireObject(envelope.source, `${envelopeRel}.source`);
  if (!/^[0-9a-f]{40}$/.test(source.git_commit ?? "")) {
    fail(`${envelopeRel}.source.git_commit must be lowercase 40-hex`);
  }
  if (source.tree_clean !== true) fail(`${envelopeRel}.source.tree_clean must be true`);

  const environment = requireObject(envelope.environment, `${envelopeRel}.environment`);
  if (!["controlled-single-process-ec2", "dedicated-bare-metal"].includes(environment.class)) {
    fail(`${envelopeRel}.environment.class is not a supported canonical host class`);
  }
  requireString(environment.host_label, `${envelopeRel}.environment.host_label`);
  requireString(environment.os, `${envelopeRel}.environment.os`);
  if (environment.os.toLowerCase() !== "linux") fail(`${envelopeRel}: canonical timing must be Linux`);
  requireString(environment.cpu_affinity, `${envelopeRel}.environment.cpu_affinity`);
  if (!/^[0-9]+$/.test(environment.cpu_affinity)) {
    fail(`${envelopeRel}.environment.cpu_affinity must identify exactly one logical CPU`);
  }
  if (environment.rayon_threads !== 1) fail(`${envelopeRel}.environment.rayon_threads must be 1`);
  requireString(environment.tenancy, `${envelopeRel}.environment.tenancy`);
  requireString(environment.noise_limitation, `${envelopeRel}.environment.noise_limitation`);
  if (
    environment.class === "controlled-single-process-ec2" &&
    environment.tenancy !== "shared"
  ) {
    fail(`${envelopeRel}: controlled EC2 runs must honestly declare tenancy='shared'`);
  }

  const workload = requireObject(envelope.workload, `${envelopeRel}.workload`);
  requireString(workload.label, `${envelopeRel}.workload.label`);
  requireString(workload.dataset, `${envelopeRel}.workload.dataset`);
  requireString(workload.query_scope, `${envelopeRel}.workload.query_scope`);
  verifyHashedFile(root, workload.protocol, `${envelopeRel}.workload.protocol`);
  requireString(workload.protocol.version, `${envelopeRel}.workload.protocol.version`);

  const analysis = requireObject(envelope.analysis, `${envelopeRel}.analysis`);
  if (!/^[0-9a-f]{40}$/.test(analysis.git_commit ?? "")) {
    fail(`${envelopeRel}.analysis.git_commit must be lowercase 40-hex`);
  }
  if (analysis.tree_clean !== true) fail(`${envelopeRel}.analysis.tree_clean must be true`);
  if (analysis.canonical_validation_passed !== true) {
    fail(`${envelopeRel}.analysis.canonical_validation_passed must be true`);
  }
  verifyHashedFile(root, analysis.script, `${envelopeRel}.analysis.script`);
  requireObject(analysis.publisher, `${envelopeRel}.analysis.publisher`);
  requireOnlyKeys(
    analysis.publisher,
    ["path", "sha256"],
    `${envelopeRel}.analysis.publisher`,
  );
  verifyHashedFile(root, analysis.publisher, `${envelopeRel}.analysis.publisher`);
  requireInteger(analysis.bootstrap_draws, `${envelopeRel}.analysis.bootstrap_draws`, 100);
  requireInteger(analysis.bootstrap_seed, `${envelopeRel}.analysis.bootstrap_seed`, 0);
  if (!Array.isArray(analysis.input_files) || analysis.input_files.length === 0) {
    fail(`${envelopeRel}.analysis.input_files must be a non-empty array`);
  }
  if (!Array.isArray(analysis.output_files) || analysis.output_files.length === 0) {
    fail(`${envelopeRel}.analysis.output_files must be a non-empty array`);
  }
  const outputsByName = new Map();
  for (const [index, output] of analysis.output_files.entries()) {
    requireObject(output, `${envelopeRel}.analysis.output_files[${index}]`);
    requireOnlyKeys(
      output,
      ["name", "sha256", "bytes"],
      `${envelopeRel}.analysis.output_files[${index}]`,
    );
    requireString(output.name, `${envelopeRel}.analysis.output_files[${index}].name`);
    if (!/^[0-9a-f]{64}$/.test(output.sha256 ?? "")) {
      fail(`${envelopeRel}.analysis.output_files[${index}].sha256 must be lowercase 64-hex`);
    }
    requireInteger(output.bytes, `${envelopeRel}.analysis.output_files[${index}].bytes`, 1);
    if (outputsByName.has(output.name)) {
      fail(`${envelopeRel}.analysis.output_files contains duplicate name '${output.name}'`);
    }
    outputsByName.set(output.name, output);
  }
  const seenInputPaths = new Set();
  const seenArchiveMembers = new Set();
  for (const [index, input] of analysis.input_files.entries()) {
    requireObject(input, `${envelopeRel}.analysis.input_files[${index}]`);
    requireOnlyKeys(
      input,
      ["path", "sha256", "archive_member"],
      `${envelopeRel}.analysis.input_files[${index}]`,
    );
    const inputPath = requireString(input.path, `${envelopeRel}.analysis.input_files[${index}].path`);
    if (!/^[0-9a-f]{64}$/.test(input.sha256 ?? "")) {
      fail(`${envelopeRel}.analysis.input_files[${index}].sha256 must be lowercase 64-hex`);
    }
    const archiveMember = requireArchiveMember(
      input.archive_member,
      `${envelopeRel}.analysis.input_files[${index}].archive_member`,
    );
    if (seenInputPaths.has(inputPath)) {
      fail(`${envelopeRel}.analysis.input_files contains duplicate path '${inputPath}'`);
    }
    if (seenArchiveMembers.has(archiveMember)) {
      fail(`${envelopeRel}.analysis.input_files contains duplicate archive_member '${archiveMember}'`);
    }
    seenInputPaths.add(inputPath);
    seenArchiveMembers.add(archiveMember);
  }

  // The complete result tree remains in the envelope, while only values actually cited by the
  // manuscript become generated paper-evidence records. Every finite result scalar is still
  // provenance-bound to one or more hashed analyzer inputs/outputs here, so compact publication
  // does not weaken the derivation chain.
  const resultBindings = requireObject(
    analysis.result_bindings,
    `${envelopeRel}.analysis.result_bindings`,
  );
  const bindingPointers = Object.keys(resultBindings);
  if (bindingPointers.length === 0) {
    fail(`${envelopeRel}.analysis.result_bindings must not be empty`);
  }
  const sortedBindingPointers = [...bindingPointers].sort();
  if (bindingPointers.some((pointer, index) => pointer !== sortedBindingPointers[index])) {
    fail(`${envelopeRel}.analysis.result_bindings keys must be serialized in lexical order`);
  }
  const scalarPointers = finiteResultPointers(
    requireObject(envelope.results, `${envelopeRel}.results`),
  ).sort();
  const scalarPointerSet = new Set(scalarPointers);
  const missingBindings = scalarPointers.filter((pointer) => !Object.hasOwn(resultBindings, pointer));
  const extraBindings = bindingPointers.filter((pointer) => !scalarPointerSet.has(pointer));
  if (missingBindings.length > 0 || extraBindings.length > 0) {
    fail(
      `${envelopeRel}.analysis.result_bindings must exactly cover every finite numeric/boolean ` +
        `result scalar (missing: ${missingBindings.join(", ") || "none"}; ` +
        `extra: ${extraBindings.join(", ") || "none"})`,
    );
  }
  const artifacts = new Map();
  for (const input of analysis.input_files) {
    artifacts.set(input.path, { sha256: input.sha256, kind: "input" });
  }
  for (const output of analysis.output_files) {
    if (artifacts.has(output.name)) {
      fail(
        `${envelopeRel}: analysis artifact identifier '${output.name}' is ambiguous between ` +
          "input_files[].path and output_files[].name",
      );
    }
    artifacts.set(output.name, { sha256: output.sha256, kind: "output" });
  }
  for (const pointer of bindingPointers) {
    const bindings = resultBindings[pointer];
    if (!Array.isArray(bindings) || bindings.length === 0) {
      fail(`${envelopeRel}.analysis.result_bindings['${pointer}'] must be a non-empty array`);
    }
    const seenBindings = new Set();
    for (const [index, binding] of bindings.entries()) {
      const label = `${envelopeRel}.analysis.result_bindings['${pointer}'][${index}]`;
      requireObject(binding, label);
      requireOnlyKeys(binding, ["artifact", "sha256", "locator"], label);
      const artifact = requireString(binding.artifact, `${label}.artifact`);
      const locator = requireString(binding.locator, `${label}.locator`);
      if (!/^[0-9a-f]{64}$/.test(binding.sha256 ?? "")) {
        fail(`${label}.sha256 must be lowercase 64-hex`);
      }
      const describedArtifact = artifacts.get(artifact);
      if (!describedArtifact) {
        fail(`${label}.artifact '${artifact}' is absent from analysis input/output descriptors`);
      }
      if (describedArtifact.sha256 !== binding.sha256) {
        fail(`${label}.sha256 disagrees with the descriptor for artifact '${artifact}'`);
      }
      const identity = `${artifact}\u0000${binding.sha256}\u0000${locator}`;
      if (seenBindings.has(identity)) fail(`${label} duplicates an earlier result binding`);
      seenBindings.add(identity);
    }
  }
  // Prefer a sanitized archive committed with the envelope. Sync/build then re-hash the exact
  // bytes locally. An external content-addressed archive remains a documented fallback when the
  // raw corpus is too large to commit; that path is deliberately labelled descriptor-only.
  const rawArchive = requireObject(analysis.raw_archive, `${envelopeRel}.analysis.raw_archive`);
  if (!/^[0-9a-f]{64}$/.test(rawArchive.sha256 ?? "")) {
    fail(`${envelopeRel}.analysis.raw_archive.sha256 must be lowercase 64-hex`);
  }
  requireInteger(rawArchive.bytes, `${envelopeRel}.analysis.raw_archive.bytes`, 1);
  const hasPath = Object.hasOwn(rawArchive, "path");
  const hasUrl = Object.hasOwn(rawArchive, "url");
  if (hasPath === hasUrl) {
    fail(`${envelopeRel}.analysis.raw_archive must contain exactly one of path or url`);
  }
  if (hasPath) {
    requireOnlyKeys(
      rawArchive,
      ["path", "sha256", "bytes", "public_url"],
      `${envelopeRel}.analysis.raw_archive`,
    );
    if (rawArchive.build_verification !== undefined) {
      fail(`${envelopeRel}.analysis.raw_archive.build_verification is derived for a committed archive`);
    }
    const archivePath = resolveUnder(
      root,
      rawArchive.path,
      "bench/canonical-competitor-results",
      `${envelopeRel}.analysis.raw_archive.path`,
    );
    if (dirname(archivePath) !== dirname(envelopePath)) {
      fail(`${envelopeRel}.analysis.raw_archive.path must be beside its run envelope`);
    }
    if (!rawArchive.path.endsWith(".tar.zst")) {
      fail(`${envelopeRel}.analysis.raw_archive.path must name a .tar.zst bundle`);
    }
    verifyHashedFile(root, rawArchive, `${envelopeRel}.analysis.raw_archive`);
    const bytes = fileBytes(archivePath);
    if (bytes !== rawArchive.bytes) {
      fail(
        `${envelopeRel}.analysis.raw_archive byte length drift: expected ${rawArchive.bytes}, got ${bytes}`,
      );
    }
    if (rawArchive.public_url !== undefined) {
      const publicUrl = requireString(
        rawArchive.public_url,
        `${envelopeRel}.analysis.raw_archive.public_url`,
      );
      let parsedPublicUrl;
      try {
        parsedPublicUrl = new URL(publicUrl);
      } catch {
        fail(`${envelopeRel}.analysis.raw_archive.public_url must be an absolute URL`);
      }
      if (parsedPublicUrl.protocol !== "https:") {
        fail(`${envelopeRel}.analysis.raw_archive.public_url must use https`);
      }
      if (!parsedPublicUrl.pathname.endsWith(".tar.zst")) {
        fail(`${envelopeRel}.analysis.raw_archive.public_url must name a .tar.zst bundle`);
      }
    }
  } else {
    requireOnlyKeys(
      rawArchive,
      ["url", "sha256", "bytes", "build_verification"],
      `${envelopeRel}.analysis.raw_archive`,
    );
    const archiveUrl = requireString(rawArchive.url, `${envelopeRel}.analysis.raw_archive.url`);
    let parsedArchiveUrl;
    try {
      parsedArchiveUrl = new URL(archiveUrl);
    } catch {
      fail(`${envelopeRel}.analysis.raw_archive.url must be an absolute URL`);
    }
    if (parsedArchiveUrl.protocol !== "https:") {
      fail(`${envelopeRel}.analysis.raw_archive.url must use https`);
    }
    if (!parsedArchiveUrl.pathname.endsWith(".tar.zst")) {
      fail(`${envelopeRel}.analysis.raw_archive.url must name a .tar.zst bundle`);
    }
    if (rawArchive.build_verification !== "descriptor-only") {
      fail(
        `${envelopeRel}.analysis.raw_archive.build_verification must be 'descriptor-only' ` +
          "for an external archive that clean builds do not fetch",
      );
    }
  }

  // This is a publisher-recorded attestation, not a build-time unpack/re-hash. We locally
  // verify the publisher's bytes above and require a complete, internally consistent statement
  // here. The build separately verifies only the archive container when it is committed.
  const archiveVerification = requireObject(
    analysis.raw_archive_verification,
    `${envelopeRel}.analysis.raw_archive_verification`,
  );
  requireOnlyKeys(
    archiveVerification,
    [
      "manifest_member",
      "manifest",
      "regular_members",
      "all_members_rehashed",
      "exact_member_set",
      "sanitization_scan_passed",
      "deterministic_tar_headers",
      "zstd",
    ],
    `${envelopeRel}.analysis.raw_archive_verification`,
  );
  const manifestMember = requireArchiveMember(
    archiveVerification.manifest_member,
    `${envelopeRel}.analysis.raw_archive_verification.manifest_member`,
  );
  if (seenArchiveMembers.has(manifestMember)) {
    fail(`${envelopeRel}.analysis.raw_archive_verification.manifest_member duplicates an input member`);
  }
  const manifest = requireObject(
    archiveVerification.manifest,
    `${envelopeRel}.analysis.raw_archive_verification.manifest`,
  );
  requireOnlyKeys(
    manifest,
    ["sha256", "bytes", "entry_count"],
    `${envelopeRel}.analysis.raw_archive_verification.manifest`,
  );
  if (!/^[0-9a-f]{64}$/.test(manifest.sha256 ?? "")) {
    fail(`${envelopeRel}.analysis.raw_archive_verification.manifest.sha256 must be lowercase 64-hex`);
  }
  requireInteger(
    manifest.bytes,
    `${envelopeRel}.analysis.raw_archive_verification.manifest.bytes`,
    1,
  );
  const manifestEntryCount = requireInteger(
    manifest.entry_count,
    `${envelopeRel}.analysis.raw_archive_verification.manifest.entry_count`,
    1,
  );
  const regularMembers = requireInteger(
    archiveVerification.regular_members,
    `${envelopeRel}.analysis.raw_archive_verification.regular_members`,
    2,
  );
  if (regularMembers !== manifestEntryCount + 1) {
    fail(
      `${envelopeRel}.analysis.raw_archive_verification.regular_members must equal ` +
        "manifest.entry_count + 1",
    );
  }
  if (analysis.input_files.length > manifestEntryCount) {
    fail(
      `${envelopeRel}.analysis.raw_archive_verification.manifest.entry_count cannot be smaller ` +
        "than analysis.input_files.length",
    );
  }
  for (const flag of [
    "all_members_rehashed",
    "exact_member_set",
    "sanitization_scan_passed",
    "deterministic_tar_headers",
  ]) {
    if (archiveVerification[flag] !== true) {
      fail(`${envelopeRel}.analysis.raw_archive_verification.${flag} must be true`);
    }
  }
  const zstd = requireObject(
    archiveVerification.zstd,
    `${envelopeRel}.analysis.raw_archive_verification.zstd`,
  );
  requireOnlyKeys(
    zstd,
    ["version", "executable_sha256"],
    `${envelopeRel}.analysis.raw_archive_verification.zstd`,
  );
  requireString(zstd.version, `${envelopeRel}.analysis.raw_archive_verification.zstd.version`);
  if (!/^[0-9a-f]{64}$/.test(zstd.executable_sha256 ?? "")) {
    fail(
      `${envelopeRel}.analysis.raw_archive_verification.zstd.executable_sha256 ` +
        "must be lowercase 64-hex",
    );
  }

  const correctness = requireObject(envelope.correctness, `${envelopeRel}.correctness`);
  if (correctness.passed !== true) fail(`${envelopeRel}.correctness.passed must be true`);
  requireInteger(correctness.records, `${envelopeRel}.correctness.records`, 1);

  requireObject(envelope.results, `${envelopeRel}.results`);
  const figures = requireObject(envelope.figures, `${envelopeRel}.figures`);
  for (const [name, figure] of Object.entries(figures)) {
    const figurePath = resolveUnder(
      root,
      figure.path,
      "site/papers/figures/canonical-timing",
      `${envelopeRel}.figures.${name}.path`,
    );
    verifyHashedFile(root, figure, `${envelopeRel}.figures.${name}`);
    if (figure.media_type !== "image/svg+xml") {
      fail(`${envelopeRel}.figures.${name}.media_type must be image/svg+xml`);
    }
    if (!figure.path.startsWith("site/papers/figures/canonical-timing/") || !figure.path.endsWith(".svg")) {
      fail(
        `${envelopeRel}.figures.${name}.path must be an SVG below ` +
          "site/papers/figures/canonical-timing/",
      );
    }
    // A timing SVG is admitted only when three independently useful statements agree:
    // the committed bytes, the figure descriptor used by timing_figure(), and the analyzer's
    // immutable output manifest. `output_name` avoids a basename convention hiding ambiguity.
    const outputName = requireString(figure.output_name, `${envelopeRel}.figures.${name}.output_name`);
    const output = outputsByName.get(outputName);
    if (!output) {
      fail(`${envelopeRel}.figures.${name}.output_name '${outputName}' is absent from analysis.output_files`);
    }
    if (output.sha256 !== figure.sha256) {
      fail(`${envelopeRel}.figures.${name} hash disagrees with analysis.output_files['${outputName}']`);
    }
    const bytes = fileBytes(figurePath);
    if (output.bytes !== bytes) {
      fail(
        `${envelopeRel}.figures.${name} byte length ${bytes} disagrees with ` +
          `analysis.output_files['${outputName}'].bytes=${output.bytes}`,
      );
    }
    if (basename(figure.path) !== outputName) {
      fail(
        `${envelopeRel}.figures.${name}.output_name must equal the committed SVG basename ` +
          `'${basename(figure.path)}'`,
      );
    }
  }

  return {
    values: requireObject(envelope.paper_evidence, `${envelopeRel}.paper_evidence`),
    figures,
    paperFigures: requireObject(envelope.paper_figures, `${envelopeRel}.paper_figures`),
  };
}

function generatedRecord(envelopeRel, envelope, key, declaration) {
  requireObject(declaration, `${envelopeRel}.paper_evidence.${key}`);
  requireOnlyKeys(
    declaration,
    ["pointer", "unit", "estimator", "hypothesis", "note", "papers"],
    `${envelopeRel}.paper_evidence.${key}`,
  );
  if (!/^[a-z0-9][a-z0-9_.-]+$/.test(key)) fail(`${envelopeRel}: invalid evidence key '${key}'`);
  const pointer = requireString(declaration.pointer, `${key}.pointer`);
  if (!pointer.startsWith("/results/")) fail(`${key}.pointer must point below /results/`);
  const value = resolveJsonPointer(envelope, pointer);
  const isNumber = typeof value === "number" && Number.isFinite(value);
  const isVerdict = typeof value === "boolean";
  if (!isNumber && !isVerdict) {
    fail(`${key}: canonical timing evidence must be a finite number or derived boolean verdict`);
  }
  const unit = requireString(declaration.unit, `${key}.unit`);
  if (isVerdict && unit !== "boolean") fail(`${key}: a derived boolean verdict must use unit='boolean'`);
  const hypothesis = declaration.hypothesis;
  if (isVerdict && !["H1", "H2"].includes(hypothesis)) {
    fail(`${key}: a derived boolean verdict must identify hypothesis='H1' or 'H2'`);
  }
  if (isNumber && hypothesis !== undefined) {
    fail(`${key}: numeric/descriptive evidence must not declare a mechanical hypothesis verdict`);
  }
  const note = requireString(declaration.note, `${key}.note`);
  const papers = validatePaperList(declaration.papers, `${key}.papers`);
  const resultBindings = envelope.analysis.result_bindings[pointer];
  if (!Array.isArray(resultBindings) || resultBindings.length === 0) {
    fail(`${key}.pointer has no analysis.result_bindings provenance`);
  }
  return {
    value,
    unit,
    environment: TIMING_ENVIRONMENT,
    kind: isVerdict ? "canonical-timing-verdict" : "canonical-timing",
    source: `${envelopeRel}#${pointer}`,
    binding: { kind: "json-pointer", file: envelopeRel, pointer },
    result_bindings: resultBindings,
    timing_provenance: timingProvenance(envelope),
    papers,
    estimator: declaration.estimator ?? "",
    ...(isVerdict ? { hypothesis } : {}),
    note,
    _generated_by: GENERATED_BY,
  };
}

function generatedFigureRecord(envelopeRel, envelope, figures, key, declaration) {
  requireObject(declaration, `${envelopeRel}.paper_figures.${key}`);
  requireOnlyKeys(
    declaration,
    ["figure", "alt", "note", "papers"],
    `${envelopeRel}.paper_figures.${key}`,
  );
  if (!/^[a-z0-9][a-z0-9_.-]+$/.test(key)) fail(`${envelopeRel}: invalid figure key '${key}'`);
  const figureName = requireString(declaration.figure, `${key}.figure`);
  if (!Object.hasOwn(figures, figureName)) {
    fail(`${key}.figure names unknown envelope figure '${figureName}'`);
  }
  const figure = figures[figureName];
  const output = envelope.analysis.output_files.find((entry) => entry.name === figure.output_name);
  const alt = requireString(declaration.alt, `${key}.alt`);
  const note = requireString(declaration.note, `${key}.note`);
  const papers = validatePaperList(declaration.papers, `${key}.papers`);
  const pointer = `/figures/${pointerToken(figureName)}/sha256`;
  const typstPath = `/${figure.path.slice("site/".length)}`;
  return {
    // A figure record binds its bytes, not an image-embedded number, to the envelope.
    // The paper can obtain the path only through timing_figure(), which emits provenance.
    value: figure.sha256,
    unit: "sha256",
    environment: TIMING_ENVIRONMENT,
    kind: "canonical-timing-figure",
    source: `${envelopeRel}#${pointer}`,
    binding: { kind: "json-pointer", file: envelopeRel, pointer },
    figure: {
      typst_path: typstPath,
      media_type: figure.media_type,
      alt,
      output_name: figure.output_name,
      bytes: output.bytes,
    },
    timing_provenance: timingProvenance(envelope),
    papers,
    note,
    _generated_by: GENERATED_BY,
  };
}

export function materializeCanonicalTiming({ root, sourcesPath }) {
  const sources = readJson(sourcesPath, "canonical timing source registry");
  if (sources.schemaVersion !== 1 || !Array.isArray(sources.envelopes)) {
    fail("canonical timing source registry needs schemaVersion=1 and an envelopes array");
  }
  const records = {};
  const seenEnvelopes = new Set();
  for (const [index, envelopeRel] of sources.envelopes.entries()) {
    requireString(envelopeRel, `envelopes[${index}]`);
    if (seenEnvelopes.has(envelopeRel)) fail(`duplicate canonical timing envelope: ${envelopeRel}`);
    seenEnvelopes.add(envelopeRel);
    if (!envelopeRel.startsWith("bench/canonical-competitor-results/")) {
      fail(
        "canonical timing envelope must be a committed path below " +
          `bench/canonical-competitor-results/: ${envelopeRel}`,
      );
    }
    const envelopePath = resolveUnder(
      root,
      envelopeRel,
      "bench/canonical-competitor-results",
      `envelopes[${index}]`,
    );
    verifyRegularFileWithin(root, envelopePath, `envelopes[${index}]`);
    const envelope = readJson(envelopePath, "canonical timing envelope");
    const declarations = validateEnvelope(root, envelopeRel, envelope);
    for (const key of Object.keys(declarations.values).sort()) {
      if (Object.hasOwn(records, key)) fail(`duplicate generated canonical timing key: ${key}`);
      records[key] = generatedRecord(envelopeRel, envelope, key, declarations.values[key]);
    }
    for (const key of Object.keys(declarations.paperFigures).sort()) {
      if (Object.hasOwn(records, key)) fail(`duplicate generated canonical timing key: ${key}`);
      records[key] = generatedFigureRecord(
        envelopeRel,
        envelope,
        declarations.figures,
        key,
        declarations.paperFigures[key],
      );
    }
  }
  return {
    _comment:
      "GENERATED by site/scripts/sync-canonical-timing.mjs from the committed canonical envelopes listed in canonical-timing-sources.json. Do not edit values by hand.",
    schemaVersion: 1,
    records,
  };
}

export function serializedGeneratedEvidence(document) {
  return `${JSON.stringify(document, null, 2)}\n`;
}

export function synchronize({ root, sourcesPath, outputPath, mode }) {
  const generated = materializeCanonicalTiming({ root, sourcesPath });
  const expected = serializedGeneratedEvidence(generated);
  if (mode === "write") {
    writeFileSync(outputPath, expected, "utf8");
    return { records: Object.keys(generated.records).length, changed: true };
  }
  if (mode !== "check") fail(`unsupported mode: ${mode}`);
  const actual = existsSync(outputPath) ? readFileSync(outputPath, "utf8") : "";
  if (actual !== expected) {
    fail(
      `canonical timing evidence is stale: ${outputPath}\n` +
        "run `npm --workspace site run sync-canonical-timing` and commit the generated file",
    );
  }
  return { records: Object.keys(generated.records).length, changed: false };
}

function parseArgs(argv) {
  let mode = "check";
  let root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  let sourcesPath;
  let outputPath;
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--check") mode = "check";
    else if (arg === "--write") mode = "write";
    else if (arg === "--root") root = resolve(argv[++index]);
    else if (arg === "--sources") sourcesPath = resolve(argv[++index]);
    else if (arg === "--out") outputPath = resolve(argv[++index]);
    else fail(`unknown argument: ${arg}`);
  }
  sourcesPath ??= resolve(root, "site/src/data/canonical-timing-sources.json");
  outputPath ??= resolve(root, "site/src/data/paper-evidence.canonical-timing.generated.json");
  return { root, sourcesPath, outputPath, mode };
}

function isMain() {
  return process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
}

if (isMain()) {
  try {
    const result = synchronize(parseArgs(process.argv.slice(2)));
    console.log(
      `[canonical-timing] ${result.changed ? "wrote" : "verified"} ${result.records} record(s)`,
    );
  } catch (error) {
    console.error(`[canonical-timing] FAILED: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
