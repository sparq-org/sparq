<!-- 🤖 SPARQ agent — design record for the versioned durable store. Not implemented:
     this is the design the #7115 review loop converged on after five rounds of
     swap- and alias-class findings against the in-place-swap layout, and revised
     after a round-D design audit of the record itself (§10). Supersedes
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
    wal.log               this version's write-ahead log (append-only)
    txn.log               the tree's central commit log (append-only)
    IDENTITY              which store and version these bytes belong to
    named.bin             ownership manifest: the named graphs of THIS version
    named/0/ named/1/ …   one directory per named graph, same shape recursively
  v8/                     a version being built, or garbage from a crash
```

`R` holds no base files of its own. Everything a reader loads sits under one `v<N>`. A
child directory holds its own `wal.log` and **no** `txn.log`: the version's single
`txn.log` is the commit-ordering authority for the whole tree, children included.

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

**IDENTITY.** Every version directory and every descendant of one carries an `IDENTITY`
file naming the store's id (a random value minted when `R` is created) and the version it
belongs to. It is what makes ownership persistent rather than inferred: `Graph::open`
refuses a directory whose `IDENTITY` says it is part of a version or a child of one, so a
named sub-graph cannot be opened as a store of its own through any alias, and a directory
moved out of a store is still recognisable as not being a root. Parent-path inspection is
not relied on (finding 2.3).

**Append-only logs, and the commit position.** Within a published version, `wal.log` and
`txn.log` are **append-only**: neither is cleared or rewritten in place while its version
is live. `clear_txn` therefore appends a *retire* record naming the commit it consumes
instead of truncating; log bytes are reclaimed only by building a new version, whose logs
start empty. The one in-place shortening still permitted is the removal of an **incomplete
trailing record** under the writer lock — bytes no reader ever folded, because an
incomplete record is below every reader's boundary by definition.

Each frame appended to `txn.log` carries a monotonically increasing **commit sequence**,
and every `wal.log` record written while materialising a frame carries that frame's
sequence; a plain single-graph write (`apply_delta` outside a frame) carries none, since
its atomicity is one record in one log. That gives two derived values:

- **The reader's boundary** (§3) is the durable length of `txn.log` at the moment the
  reader starts loading, and the highest commit sequence completely present within it.
- **The writer revision** (§4) is the commit position: that same `txn.log` length together
  with each log's length in the tree. It is read by statting the logs, so it costs nothing
  on the per-write path and one stat per log per locked operation.

The commit position is deliberately distinct from the version number: ordinary commits
advance it without publishing a version, which is exactly the case a version comparison
alone misses (finding 1.3 / D2).

## 2. Coordination state

All of it lives on stable inodes in `R`, and none of it is derived from the path used to
reach `R`:

| State | Where | Survives |
|---|---|---|
| writer lock | `R/LOCK` | yes |
| live version | `R/CURRENT` | yes |
| reader pins | `R/pin/v<N>` | yes, per version |
| store and version identity | `R/v<N>/IDENTITY` and each descendant's | yes, per directory |
| commit position | log lengths inside `R/v<N>` | yes |
| writer generation / seqlock counter | — | **no** |

Because `R` is never renamed, a lock file inside it is never moved, never replaced and
never needs releasing for a swap. One path inside `R` is one inode through every alias —
a bind mount, a second mount of the same filesystem, a symlink, a `./`-or-`..` detour —
so path-keyed and inode-keyed exclusion coincide: one store has one writer however it is
named, with one lock file and no sibling control files.

Canonicalisation of `R` is still performed, for symlinks and for a tidy error message,
but nothing depends on it for correctness any more.

**Filesystem contract.** The durable store requires a local POSIX filesystem with working
`flock` and atomic `rename` within a directory. Known network filesystems are **refused at
durable open** by inspecting `statfs`'s `f_type` (NFS and SMB/CIFS at least), because
neither their locking nor their rename and `fsync` visibility can be relied on for the
exclusion and publication this design rests on. The refusal is explicit and documented
rather than silent degradation.

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
3. **Capture the boundary**: read `v<N>/txn.log` once and record its durable length and
   the highest commit sequence complete within it. This happens *before* any base file or
   WAL is loaded, and it is the snapshot — not CURRENT alone (D1).
4. Load the base files, `named.bin`, `IDENTITY` and the children of that version.
5. `wal.log`, per graph: a record carrying no commit sequence, or a sequence at or below
   the boundary, is folded in; a record carrying a **higher** sequence belongs to a
   transaction the reader is not reading and is skipped, as is a torn tail. Because logs
   are append-only within a version (§1), nothing the reader decided to fold can have been
   removed underneath it.
6. `txn.log` up to the boundary: a **complete**, non-retired frame is redone over what was
   loaded, in memory, with the same idempotent helpers the locked replay uses. A frame may
   already be materialised in the WALs, wholly or partly; redo is idempotent, so the result
   is the same either way. A frame beyond the boundary, a torn frame and a frame a retire
   record consumed at or below the boundary are all ignored.

Steps 3, 5 and 6 together are the snapshot: every multi-graph transaction at or below the
boundary is read whole, and every transaction above it is read not at all, whatever a
writer does during the load. That is what the seqlock was for, and no reader writes
anything to get it.

Readers take no exclusive lock, so any number coexist with each other and with a writer.

## 4. Writer paths

**Taking the lock.** A writer that has just acquired `R/LOCK` does three things before it
accepts a write or collects anything:

- **Make the observed CURRENT durable.** The previous writer may have renamed CURRENT and
  exited before fsyncing `R`, so a later power failure could restore the old pointer under
  acknowledged log writes. The new writer fsyncs `R` (rewriting CURRENT durably if it
  cannot) under the lock (D6). A failure here is fail-closed: the handle refuses durable
  writes and says the store's pointer could not be made durable.
- **Revalidate.** It re-reads CURRENT and the commit position (§1). If either differs from
  what this handle loaded, the handle **reloads the tree under the lock** before its first
  write; a reload that fails is likewise fail-closed. A version comparison alone is not
  enough, because ordinary commits move the commit position without moving CURRENT (D2).
- **Finish any retained frame** (§5) before starting unrelated work.

Compaction and every other version build therefore build only from state loaded or
revalidated under the lock.

**Per-commit, inside the live version.** `apply_delta`, `apply_delta_nquads`, a journal
commit, its materialisation and `clear_txn` **append** to `v<N>/wal.log` and
`v<N>/txn.log` and mutate nothing else: no version is built, nothing is renamed, no
control file is written, no fsync is added beyond the log fsyncs that exist today.

**Version-building operations** — compaction, vacuum, restore, named-graph creation, and
the renumbering a named-graph drop performs:

1. take the lock (or hold it already), with the three steps above;
2. remove any stale `v<N+1>`;
3. build `v<N+1>` **completely**, in this order: `IDENTITY` and ownership markers, then
   manifests, then base files and logs;
4. make it durable, bottom-up: **fsync every file's contents** — written and copied alike
   — then fsync the version's directories from the deepest child up to `v<N+1>`. The save
   helpers today flush without syncing, which is not enough: a durable CURRENT flip must
   not be able to name incomplete indexes, dictionaries or markers (D3);
5. publish: write and fsync `CURRENT.tmp`, rename it over `CURRENT`, fsync `R`;
6. switch this handle to the new version, and reset its commit position to the new
   version's empty logs;
7. collect `v<N>` when nothing pins it, else leave it for a later locked operation.

A failure in step 5 is fail-closed: the handle refuses further durable writes and reports
that publication failed, leaving `v<N>` live and `v<N+1>` as collectable garbage.

Two consequences carry the design:

- **A failure anywhere before step 4 leaves `v<N>` live and every open handle on it
  writable, with its WAL intact.** Nothing about the live version has been touched, so
  there is no state in which a handle holds a lock, believes it may write, and has no log
  to write to.
- **Marker and manifest ordering inside step 3 is a crash-safety property, not a
  write-order convention.** Nothing in `v<N+1>` is reachable until the flip, so a child's
  marker, its manifest entry and its files become visible together or not at all — but
  only because step 4 syncs contents before step 5 publishes them.

Named-graph creation is a version flip for the same reason: a child exists only once a
flip publishes it.

**Structural transactions.** A committed frame may hold CREATE and DROP ops (slice 0's
`TxnOp`) beside triple deltas, and graph existence is a layout change, so materialising
such a frame is a version build. The protocol is one build and one flip for the **whole
frame** (D4):

1. under the lock, with the frame complete in `txn.log`, build `v<N+1>` by applying every
   op of the frame in commit order — creates and drops as directory structure and
   manifest entries of the version being built, deltas into its logs or base files;
2. make `v<N+1>` durable (step 4 above) and publish it with **one** flip. No op of the
   frame is ever published on its own, so there is no state in which half a frame is live;
3. only then append the retire record for that commit (§1).

This refines the "refuse builds while a frame is retained" rule rather than contradicting
it: while a frame is retained the **only** build permitted is the one materialising that
frame. Unrelated builds — a user-called compaction, a vacuum — are still refused, and the
poison state still refuses unrelated durable writes (I2, I5). Recovery uses exactly the
same three steps (§5), which is why "publish only after replay finishes" follows from the
protocol instead of being an extra rule.

## 5. Recovery

The crash-state space is two states: a garbage `v<N+1>` beside the live version, or
CURRENT naming `v<N+1>` with a stale `v<N>` still present. Both are collected by the next
locked operation; neither needs a probe for staging siblings, a `.compact-old`/
`.compact-new` pair or a staged manifest.

Log recovery happens inside a version, under the lock. A torn trailing record is dropped
in place (§1); a retained complete frame is completed by the structural-transaction
protocol — replay the whole frame into an unpublished `v<N+1>`, one flip, then retire the
commit — whether or not it contains CREATE/DROP ops. So **nothing is published until
replay has finished**, and that is a consequence of the protocol rather than an extra
rule: a crash mid-recovery leaves either `v<N>` live with the frame still retained, or
`v<N+1>` live with the frame materialised, and the next locked open retries from
whichever it finds. The retired-commit record is appended only after the flip is durable,
so a crash between them replays an already-materialised frame — idempotently.

An unlocked open that finds work it may not do loads a consistent snapshot by redoing
complete frames at or below its boundary in memory (§3); it never exposes partial state
and never rewrites anything.

## 6. Legacy fallback and migration

A legacy store is a root with base files directly inside it and no CURRENT.

- **The first locked open migrates it, non-destructively until the flip is durable:**
  build `v0` beside the legacy files by hardlinking (or copying) the existing base files
  and children into it, **folding in every legacy WAL — the root's and every child's — and
  every complete committed journal frame**, so nothing acknowledged is left behind in a
  file about to be deleted; make `v0` durable (§4 step 4); flip CURRENT and fsync `R`.
  **Only then** delete the legacy files. One flip, after which the store is
  indistinguishable from a new one.
- **An unlocked open still reads a legacy store, but validates the read.** It records that
  CURRENT was absent, loads the legacy layout, then re-reads CURRENT **and** re-checks that
  every legacy file it relied on is still present. If CURRENT has appeared or a file has
  gone, migration ran underneath it and the load may be missing named graphs — a missing
  `named.bin` must never read as "this store has no named graphs" mid-migration. It retries
  a bounded number of times and otherwise **fails closed** with an error naming the store
  (D5). Refusing legacy reads outright would break every existing read-only consumer the
  moment the binary is upgraded; reading one unvalidated would silently drop data.

Migration runs under the lock and is idempotent: a crash leaves either the legacy layout
plus a garbage `v0`, or a migrated store, and the next locked open finishes the job.

**Older binaries on a migrated store are unsupported.** A pre-versioning binary opening a
migrated root finds no base files and will report the store as absent or empty rather than
reading `v<N>`. This is a one-way upgrade per store; it belongs in the crate README, the
skills and the changelog when slice 2 lands.

## 7. Pinning and garbage collection

A reader of `v<N>` holds a shared `flock` on `R/pin/v<N>` for as long as its mappings
live, because its `Graph` owns both. **Liveness comes from the held descriptor, never from
a pid**: a crashed reader's lock is released by the kernel, so there is no stale-pid
problem to solve and no pid is recorded. A writer collecting `v<N>` first tries an
exclusive non-blocking `flock` on that pin file: refused means a reader still holds the version, so
it stays and a later locked operation retries; granted means the directory and its pin
file can be removed — and the exclusive pin lock is **held across the whole deletion**, so
a reader that re-reads CURRENT (§3 step 2) can never start loading a version mid-removal. A pinned version is never collected; an unpinned, non-current
version is collectable by any writer.

**Windows is out of scope** until `DirLock` exists there: `DirLock::acquire` is a no-op on
non-unix today, so durable stores have no writer exclusion on Windows at all, and the code
says so. The pin scheme here is unix-only. A port would need `LockFileEx` on the pin file,
because deleting an open file is refused on that platform — recorded only so the port has
a starting shape; none of it is in this series.

## 8. Invariants

**Structural — the enforcing code goes away.** An abandoned consistency window cannot
wedge a store (was I11): there is no window. Recovery probing covers every swap crash
point (was I12): the crash-state space is two states, both collected. A base replacement
never runs under a live writer's feet (was part of I10): a version is built beside the
live one and published by pointer. One store has one lock however its path is spelled
(was I8): the lock is an inode in `R`. A published version's bytes are complete: contents
are fsynced before the flip that names them (§4 step 4).

**Still enforced, with new mechanics.** One writer per store (I1), by `R/LOCK`. A reader
sees a whole transaction or none of it (I7), by the captured boundary and append-only logs
(§3) rather than by a seqlock — structural once logs stop being cleared in place, but the
boundary filter itself is code and carries a test. A handle whose store moved on is
refused or reloaded (I10), by comparing its **version and commit position** against the
store's under the lock (§4). A kept journal frame refuses every unrelated durable write in
the tree until it is materialised (I2, I5), unchanged and still per tree. Graph existence
is journalled (I6), and a frame's structural ops are replayed in commit order into one
unpublished version (§4). A named sub-graph is reachable only through its root (I9):
ownership lives in the version's `named.bin` and in each directory's `IDENTITY`, and
`Graph::open` accepts only a root with CURRENT or a legacy root — `R/v<N>` and
`R/v<N>/named/<i>` are refused by their `IDENTITY`, through any alias.

## 9. Carried over from #7115

Layout-independent work on `fix/core-journal-completeness` that should land first, from
`main`, as **slice 0** — it is what makes the later slices small:

| Piece | Note |
|---|---|
| `TxnOp` create/drop records and the `WTX2` frame format, with `WTX1` frames still replayed | the journal's record vocabulary; nothing about it depends on where the journal file sits |
| `sparq-engine`'s effect commit emitting create/drop ops (`update.rs`) | the producer side of the same vocabulary |
| the shared `TreeState` poison flag (one flag per tree, named graphs included) | per-tree state, not per-directory |
| `begin_durable_write` on every durable path, `clear_txn` included | the gate stays and stays central; its identity check becomes the version-and-commit-position check (§1, §4) |
| `OpenCtx` and the read-only, non-repairing open | an unlocked open must still rewrite nothing; §3 and §5 keep that distinction |
| `refuse_child_open` / ownership markers | the mechanism carries; the authority becomes each directory's own `IDENTITY` plus the version manifest (§2, §8) |
| the test suites: crash points, invariants, mutation tests | port per §10 |

**Discarded with the old layout:** the sibling control files (`.<dir>.writer.lock`,
`.<dir>.writer.gen`); the writer generation and its seqlock, `SeqWindow` and the nested
window depth; the second inode-keyed inner lock and its re-locking after a swap; the
`(dev, ino)` identity check on the store directory, replaced by the persistent `IDENTITY`
file (§2); `swap_dir_to_new_base` and the two-rename protocol; the
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
| 1.3 | A handle that loaded before another writer's changes kept writing | **Test**: a handle whose version **or commit position** differs from the store's reloads under the lock before its first write (D2) |
| 1.4 | `clear_txn` truncated the journal with no durable-write gate | **Test**: a journal clear is refused on a poisoned tree, by a non-holder of the lock, and by a stale handle |
| 1.5 | A poisoned tree's named graphs kept accepting writes | **Test**: every durable mutator of every graph in the tree is refused while a frame is retained |
| 1.6 | Graph create/drop was not journalled, so a crash recovered a different graph set | **Test**: a committed create/drop frame is redone in commit order on reopen (slice 0) |
| 2.1 | A reader could load a half-materialised multi-graph transaction | **Test**: the snapshot is CURRENT plus the captured commit boundary; a reader loading across a multi-graph commit sees the whole frame or none of it (D1) |
| 2.2 | Two path spellings of one store took two locks | **Construction**: the lock is an inode in `R` |
| 2.3 | A named sub-graph directory could be opened as a store of its own | **Test**: opening `R/v<N>` or `R/v<N>/named/<i>` is `InvalidInput`, decided by the directory's own `IDENTITY` rather than its parent path, and through an alias too |
| 2.4 | A restore ran without the store's lock, and a swapped-out handle kept writing | **Construction** for the exclusion (a restore is a version build under the lock, and the old version stays intact) + **Test** for the stale-handle check (D2) |
| 3.1 | Directory swaps bypassed the reader-consistency window | **Construction**: a version is invisible until the flip, and its contents are durable before it (D3) |
| 3.2 | An abandoned odd counter wedged every later open | **Construction**: no counter |
| 3.3 | The recovery probe stopped at the first absent child position | **Construction**: no staging siblings to probe |
| 3.4 | A child swap overwrote the tree's shared root identity | **Construction**: no per-directory identity; one CURRENT per store |
| 4.a | A failed swap left a handle holding the lock with no WAL | **Construction**: a failure before the flip leaves the live version and its WAL untouched |
| 4.b | A child became reachable before it carried its ownership marker | **Construction**: nothing in a version is reachable before the flip, and marker contents are fsynced before it (D3) |
| 4.c | The drop backstop published a quiescent counter over a retained frame | **Construction**: no counter; a retained frame is visible in `txn.log` |
| 4.d | Bind-mount aliases derived different sibling locks | **Construction**: lock inode in `R` |
| 5.1 | Failed named-drop staging left survivors accepting unlogged writes | **Construction**: the renumbering is a version build; a failure keeps the live version and its logs |
| 5.2 | A child swap replaced the root's inner lock | **Construction**: one lock, on `R`, never replaced |
| 5.3 | Aliases kept independent generation counters | **Construction**: no counter, and CURRENT is an inode in `R` |
| 5.4 | Replacement directories were published before their lock was held, and recovery with the root absent never took it | **Construction**: `R` cannot be absent, and publishing is a rename of a file inside it under the lock |
| 5.5 | Recovery published a generation mid-replay | **Construction**: recovery replays a whole frame into an unpublished version and publishes it with one flip (D4) |
| 5.6 | Markers were not fsynced before manifests | **Construction**: every file's contents, then the directories bottom-up, are fsynced before the flip (§4, step 4) |

Six rows need tests. Three (1.3, 1.4, 1.5) are the durable-write gate and the poison
state, which survive the layout change and stay centrally enforced; 2.3 is the child-open
refusal, now keyed on `IDENTITY`; 2.1 is the reader boundary; 2.4's stale-handle half is
1.3's mechanism. 1.6 is slice-0 behaviour with its own test already written.

### Design-audit findings (round D, against this record)

Raised against `e2996d03` by the automated design review; the resolutions above are the
coordinator's calls, and each row says what makes it true.

| # | Finding | Resolution |
|---|---|---|
| D1 | CURRENT alone gave no snapshot of mutable logs: a reader could load one graph's WAL before a transaction materialised and another's after, then find `txn.log` cleared, and return a partial transaction | **Construction + Test**: logs are append-only within a version and a frame is visible only through one record in the central `txn.log`; the reader captures the boundary before loading and filters WAL records above it (§1, §3). Test: 2.1 |
| D2 | A version comparison accepted a stale writer after ordinary commits, so a compaction from it could discard acknowledged changes | **Test**: the writer revision is the commit position, distinct from the version; a handle whose pair differs reloads under the lock, and builds use only state loaded under the lock (§4). Test: 1.3 |
| D3 | The build protocol fsynced directories but not file contents, so a durable flip could name incomplete files | **Construction**: fsync every file's contents, then the directories bottom-up, then flip, then fsync `R`; the save helpers must sync, not just flush (§4 step 4) |
| D4 | Structural (CREATE/DROP) frames had no atomic materialisation protocol, and "refuse builds while a frame is retained" left recovery with no completion path | **Construction**: a whole frame is applied to one unpublished version, published with one flip, then retired; the only build allowed while a frame is retained is the one materialising it; recovery does the same (§4, §5) |
| D5 | A legacy reader could load the default graph before migration and `named.bin` after, silently dropping named graphs | **Construction + Test**: migration is non-destructive until the flip is durable and folds every legacy WAL and committed frame first; a legacy reader re-checks CURRENT and its files after loading, retries, then fails closed (§6) |
| D6 | A replacement writer could inherit an unsynced CURRENT rename and acknowledge writes that a power failure would undo | **Construction**: under the lock, before accepting writes or collecting, a writer makes the observed CURRENT durable; publication errors are fail-closed (§4) |
| D7 | Intermediate slices exposed operations before their safeguards existed | **Construction**: slice gates — slice 1 refuses mutations on a versioned root, and no collection exists before pinning (§12) |

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

New tests the round-D findings require, beyond the ports:

- a reader that starts loading during a multi-graph commit sees the whole frame or none of
  it, including the case where one graph's WAL has been materialised and another's has not
  (D1);
- a handle that loaded before a second writer's ordinary commits reloads instead of
  compacting over them (D2);
- a legacy reader racing migration either reads the complete legacy dataset or fails
  closed, and never reports a migrated store as having no named graphs (D5);
- a writer that inherits a CURRENT rename with no root fsync makes it durable before its
  first write (D6);
- slice 1's mutation refusal and the absence of collection before slice 3 (D7).

## 12. Slices

One PR each, green in both feature states, with its own tests. **Each slice must be safe
on its own, not merely compile** (D7): a capability is gated off until the slice that makes
it safe lands, and the gate is a refusal with a clear message, not a comment.

**Slice 0 — layout-independent carry-over (from `main`).** The §9 table: the `WTX2`
record vocabulary with `WTX1` replay, the engine's create/drop emission, the shared
poison flag, `begin_durable_write` on every durable path including `clear_txn`, `OpenCtx`
and the non-repairing open, the child-open refusal. *Acceptance:* a committed create/drop
frame is redone in commit order on reopen; a `WTX1` frame still replays; every durable
mutator in a tree is refused while a frame is retained; a journal clear is refused by a
non-holder; an unlocked open rewrites nothing.

**Slice 1 — read path, read-only.** The CURRENT format with its reader and writer, the
`IDENTITY` file, version resolution, the captured boundary, `Graph::open` accepting a
versioned root and a legacy root, refusal of a path inside a version. Writers still write
flat, so the versioned reader is exercised by hand-built fixtures — and **a versioned root
refuses every durable mutation** until slice 2, because the flat writers would otherwise
rewrite a published version in place. *Acceptance:* a hand-built `R/v0` plus CURRENT opens
and reads; a truncated or checksum-mismatched CURRENT is a clear error; a legacy root
still opens and still writes; opening `R/v0` is `InvalidInput`; a durable write on a
versioned root is refused with a message naming the unimplemented writer.

**Slice 2 — writers.** Version building, the durability ordering and the flip for
compaction, vacuum, restore, named creation and named-drop renumbering; the
structural-frame protocol and its recovery; the lock-entry steps (CURRENT durability,
revalidation); legacy migration on first locked open; hardlink with copy fallback. This
slice lifts slice 1's mutation refusal. **No collection yet** — a version is never
removed, so a reader cannot lose one underneath it before pinning exists. *Acceptance:*
each operation publishes a new version and leaves the old one intact; a failure injected
before the flip leaves the handle writing to the live version with its WAL intact; crashes
simulated before and after the flip recover to old-or-new; a structural frame is published
by one flip; a stale handle reloads under the lock; migration is idempotent and folds every
legacy log.

**Slice 3 — pinning and GC.** Pin files, the shared/exclusive protocol, collection under
the lock holding the exclusive pin through the deletion, collection of garbage versions on
a locked open. Collection is introduced **here and not earlier**. *Acceptance:* a pinned
version is not collected and its reader keeps reading; once the reader drops, the next
locked operation collects it; a garbage `v<N+1>` from a crash is collected.

**Slice 4 — deletions.** Remove everything in §9's discard list and rewrite the invariant
table. *Acceptance:* the ported crash-point tests pass; no `.compact-*` or `writer.gen`
path remains in the crate; the per-commit path writes only logs.

Slices 0 to 3 keep the old machinery alive beside the new, so each lands independently;
the gates above are what make "beside" safe rather than merely compiling.
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
- **Do a per-operation check, not a per-nested-gate check.** The version-and-commit-
  position check that replaces the identity check runs once per public durable operation —
  one CURRENT read and one stat per log — not once per nested gate; the old check ran per
  gate, and one transaction passes several.
- **Append-only logs cost nothing extra on the write path.** Retiring a frame by appending
  a record rather than truncating is the same one small append plus the fsync already
  there; the commit sequence rides in the records being written.
- **When measuring a slice**, use interleaved pinned pairs, an A/A control of the baseline
  against itself, and a bootstrap confidence interval on the median ratio; report a result
  only against its A/A band.

## 14. Open questions

- **Journal scope — decided.** A frame committed immediately before a flip is
  **materialised into the new version during the build**, never carried across: the build
  that materialises the frame is the one build permitted while it is retained, and the
  frame is retired only after the flip is durable (§4, §5). Unrelated builds are still
  refused while a frame is retained. What remains open is only the shape of the retire
  record's interaction with log reclamation — a new version starts with empty logs, so a
  retire record never has to survive a flip.
- **Child versioning.** This design versions the store as a whole. Versioning each child
  independently would make a single-graph compaction cheaper, at the cost of a manifest
  that names a version per child. Not proposed; recorded as the obvious extension.
- **Pin-file lifetime.** `R/pin/` accumulates one file per version ever published unless
  collection removes them with their version; the GC in slice 3 must remove both, and a
  pin file with no version directory should be treated as collectable garbage.
