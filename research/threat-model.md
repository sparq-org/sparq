<!-- [OPUS-4.8] Threat model for the STABLE core authored by Opus 4.8 (1M context); Fable unavailable — re-review when Fable returns. Bead sq-o9u4. -->
# Threat model — sparq STABLE core

A STRIDE-style threat model for the **production-candidate** part of sparq: the
crates that are W3C-conformance-gated, have high test coverage, and are intended
to be published and run against real data. Authored against the worktree at
`research/threat-model.md` (bead **sq-o9u4**); evidence cites concrete files,
lines, and tests as of the commit on branch `threat-model-core`.

This document is **evidence-based, not aspirational**. Where a mitigation exists
it is cited by test/gate; where it does not, the gap is stated plainly and mapped
to a bead. Do not read a cited mitigation as more than what the cited test
actually verifies.

## Scope

**In scope (STABLE, production-candidate):**

| Crate / path | Why in scope | `unsafe`? |
|---|---|---|
| `sparq-core` | Storage, dictionary, mmap on-disk format, RDF text parsers, fused decompress ingest | **Yes — 42 sites** (mmap, dict-spill, parallel scatter, SIMD); canonical count via `scripts/unsafe-gate.py` |
| `sparq-engine` | SPARQL planner + streaming executor, `QueryBudget`, optional `SERVICE` | Four registered `CancelPtr` sites: `Arc`-owned `AtomicBool`, lifetime-bound guard, atomic loads only (sq-kq9ia) |
| `sparq-server` | The W3C SPARQL 1.1 Protocol HTTP surface | No |
| `sparq-reason` | RDFS / OWL-RL / N3 materialization, inference-conformance-gated | **No `unsafe`** (pure-safe Rust) |
| `sparq-shacl` | SHACL validation, W3C-SHACL-conformance-gated | **No `unsafe`** |
| Parser path | `sparq-core` N-Triples/Turtle/N-Quads/TriG (delegates to `oxttl`) + vendored `spargebra` SPARQL parser | No `unsafe` in the parse layer; `spargebra` is a `peg`-based recursive-descent parser |

**Out of scope (tracked elsewhere — do NOT infer these are safe):**

| Surface | Why excluded | Where it is tracked |
|---|---|---|
| `sparq-zk`, `sparq-zk-compose` | The ZK estate has its **own** adversarial threat model. **Verdict: v1 verifier soundness is BROKEN** (a prover can forge arbitrary accepted results). | `research/zk-soundness-audit.md` (12 confirmed issues, 6 critical) |
| `sparq-mpc` | `publish = false` research scaffold; crypto primitive not yet chosen — not ready for a production threat model. | Bead **sq-jskw** (DEFER until a primitive is chosen) |
| `sparq-gpu` | `publish = false`; measure-first PCIe-break-even prototype, depended on by nothing, so no adversary can reach a kernel. | [`research/gpu-threat-model-deferral.md`](gpu-threat-model-deferral.md) — bead **sq-vrye**: the deferral RECORD (justification, exit trigger, pre-scoped outline), not a model. The trigger is enforced by `crates/sparq-gpu/tests/deferral_premise.rs` <!-- [OPUS-5] sq-vrye --> |
| `sparq-nlq` | LLM→SPARQL; hands untrusted text to a model and executes the result, so it has its **own** model (prompt/data injection, exfiltration, token budget). | [`research/nlq-threat-model.md`](nlq-threat-model.md) — bead **sq-j1wv**, deferral LIFTED by the GPT-5.6 strategy review |

Capability crates not load-bearing for the core production path (`sparq-geo`,
`sparq-text`, `sparq-vectors`, `sparq-rsp`, `sparq-hdt`, `sparq-solid`) are not
covered here; each should get its own model as it stabilizes (out of scope for
sq-o9u4).

## Assets

What an attacker would target, in priority order:

1. **Query-result integrity** — that an answer returned by the engine/server is
   the correct answer to the asked query over the loaded data (no silent
   wrong answers, no silent truncation).
2. **Memory-safety of the `unsafe` mmap / dict-spill paths** — that loading an
   on-disk index never causes undefined behaviour (UB), regardless of how
   hostile/corrupt the file is. This is the highest-severity asset because a
   breach is UB, not merely a wrong answer or a crash.
3. **Service availability** — the server process and the engine staying up and
   responsive under adversarial input (no stack-overflow abort, no OOM, no
   unbounded CPU).
4. **Dataset confidentiality** — when the engine fronts private data, that data
   is not disclosed to a party that should not see it (relevant the moment the
   server is exposed, because there is no auth — see T-HTTP-EoP).
5. **On-disk index files** (`perm*.bin`, `dict-*.bin`, the compressed `.spq`
   format) — integrity of the persisted store; a tampered store must not
   silently corrupt results or, worse, trigger UB on load.
6. **Host environment** (internal network, cloud-metadata endpoint) — protected
   against SSRF when federation is enabled.

## Trust boundaries

```text
                          ┌─────────────────────────────────────────────────┐
 untrusted RDF bytes ────▶│ (B1) parser  (oxttl + sparq-core chunking)       │
 (file / HTTP body)       └─────────────────────────────────────────────────┘
                                              │ triples
                                              ▼
 untrusted SPARQL str ───▶┌─────────────────────────────────────────────────┐
 (HTTP query/body)        │ (B2) spargebra parse → algebra → planner → exec  │
                          └─────────────────────────────────────────────────┘
                                              │ results            │ SERVICE
                                              ▼                    ▼ (B4)
 untrusted HTTP client ──▶┌──────────────────┐         ┌──────────────────────┐
 (anyone on the port)     │ (B3) sparq-server│         │ remote endpoint fetch │
                          └──────────────────┘         │ (only if feature on)  │
                                                        └──────────────────────┘
 hostile on-disk index ──▶┌─────────────────────────────────────────────────┐
 (.spq / dict-*.bin)      │ (B5) mmap loader → UNSAFE pointer reinterpret    │  ◀── highest-severity
                          └─────────────────────────────────────────────────┘
```

- **B1 — untrusted RDF input → parser.** Any byte stream the parser is asked to
  read (a loaded file, an HTTP request body, a federated response).
- **B2 — untrusted SPARQL string → spargebra → planner → executor.** A query
  string is attacker-controlled; it flows
  `string → PreparedQuery::parse` (`crates/sparq-engine/src/lib.rs:380`)
  `→ spargebra::SparqlParser::parse_query` (`vendor/spargebra/src/parser.rs:121`)
  `→ algebra → planner → executor`.
- **B3 — untrusted HTTP client → server.** Anyone who can reach the listening
  port. There is **no authentication** (see T-HTTP-EoP), so this boundary is
  wide open the instant the server binds a non-loopback address.
- **B4 — engine → remote SPARQL endpoint** (the `SERVICE` federation transport).
  Only present if the non-default `service` cargo feature is compiled in. When
  present it is an outbound boundary with no egress filtering (see
  T-SERVICE-SSRF).
- **B5 — hostile on-disk index file → mmap loader → `unsafe` code.** The
  load-bearing memory-safety boundary: the loader `mmap`s a file and
  reinterprets its bytes as typed values through `from_raw_parts` /
  `from_utf8_unchecked` using offsets and lengths **read out of that same
  (untrusted) file**. This is an *untrusted-input → unsafe-code* boundary, the
  most dangerous class in the system.

---

## Threats (STRIDE) per boundary

Each threat: the mechanism, the **existing mitigation** (cited by test/gate),
and the **GAP** with its bead.

### B5 — hostile on-disk index → unsafe mmap loader

This boundary carries the only memory-safety (rather than wrong-answer or DoS)
risk, so it leads. The canonical count is **42 `unsafe` sites** in `sparq-core`,
across `extsort.rs`, `dict.rs`, `store.rs`, `dictspill.rs`, `lib.rs`, `diskann.rs`,
`main.rs` — produced reproducibly by the count-ratchet
(`python3 scripts/unsafe-gate.py --check`, the `sparq-core` row; `--list` emits
the `file:line` of every counted site). This is the same method enforced by the
`unsafe-register (count ratchet)` CI lane and documented per-site in
[`compliance/memsafety/unsafe-register.md`](../compliance/memsafety/unsafe-register.md);
a raw `grep -rn "unsafe" crates/sparq-core/src` over-counts (it also matches
`// SAFETY:` arguments, the `#![warn(clippy::undocumented_unsafe_blocks)]`
attribute, and prose), so the ratchet number is authoritative. [OPUS-4.8 sq-hday]
The
on-disk store is a *directory* of files (no single `.spq` blob), opened by
`Graph::open` (`crates/sparq-core/src/lib.rs:1105`): permutation files
(`perm0..5.bin`), the dictionary (`dict-meta/terms/offs/hash/hid.bin`), and the
numerics/temporals caches.

**T-MMAP-UB — Tampering / Information disclosure (memory-safety, UB).**
*Mechanism:* `Dict::open_mmap` (`crates/sparq-core/src/dict.rs:1272`) validates
only `dict-meta.bin` (magic `DMV1`, version, `INLINE_BASE`); the four data files
are mapped with **no cross-validation**. `MappedDict::stored`
(`dict.rs:567`) reads an attacker-controlled `u64` offset from `dict-offs.bin`
and slices `self.blob[off..]`; `rd_str` (`dict.rs:380`) then reads an
attacker-controlled `u32` length `n` and calls
`std::str::from_utf8_unchecked(&b[*p..*p+n])` — **with no UTF-8 check**. A
hostile/corrupt store whose blob contains non-UTF-8 bytes therefore produces a
`&str` over invalid bytes, which is **immediate UB** the moment it is used
(format, compare). A bad offset/length instead panics (a DoS, in-bounds by
Rust's slice check).
*Existing mitigation:* the `dict-meta.bin` magic/version/`INLINE_BASE` rejection
(`dict.rs:1279-1305`, tested by the legacy-header rejection test at
`dict.rs:2168-2197`) — but it covers **only the meta file**, not the blob/offset
data. The numerics/temporals caches *are* well-defended: they are mapped only if
`file.len() == dict.len()*8` / `*9` and otherwise recomputed
(`lib.rs:1129,1137`).
*GAP:* no UTF-8 check on the blob, and no validation that `dict-offs.bin` length
`== len*8` or that every offset `< dict-terms.bin.len()`. **Bead sq-znld**
(new): reject non-UTF-8 blob (`from_utf8` not `_unchecked`) + bounds-check
offsets at open time.

**T-MMAP-DoS — Denial of service (panic / OOB-read) on the compressed perm.**
*Mechanism:* `CompressedPerm::from_mmap` (`crates/sparq-core/src/compress.rs:147`)
checks `FILE_MAGIC` (`b"SPQCPRM1"`) and a header-length equality, but (a)
computes `n_blocks*16` and `dir_end+blocks_len` in `usize` with **no overflow
guard** (a hostile `n_blocks`/`blocks_len` near `u64::MAX` wraps so the equality
passes against an undersized file), and (b) never validates per-block byte
offsets against `blocks_len` before `decode_block_at` (`compress.rs:188`), whose
`get_varint` (`compress.rs:38`) reads `buf[*pos]` with no bounds check and uses an
attacker-controlled varint as a loop bound → panic / OOB-read on a corrupt
compressed perm file. The *raw* (default) perm path
(`TripleStore::open`, `store.rs:438`, reinterpret at `store.rs:97-107`) is
length-tolerant (integer-divides by 12) but **semantically unvalidated**: it does
not check that rows are sorted in the permutation order that every
`lower_bound`/`partition_point` consumer assumes, so a tampered raw perm yields
silently wrong results (a Tampering→integrity issue, not UB).
*Existing mitigation:* the compressed-perm header-length equality check
(`compress.rs:147`); the raw path's integer-division keeps `from_raw_parts`
in-bounds. The WAL replay path is separately hardened (magic + checksum +
commit-marker, tested at `lib.rs:4360-4432`), but that is a *different* loader.
*GAP:* `checked_mul`/`checked_add` the header arithmetic; bounds-check directory
offsets and every varint. **Bead sq-ed2i** (new). The raw-perm sortedness gap is
a known integrity assumption of the format (the store is trusted to be produced
by sparq); it is folded into the fuzzing scope below rather than a separate bead.

**T-MMAP-FUZZ — Spoofing / Tampering (no negative-input testing of the loader).**
*Mechanism:* none of the above paths are exercised against corrupt / truncated /
hostile files. There is **no fuzzing of the mmap loader.**
*Existing mitigation:* the only corrupt-file tests in `sparq-core` target the WAL
(`lib.rs:4360-4432`), not `Dict::open_mmap` / `TripleStore::open` /
`CompressedPerm::from_mmap`. The compressed round-trip
(`store.rs:829`) and the `dict-meta` rejection test (`dict.rs:2168-2197`) are
happy-path / meta-only. The only fuzz targets in the repo
(`crates/sparq-bench/src/fuzz.rs`, `crates/sparq-zk-compose/tests/differential_fuzz.rs`)
do **not** touch the mmap loader.
*GAP:* a fuzz target that feeds corrupt/truncated/hostile store files to the
loader and asserts *error-not-UB*. **Bead sq-ky2a** (already exists): "Fuzz the
mmap index loader against hostile .spq files." sq-znld and sq-ed2i are the
specific fixes the fuzzer should drive out.

**T-DICTSPILL — note (build-time, not a load boundary).** The `dict-spill`
feature (`crates/sparq-core/src/dictspill.rs`, behind `SPARQ_DICT_SPILL`) is an
*ingest* path: it spills term records to **its own** temp files and externally
sorts them. Its `unsafe` (libc `sysconf`/`statvfs`, `from_utf8_unchecked` on its
own spill records at `dictspill.rs:206,211`, a parallel scatter `ptr.add().write`
at `dictspill.rs:719`) operates on internally-produced data, **not** on hostile
on-disk index files. Its only untrusted-on-disk exposure is a torn temp file it
wrote this run (a DoS-class panic; the fixed-size framing is guarded by
`read_full`'s `"truncated dict-spill record"` at `dictspill.rs:410`). It is *not*
the B5 attack surface, but sq-ky2a's fuzzing should cover it under
`--features dict-spill` for completeness.

### B1 — untrusted RDF input → parser

Parsing delegates the byte-level grammar to **`oxttl`** (Oxigraph's parser);
`sparq-core` adds a parallel chunking layer on top (entry points in
`crates/sparq-core/src/lib.rs`: `load_str`:612, `load_reader`:804,
`load_reader_parallel`:862; N-Triples parallel at :2301, Turtle parallel at
:2736).

**T-PARSE-CORRECTNESS — Tampering (parser-correctness-as-security: silent wrong
parse / silent over-eager accept).**
*Mechanism:* the parallel chunker could (a) disagree with the serial parse
(producing wrong triples or wrong blank-node scoping across a chunk boundary) or
(b) silently *accept* malformed RDF that the spec rejects (an over-eager split
swallowing a syntax error).
*Existing mitigation — strong, this is a genuinely well-tested boundary:*
- **Chunked-vs-serial differential oracle**: `parallel_turtle_bnodes_match_serial`
  (`lib.rs:3357`) asserts `canon_bnodes(chunked) == canon_bnodes(serial)` over
  pathological docs (shared bnode labels across boundaries, anonymous nests,
  collections, RDF 1.2 triple terms); companions at `lib.rs:3205, 3255, 3488`; N-Triples
  equivalents at `lib.rs:4109` and `lib.rs:4159` (short-read streaming path).
- **Rejection oracles**: `parallel_turtle_rejects_malformed` (`lib.rs:3590`,
  8 malformed inputs, asserts both serial and chunked return `Err` at fan-out
  `[1,2,8,32]`) and `turtle_path_rejection_oracle` (`lib.rs:3652`) — the
  strongest: a **differential against serial `oxttl` on the public entry point**
  (16 malformed inputs; first asserts `oxttl` rejects so the corpus stays honest,
  then asserts sparq rejects; 5 positive controls prove non-vacuity).
- **W3C conformance + CI ratchet**: `crates/sparq-conformance` runs the W3C
  SPARQL/Turtle/TriG accept-reject-eval suites; CI ratchets pass+divergence
  `>= 1229` (`.github/workflows/ci.yml:113-139`) — a drop fails the build. The
  Turtle negative-syntax tests (`turtle_suite.rs:96,123-132`) require rejection.
*GAP:* none of high severity for correctness. The parser is the best-defended
boundary. Two residual notes: (i) the differential fuzzer is **not CI-gated**
(see T-PARSE-FUZZ); (ii) `spargebra` is a *modified vendored fork* (patches in
`vendor/spargebra/SPARQ-PATCHES.md`), so the posture is sparq's, not upstream
oxigraph's — keep the upstream PRs live per the vendoring policy.

**T-PARSE-DoS-BOMB — Denial of service (decompression bomb).**
*Mechanism:* the fused decompress-while-parse ingest (`sparq-cli/src/main.rs:215-228`
sniffs `.bz2`/`.gz`/`.zst` by extension → `MultiBzDecoder`/`MultiGzDecoder`/zstd
`Decoder` → streaming parse) expands a small compressed file into an unbounded
byte stream. A decompression bomb (tiny `.gz` → huge expansion) is not bounded.
*Existing mitigation:* the streaming/pipelined design (`fused-decompress-parse`
skill; `lib.rs:854-860`) means the engine does not *decompress-to-RAM-then-parse*
— it streams, so peak memory is the parse working set, not the full expansion.
That bounds *peak RAM* but **not total CPU/IO or the resulting store size**.
*GAP:* **no decompression-ratio cap and no output-size limit.** The only bound is
an optional, default-off CLI triple-count cap (`max_millions`, `main.rs:208`,
default `u64::MAX`), which limits parsed triples, not decompressed bytes.
**Bead sq-ebii** (already exists) covers "decompression-ratio cap" within the
server DoS policy doc. Note the bomb is **not network-reachable through the
shipped server** (the server does not decompress request bodies — see
T-HTTP-DoS), so this is primarily a CLI / operator-ingest risk today.

**T-PARSE-FUZZ — no continuous parser-robustness fuzzing.**
*Mechanism:* there is no fuzz target that throws random/hostile bytes at the
parser asserting *no panic / no hang*.
*Existing mitigation:* `sparq-bench fuzz` (`crates/sparq-bench/src/fuzz.rs`) is a
**query-semantics** differential against Oxigraph (random small graphs + random
queries; checks result-count / ordering / JSON-binding parity), **not** a
malformed-input parser fuzzer, and it is **not CI-gated** (no `fuzz` hit in
`.github/workflows/*.yml`). It treats a parse error as "unsupported → skip", so
it is only incidentally a panic oracle.
*GAP:* a dedicated parser-robustness fuzz target (hostile bytes → must error, not
panic/hang), ideally CI-gated. This is in-family with the loader fuzzing of
**sq-ky2a**; the parser-input variant is folded there (extend sq-ky2a's scope to
the text parsers) rather than opening a duplicate.

### B2 — untrusted SPARQL string → spargebra → planner → executor

**T-PARSE-DoS — Denial of service (stack-overflow on deeply-nested query).**
*Mechanism:* `spargebra` is a `peg` recursive-descent parser
(`vendor/spargebra/src/parser.rs:10`, grammar from line 1115). Recursive rules
(`SubSelect`, nested `GroupGraphPattern`, `Expression → BrackettedExpression →
Expression`) map nesting depth directly onto **native call-stack depth**. A query
with thousands of nested `(` or `{` recurses unbounded and **overflows the stack
→ process abort**. Reachable from the production entry `PreparedQuery::parse`
(`crates/sparq-engine/src/lib.rs:380`) and hence the unauthenticated `/sparql`
endpoint.
*Mitigation (IMPLEMENTED — sq-v5dg):* the vendored parser now caps syntactic
nesting at `MAX_RECURSION_DEPTH = 128` (`vendor/spargebra/src/parser.rs`):
recursive productions call `enter_recursion`/`leave_recursion` around every
nested delimiter (group graph patterns, sub-SELECTs, bracketed/unary
expressions, property paths, RDF collections, blank-node property lists, RDF-1.2
triple terms), and exceeding the cap returns a clean `TooDeeplyNested` syntax
error (`SparqlSyntaxErrorKind::TooDeeplyNested`) instead of recursing until the
native stack overflows. 128 is ~8× the deepest W3C conformance query yet leaves
~30% headroom under the debug-build overflow point on the smallest (2 MiB tokio
worker / blocking-pool) stack the server parses on. The conformance harness's 20s
parse watchdog (`crates/sparq-conformance/src/run.rs:170-201`) remains as
defence-in-depth but is no longer the only line.
*Regression coverage:* `vendor/spargebra/tests/recursion_depth.rs` exercises every
recursion axis on a 2 MiB stack (overflow there aborts the process, so a passing
run *proves* graceful rejection); `crates/sparq-engine/tests/parse_recursion_depth.rs`
pins the same guarantee at the production seam `PreparedQuery::parse` and at the
default `--max-body-bytes` (1 MiB) request-body envelope (ASVS V5.5.2 / cert gap
ASVS-G4, **bead sq-1ukn**). Upstreaming the cap to `oxigraph/spargebra` per the
vendoring policy is tracked separately.

**T-EXEC-DoS — Denial of service (pathological query: unbounded CPU / memory /
result set).**
*Mechanism:* a syntactically small query can be evaluation-expensive (large
cartesian product, exploding property path, unbounded result set) and exhaust
CPU or memory.
*Existing mitigation:* the streaming executor
(`crates/sparq-engine/src/exec.rs`: streaming single-pattern + 2-pattern-join
paths at :660,:714; LIMIT-without-ORDER-BY early termination "true streaming" at
:1154; streaming group-counts at :1259) bounds memory for those operator shapes —
**but not universally** (sorts and several joins materialize). The real runtime
guard is `QueryBudget` (`crates/sparq-engine/src/lib.rs:51-76`): an optional
wall-clock `deadline` + `max_rows` working-set bound, checked cooperatively at
operator boundaries (`exec.rs:908`). At the server it is wired to a 30s default
timeout and an optional row cap (see T-HTTP-DoS).
*GAP:* `QueryBudget` is **opt-in** — every non-budgeted entry point uses
`QueryBudget::unlimited()` (`lib.rs:73`), so library callers get no guard by
default; and there is **no query-complexity / cost-based rejection** anywhere (a
query is bounded only by time and the optional row cap, never rejected up-front
for cost). Tracked under **sq-ebii** (server-side timeout/memory caps) for the
server, with the cost-bound noted as a residual recommendation below.

### B3 — untrusted HTTP client → server

Routes (`crates/sparq-server/src/http.rs:480-501`): `/sparql` (GET/HEAD/POST —
query, and `application/sparql-update` mutation), `/sparql/graph` &
`/graphs/*` (Graph Store Protocol; writes return 501), `/subscriptions`
(WebSocket), `/health`, `/metrics`.

**T-HTTP-EoP — Elevation of privilege / Information disclosure (open read+write
without auth).**
*Mechanism:* by default there is **no authentication on any endpoint** —
including the mutating `application/sparql-update` path and the `/subscriptions`
WebSocket. With no token configured, anyone who can reach the port can **read and
mutate the entire dataset** (breaching dataset confidentiality *and* integrity).
The binary defaults to `127.0.0.1:3030`.
*Existing mitigation (sq-zcby — RESOLVED for the write surface):* a **required
Bearer-token gate on the write surface** — `--auth-token <TOKEN>` (env
`SPARQ_AUTH_TOKEN`) requires `Authorization: Bearer <TOKEN>` on every request that
MUTATES the dataset (the `application/sparql-update` path, an update smuggled
through the query path — classification keys on whether the request mutates, not
the route — and the GSP `PUT`/`POST`/`DELETE` methods), `401` otherwise (token
compared in constant time; mirrors QLever's `-a`). `--auth-token-read` additionally
gates reads. Implemented in `crates/sparq-server/src/http.rs` (`auth_gate` /
`constant_time_eq` / `payload_mutates`) and wired into the `router` handlers, so
embedders get the gate too. The bind posture (sq-o4qf) now refuses a non-loopback
bind unless `--allow-remote` is set OR the whole surface is authenticated
(`--auth-token` AND `--auth-token-read`) — a write-token alone still leaves reads
open, so it is not sufficient on its own; even an allowed remote bind warns. Update
*write* via `LOAD` is constrained separately (only `file://`, default-disabled
unless `with_load_base` installs an allowlisted, canonicalized, containment-checked
base dir — `crates/sparq-engine/src/update.rs`; the server never calls
`with_load_base`, so `file://` LOAD always fails), so SSRF via `LOAD` is *not*
reachable.
*RESIDUAL GAP:* the gate is a single shared secret (no per-user identity / scopes
/ TLS of its own — deliver it over TLS via a proxy, and use a real authz layer
such as `sparq-solid` for per-user authz), and the `/subscriptions` WebSocket
(a read surface) is **not** gated by the token yet — **bead sq-cxk5** tracks
gating it. Beads: **sq-zcby** (write gate, done), **sq-o4qf** (bind posture),
**sq-cxk5** (subscriptions gate, open), **sq-ebii** (deploy-policy doc).

**T-HTTP-DoS — Denial of service (request flood / pathological query).**
*Mechanism:* expensive queries, large bodies, connection floods.
*Existing mitigation — decent and worth crediting:*
- **Query timeout**: default 30s (`http.rs:111`), a cooperative `QueryBudget`
  deadline plus a hard await-cap (`http.rs:915-937`); exceed → 503.
- **Body-size cap**: default 1 MiB (`DefaultBodyLimit`, `http.rs:538`); over → 413.
- **Concurrency limit**: default 32 (`tower` `load_shed().concurrency_limit`,
  `http.rs:533-534`); excess → 429. Subscription caps 16/conn, 256 global.
- **Result-row cap**: opt-in `max_results` (`http.rs:113,918`); exceed → honest
  413, never silent truncation. (Not applied to ASK.)
- Panic→500 via `CatchPanicLayer` (`http.rs:521-523`).
- The server does **not** decompress request bodies (`tower-http` is built with
  only `trace`+`catch-panic`; no `RequestDecompressionLayer`), so the
  decompression bomb (T-PARSE-DoS-BOMB) is **not network-reachable** here.
*GAP:* (i) **no rate limit** — a flood of distinct expensive-but-under-timeout
queries within the concurrency window is unthrottled; (ii) `max_results`
**defaults to unlimited**; (iii) **no query-complexity bound**; (iv) the parser
stack-overflow (T-PARSE-DoS) was reachable here — now bounded by the
`MAX_RECURSION_DEPTH` cap (sq-v5dg), with the engine-seam / body-cap assertion
under sq-1ukn. Items (i)–(iii) → **sq-ebii** + **sq-o4qf**; (iv) → **sq-v5dg**
(done) + **sq-1ukn** (cert assertion).

**T-HTTP-INFO — Information disclosure (error messages).**
*Mechanism:* an error response leaking filesystem paths, stack traces, or
internals.
*Existing mitigation:* every error is structured JSON `{"error":"..."}` with
control-char escaping (`json_error`, `http.rs:1227-1247`); malformed query → 400
with the spargebra parse message (describes the *query*, not the FS); engine
errors mapped in `engine_error_response` (`http.rs:983-995`). Sampling the engine
error strings (`exec.rs`): they are SPARQL-semantic
(`unsupported graph pattern: {…:?}`, `unsupported SPARQL function: {…:?}`) and do
**not** contain server FS paths or Rust backtraces.
*GAP — low severity:* a 500 reflects internal engine error prose and a
`{:?}`-debug-formatted algebra node back to the client (engine-internals
disclosure, not host/FS disclosure). Worth a redaction pass but low risk. Folded
into **sq-ebii** as a minor item (no separate bead).

### B4 — engine → remote SPARQL endpoint (SERVICE federation)

**T-SERVICE-SSRF — Information disclosure / EoP (SSRF to internal endpoints).**
*Mechanism:* a `SERVICE <http://169.254.169.254/…>` (or `http://127.0.0.1`,
RFC1918) clause makes the server fetch an attacker-chosen internal URL — classic
SSRF into cloud metadata / internal services. Combined with the unauthenticated
server (T-HTTP-EoP), this would be an open SSRF primitive.
*Existing mitigation — strong by default:* `SERVICE` is **OFF in the shipped
server**. It is gated behind the non-default `service` cargo feature
(`crates/sparq-engine/src/exec.rs:1777-1787`; transport
`crates/sparq-engine/src/service.rs:206-232` is `#[cfg(feature="service")]`;
feature OFF by default in `sparq-engine/Cargo.toml`). `sparq-server` pulls the
engine with no features, so `ureq` is absent from its dependency tree (verified
via `cargo tree`); a `SERVICE` clause is *rejected* as an unsupported pattern,
not executed. Variable (non-constant) `SERVICE` endpoints are unsupported even
when the feature is on (`exec.rs:1823-1832`).
*GAP:* **if** a deployer enables the `service` feature, `HttpTransport::fetch`
(`service.rs:208-231`) POSTs to the endpoint IRI **verbatim** with *zero* SSRF
protection (no block on loopback / RFC1918 / link-local / `169.254.169.254`),
only a 30s timeout. **Bead sq-2v6f** (new): default-deny private/loopback/
link-local/metadata ranges via a config allowlist/denylist, documented in the
SKILL. (sq-ebii tracks the broader server SSRF-policy doc; sq-2v6f is the
engine-side egress guard.)

### sparq-reason / sparq-shacl

Both are **pure-safe Rust** (`grep "unsafe"` → 0 in
`crates/sparq-reason/src` and `crates/sparq-shacl/src`), so they carry no
memory-safety asset. Their risk surface is correctness and DoS only.

**T-REASON-CORRECTNESS — Tampering (unsound inference → wrong results).**
*Existing mitigation:* the inference-conformance ratchet (RDFS / OWL-RL,
`inference-conformance-report.md` + `.github/workflows/ci.yml`, premise →
`sparq_reason::materialize_*` → blank-node-homomorphism entailment check) plus
the documented-divergence accounting (every test in exactly one bucket, no silent
skips) and the incremental==batch property tests noted in the AGENTS.md
re-evaluation table. *GAP:* none of high severity; soundness is gated.

**T-REASON-DoS — Denial of service (closure explosion).** RDFS/OWL-RL
materialization can blow up on adversarial ontologies (the materializer is run to
fixpoint). *Existing mitigation:* the RL/RDF rules are *deliberately incomplete*
for arbitrary TBox conclusions (documented theorem PR1 in the inference
conformance doc), which bounds some explosions; but there is no explicit
materialization budget. *GAP — residual:* a materialization size/time budget for
untrusted ontologies. This is only reachable if untrusted ontologies are loaded
and reasoned over (not a default server path — the server does not reason). Folded
into **sq-ebii** as a residual recommendation (no separate bead; low priority
until reasoning is exposed to untrusted input).

**T-SHACL-DoS — Denial of service.** SHACL `sh:pattern` regexes and SPARQL-based
constraints could be expensive. *Existing mitigation:* the W3C SHACL conformance
ratchet (core ≥98, sparql ≥5, `ci.yml:147-183`) gates correctness; the engine's
`regex` is the standard Rust `regex` crate (no catastrophic backtracking by
construction). *GAP — residual:* no per-validation budget; same disposition as
T-REASON-DoS.

---

## Residual risks & recommendations

Ordered by severity. Every gap maps to a bead (existing or new).

| # | Residual risk | Severity | Boundary | Recommendation | Bead |
|---|---|---|---|---|---|
| 1 | Non-UTF-8 dictionary blob → `from_utf8_unchecked` → **UB** on a hostile/corrupt store | **Critical** (UB) | B5 | Replace `from_utf8_unchecked` with checked `from_utf8`; validate `dict-offs.bin` length & every offset at open | **sq-znld** (new, P1) |
| 2 | mmap loader unfuzzed against hostile/truncated files (the whole B5 surface incl. dict-spill & raw-perm sortedness, plus parser-robustness) | **High** | B5 / B1 | Add a fuzz target: corrupt/truncated/hostile store + hostile RDF/SPARQL bytes → must error, never UB/panic-loop; run under `--features dict-spill` | **sq-ky2a** (exists, P2) |
| 3 | Compressed-perm header arithmetic overflow + unchecked block offsets/varints → panic / OOB-read | **High** | B5 | `checked_mul`/`checked_add` header math; bounds-check directory offsets and varint reads | **sq-ed2i** (new, P2) |
| 4 | SPARQL parser stack-overflow on deeply-nested query → process abort | **High** → mitigated | B2 | Recursion-depth cap (`MAX_RECURSION_DEPTH = 128`) at the parse productions returning a clean `TooDeeplyNested` error; engine-seam + 1 MiB body-cap regression assertion; upstream the cap | **sq-v5dg** (cap, done) + **sq-1ukn** (cert assertion, done) |
| 5 | Server auth: write surface now has a Bearer-token gate (`--auth-token`, sq-zcby) + bind-posture refusal (sq-o4qf); RESIDUAL — `/subscriptions` WS not yet token-gated, no per-user authz, `max-results` unlimited, no rate limit / query-complexity bound | **Medium** (was High) | B3 / B2 | Gate the subscriptions WS (sq-cxk5); default-on rate limit + sensible `max-results`; document gateway expectation | **sq-zcby** (write gate, done) + **sq-o4qf** (bind, done) + **sq-cxk5** (WS, new) + **sq-ebii** (exists, P2) |
| 6 | No documented/enforced server timeout-memory-decompression-SSRF policy; minor 500-error engine-internal disclosure; reason/SHACL materialization budgets | **Medium** | B3 / B1 | Write+enforce the DoS/SSRF policy doc; redact `{:?}` algebra from 500s; add reasoning/validation budgets when exposed to untrusted input | **sq-ebii** (exists, P2) |
| 7 | SERVICE federation has zero SSRF egress filtering **if** the `service` feature is enabled | **Medium** (gated off by default) | B4 | Default-deny loopback/RFC1918/link-local/metadata; config allowlist; document the sharp edge | **sq-2v6f** (new, P2) |
| 8 | Decompression bomb on CLI/operator ingest (not network-reachable in the shipped server) | **Medium** | B1 | Decompression-ratio + output-size cap on the fused-decompress ingest | **sq-ebii** (exists, P2) |

### Cross-references

- **ZK estate** (`sparq-zk*`): see `research/zk-soundness-audit.md` — **v1
  verifier soundness is BROKEN**; do not present it as proving anything to a
  relying party.
- **Research scaffolds** (`sparq-mpc` / `sparq-gpu`): deferred, beads **sq-jskw** /
  **sq-vrye**. The `sparq-gpu` deferral is written down and mechanically enforced —
  `research/gpu-threat-model-deferral.md` states why deferring is defensible today,
  what ends the deferral, and what the model must cover when it does. Read its §3
  before quoting the deferral: two exposures (an unaudited `wgpu`/`naga` build-graph
  subtree, and kernel tests that skip-pass without a GPU) are **not** deferred by it.
  <!-- [OPUS-5] sq-vrye -->
- **`sparq-nlq`** (NL→SPARQL): no longer deferred — see
  `research/nlq-threat-model.md` (bead **sq-j1wv**), which models prompt/data
  injection, `SERVICE` exfiltration, and the token budget. Injection itself is
  contained on *consequences*, not prevented; read that document's posture summary
  before quoting it.
- This model is parented under the production-readiness program (bead **sq-bqjv**:
  SBOM, supply-chain, threat model).

### Posture summary (non-sycophantic)

- The **parser correctness** boundary (B1) is genuinely well-defended:
  chunked-vs-serial differential + rejection oracles + a ratcheted W3C
  conformance suite. Credit where due.
- The **server DoS** posture (B3) is decent (timeout, body cap, concurrency
  shed, opt-in row cap) but **incomplete** (no rate limit, unlimited results by
  default, no cost bound).
- The two **sharpest edges** are: (a) the **memory-safety** boundary B5 — an
  unfuzzed unsafe loader with at least one outright soundness hole
  (`from_utf8_unchecked`, sq-znld); and (b) the **unauthenticated server** B3 —
  open read+write the instant it binds a non-loopback address (sq-o4qf). Neither
  is "mitigated"; both are tracked.
- SERVICE-SSRF (B4) is the one place where the *default* (feature off) is the
  mitigation — fine today, a trap for a deployer who flips the feature without
  reading this (sq-2v6f).
