// [OPUS-4.8] sq-jpki.2 — single-source-of-truth conformance assertion for the wasm `Store`
// surface, from the `js/` (`@sparq-org/sparq`) side.
//
// `@sparq-org/sparq` types the wasm engine against the generated `Store` surface it SHIPS
// (`js/wasm/sparq_wasm.d.ts`, in the publish `files` allowlist), so a TypeScript consumer of
// `@sparq-org/sparq` needs no other package. The shared `@sparq/client` workspace package
// re-exports the SAME generated surface (from its tracked `src/generated/sparq_wasm.d.ts`) so
// the site and the (proposed) GUI consume one canonical type (research/gui-design.md §0/§4).
//
// Two guards keep those copies ONE source of truth:
//   1. `@sparq/client`'s `check:wasm-types` asserts BYTE-IDENTITY between the freshly built
//      `js/wasm/sparq_wasm.d.ts` and `@sparq/client`'s tracked copy.
//   2. THIS file asserts, purely at the TYPE level, that the `Store` (+ cursor) surface
//      `js/` ships is STRUCTURALLY interchangeable with the `@sparq/client` surface — a
//      stronger, type-system-checked statement than a text diff, and one that lives on the
//      `js/` side so a binding change that lands in `js/wasm/` but not `@sparq/client` (or
//      vice-versa) fails this typecheck.
//
// It is DEV-ONLY: typechecked via `tsconfig.conformance.json` (`noEmit`) and EXCLUDED from the
// published `tsc` emit (`tsconfig.json`), so nothing here reaches `js/dist/` and the published
// `@sparq-org/sparq` never references the private `@sparq/client` package. No runtime code.

import type { Store as ShippedStore, SolutionCursor as ShippedCursor } from '../wasm/sparq_wasm.js';
import type {
  Store as SharedStore,
  SolutionCursor as SharedCursor,
} from '@sparq/client/wasm-store';

// `Interchangeable<A, B>` is `true` only when `A` is assignable to `B` AND `B` to `A`, i.e.
// the two structural surfaces are mutually substitutable. A divergence on either side (a
// method added/removed, a signature changed) breaks one of the `extends` checks and trips
// this typecheck.
type Assignable<A, B> = A extends B ? true : false;
type Interchangeable<A, B> = Assignable<A, B> extends true
  ? Assignable<B, A> extends true
    ? true
    : false
  : false;

// `Expect<true>` fails to typecheck if its argument is not exactly `true` — so each surface
// below must be interchangeable between the shipped (`@sparq-org/sparq`) and shared
// (`@sparq/client`) copies.
type Expect<T extends true> = T;

// The `Store` instance surface, the `SolutionCursor` it returns, and the static (constructor)
// surface — the `new Store()` constructor plus the `load` / `loadWithBase` / `loadDataset` /
// `loadCompressed` factories `@sparq-org/sparq`'s `SparqStore` invokes — must all match across the
// two copies.
type _StoreConforms = Expect<Interchangeable<ShippedStore, SharedStore>>;
type _CursorConforms = Expect<Interchangeable<ShippedCursor, SharedCursor>>;
type _StoreCtorConforms = Expect<Interchangeable<typeof ShippedStore, typeof SharedStore>>;

// Export so an unused-type lint/compiler setting does not strip the assertions. Pure types —
// emits nothing.
export type WasmStoreConformance = [_StoreConforms, _CursorConforms, _StoreCtorConforms];
