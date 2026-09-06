# sparq patches on top of spargebra 0.4.6

This tree started as a byte-identical copy of the `spargebra-0.4.6` sources from
crates.io (the only non-upstream change in the base commit is a `[workspace]`
table in `Cargo.toml`, required for the `[patch.crates-io]` path patch).

Patches **§1–§6** are surgical spec-conformance parser fixes, each verified
against the W3C SPARQL test suite (w3c/rdf-tests @ `f25dbc0`) and prepared as an
upstream PR against oxigraph/oxigraph — see `docs/upstream-proposals.md` at the
repo root. Patches **§7–§10** are sparq-local (a build guard, DoS hardening, a
custom-aggregate fix, and a vendor extension); §10 is explicitly *not* an
upstream candidate. Retiring this tree therefore takes more than an upstream
release — see the release watch below.

## Upstream release watch (bead `sq-98w7z.8`)

Re-check with **`python3 scripts/check-spargebra-release.py`** (exit `0` = still
blocked, `10` = released, `2` = indeterminate). Append a dated row here each
time the bead is picked up.

| checked | newest stable `spargebra` on crates.io | upstream main | verdict |
|---|---|---|---|
| 2026-06-11 | 0.4.6 | fixes on main, unreleased (`dabda10`, `c29be03`) | blocked — keep tree |
| 2026-07-27 | **0.4.6** (unchanged) | `lib/spargebra` = `0.5.0-dev` | blocked — keep tree, re-deferred |

**2026-07-27 check.** No release above 0.4.6 exists. Evidence: the crates.io
sparse index tops out at `0.4.6` (every higher entry is a pre-release of an
*older* line); oxigraph's released tag `v0.5.9` still ships `lib/spargebra` at
version `0.4.6`; upstream `main` set the crate to `0.5.0-dev` on 2026-07-19
(`a3d8311e`). The six conformance fixes remain unreleased, so the published
crate is still buggy and this tree stays.

Three findings that change the shape of the eventual retirement — the bead's
original scope line (`vendor/spargebra/**` + the root patch table + `Cargo.lock`)
is **incomplete**:

1. **13 manifests depend on this tree, not 1.** The root `Cargo.toml` uses
   `[patch.crates-io]`, but `bench/*` (11 manifests) and
   `zk/xpath/differential` are *separate workspaces* that the root patch table
   never reaches — each pins `path = ".../vendor/spargebra"` directly and has to
   be repointed at the registry version by hand.
2. **The next release is `0.5.0`, not `0.4.7`** — a semver-major bump. The root
   requirement `spargebra = { version = "0.4", … }` will not resolve it, and
   spargebra 0.5.0 will pull a newer `oxrdf` than this tree's `=0.3.3` pin.
   Since `spargebra::Query` embeds oxrdf term types across every crate seam, a
   duplicate `oxrdf` major in the lock is a hard type error, not a warning.
3. **§7–§10 have no upstream home.** An upstream release does not make this tree
   droppable by itself: each sparq-local patch must be re-landed on the release,
   upstreamed, or consciously dropped along with its dependents. §8 (parser
   recursion-depth cap) guards the unauthenticated `/sparql` endpoint against
   stack-overflow DoS (threat-model B2 / T-PARSE-DoS, bead `sq-v5dg`), so
   dropping it silently is a security regression; §10 (`MULTIPLICITY()`) is
   depended on by `sparq-engine` evaluation.

The invariant is unchanged: after unvendoring, the W3C conformance ratchet
(`ci.yml` → `conformance`, currently 0-fail) must not regress. A new failure
means the release is missing a fix — keep the tree and report it upstream.

## 1. Reject nested aggregate functions

- **Spec**: SPARQL 1.2 Query §18.3.4.1 *Grouping and Aggregation* — the
  aggregation step replaces each aggregate `R(args; scalarvals)` found in
  SELECT/HAVING/ORDER BY with a fresh `agg_i`; no derivation in the algorithm
  (or in §11.4's projection conditions) admits an aggregate inside another
  aggregate's argument. The suite states it directly:
  `mf:comment "The expression argument of an aggregate function can not
  contain an aggregate function."`
- **Test**: `sparql12/syntax#nested-aggregate-functions` (NegativeSyntaxTest,
  `SELECT (COUNT(COUNT(*)) AS ?c) WHERE {}`) — accepted before, rejected after.
- **Fix**: `ParserState::new_aggregation` already replaces each parsed
  aggregate with a synthetic random-named variable. A nested aggregate
  therefore surfaces as one of those synthetic variables inside the argument
  of the aggregate being registered: walk the argument expression
  (`mentions_aggregate_variable`) and error out when one is found. Synthetic
  names are 128-bit random hex, so user variables cannot collide; sub-SELECTs
  push their own aggregate scope, so legal aggregates inside `EXISTS
  { { SELECT ... } }` are unaffected (the walker does not descend into
  `Expression::Exists`).

## 2. Reject literals / triple terms in the subject position of an expression triple term

- **Spec**: SPARQL 1.2 Query §19.7 grammar production
  `[138] ExprTripleTermSubject ::= iri | Var` (an RDF 1.2 triple term subject
  is an IRI or blank node, so the grammar excludes literals and nested triple
  terms from the subject slot). spargebra instead reused
  `ExprTripleTermObject` ([139], which allows `RDFLiteral | NumericLiteral |
  BooleanLiteral | ExprTripleTerm`) for the subject, accepting the invalid
  forms. The data (`TripleTermData`, [127]) and pattern paths already
  enforced the restriction — only the expression path was loose.
- **Tests**: `sparql12/syntax-triple-terms-negative#tripleterm-subject-03`
  (*Triple term in the subject position of a triple term (expression)*,
  `BIND( <<( <<(:s :p :o )>> :q :z )>> AS ?X )`) and `#tripleterm-subject-06`
  (*Literal in the subject position of a triple term (expression)*,
  `BIND( <<( "literal" :q :z )>> AS ?X )`) — both NegativeSyntaxTest,
  accepted before, rejected after.
- **Fix**: `ExprTripleTermSubject` now derives exactly `iri | Var` instead of
  delegating to `ExprTripleTermObject`.

## 3. Longest-match tokenization: `<` starting an IRIREF token is not the less-than operator

- **Spec**: SPARQL 1.2 Query §19.7 (same note in SPARQL 1.1 §19.8), note 3:
  *"When tokenizing the input and choosing grammar rules, the longest match is
  chosen."* In `FILTER (?x<?a&&?b>?y)` the characters starting at `<` form a
  valid IRIREF token (`<?a&&?b>` — none of ``<>"{}|^`\`` or #x00–#x20 occur
  inside), so the input tokenizes as `?x`, IRIREF, `?y`, which has no parse —
  the document is a syntax error. The suite's comment in `syn-bad-26.rq`:
  *"longest token rule means this isn't a '<' and '&&'"*. spargebra's
  scannerless grammar happily read `<` as less-than and accepted the document.
- **Test**: `sparql10/syntax-sparql3#syn-bad-26` (NegativeSyntaxTest) —
  accepted before, rejected after. All other syntax suites (1.0/1.1/1.2,
  including every `<`-comparison with whitespace and `?x<<http://iri>` forms,
  where the second `<` terminates the IRIREF scan) are unchanged.
- **Fix**: the `"<=" / "<"` alternative of `RelationalExpression_inner` now
  carries a negative lookahead `!IRIREF_TOKEN()`, where `IRIREF_TOKEN` is the
  IRIREF terminal exactly as tokenized ([172]). The guard also covers `<=`
  (`<=?a>` is likewise the longer IRIREF token), keeping tokenization
  consistent.

## 4. Accept reuse of an earlier SELECT-expression variable in an aggregating query

- **Spec**: SPARQL 1.2 Query §11.4 *Aggregate Projection Restrictions* — every
  variable occurrence in projection/SELECT expressions of a grouping query
  level must satisfy one of three conditions, the third being *"the variable
  is introduced by an earlier SELECT expression in the same SELECT clause"*.
  spargebra validated SELECT expressions in aggregating queries against only
  the WHERE-visible variables, so the legal
  `SELECT (COUNT(?v) AS ?count) (?count + 1 AS ?countPlusOne)` was rejected
  with *"The SELECT contains an expression with a variable that is unbound"*.
- **Test**: `sparql12/grouping#select-variable-reuse` (*Reuse of SELECT
  variable in an aggregating query*, QueryEvaluationTest) — failed at parse
  before, passes end-to-end after.
- **Fix**: `build_select` adds each select-expression alias to the visible set
  after emitting its `Extend`, so later expressions in the same SELECT clause
  see it. The "SELECT overrides an existing variable" check reads the same
  set, so rebinding an alias stays an error (matching §18.3.4.4: *"var must
  not appear in VS nor in PV"*).

## 5. Match the boolean keywords case-insensitively

- **Spec**: SPARQL 1.2 Query §19.7 note 1 (same note in SPARQL 1.1 §19.8):
  *"Keywords are matched in a case-insensitive manner with the exception of
  the keyword 'a'"* — `true`/`false` are keyword terminals of the grammar
  (`BooleanLiteral`, [173]), and §4.1.2 maps the token to a literal of
  datatype `xsd:boolean`. `TRUE`/`False` therefore denote the boolean
  literals with the canonical lexical forms `"true"`/`"false"` (the suite's
  comment: *"Boolean keywords are case insensitive, and produce valid boolean
  literals"*).
- **Test**: `sparql10/expr-builtin#case-insensitive-booleans`
  (QueryEvaluationTest, `SELECT (TRUE as ?t) (False as ?f) {}` expecting
  `"true"`/`"false"^^xsd:boolean`) — parse error before, passes after.
- **Fix**: `BooleanLiteral` uses the grammar's case-insensitive keyword
  matcher `i()` and always emits the lowercase lexical forms. Every
  alternation tries `iri()` before `BooleanLiteral`, so prefixed names with a
  `true`/`false` prefix (e.g. `true:x`) are unaffected.

## 6. `OPTIONAL { { P FILTER(F) } }`: do not hoist the nested group's filter

- **Spec**: SPARQL 1.2 Query §18.3.2 (same in SPARQL 1.1 §18.2.2) — the
  working group notes the SPARQL 1.0 ambiguity for `OPTIONAL { { ... FILTER
  (...?x...) } }` and resolves it: *"Applying the simplification step after
  all the translation of graph patterns is the preferred reading."* Under
  that reading, when the OPTIONAL translation (§18.3.2.6) runs, the doubly
  braced group is still `Join(Z, Filter(F, A))` — not the `Filter(F, A)` form
  — so `F` must NOT become the LeftJoin expression (where it would see the
  left side's bindings). spargebra's `new_join` simplifies `Join(Z, A) = A`
  eagerly during parsing, making the group indistinguishable from a top-level
  filter and hoisting `F`. The W3C manifest includes ONLY the
  `-not-simplified` variant of the test pair ("Preferred reading and SPARQL
  1.1"); the `-simplified` variant is commented out.
- **Test**: `sparql10/optional-filter#dawg-optional-filter-005-not-simplified`
  (QueryEvaluationTest, `expr-5.rq`) — failed before (the hoisted
  `?title = "TITLE 2"` saw the outer `?title` binding and kept a `?price`),
  passes after (the filter's `?title` is unbound inside the nested group, so
  the inner pattern yields nothing and OPTIONAL adds no bindings).
- **Fix**: `GroupGraphPatternSub` keeps the spec's pre-simplification shape:
  when a group has no FILTER clause of its own but its body reduced to a bare
  `Filter` (i.e. the filter bubbled up from a nested group), it returns
  `Join(Z, Filter(F, A))` instead. Groups with their own top-level FILTER
  still translate to `Filter(F, A)` and keep hoisting, per §18.3.2.6.

## 7. Manifest: pin `rand` to 0.9 (wasm build guard — not an upstream change)

- **Why**: upstream declares `rand = ">=0.8,<0.10"`. The published
  spargebra 0.4.6 artifact resolves that to rand 0.9 (getrandom 0.3). As a
  `[patch.crates-io]` *path* dependency this manifest re-resolves inside the
  sparq workspace instead, and cargo unifies the range onto the lockfile's
  pre-existing `rand 0.8.6` (tungstenite's) — whose getrandom 0.2 has no
  enabled `wasm32-unknown-unknown` backend. Result: `cargo build -p sparq-wasm
  --target wasm32-unknown-unknown` fails to compile `getrandom 0.2`
  (`sparq-wasm` only configures getrandom **0.3**'s `wasm_js` backend), and
  `scripts/ci-bench.sh` skips its wasm-size step *silently* on build failure,
  so nothing gated it. Narrowing to `rand = "0.9"` keeps the vendored
  dependency graph byte-identical to the published crate's and makes the
  resolution sticky against future `cargo update`s.
- **Verified**: `cargo build --release -p sparq-wasm --target
  wasm32-unknown-unknown` compiles; bundle 1,554,093 B on `main` (merge
  `9917404`) vs 1,552,190 B on the pre-final-eleven `main` (`0fefc9a`) —
  the six parser patches cost ≈1.9 KB (+0.12%, a few bytes of build-path
  noise aside), all parser code that conformance requires.
- **Upstream**: nothing to propose — this is an artifact of path-patching, not
  an upstream bug. (At most, oxigraph could drop the rand 0.8 compatibility
  range, but it is harmless when consumed from crates.io.)

## 8. Cap the parser recursion depth (DoS hardening) [OPUS-4.8]

- **Why (security, not spec)**: `spargebra` is a `peg` recursive-descent parser,
  so each level of syntactic nesting — group graph patterns (`{ … }`,
  OPTIONAL/GRAPH/MINUS/UNION/LATERAL/SERVICE bodies, sub-SELECT WHERE,
  EXISTS/NOT EXISTS), bracketed/unary expressions (`( … )`, `!!…`), parenthesised
  property paths, RDF collections (`( … )`), blank-node property lists (`[ … ]`),
  and RDF 1.2 reified triples / triple terms (`<< … >>`, `<<( … )>>`) — maps onto
  one or more *native call-stack frames*. Upstream applies no recursion-depth,
  input-length, or stack-growth bound, so an attacker-controlled query string of
  a few thousand nested delimiters recurses until the call stack **overflows and
  ABORTS the process** (SIGABRT/SIGSEGV). This is reachable from sparq's
  unauthenticated `/sparql` endpoint via
  `PreparedQuery::parse → SparqlParser::parse_query` — a denial of service
  (sparq threat-model boundary **B2 / T-PARSE-DoS**, bead **sq-v5dg**).
- **Measured** (against this exact grammar, on a 2 MiB stack — the default for
  the tokio worker / blocking-pool threads on which the server parses, *not* the
  larger main-thread `ulimit -s`): the unmodified parser overflows at roughly
  **~180 nested levels in a debug build** and **~900 in a release build** (group
  nesting and bracketed-expression nesting behave alike). The shell `ulimit -s`
  (often 8 MiB) is irrelevant to the server's actual parse threads.
- **Fix**: thread a single shared nesting counter through `ParserState`
  (`recursion_depth`, cap `MAX_RECURSION_DEPTH = 128`). An empty-matching PEG
  guard rule `RecursionGuard() = {? state.enter_recursion() }` is placed in each
  recursive production *immediately after it commits to its opening delimiter*; it
  increments the counter and fails the parse with a clean syntax error once the
  cap is reached. Each guarded production calls `state.leave_recursion()`
  (saturating) in its success action, so on the matched path the counter is
  exactly balanced; backtracking can only leave it transiently *higher* (never
  lower), which at worst makes the guard marginally more conservative and is
  discarded with the `ParserState` on a failing parse — the same
  mutate-during-PEG-parse pattern the crate already relies on for its
  `aggregates` stack. Because PEG only surfaces the failure at the *furthest*
  input position (which masks the depth-guard failure for a uniformly nested
  input), `parse_query`/`parse_update` additionally check a `hit_recursion_limit`
  flag and, when set on a failed parse, return a dedicated
  `SparqlSyntaxErrorKind::TooDeeplyNested` error ("The SPARQL query is too deeply
  nested …") in preference to the raw "unexpected token" message. The cap chosen
  (128) sits with comfortable headroom (~30%) below the debug-build 2 MiB
  overflow point and ~8× above the deepest query in the W3C SPARQL 1.0/1.1/1.2
  conformance suites, so it overflows nothing yet rejects no legitimate query.
- **Tests**: `vendor/spargebra/tests/recursion_depth.rs` — a pathologically
  nested query for *each* recursion axis (groups, bracketed expressions, `!!`
  chains, property paths, collections, blank-node lists, triple terms, and the
  UPDATE template path) is run on a 2 MiB thread and must return a clean `Err`
  (reaching `join()` at all proves no overflow); positive controls assert that
  64-deep nesting and a spread of ordinary queries still parse, and a boundary
  test pins that crossing the cap flips OK→Err in a sane range.
- **Verified**: `cargo test -p spargebra` (incl. doctests) and the full W3C
  SPARQL conformance suite stay green (Overall ≥ 1229 pass+divergence — see the
  report). `cargo clippy --all-targets` is clean for the changed code (the two
  remaining `build_select` lints — `too_many_arguments`, `type_complexity` — are
  pre-existing upstream and untouched; they never gate because the project lints
  spargebra as a `[patch.crates-io]` dependency, which clippy does not lint).
- **Upstream**: proposed to oxigraph/oxigraph (the recursion cap is generally
  useful, not sparq-specific). See the PR description prepared with this change.

> Manifest note: the upstream manifest sets `autotests = false` (the published
> crate ships no `tests/` dir), so this patch also adds an explicit `[[test]]`
> target entry for `tests/recursion_depth.rs`. Like the `[workspace]` table and
> the `rand` pin (§7), that is a vendoring artifact, not an upstream change.

## 9. Custom-aggregate `DISTINCT` is unreachable in `iriOrFunction` [OPUS-4.8]

- **Spec / intent**: SPARQL 1.1 §11.6 makes custom aggregates an extension point
  (`Aggregate ::= … | FunctionCall`). spargebra goes a step further and already
  has an `Aggregate` grammar alternative for `<agg>(DISTINCT expr)` over a
  *registered* custom-aggregate IRI (`parser.rs`, the `name:iri() "(" DISTINCT
  Expression ")"` arm) — so the DISTINCT modifier on a custom aggregate is an
  intended, supported form. It just never parsed.
- **Bug**: in `PrimaryExpression`, `iriOrFunction()` is tried *before*
  `BuiltInCall()` (which owns `Aggregate()`). `iriOrFunction = iri() ArgList()?`
  has an OPTIONAL argument list. For `<agg>(DISTINCT ?x)` the regular `ArgList`
  cannot parse `(DISTINCT ?x)` (DISTINCT is not an `Expression`), so `ArgList()?`
  matches `None` and the rule greedily succeeds treating the bare IRI as a
  standalone term. Because `iriOrFunction` already succeeded, PEG never
  backtracks into the `Aggregate` rule, and the parse then fails downstream at
  the unexpected `(` with a misleading `expected ENCODE_FOR_URI`. The
  DISTINCT-free form (`<agg>(?x)`) was unaffected: there the regular `ArgList`
  *does* parse, `iriOrFunction` errors ("…is an aggregate function and not a
  regular function"), and PEG backtracks into `Aggregate` as designed.
- **Fix**: in `iriOrFunction`'s `else` branch (no `ArgList` parsed), reject a
  bare IRI that is a *registered custom aggregate* with the same
  "…is an aggregate function and not a regular function" error the with-args
  branch uses. That failure makes PEG backtrack into `BuiltInCall → Aggregate`,
  whose custom-IRI arms then parse both `(DISTINCT expr)` and `(expr)`. A bare
  aggregate IRI used as a standalone term is not valid SPARQL, so the rejection
  removes nothing legitimate; non-aggregate IRIs are untouched.
- **Tests**: `vendor/spargebra/tests/custom_aggregate_distinct.rs` — the
  prefixed-name and full-IRI DISTINCT forms must parse, the DISTINCT-free form
  must still parse, the `DISTINCT` flag must survive into the algebra
  (`Display`), and DISTINCT in a call to an *undeclared* (regular) IRI must
  remain an error. Engine-level WITH/WITHOUT-DISTINCT evaluation coverage lives
  in `crates/sparq-engine/src/aggregate.rs` (bead sq-fldo).
- **Upstream**: candidate for oxigraph/oxigraph — the `Aggregate` rule's intent
  is plainly to support `<agg>(DISTINCT …)`, which the rule-ordering defeats.

> Manifest note: as in §8, `autotests = false` means the new
> `tests/custom_aggregate_distinct.rs` target is declared explicitly in
> `Cargo.toml`. A vendoring artifact, not an upstream change.

## 10. Parse `MULTIPLICITY()` as a reserved-IRI extension builtin [OPUS-4.8]

- **Context (sq-v411r, survey §B2)**: the SPARQL 1.2 algebra uses `multiplicity(μ|Ω)`
  — replacing the informal `card[Ω](μ)` — as the multiset-cardinality device inside
  the Set-Function definitions and §18.4 BGP matching. It is a *definition* device:
  the W3C SPARQL 1.2 Query rec (and its editor's draft) define **no callable
  `multiplicity()` builtin** and ship **no `multiplicity` conformance test**. sparq
  exposes the device as an opt-in vendor extension so a query can write the
  multiset-weighted aggregate `SUM(?x * MULTIPLICITY())` the survey calls for.
- **Why a reserved IRI, not a new `Function` enum variant**: the shared `Function`
  enum is matched **exhaustively (no `_` wildcard)** by downstream crates we do not
  control — `sparopt`/`spareval` (pulled in via `oxigraph` for `sparq-bench`).
  Adding a variant breaks their compile under `--workspace --all-features`. So the
  parser maps the keyword `MULTIPLICITY` to `Function::Custom(<urn:sparq:fn:multiplicity>)`
  (a zero-arg call); the enum is byte-identical and every downstream matcher hits its
  existing `Function::Custom(_)` arm. The sparq engine recognises the reserved IRI in
  aggregate evaluation + function dispatch.
- **Fix**: one `BuiltInCall` alternative — `i("MULTIPLICITY") _ NIL()` (the zero-arg
  shape of `NOW()`/`RAND()`), gated behind `sparql-12`, emitting the reserved-IRI
  `Function::Custom`. No `Function`/`Display` change.
- **Tests**: engine-level evaluation + the parse round-trip live in
  `crates/sparq-engine/src/exec.rs` (`mod multiplicity_builtin`) and
  `crates/sparq-engine/src/aggregate.rs`. Semantics: folding the **distinct** group
  solutions weighted by each one's bag cardinality, so `SUM(?x * MULTIPLICITY())`
  equals plain `SUM(?x)` over the bag.
- **Not upstream**: this is a sparq extension, not a spec conformance fix — it is NOT
  prepared as an oxigraph PR (unlike §1–§6). Drop if/when W3C standardises a callable
  multiplicity builtin.
