// [OPUS-4.8] sq-iwhl8 (#1116) — the RDF/JS Stream-spec surface for `@sparq-org/sparq`:
// a browser-safe quad `Stream`, and a `Source` / `Sink` / `Store` adapter over a
// {@link SparqStore} so the engine is usable wherever an RDF/JS Stream interface is expected
// (https://rdf.js.org/stream-spec/). This completes the term/factory/dataset surface with the
// Stream/Source/Sink corner the conformance audit flagged.
//
// `Source.match` re-views the store's `matchStream(...)` GENERATOR — which pulls solutions from
// the engine in ~64 KiB chunks (see {@link SparqStore.queryBindingsStream}) rather than
// materialising the whole result — as the spec's EVENT-BASED `Stream` (a `data`-per-quad-then-
// `end` EventEmitter). The stream PULLS the next quad from that iterator on demand, so a very
// large `match` is never held whole on the JS side. It does this WITHOUT pulling in
// `node:events`, so it runs unchanged in the browser.
//
// [SONNET-4.6] sq-y9v8n hardens the WRITE half of that surface, symmetrically:
//   * `import`/`remove` apply the incoming stream in `chunkSize` batches (default 1024) instead
//     of buffering every quad, so a huge parser stream costs one chunk of JS heap, not all of it;
//     `chunkSize: 0` restores the all-or-nothing single-delta form.
//   * every mutation (`import`/`remove`/`removeMatches`/`deleteGraph`) returns a
//     `QuadStream.pending()` out-channel, so `end` fires when the delta has ACTUALLY been
//     applied — a plain `new QuadStream()` auto-emits `end` on the next microtask, which let a
//     consumer's `end` handler observe the store before the write landed.
//   * `deleteGraph` validates its argument (see `toGraphName`) rather than letting a `Variable`
//     graph fall through to `match`'s "every graph" wildcard and delete the whole dataset.
import type * as RDF from '@rdfjs/types';
import { DefaultGraph, NamedNode } from './terms.js';
import type { SparqStore } from './store.js';

const DEFAULT_GRAPH = new DefaultGraph();

/**
 * [SONNET-4.6] sq-y9v8n — default quads-per-delta while CONSUMING an incoming quad `Stream`
 * ({@link SparqSource.import} / {@link SparqSource.remove}). Bounds the JS-side buffer to one
 * chunk instead of the whole stream, while still batching enough quads that each
 * {@link SparqStore.applyDelta} stays an O(batch) overlay write rather than a per-quad call.
 */
const DEFAULT_CHUNK_SIZE = 1024;

/** RDF/JS `match()` term position: a concrete term, or `null`/`undefined` for a wildcard. */
type MatchTerm = RDF.Term | null | undefined;

/** Options controlling how a {@link SparqSource} consumes an incoming quad `Stream`. */
export interface SparqSourceOptions {
  /**
   * Quads applied per delta while consuming an incoming stream ({@link SparqSource.import} /
   * {@link SparqSource.remove}). Defaults to 1024.
   *
   * - `n > 0` — apply every `n` buffered quads (plus the remainder on `end`), so at most `n`
   *   quads are held on the JS side no matter how long the stream is. Quads applied before a
   *   later `error` STAY applied (the same incremental contract as N3.js's `Store.import`).
   * - `0` — buffer the WHOLE stream and apply it as ONE delta on `end`, so nothing is written
   *   if the stream errors part-way. All-or-nothing, at the cost of holding every quad.
   *
   * Chunking bounds MEMORY; it is not back-pressure. An RDF/JS `Stream` pushes `data` events and
   * sparq's apply is synchronous, so there is no await point at which the producer could be
   * slowed down — the win is that the JS heap never holds more than one chunk.
   */
  chunkSize?: number;
}

/** Normalizes anything thrown into an `Error` for an RDF/JS `error` event. */
function toError(err: unknown): Error {
  return err instanceof Error ? err : new Error(String(err));
}

/**
 * Validates + normalizes an RDF/JS `Store.deleteGraph` argument into the graph term the
 * store's `match`/delta primitives address, throwing on an argument that cannot NAME a graph.
 *
 * The string form follows the convention RDF/JS consumers inherit from N3.js's `termFromId`
 * (which is what N3's own `Store.deleteGraph(graph)` accepts): `''` is the DEFAULT graph and any
 * other string is a named-graph IRI. sparq diverges from N3 in one place — N3 reads a leading
 * `_:` as a blank-node graph name, whereas sparq treats the whole string as an IRI; pass an
 * actual `BlankNode` term for a blank-node-named graph.
 *
 * A `Variable` is rejected rather than treated as a wildcard: {@link SparqStore.match} reads a
 * variable graph position as "every graph", so silently accepting one would turn
 * `deleteGraph(someVariable)` into a delete of the ENTIRE dataset — the destructive reading of an
 * ambiguous argument. `Literal`/`Quad`/`null`/`undefined` are rejected for the same reason (they
 * would otherwise match nothing and report a successful no-op delete).
 */
function toGraphName(graph: RDF.Quad_Graph | string): RDF.Quad_Graph {
  if (typeof graph === 'string') return graph === '' ? DEFAULT_GRAPH : new NamedNode(graph);
  const termType: string | undefined = (graph as { termType?: string } | null | undefined)?.termType;
  if (termType === 'NamedNode' || termType === 'BlankNode' || termType === 'DefaultGraph') {
    return graph as RDF.Quad_Graph;
  }
  throw new TypeError(
    `deleteGraph() cannot name a graph with a ${String(termType ?? typeof graph)} — pass a NamedNode/BlankNode/DefaultGraph term, an IRI string, or '' for the default graph`,
  );
}

/**
 * An event listener registered on {@link QuadStream}. The RDF/JS Stream events this class emits
 * (`data` / `end` / `error`) all carry AT MOST ONE argument (the quad, or the error — `end`
 * carries none), so the listener takes a single optional argument, matching how {@link
 * QuadStream.emit} invokes it.
 */
type StreamListener = (arg?: unknown) => void;

/**
 * A minimal, browser-safe RDF/JS `Stream<Quad>` that PULLS quads lazily from an underlying
 * iterable (a generator over the store's match) rather than holding a materialised array. It
 * implements exactly the `EventEmitter` subset the RDF/JS Stream spec exercises (`on` / `once` /
 * `removeListener` / `off` / `emit` / `read`) rather than depending on `node:events`, so it runs
 * unchanged in the browser. It pulls one quad at a time: `read()` returns the next quad (or
 * `null` at end), and on the next microtask it drains the iterator, emitting a `data` event per
 * quad then `end` (or `error` if the iterator throws). Listeners attached synchronously after
 * construction still receive every event.
 *
 * It implements the `EventEmitter` SUBSET the RDF/JS Stream spec actually exercises rather than
 * the full `node:events` surface (so it stays browser-safe); call sites bridge to the full
 * `RDF.Stream` type via {@link asStream}.
 */
export class QuadStream {
  readonly #iterator: Iterator<RDF.Quad>;
  #done = false;
  #flushed = false;
  readonly #listeners = new Map<string, Set<StreamListener>>();

  /**
   * @param source the quads to stream — any iterable (a generator/iterator-backed source is
   *   pulled lazily one quad at a time; an array is iterated without being copied). Defaults to
   *   an empty stream (for the consume/`removeMatches` paths that only ever emit `end`/`error`).
   */
  constructor(source: Iterable<RDF.Quad> = []) {
    this.#iterator = source[Symbol.iterator]();
    queueMicrotask(() => this.#flush());
  }

  /**
   * [SONNET-4.6] sq-y9v8n — an OUT-CHANNEL stream: one that carries no quads and emits NOTHING
   * on its own, so the producer decides when `end`/`error` fires.
   *
   * A plain `new QuadStream()` auto-emits `end` on the next microtask (correct for an empty quad
   * source). Using that as the return value of a *mutation* — `import` / `remove` /
   * `removeMatches` / `deleteGraph` — makes the consumer's `end` listener fire on that microtask,
   * i.e. BEFORE the mutation the consumer is waiting for has been applied. Whenever the work
   * finishes later than that first microtask (any stream that emits on a timer, a fetch, or a
   * parser), `await`-ing `end` then observing the store reads STALE state. Those methods return
   * a `pending()` stream and emit `end` themselves once the delta is actually applied.
   */
  static pending(): QuadStream {
    const stream = new QuadStream();
    // Suppress the queued auto-flush: `#flush` is a no-op once `#flushed` is set, so the
    // constructor's microtask does nothing and only the producer's explicit `emit` is seen.
    stream.#flushed = true;
    return stream;
  }

  /** Pulls the next quad from the underlying source, or `null` once exhausted (per the spec). */
  read(): RDF.Quad | null {
    if (this.#done) return null;
    const next = this.#iterator.next();
    if (next.done) {
      this.#done = true;
      return null;
    }
    return next.value;
  }

  on(event: string | symbol, listener: StreamListener): this {
    const key = String(event);
    let set = this.#listeners.get(key);
    if (!set) this.#listeners.set(key, (set = new Set()));
    set.add(listener);
    return this;
  }

  once(event: string | symbol, listener: StreamListener): this {
    const wrapper: StreamListener = (arg) => {
      this.removeListener(event, wrapper);
      listener(arg);
    };
    return this.on(event, wrapper);
  }

  removeListener(event: string | symbol, listener: StreamListener): this {
    this.#listeners.get(String(event))?.delete(listener);
    return this;
  }

  off(event: string | symbol, listener: StreamListener): this {
    return this.removeListener(event, listener);
  }

  emit(event: string | symbol, arg?: unknown): boolean {
    const set = this.#listeners.get(String(event));
    if (!set || set.size === 0) return false;
    // Iterate the listener Set directly: a `once` wrapper deletes itself before invoking the
    // user callback, and deleting the current entry mid-iteration is well-defined for a Set.
    for (const listener of set) listener(arg);
    return true;
  }

  /** Drains the underlying iterator on the microtask, emitting `data` per quad then `end`. */
  #flush(): void {
    if (this.#flushed) return;
    this.#flushed = true;
    try {
      for (let next = this.#iterator.next(); !next.done; next = this.#iterator.next()) {
        this.#done = false;
        this.emit('data', next.value);
      }
      this.#done = true;
      this.emit('end');
    } catch (err) {
      this.#done = true;
      this.emit('error', err instanceof Error ? err : new Error(String(err)));
    }
  }
}

/**
 * Bridges a browser-safe {@link QuadStream} (which implements only the `EventEmitter` subset the
 * RDF/JS Stream spec exercises) to the full `RDF.Stream<Quad>` interface (whose nominal type
 * extends the whole `node:events` `EventEmitter`). The subset is exactly what consumers use, so
 * the cast is faithful — see the same pattern in `dataset.ts`.
 */
function asStream(s: QuadStream): RDF.Stream<RDF.Quad> {
  return s as unknown as RDF.Stream<RDF.Quad>;
}

/**
 * [OPUS-4.8] sq-iwhl8 (#1116) — an RDF/JS **`Source`** + **`Sink`** + **`Store`** adapter over a
 * {@link SparqStore}. `Source.match` returns a quad `Stream` (the spec's event-based shape, not
 * the store's synchronous `Quad[]`); `Sink.import` / `Store.remove` consume a quad `Stream` and
 * return an EventEmitter that signals `end` / `error`; `removeMatches` / `deleteGraph` mutate by
 * pattern. The backing store stays the source of truth — the adapter only re-views its
 * synchronous primitives as the streaming interface, so a sparq store drops into any RDF/JS
 * pipeline that speaks the Stream spec (a parser sink, a serializer source, …).
 */
export class SparqSource implements RDF.Store<RDF.Quad> {
  readonly #store: SparqStore;
  readonly #chunkSize: number;

  /**
   * @param store the backing {@link SparqStore} (stays the source of truth).
   * @param options see {@link SparqSourceOptions} — `chunkSize` sets the default quads-per-delta
   *   for {@link import} / {@link remove} (1024; `0` buffers the whole stream).
   */
  constructor(store: SparqStore, options: SparqSourceOptions = {}) {
    this.#store = store;
    this.#chunkSize = normalizeChunkSize(options.chunkSize, DEFAULT_CHUNK_SIZE);
  }

  /** The backing {@link SparqStore} — the full SPARQL + delta surface. */
  get store(): SparqStore {
    return this.#store;
  }

  /**
   * RDF/JS `Source.match`: a quad `Stream` of the quads matching the pattern (wildcards are
   * `null`). The stream PULLS lazily from the store's {@link SparqStore.matchStream} generator —
   * which streams solutions from the engine in ~64 KiB chunks — so a very large match is never
   * materialised whole on the JS side.
   */
  match(subject?: MatchTerm, predicate?: MatchTerm, object?: MatchTerm, graph?: MatchTerm): RDF.Stream<RDF.Quad> {
    return asStream(new QuadStream(this.#store.matchStream(subject, predicate, object, graph)));
  }

  /**
   * RDF/JS `Sink.import` / `Store` insert: consumes the quad `Stream` and applies its quads as
   * O(batch) deltas — one per `chunkSize` quads, so a very large stream is never held whole on
   * the JS side (see {@link SparqSourceOptions.chunkSize}, and pass `0` for the all-or-nothing
   * single-delta-on-`end` form). Returns an EventEmitter that emits `end` once every quad has
   * been applied, or `error` if the source stream or an apply fails.
   *
   * Because chunks land WHILE the source stream is still emitting, do not `import` a stream that
   * reads from the same store you are importing into unless you set `chunkSize: 0`.
   *
   * @param options per-call override of the constructor's {@link SparqSourceOptions}.
   */
  import(stream: RDF.Stream<RDF.Quad>, options: SparqSourceOptions = {}): RDF.Stream<RDF.Quad> {
    return this.#consume(stream, (quads) => this.#store.addQuads(quads), options);
  }

  /**
   * RDF/JS `Store.remove`: consumes the quad `Stream` and removes its quads as O(batch) deltas,
   * chunked exactly as {@link import} is.
   *
   * @param options per-call override of the constructor's {@link SparqSourceOptions}.
   */
  remove(stream: RDF.Stream<RDF.Quad>, options: SparqSourceOptions = {}): RDF.Stream<RDF.Quad> {
    return this.#consume(stream, (quads) => this.#store.removeQuads(quads), options);
  }

  /**
   * RDF/JS `Store.removeMatches`: removes every quad matching the pattern; emits `end` once the
   * removal has been applied (never before it — see {@link QuadStream.pending}).
   *
   * Unlike {@link import}, this materialises the matched quads before removing them: the removal
   * would otherwise mutate the store while its own match cursor is still being drained.
   */
  removeMatches(subject?: MatchTerm, predicate?: MatchTerm, object?: MatchTerm, graph?: MatchTerm): RDF.Stream<RDF.Quad> {
    const out = QuadStream.pending();
    queueMicrotask(() => {
      try {
        const toRemove = this.#store.match(subject, predicate, object, graph);
        if (toRemove.length > 0) this.#store.removeQuads(toRemove);
        out.emit('end');
      } catch (err) {
        out.emit('error', toError(err));
      }
    });
    return asStream(out);
  }

  /**
   * RDF/JS `Store.deleteGraph`: removes every quad in the given graph, emitting `end` once the
   * removal has been applied. A `NamedNode`/`BlankNode` term (or a non-empty string, read as a
   * named-graph IRI) targets that named graph; a `DefaultGraph` term or `''` targets the default
   * graph — the same string convention N3.js's `Store.deleteGraph` accepts, bar a leading `_:`
   * (sparq reads that as an IRI, not a blank-node graph name; pass a `BlankNode` term instead).
   *
   * An argument that cannot NAME a graph — a `Variable` (which the store's `match` would read as
   * "every graph", making this delete the whole dataset), a `Literal`, or a missing argument —
   * emits `error` and deletes nothing.
   */
  deleteGraph(graph: RDF.Quad_Graph | string): RDF.Stream<RDF.Quad> {
    let graphName: RDF.Quad_Graph;
    try {
      graphName = toGraphName(graph);
    } catch (err) {
      const out = QuadStream.pending();
      // Deferred so the caller's `.on('error', …)` — attached after this returns — still sees it.
      queueMicrotask(() => out.emit('error', toError(err)));
      return asStream(out);
    }
    return this.removeMatches(undefined, undefined, undefined, graphName);
  }

  /**
   * Drives an incoming quad `Stream` into `apply`, in `chunkSize`-quad batches (or one batch on
   * `end` when `chunkSize` is 0). At most one chunk is buffered on the JS side.
   */
  #consume(
    stream: RDF.Stream<RDF.Quad>,
    apply: (quads: RDF.Quad[]) => void,
    options: SparqSourceOptions,
  ): RDF.Stream<RDF.Quad> {
    const out = QuadStream.pending();
    const chunkSize = normalizeChunkSize(options.chunkSize, this.#chunkSize);
    let buffered: RDF.Quad[] = [];
    let failed = false;

    /** Terminates the consume with `error`, once — no `end` follows, and no further applies. */
    const fail = (err: unknown): void => {
      if (failed) return;
      failed = true;
      buffered = [];
      out.emit('error', toError(err));
    };

    stream.on('data', (quad: RDF.Quad) => {
      if (failed) return;
      buffered.push(quad);
      if (chunkSize === 0 || buffered.length < chunkSize) return;
      const chunk = buffered;
      buffered = [];
      try {
        apply(chunk);
      } catch (err) {
        fail(err);
      }
    });
    stream.on('error', (err: Error) => fail(err));
    stream.on('end', () => {
      if (failed) return;
      try {
        if (buffered.length > 0) apply(buffered);
        buffered = [];
        out.emit('end');
      } catch (err) {
        fail(err);
      }
    });
    return asStream(out);
  }
}

/** Validates a {@link SparqSourceOptions.chunkSize}, falling back to `fallback` when unset. */
function normalizeChunkSize(chunkSize: number | undefined, fallback: number): number {
  if (chunkSize === undefined) return fallback;
  if (!Number.isSafeInteger(chunkSize) || chunkSize < 0) {
    throw new RangeError(`chunkSize must be a non-negative safe integer (0 = buffer the whole stream), got ${String(chunkSize)}`);
  }
  return chunkSize;
}
