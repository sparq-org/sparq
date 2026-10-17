<!-- 🤖 SPARQ agent — design record for the versioned durable store. Not implemented:
     this is the design the #7115 review loop converged on after five rounds of
     swap- and alias-class findings against the in-place-swap layout. Supersedes
     that layout. Keep neutral; no measured numbers (work-box timings are
     non-canonical and belong in a PR body, not here). -->

# Versioned durable store (design record)

Status: **design, not implemented.** `sparq-core`'s durable store currently publishes a
new base by renaming directories in place. Five review rounds against
`fix/core-journal-completeness` (PR #7115) found successive swap- and alias-class defects
in that scheme; each fix was local and sound, and the next round found another. The
pattern is the layout, not the patches, so the layout is replaced.

**The rule the whole design follows:** the store root is created once and never renamed,
every published base is immutable, all coordination state lives on stable inodes inside
that root, and the only mutation that changes which bytes are canonical is one rename of
a pointer file.

## 1. On-disk shape

```text
R/                        the store root; created once, never renamed
  CURRENT                 which version is live
  LOCK                    the writer flock
  pin/v7                  pin file for v7 (see §7)
  v7/                     a published version; immutable once CURRENT names it
    perm0.bin perm1.bin … base indexes
    dict-meta.bin dict-*  dictionary
    numerics.bin temporals-v4.bin
    wal.log               this version's write-ahead log (mutable)
    txn.log               this version's redo journal (mutable)
    named.bin             ownership manifest: the named graphs of THIS version
    named/0/ named/1/ …   one directory per named graph, same shape recursively
  v8/                     a version being built, or garbage from a crash
```

`R` holds no base files of its own. Everything a reader loads sits under one `v<N>`.

**CURRENT.** One line, `v<N>\n`, then a line holding the CRC-32C of the first line in
lowercase hex. A reader that cannot parse both lines, or whose checksum disagrees, treats
CURRENT as unreadable and errors — it never guesses a version. Publishing writes
`R/CURRENT.tmp`, fsyncs it, renames it over `R/CURRENT`, then fsyncs `R`.

`N` is a monotonically increasing decimal, no padding, read from CURRENT. A writer builds
`v<N+1>`; if that directory already exists it is garbage from an earlier crash and is
removed first, under the lock.

**Hardlinked vs copied.** A new version hardlinks every base file it does not change and
writes fresh files for the rest, so publishing costs the changed bytes rather than the
whole store. `wal.log` and `txn.log` are never hardlinked: each version owns its logs,
and a new version starts with empty ones because the previous version's log content has
been folded into the new base. `named.bin` is always written fresh. Children follow the
same rule recursively — an unchanged child is hardlinked file by file, a rebuilt child
gets fresh files. Where `link(2)` is refused the writer copies instead, as a
per-operation fallback, not a store-level mode.

## 2. Coordination state

All of it lives on stable inodes in `R`, and none of it is derived from the path used to
reach `R`:

| State | Where | Survives |
|---|---|---|
| writer lock | `R/LOCK` | yes |
| live version | `R/CURRENT` | yes |
| reader pins | `R/pin/v<N>` | yes, per version |
| writer generation / seqlock counter | — | **no** |

Because `R` is never renamed, a lock file inside it is never moved, never replaced and
never needs releasing for a swap. One path inside `R` is one inode through every alias —
a bind mount, a second mount of the same filesystem, a symlink, a `./`-or-`..` detour —
so path-keyed and inode-keyed exclusion coincide: one store has one writer however it is
named, with one lock file and no sibling control files.

Canonicalisation of `R` is still performed, for symlinks and for a tidy error message,
but nothing depends on it for correctness any more.

**The generation counter does not survive.** It exists today because a committed
transaction is materialised into the live directory graph by graph, so a reader can
observe a partial frame. Here a reader's snapshot is fixed by CURRENT, and a writer's
per-commit work happens inside the version the reader already pinned, so the only
residual question is how a reader treats `txn.log` (§3). "Did the store move on under my
handle" is answered by comparing the version this handle resolved against the version
CURRENT names now — one small read, on the paths that already check staleness. That
replaces the counter; the two are not kept side by side.

## 3. Reader path

1. Canonicalise `R`.
2. Read CURRENT. Take the shared pin for that version (§7), then re-read CURRENT; if it
   moved, drop the pin and retry a bounded number of times. This is a reader's only retry
   loop.
3. Load the base files, `named.bin` and the children of that version.
4. `wal.log`: fully-committed records are folded in; a torn tail is ignored by a reader
   and truncated only by a writer.
5. `txn.log`: a **complete** frame is redone over what was loaded, in memory, using the
   same idempotent helpers the locked replay uses; a torn frame is ignored. A reader
   therefore never depends on how far a writer has got with materialising a frame — which
   is what the seqlock was for — and writes nothing while doing it.

Readers take no exclusive lock, so any number coexist with each other and with a writer.

## 4. Writer paths

**Per-commit, inside the live version.** `apply_delta`, `apply_delta_nquads`, a journal
commit, its materialisation and `clear_txn` append to `v<N>/wal.log` and `v<N>/txn.log`
and mutate nothing else: no version is built, nothing is renamed, no control file is
written, no fsync is added beyond the log fsyncs that exist today.

**Version-building operations** — compaction, vacuum, restore, named-graph creation, and
the renumbering a named-graph drop performs:

1. take the lock (or hold it already);
2. remove any stale `v<N+1>`;
3. build `v<N+1>` **completely**, in this order: ownership markers, then manifests, then
   base files and logs; then fsync the version's directories;
4. publish: write and fsync `CURRENT.tmp`, rename it over `CURRENT`, fsync `R`;
5. switch this handle to the new version;
6. collect `v<N>` when nothing pins it, else leave it for a later locked operation.

Two consequences carry the design:

- **A failure anywhere before step 4 leaves `v<N>` live and every open handle on it
  writable, with its WAL intact.** Nothing about the live version has been touched, so
  there is no state in which a handle holds a lock, believes it may write, and has no log
  to write to.
- **Marker and manifest ordering inside step 3 is a crash-safety property, not a
  write-order convention.** Nothing in `v<N+1>` is reachable until the flip, so a child's
  marker, its manifest entry and its files become visible together or not at all.

Named-graph creation is a version flip for the same reason: a child exists only once a
flip publishes it.

## 5. Recovery

The crash-state space is two states: a garbage `v<N+1>` beside the live version, or
CURRENT naming `v<N+1>` with a stale `v<N>` still present. Both are collected by the next
locked operation; neither needs a probe for staging siblings, a `.compact-old`/
`.compact-new` pair or a staged manifest.

Log recovery (a committed frame to redo, a torn tail to truncate) happens inside a
version, under the lock, and **publishes nothing until replay has fully finished**: the
redone state becomes visible to other processes only through a CURRENT flip, never
through a counter or a partially rewritten live directory. An unlocked open that finds
work it may not do either loads a consistent snapshot by redoing complete frames in
memory (§3) or returns an error naming the store; it never exposes partial state and
never rewrites anything.

## 6. Legacy fallback and migration

A legacy store is a root with base files directly inside it and no CURRENT.

- **An unlocked open reads it as it does today.** Refusing would break every existing
  read-only consumer the moment the binary is upgraded, for a store that is readable.
- **The first locked open migrates it:** build `v0` by hardlinking (or copying) the
  existing base files and children into it, write CURRENT, fsync `R`, then remove the old
  files. One flip, after which the store is indistinguishable from a new one.

Migration runs under the lock and is idempotent: a crash leaves either the legacy layout
plus a garbage `v0`, or a migrated store, and the next locked open finishes the job.

## 7. Pinning and garbage collection

A reader of `v<N>` holds a shared `flock` on `R/pin/v<N>` for as long as its mappings
live, because its `Graph` owns both. A writer collecting `v<N>` first tries an exclusive
non-blocking `flock` on that pin file: refused means a reader still holds the version, so
it stays and a later locked operation retries; granted means the directory and its pin
file can be removed. A pinned version is never collected; an unpinned, non-current
version is collectable by any writer.

**Windows is out of scope** until `DirLock` exists there: `DirLock::acquire` is a no-op on
non-unix today, so durable stores have no writer exclusion on Windows at all, and the code
says so. The pin scheme here is unix-only. A port would need `LockFileEx` on the pin file,
because deleting an open file is refused on that platform — recorded only so the port has
a starting shape; none of it is in this series.

## 8. Invariants

**Structural — the enforcing code goes away.** A reader sees a whole transaction or none
of it (was I7): a reader loads one immutable version and redoes only complete frames. An
abandoned consistency window cannot wedge a store (was I11): there is no window. Recovery
probing covers every swap crash point (was I12): the crash-state space is two states, both
collected. A base replacement never runs under a live writer's feet (was part of I10): a
version is built beside the live one and published by pointer. One store has one lock
however its path is spelled (was I8): the lock is an inode in `R`.

**Still enforced, with new mechanics.** One writer per store (I1), by `R/LOCK`. A handle
whose store moved on is refused (I10), by comparing its resolved version against CURRENT.
A kept journal frame refuses every durable write in the tree until the reopen (I2, I5),
unchanged and still per tree. Graph existence is journalled (I6), unchanged. A named
sub-graph is reachable only through its root (I9): ownership lives in the version's
`named.bin`, and `Graph::open` accepts only a root with CURRENT or a legacy root —
`R/v<N>` and `R/v<N>/named/<i>` are refused.

## 9. Carried over from #7115

Layout-independent work on `fix/core-journal-completeness` that should land first, from
`main`, as **slice 0** — it is what makes the later slices small:

| Piece | Note |
|---|---|
| `TxnOp` create/drop records and the `WTX2` frame format, with `WTX1` frames still replayed | the journal's record vocabulary; nothing about it depends on where the journal file sits |
| `sparq-engine`'s effect commit emitting create/drop ops (`update.rs`) | the producer side of the same vocabulary |
| the shared `TreeState` poison flag (one flag per tree, named graphs included) | per-tree state, not per-directory |
| `begin_durable_write` on every durable path, `clear_txn` included | the gate stays; its identity check becomes the version comparison (§2) |
| `OpenCtx` and the read-only, non-repairing open | an unlocked open must still rewrite nothing; §3 and §5 keep that distinction |
| `refuse_child_open` / ownership markers | the mechanism carries; the authority moves into the version manifest (§8) |
| the test suites: crash points, invariants, mutation tests | port per §10 |

**Discarded with the old layout:** the sibling control files (`.<dir>.writer.lock`,
`.<dir>.writer.gen`); the writer generation and its seqlock, `SeqWindow` and the nested
window depth; the second inode-keyed inner lock and its re-locking after a swap; the
inode identity check; `swap_dir_to_new_base` and the two-rename protocol; the
`.compact-new` / `.compact-old` and `named.drop-new` / `named.drop-old` siblings; the
staged manifest and its promotion; `recover_compaction` and `recover_named_drop`; the
swap-sibling recovery probe. Canonicalisation stays, for symlinks to `R` only.

## 10. Findings from rounds 1–5

Every finding the five review rounds raised, against the property that rules it out or
the test that must pin it. Rounds 1 and 2 are reconstructed from the invariant lists in
the commits that fixed them (I1–I10); rounds 3 to 5 are as raised. "Construction" rows
need no test to stay true; test-only rows are the ones a future change could break
silently, so each is named.

| # | Finding | Disposition |
|---|---|---|
| 1.1 | Two handles or processes could both write one directory | **Construction**: one lock on a stable inode in `R` |
| 1.2 | The lock lived inside the directory, so a rename moved it and exclusion lapsed | **Construction**: `R` is never renamed |
| 1.3 | A handle that loaded before another writer's changes kept writing | **Test**: a handle whose resolved version is no longer CURRENT is refused on its next durable write |
| 1.4 | `clear_txn` truncated the journal with no durable-write gate | **Test**: a journal clear is refused on a poisoned tree, by a non-holder of the lock, and by a stale handle |
| 1.5 | A poisoned tree's named graphs kept accepting writes | **Test**: every durable mutator of every graph in the tree is refused while a frame is retained |
| 1.6 | Graph create/drop was not journalled, so a crash recovered a different graph set | **Test**: a committed create/drop frame is redone in commit order on reopen (slice 0) |
| 2.1 | A reader could load a half-materialised multi-graph transaction | **Construction**: the reader's snapshot is CURRENT plus complete frames only |
| 2.2 | Two path spellings of one store took two locks | **Construction**: the lock is an inode in `R` |
| 2.3 | A named sub-graph directory could be opened as a store of its own | **Test**: opening `R/v<N>/named/<i>` is `InvalidInput`; ownership is in the version manifest |
| 2.4 | A restore ran without the store's lock, and a swapped-out handle kept writing | **Construction**: a restore is a version build under the lock, and the old version stays intact |
| 3.1 | Directory swaps bypassed the reader-consistency window | **Construction**: a version is invisible until the flip |
| 3.2 | An abandoned odd counter wedged every later open | **Construction**: no counter |
| 3.3 | The recovery probe stopped at the first absent child position | **Construction**: no staging siblings to probe |
| 3.4 | A child swap overwrote the tree's shared root identity | **Construction**: no per-directory identity; one CURRENT per store |
| 4.a | A failed swap left a handle holding the lock with no WAL | **Construction**: a failure before the flip leaves the live version and its WAL untouched |
| 4.b | A child became reachable before it carried its ownership marker | **Construction**: nothing in a version is reachable before the flip |
| 4.c | The drop backstop published a quiescent counter over a retained frame | **Construction**: no counter; a retained frame is visible in `txn.log` |
| 4.d | Bind-mount aliases derived different sibling locks | **Construction**: lock inode in `R` |
| 5.1 | Failed named-drop staging left survivors accepting unlogged writes | **Construction**: the renumbering is a version build; a failure keeps the live version and its logs |
| 5.2 | A child swap replaced the root's inner lock | **Construction**: one lock, on `R`, never replaced |
| 5.3 | Aliases kept independent generation counters | **Construction**: no counter, and CURRENT is an inode in `R` |
| 5.4 | Replacement directories were published before their lock was held, and recovery with the root absent never took it | **Construction**: `R` cannot be absent, and publishing is a rename of a file inside it under the lock |
| 5.5 | Recovery published a generation mid-replay | **Construction**: recovery publishes only by flip, after replay finishes |
| 5.6 | Markers were not fsynced before manifests | **Construction**: the whole version is fsynced before the flip (§4, step 3) |

Four rows need tests. Three of them (1.3, 1.4, 1.5) are the durable-write gate, which
survives the layout change; the fourth (2.3) is the child-open refusal. 1.6 is slice-0
behaviour with its own test already written.

## 11. Crash-point tests to port

Every test that stages a crash by touching `.compact-new`, `.compact-old`,
`named.drop-new`, `named.drop-old` or a staged manifest restages as "a garbage `v<N+1>`
exists" or "CURRENT names `v<N+1>` and `v<N>` still exists". Specifically:

- compaction-swap, named-drop and mid-compaction-child recovery tests become those two
  shapes: crash before the flip, crash after it;
- WAL and journal torn-tail tests port unchanged — they are about log bytes inside a
  version;
- writer-lock tests port with the lock path changed to `R/LOCK`;
- the bind-mount alias test ports and becomes the direct test of §2 (it skips where
  `mount --bind` is unavailable);
- "a failed swap leaves a handle that refuses writes" inverts into "a failed version build
  leaves a handle that keeps writing to the live version";
- the reader-consistency tests (busy under a half-materialised transaction) become "a
  reader loading while a frame is committed sees the whole frame, or none of it";
- `tests/mmap_corruption_oracle.rs` points at a version directory; its subject is file
  bytes, not the layout.

## 12. Slices

One PR each, green in both feature states, with its own tests.

**Slice 0 — layout-independent carry-over (from `main`).** The §9 table: the `WTX2`
record vocabulary with `WTX1` replay, the engine's create/drop emission, the shared
poison flag, `begin_durable_write` on every durable path including `clear_txn`, `OpenCtx`
and the non-repairing open, the child-open refusal. *Acceptance:* a committed create/drop
frame is redone in commit order on reopen; a `WTX1` frame still replays; every durable
mutator in a tree is refused while a frame is retained; a journal clear is refused by a
non-holder; an unlocked open rewrites nothing.

**Slice 1 — read path.** The CURRENT format with its reader and writer, version
resolution, `Graph::open` accepting a versioned root and a legacy root, refusal of a path
inside a version. Writers still write flat, so the versioned reader is exercised by
hand-built fixtures. *Acceptance:* a hand-built `R/v0` plus CURRENT opens and reads; a
truncated or checksum-mismatched CURRENT is a clear error; a legacy root still opens;
opening `R/v0` is `InvalidInput`.

**Slice 2 — writers.** Version building and the flip for compaction, vacuum, restore,
named creation and named-drop renumbering; legacy migration on first locked open;
hardlink with copy fallback. *Acceptance:* each operation publishes a new version and
leaves the old one intact; a failure injected before the flip leaves the handle writing
to the live version with its WAL intact; crashes simulated before and after the flip
recover to old-or-new; migration is idempotent.

**Slice 3 — pinning and GC.** Pin files, the shared/exclusive protocol, collection under
the lock, collection of garbage versions on a locked open. *Acceptance:* a pinned version
is not collected and its reader keeps reading; once the reader drops, the next locked
operation collects it; a garbage `v<N+1>` from a crash is collected.

**Slice 4 — deletions.** Remove everything in §9's discard list and rewrite the invariant
table. *Acceptance:* the ported crash-point tests pass; no `.compact-*` or `writer.gen`
path remains in the crate; the per-commit path writes only logs.

Slices 0 to 3 keep the old machinery alive beside the new, so each lands independently.
Slice 4 should not be split further: deleting the old recovery paths and rewriting the
tests that cover them belong in one change.

## 13. Performance constraints

Learned from measuring #7115's commit path. No figures here by design — work-box timings
are not canonical — but the constraints are binding:

- **The per-commit path adds no fsync, no `openat` and no control-file write.** It touches
  `wal.log` and `txn.log` only. The commit path is fsync-bound, so anything else added
  there is both unnecessary and hard to measure honestly.
- **Derive no control path per write.** The helper that derived control paths
  re-canonicalised the store directory on every call, which costs a `readlink` per path
  component. Resolve `R`, CURRENT and the pin path once per handle and cache them.
- **Keep a handle open for any file written more than once per operation**, rather than
  re-opening it per transaction.
- **Do a per-operation check, not a per-nested-gate check.** The version comparison that
  replaces the identity check runs once per public durable operation; the old check ran
  once per gate, and one transaction passes several.
- **When measuring a slice**, use interleaved pinned pairs, an A/A control of the baseline
  against itself, and a bootstrap confidence interval on the median ratio; report a result
  only against its A/A band.

## 14. Open questions

- **Journal scope.** `txn.log` is per version here, which is right for a frame that
  describes changes within a version, but a frame committed immediately before a flip has
  to be either materialised into the new version during the build or carried across. The
  simplest rule — refuse a version build while a frame is retained, which is today's
  behaviour — is assumed above; it should be confirmed before slice 2.
- **Child versioning.** This design versions the store as a whole. Versioning each child
  independently would make a single-graph compaction cheaper, at the cost of a manifest
  that names a version per child. Not proposed; recorded as the obvious extension.
- **Pin-file lifetime.** `R/pin/` accumulates one file per version ever published unless
  collection removes them with their version; the GC in slice 3 must remove both, and a
  pin file with no version directory should be treated as collectable garbage.
