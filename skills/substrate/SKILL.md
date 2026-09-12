---
name: substrate
description: The opt-in shared zero-overhead evaluation substrate for the sparq engine and the reasoners — id-tuple row/key/posting vocabulary (`rows`), the XSD numeric value tower (`numeric`), the four id-tuple join kernels (`join`), and the SPARQL term total order (`compare`). All features are DEFAULT-OFF; opt in to exactly the slice you need. Use this crate when wiring a new reasoner that shares joins or numeric evaluation with the engine, or when working on the sparq-substrate crate itself. [SONNET-4.6] sq-qonbz.4
---

# sparq-substrate

`sparq-substrate` is the leaf crate (below `sparq-engine`, zero-overhead) that hosts the
id-tuple evaluation primitives shared between the SPARQL engine and every reasoner. It depends
ONLY on `sparq-core` — no cycle with `sparq-engine` — so both consumers can share one join
kernel / numeric value tower with no dependency problem.

All four modules are gated behind **DEFAULT-OFF** cargo features; enabling one never forces a
heavy dependency on the engine's or the wasm bundle's build graph.

## Four public modules

### `rows` — `#[cfg(feature = "rows")]`

The shared id-tuple vocabulary: `SmallVec`-based `Row` (`[Id; 4]`), `Key` (`[Id; 2]`) and
`Posting` (`[usize; 2]`) aliases over the `sparq-core` dictionary `Id = u32`, plus the
re-exported inline-integer id helpers `inline_id_of_int`, `is_inline`, and `NO_ID`.

```toml
[dependencies]
sparq-substrate = { version = "0.1.0", features = ["rows"] }
```

```rust,ignore
use sparq_substrate::rows::{Id, Row, Key, Posting, inline_id_of_int, is_inline, NO_ID};

let mut row: Row = Row::new();       // SmallVec<[Id; 4]> — inline up to 4 columns, no alloc
row.extend_from_slice(&[1, 2, 3]);
let int_id: Option<Id> = inline_id_of_int(42);
```

### `numeric` — `#[cfg(feature = "numeric")]`

The XSD numeric value tower: `Num` (`Int(i64)` / `Dec(Dec)` / `Float(f32)` / `Double(f64)`),
the fixed-point `Dec` struct (EXACT integer/decimal arithmetic, no f64 rounding), `ArithOp`
(`Add`/`Sub`/`Mul`/`Div`), `RoundMode`, `as_numeric` (classifies an `oxrdf::Literal` into the
tower), and the XSD lexical helpers `split_decimal`, `parse_xsd_f64`, `parse_xsd_f32`,
`fmt_xsd_double`. `parse_xsd_f64` delegates to `sparq_core::parse_xsd_f64` (sq-9781x) — the
shared XSD f64 SPELLING body. The `sparq-core` numeric-value cache layers a DATATYPE-AWARE
gate on top (`sparq_core::numeric_cache_value`, sq-74oy4/sq-6b1lj: integers scale-0, decimals
no-exponent, i128-fit, trimmed), so a cache-hit ⟺ `Num::of_literal` accepts — a lexical
ill-formed for its datatype (`"1.5"^^xsd:integer`) misses the cache exactly as `of_literal`
type-errors it, uniformly on `=`/`<`/`>`. The differential test
`cache_f64_seam_vs_as_numeric_differential` pins that agreement.
Pulls `oxrdf` only when enabled. Two ordering methods on `Num`:
- `Num::cmp_total` — ORDER BY / MIN/MAX total order; NaN totalised FIRST.
- `Num::cmp_relational` — SPARQL `<`/`>` / D-entailment / RIF numeric equality; NaN → `None`
  (type error). [OPUS-4.8] sq-v5evr.

**`xs:float` promotion rounds ONCE.** `Num::f32` (and `Dec::f32` under it) is the single
shared helper the float tier of `binop` — and the `overhead` kernel's inline replica of it —
promote through, so the two cannot drift apart. Integer → `f32` and decimal-lexical → `f32`
are each a SINGLE correctly-rounded conversion; routing through `f64` first and narrowing
rounds TWICE, which is not what XPath/XSD numeric promotion requires. Verified by exact
rational arithmetic: the `i64` value `4611686293305294849` has correctly-rounded `f32` bits
`0x5E800001`, but `as f64 as f32` gives `0x5E800000` — so `SUM(?int, "0.0"^^xsd:float)` and
`AVG` over a mixed integer/float column returned a float one ULP off for magnitudes above
2^53. `parse_xsd_f32` likewise parses `&str -> f32` DIRECTLY (its ACCEPTANCE still delegates
to the shared `sparq_core::parse_xsd_f64` spelling rules, so which lexicals are well-formed
is unchanged). Regression tests assert `f32::to_bits()`, not a formatted decimal — the two
candidates are ADJACENT floats that print identically. [OPUS-5] issue #3796.

**Which tier a mixed comparison uses.** XPath promotes the operands of a comparison to the
**LEAST common type** in `xs:integer -> xs:decimal -> xs:float -> xs:double`, NOT always to
`xs:double`: `xs:double` is the common type only when an operand really IS a double.
`Num::cmp_relational` is therefore rank-aware — both `Int`/`Dec` compare exactly via
`Dec::cmp`; a `Double` operand promotes the pair to `f64`; otherwise a `Float` operand
promotes the pair to **`f32`** through `Num::f32`. [OPUS-5] #3796

> **CORRECTION — this section previously said the opposite, and was wrong.** It asserted
> that `cmp_relational` comparing any mixed pair in the `f64` tier is CORRECT and "must not
> be fixed". That is only true when an operand is an `xs:double`. For integer/decimal versus
> `xs:float` the least common type is **float**, and comparing in `f64` gives a different,
> wrong answer above 2^53: `Num::Int(4611686293305294849)` versus the `f32` `0x5E80_0001`
> (which is the correctly-rounded promotion of that very integer, = 2^62 + 2^39) returned
> `Less` where XPath requires `Equal`. Fixed, and pinned by
> `cmp_relational_integer_and_decimal_vs_float_compare_in_the_float_tier`. Do not
> re-introduce the unconditional `f64` fallback.

A second, unrelated `xsd:float` defect is still open in a different crate:

> **REMAINING BOUNDARY — the engine's `=`/`<` on an `xsd:float` still do not see the `f32`
> value.** This is a DIFFERENT defect from the one above, in a different crate, and it is
> still open. `sparq-engine` does not call `cmp_relational` at all; its `=`/`<` ride the
> numeric-value CACHE — `sparq_core::cached_numeric_f64` and the engine's
> `numeric_cache_f64`, which drive the sargable `=`/`<` fast path, `JKey::Num` value-joins
> and the `ORDER BY` numeric rank — and that still values an `xsd:float` literal as
> `parse_xsd_f64(lexical)`, i.e. the f64 nearest the LEXICAL rather than the f64 image of
> its correctly-rounded `f32`. Measured through `sparq_engine::query`:
> `"4611686293305294849"^^xsd:float = "4611686568183201792"^^xsd:double` is `false` (XPath
> requires `true`) and the same `<` is `true`, yet `+ "0.0"^^xsd:float` yields
> `4.6116866E18`. NOT a regression from #3796 — that path never consumed the `f32` value.
> Not fixed here because the cache f64 is shared bit-identically with the spilled on-disk
> dict cache and `LocalVocab::intern`, so correcting it must be validated against the W3C
> conformance ratchet. Tracked as issue #3825; #3796 stays OPEN for it. [OPUS-5]

```toml
[dependencies]
sparq-substrate = { version = "0.1.0", features = ["numeric"] }
oxrdf = { version = "0.3", features = ["rdf-12"] }
```

```rust,ignore
use sparq_substrate::numeric::{Num, Dec, ArithOp, as_numeric};

// Integer arithmetic — exact
let a = Num::Int(3);
let b = Num::Int(4);
let sum = a.binop(b, ArithOp::Add); // Some(Num::Int(7))

// Decimal arithmetic — exact fixed-point, no f64 rounding
let x = Num::Dec(Dec { mant: 1, scale: 1 }); // 0.1
let y = Num::Dec(Dec { mant: 2, scale: 1 }); // 0.2
let xy = x.binop(y, ArithOp::Add);           // Some(Num::Dec(...)) == 0.3 exact

// Classify a literal
let lit = oxrdf::Literal::new_typed_literal("42", oxrdf::vocab::xsd::INTEGER);
let n: Option<Num> = as_numeric(&lit);        // Some(Num::Int(42))
```

### `join` — `#[cfg(feature = "join")]`

The four id-tuple join kernels over `&[Row]` slices. Requires `rows`; pulls `rustc-hash`
(for `FxHashMap`) only when enabled.

| Function | Kind |
|----------|------|
| `merge_join` | sorted merge join — one shared key column |
| `build_table` + `build_partitioned` | build `JoinTable` (serial) or `Vec<JoinTable>` (radix-partitioned for parallel workers) |
| `probe_emit` | per-row probe emit — single-hash (FxHash once for partition + `raw_entry().from_hash`), `reserve` before materialising (batch-emission contract) |
| `probe_gather_indices` | M4 batch-emission primitive — collect build-row indices WITHOUT materialising `Row`s; morsel pipeline calls this, then materialises per output chunk (sq-pntvh.7) |
| `hash_probe_serial` | probe loop: calls `probe_emit` per row with a `Budget` cooperative-cancel poll |
| `bind_combine` | index-nested-loop combine step for indexed groups |
| `bind_combine_rows` | contiguous row-slice combine; appends one row per input, preserving duplicates and existing output ([GPT-6-ASTRA]) |
| `lftj_recurse` over `Trie`/`TrieIter` | leapfrog trie-join (WCOJ) |
| `join::delta::DeltaTable` | persistent build-side table for semi-naive Δ-vs-full shapes (built for the OWL-RL fixpoint; drives `sparq-rsp`'s Delta/Snapshot window diff) |

Pass `&rows[start..end]`, one match's `new_vals`, and `&mut out` to
`bind_combine_rows`; scanning, filtering and budget checks remain with the caller.

`JoinTable` is a public type alias for `hashbrown::HashMap<Key, Posting, FxBuildHasher>`.
All kernels are generic over a `JoinKeys` column descriptor and a `Budget` cooperative-cancel
hook — both monomorphised, never a trait object. Use `NoBudget` for an unbounded join.

**Single-hash optimisation ([SONNET-4.6] sq-7d3dj.19):** `probe_emit` and `probe_gather_indices`
compute `key_hash` ONCE — it selects the radix partition AND drives `raw_entry().from_hash` on
the `JoinTable`, eliminating the previous double-hash in the partitioned probe path. `probe_emit`
calls `out.reserve(matches.len())` before the emit loop (batch-emission contract the sq-pntvh M4
morsel pipeline inherits: reserve then materialise, not per-match).

**Single-column key fast path ([OPUS-4.8] sq-4r8uy):** `JoinKeys::left_key`/`right_key` special-case
the dominant `key_cols.len() == 1` case — the whole per-row key projection collapses to a direct
one-element `Key` push instead of iterating the heap `key_cols` `Vec` and running the general
`SmallVec::from_iter` collect. It is **result-identical** to the general projection (same length,
same id, same order), a pure performance specialization on the existing path — no feature flag, no
public-API change. Multi-column keys fall through to the general `iter().collect()` unchanged. This
closes most of the `hash_probe` descriptor-projection overhead the #1810 delta measured; the
`overhead` harness re-measures it (canonical re-measure is the acceptance gate).

```toml
[dependencies]
sparq-substrate = { version = "0.1.0", features = ["join"] }  # implies "rows"
```

```rust,ignore
use sparq_substrate::join::{merge_join, build_table, hash_probe_serial, JoinKeys, NoBudget};
use sparq_substrate::rows::Row;

fn make_row(vals: &[u32]) -> Row {
    let mut r = Row::new();
    r.extend_from_slice(vals);
    r
}

let left = vec![make_row(&[1, 10]), make_row(&[2, 20])]; // sorted on col 0
let right = vec![make_row(&[1, 100]), make_row(&[2, 200])];

// Sorted merge join on key column 0, append right column 1
let mut out = Vec::new();
merge_join(&left, 0, &right, 0, &[], &[1], &NoBudget, &mut out);
// out: [[1, 10, 100], [2, 20, 200]]

// Hash join: build left side, probe right side
let keys = JoinKeys { key_cols: vec![(0, 0)], right_only: vec![1] };
let table = build_table(&left, &keys);
let mut out2 = Vec::new();
hash_probe_serial(&right, &keys, &left, std::slice::from_ref(&table), &[1], &NoBudget, &mut out2);
```

### `compare` — `#[cfg(feature = "compare")]`

The SPARQL term total order `compare_terms`: the spec-fixed class precedence error/unbound <
blank < IRI < literal < triple, then KIND-FIRST within the literal class (sq-wjl8i): a fixed
`LiteralKind` rank between literal kinds — numeric < boolean < dateTime < date < string < lang
< other, a documented extension where the spec leaves cross-kind order undefined — and value
order only WITHIN a kind (numerics by exact rational value with NaN totalised FIRST before
-INF and f64 ties rechecked exactly via `exact_cmp`, incl. the mixed int/decimal-vs-double tie
— `Num::cmp_total`; dateTimes by timeline; else lexical), plus the recursive component-wise
triple-term order. Generic over a `CompareTerm` trait
(`term_class`/`literal_kind`/`value_str`/`as_f64`/`exact_cmp`/`strict_cmp`/`triple_parts`) — a
monomorphisation seam, never a `dyn` object. Pure-`std`; pulls nothing new. The engine's
relational `<`/`=` are deliberately untouched (XPath promotion, NaN type errors): only the
ORDER BY total order refines promoted ties and positions NaN.

```toml
[dependencies]
sparq-substrate = { version = "0.1.0", features = ["compare"] }
```

**Machine-checked order laws (Kani, sq-sqtk2.4 + sq-wjl8i).** `src/compare.rs` hosts a
`#[cfg(kani)]` bounded-proof module proving, over an adversarial model `CompareTerm` impl
whose numeric domain straddles the 2^53 f64-collapse boundary (2^53−1 / 2^53 / 2^53+1 /
2^53+2 and their shared f64 images, ±0.0, NaN): reflexivity, antisymmetry-consistency,
within-class totality and transitivity — per literal kind AND across MIXED literal kinds
(`transitivity_mixed_literal_kinds_incl_nan`), ALL with NaN included — and exact-order
agreement on the exact tier (the `exact_cmp` recheck's guarantee — delete the recheck and it
goes red). The three sq-sqtk2.4 intransitivity findings (mixed-tier collapse, numeric-vs-
string lexical fallback, NaN partiality) are FIXED and pinned by `witness_*` harnesses that
go red on regression. Run `cargo kani -p sparq-substrate --features compare` (Kani injects
the `kani` cfg; the normal build strips the module). Honest boundaries: this proves the
shared ALGORITHM over the bounded model, NOT the engine's `Value` impl
(`sparq-engine/src/exec.rs` — covered by unit tests, the sparq-reason engine-parity suite and
W3C conformance; next-wave in `research/mechanized-proof-program.md` §6); the harnesses model a
CONTRACT-CONFORMING `strict_cmp`, i.e. one TOTAL within the dateTime/date kinds (sq-2k5py made
that a documented trait obligation, satisfied by `Timeline::cmp_tl_total` — the partial
relational comparison's lexical fallback is the intransitive shape, pinned by the `witness4_*`
unit test rather than by Kani); and `exact_cmp` impls bounded by the i128 tower keep collapsed ties for lexicals
beyond ~38 significant digits (beaded).

### `overhead` — `#[cfg(feature = "overhead")]` — the zero-overhead DELTA harness

The **substrate half of the sparq-engine-systems paper's §8 protocol** (`site/papers/
sparq-engine-systems.typ`, evidence key `substrate.overhead_<kernel>`, environment="canonical"
ONLY). `overhead::OverheadReport::run(reps, environment, host_note)` times each shared kernel
against a hand-**specialised** pre-extraction equivalent — the SAME algorithm over the SAME
data structure, with ONLY the extraction's generalisation removed (the `JoinKeys` descriptor /
generic `Budget` / `CompareTerm` trait replaced by hard-coded concrete access) — and reports
the per-kernel `overhead_ratio = (substrate_ns − handrolled_ns)/handrolled_ns` plus the raw
nanoseconds. It **measures** the title-level "zero measured marginal overhead" claim; a non-zero
delta carries an honest `root_cause` and is reported as-measured, never bent to the claim. Every
kernel's two implementations are asserted result-equivalent each rep (`agree`) so a fast-but-
wrong baseline cannot flatter the ratio. Implies `join`+`numeric`+`compare`; links no new dep.

Kernels measured: `merge_join`, `hash_probe`, `num_int_add`, `num_double_add`, `compare_terms`.
The leapfrog trie-join (WCOJ) is **deliberately excluded** — net-new in the substrate, it has no
hand-specialised pre-extraction predecessor to form an honest delta against (comparing a WCOJ to
a nested-loop enumeration would measure an algorithm gap, not the extraction's overhead).

```text
# work box (non-canonical — PR-body only, never a paper headline):
cargo run -p sparq-substrate --example substrate_overhead --features overhead --release -- --json
# CANONICAL run on a dedicated quiet EC2 box (headline-eligible):
cargo run -p sparq-substrate --example substrate_overhead --features overhead --release -- \
    --canonical --reps 15 --host "c6i.4xlarge" --json
```

The envelope's `canonical` flag is true ONLY for a `--canonical` run; a work-box run is
`environment="indicative"`. Registered in `bench/benchmarks.toml` as `substrate-overhead-delta`
(`featured = false` — an internal micro-instrument). [FABLE-5] sq-atjue

## Cargo feature summary

| Feature | Enables | Extra deps |
|---------|---------|------------|
| `rows` | `sparq_substrate::rows` | — |
| `numeric` | `sparq_substrate::numeric` | `oxrdf` |
| `join` | `sparq_substrate::join` (implies `rows`) | `rustc-hash`, `hashbrown` |
| `compare` | `sparq_substrate::compare` | — |
| `overhead` | `sparq_substrate::overhead` (implies `join`+`numeric`+`compare`) | — |

All features are off by default. The default build compiles nothing from this crate (byte-
identical wasm bundle). The crate is `forbid(unsafe_code)`.

## When to use

- **Implementing a new sparq reasoner** that shares join kernels or numeric evaluation with
  the engine — depend on `sparq-substrate` directly (it is below `sparq-engine`, so no cycle).
  Direct consumers today: `sparq-engine` (all four seams), `sparq-reason` (`substrate-join`,
  opt-in), and `sparq-rsp` (`join::delta::DeltaTable` for the windowed-materialisation
  Delta/Snapshot diff). [FABLE-5] sq-2n1q3.4
- **Working on the substrate crate itself** — see `research/shared-eval-substrate.md` for the
  full design record (what is shareable vs engine-private, the options considered, the
  perf-neutrality proof).

## See also

- `research/shared-eval-substrate.md` — design record for the extraction strategy.
- `crates/sparq-core` — the storage substrate (`Id`, `Dict`, `Graph`) this crate's `rows`
  vocabulary comes from.
- `crates/sparq-engine` — the primary consumer: keeps its planner, `Bindings`, and `Value`
  private; calls the shared kernels through thin adapters.

_Status: publishable (sq-qonbz.4 [SONNET-4.6]). All four modules implemented and behaviour-
neutral vs the pre-move engine baseline (W3C SPARQL conformance floor bit-identical; join/
numeric/compare micro-benches within noise). Phase-5 reasoner adoption (consuming `join` from
`sparq-reason` / `sparq-reason-el`) is tracked separately._
