<!-- [FABLE-5] sq-0kq6k — streamed-vs-buffered CONSTRUCT/DESCRIBE response measurement. -->
# Streamed vs buffered CONSTRUCT / DESCRIBE responses

Measures what wiring `sparq-server`'s CONSTRUCT / DESCRIBE response body through a chunk sink
(`stream_graph_result`, `crates/sparq-server/src/http.rs`) actually buys, against the real
buffered path — **peak RSS** and **time-to-first-byte**. Registered as `construct-stream` in
[`bench/benchmarks.toml`](../benchmarks.toml).

```bash
cargo build --release -p sparq-server
bash bench/construct-stream/run.sh --smoke   # gates only; exit 0 = green
bash bench/construct-stream/run.sh           # full run (prints the table)
```

## The control is the real buffered path, not a flag

A `HEAD` on the same CONSTRUCT runs the same engine call and the same serialiser and then
renders the whole document into one `String` — which is exactly what a `GET` did before this
change. (`HEAD` must advertise the `Content-Length` a `GET` would carry, so it cannot stream;
that is why the streaming change deliberately left it on the buffered path.) So:

| arm | request | what it does |
|---|---|---|
| `buffered` | `HEAD` | renders the whole document, then answers |
| `streamed` | `GET` | answers after the first chunk, streams the rest |

Each arm gets its **own server process**: `VmHWM` is a monotonic high-water mark, so one
process cannot report a before and an after.

Only the **TTFB** and **peak RSS** columns are like-for-like. `HEAD` transfers no body, so the
two `total` columns are not comparable to each other and the script says so in its own output.

## Gates before stopwatch

No timing is reported until the two arms are proven to agree: the streamed body's byte length
must equal the `Content-Length` the buffered `HEAD` advertises, and (for N-Triples) the body
must carry exactly the expected triple count. The run then corrupts the body and asserts the
length gate reds — the non-vacuity self-check. Any red gate exits non-zero with no timings.

The triple-count gate has already earned its keep: it caught a defect in this harness's own
fanout template, where template clauses that could coincide collapsed (a CONSTRUCT result is
an RDF **graph**, i.e. a set) and silently under-produced.

## `CS_FANOUT` — the knob the RSS question actually turns on

`CS_SUBJECTS` scales the store and the result **together**, so the rendered document stays a
fixed fraction of a process whose high-water mark was already set during load and evaluation.
`CS_FANOUT` emits N triples per matched solution, growing the rendered document **without**
growing the store. A canonical run must include a high-fanout point; see below for why.

## First-read findings (work box — NON-CANONICAL, no timings transcribed)

Taken on a shared work box, so these are directional only and no numbers are recorded here or
anywhere else in the repo.

1. **TTFB moved in the hypothesised direction, repeatably** — the streamed arm reached first
   byte meaningfully sooner than the buffered arm, at two independent corpus shapes (a large
   store at fanout 1, and a smaller store at fanout 8). This is the mechanism working as
   designed: the first chunk leaves as soon as ~64 KiB is rendered rather than after the last
   triple.
2. **Peak RSS did NOT separate — the hypothesis's memory half is UNCONFIRMED.** Even with a
   rendered document in the hundreds of megabytes, the two arms' `VmHWM` were effectively
   equal. The reason is not that the saving is unreal — the whole-document `String` genuinely
   is no longer allocated — but that it is **hidden beneath a larger transient**: the engine
   materialises the entire `Vec<Triple>` result (and, before that, the solution set it is
   instantiated from) before serialisation begins, and that working set sets the process high-
   water mark. By the time rendering starts, the allocator can satisfy the render from pages
   it already holds, so removing it does not lower `VmHWM`.
3. **Therefore the real memory lever is upstream, not here.** Peak RSS on a large CONSTRUCT is
   dominated by materialising the result graph, not by rendering it. Streaming the response
   body is a precondition for fixing that (a streaming producer needs a streaming consumer),
   but on its own it is a TTFB change, and it should be described that way.

## What a canonical run still needs

- A quiet-box gather (`bench/ec2-bench.sh` conventions) on the canonical perf host, at both
  fanout 1 and a high fanout, with `CS_ACCEPT` covering `application/n-triples` **and**
  `text/turtle` (the Turtle writer carries grouping state across chunks, so its TTFB profile
  can differ).
- A regime where the store is out-of-core (memory-mapped) rather than heap-resident, which is
  where the result-graph transient stops dominating and the rendering saving could surface.
- Nothing here is a dominance claim, and none of these readings belong on the dashboard or the
  site.
