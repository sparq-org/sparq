Operator-only merge handoff for #7136 (lws-storage-lease).

The operator started the merge of `aa6078937` (#7192) into `e970554ed`.
The interrupted attempt had already removed the conflict markers in the working
copy, rewritten the save test, updated the skill, and migrated the new journal
tests to guarded graph accessors. Those changes were retained and completed.

Resolution:

- `EmbeddedSparqClient::save` retains the branch's `refuse_frozen` check and
  file-identity routing: the open directory compacts; other directories receive
  independent snapshots. Its rustdoc includes the compaction behavior and Errors.
- Retained #7192's create-then-canonicalise ordering before `is_backed_by`.
  Otherwise a nonexistent component before `..` could bypass compaction routing.
- Each direct, dot, missing-component/parent, and Unix symlink spelling uses a fresh
  store. After save, an unchanged pre-save parent record, updated child record,
  new child and containment all survive drop/reopen. The independent snapshot
  keeps its prior content and can be changed without changing the original.
- Added the Unix M5 case: no outside lock can be created under a read-only parent;
  refused compaction returns a backend error and reads and journaled writes keep
  working, including after reopen. Permissions restore on early return or unwind.
  As with the core permission test, this case skips if permissions are not enforced.
- Updated `skills/solid-lws-server/SKILL.md` to describe in-place compaction,
  independent snapshots, durability after save, and unavailable-compaction errors.

Semantic overlap review:

Compared the merge-base-to-HEAD and merge-base-to-MERGE_HEAD path sets, then
reviewed the incoming diff and the retained differences from MERGE_HEAD for all
10 shared paths: CHANGELOG.md; lws/intents.rs, lws/mod.rs, lws/tracked.rs;
reconcile_runtime.rs; store/counting.rs, store/embedded.rs, store/mod.rs,
store/sparq.rs; and skills/solid-lws-server/SKILL.md (Rust paths are under
crates/sparq-lws-core/src/).

No additional semantic resolution was needed. `CompositeStore::delete` retains
#7192's single `delete_resource` delegation and reclaims only the returned key.
The store/client wrappers retain this branch's `exclusive()` forwarding and
main's `call_timeout()`/atomic-delete forwarding. The startup exclusivity check
and lifecycle lease remain intact alongside main's fence/settling changes.
`commit()` retains the live graph and its storage lock/journals: it captures
updates on a disposable fork, drops the fork, then journals and materializes the
captured effects on the authoritative graph with `apply_effects_checked`.
A refused commit leaves the actor usable; an indeterminate commit stops it.
The new tests use `named()`, `dict()`, and guarded named-delta application.

Validation results will be completed below before handoff.

Operator action: no staging, commit, merge, push, or git-index write was attempted.
The working file contains no conflict markers. Git still lists embedded.rs as
unmerged because only the operator can stage the resolution and conclude the
merge. Keep this note operator-only. Existing PR_BODY.md and ROUND7.md are retained.

Suggested logical commit message: `fix(lws): preserve compaction and journaled writes during main merge`.
Keep the save implementation, regression tests, and matching skill together in
the merge resolution.
