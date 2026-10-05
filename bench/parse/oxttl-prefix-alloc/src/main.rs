// [OPUS-5] sq-98w7z.3 — oxttl prefixed-name expansion A/B harness.
//
// ONE source file, compiled TWICE against two different oxttl revisions by the
// two thin wrapper packages next to it (`released/`, `upstream/`). Both packages
// point their `[[bin]] path` here, so the measured code is byte-identical and the
// ONLY variable is which oxttl the linker resolved. See README.md.
//
// Nothing here is production code and no sparq crate is involved: the harness
// depends on oxttl + oxrdf only.

use std::env;
use std::fs;
use std::io::Write as _;
use std::process::ExitCode;
use std::time::Instant;

use oxrdf::{NamedOrBlankNode, Term, Triple};
use oxttl::TurtleParser;

// ---------------------------------------------------------------------------
// Allocator selection.
//
// Three mutually-exclusive builds, so a run measures exactly one thing:
//   * default             — the system allocator, wall-clock timing.
//   * --features mimalloc — the allocator `sparq-cli ingest` actually ships with.
//   * --features count-alloc — a counting shim over the system allocator. Its
//     counts come from no clock, so unlike the timings they are stable across
//     repeated runs of the SAME binary on the same corpus, whatever the box is
//     doing. They are still a property of that build — toolchain, target,
//     dependency resolution and features all feed them — so quote a count with
//     the configuration that produced it, not as a box-independent constant.
//     It perturbs timing, so this build's wall-clock is not reported.
// `count-alloc` wins over `mimalloc` because only one `#[global_allocator]` may
// exist; the wrapper manifests never enable both, this is belt-and-braces.
// ---------------------------------------------------------------------------

#[cfg(feature = "count-alloc")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub static ALLOCS: AtomicU64 = AtomicU64::new(0);
    pub static REALLOCS: AtomicU64 = AtomicU64::new(0);
    pub static BYTES: AtomicU64 = AtomicU64::new(0);

    pub struct Counting;

    // SAFETY: every method forwards, unchanged, to `System`; the counters are the
    // only added effect and they touch no allocator state.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            REALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size.saturating_sub(layout.size()) as u64, Ordering::Relaxed);
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    pub fn snapshot() -> (u64, u64, u64) {
        (
            ALLOCS.load(Ordering::Relaxed),
            REALLOCS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
        )
    }
}

#[cfg(feature = "count-alloc")]
#[global_allocator]
static GLOBAL: counting::Counting = counting::Counting;

#[cfg(all(feature = "mimalloc", not(feature = "count-alloc")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn feature_set() -> &'static str {
    if cfg!(feature = "rdf-12") {
        "rdf-12"
    } else {
        "default"
    }
}

fn allocator_name() -> &'static str {
    if cfg!(feature = "count-alloc") {
        "system+counting-shim"
    } else if cfg!(feature = "mimalloc") {
        "mimalloc"
    } else {
        "system"
    }
}

// ---------------------------------------------------------------------------
// Result digest.
//
// The bead's invariant is "parse is byte/count-identical". This is how the two
// builds are held to it: an order-sensitive FNV-1a over the RAW component
// strings of every triple (not over any `Display` impl, which could itself
// differ between the two oxttl revisions and would turn a serializer change into
// a false parse divergence). Equal digest + equal count across the two builds
// means the two parsers produced the same triple stream.
// ---------------------------------------------------------------------------

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn feed(h: &mut u64, s: &str) {
    for b in s.as_bytes() {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(FNV_PRIME);
    }
    // Field separator, so ("ab","c") and ("a","bc") do not collide.
    *h ^= 0xff;
    *h = h.wrapping_mul(FNV_PRIME);
}

fn feed_term(h: &mut u64, t: &Term) {
    match t {
        Term::NamedNode(n) => {
            feed(h, "I");
            feed(h, n.as_str());
        }
        Term::BlankNode(b) => {
            feed(h, "B");
            feed(h, b.as_str());
        }
        Term::Literal(l) => {
            feed(h, "L");
            feed(h, l.value());
            feed(h, l.datatype().as_str());
            feed(h, l.language().unwrap_or(""));
        }
        // Only reachable with `rdf-12`, which is the feature set the sparq
        // workspace actually enables on oxttl.
        #[cfg(feature = "rdf-12")]
        Term::Triple(t) => {
            feed(h, "T");
            feed_triple(h, t);
        }
    }
}

fn feed_triple(h: &mut u64, t: &Triple) {
    match &t.subject {
        NamedOrBlankNode::NamedNode(n) => {
            feed(h, "I");
            feed(h, n.as_str());
        }
        NamedOrBlankNode::BlankNode(b) => {
            feed(h, "B");
            feed(h, b.as_str());
        }
    }
    feed(h, t.predicate.as_str());
    feed_term(h, &t.object);
}

// ---------------------------------------------------------------------------
// Corpus generator.
//
// The sq-wrn61 corpus (`bench/parse/data/wikidata-slice.ttl`, ~60 MB) is
// gitignored and not redistributable, so the harness carries a DETERMINISTIC
// generator that reproduces its salient shape: the wd:/wdt: prefix set, and an
// object column that is overwhelmingly prefixed names rather than IRIREFs. That
// is what makes the corpus exercise `resolve_local_name`, which is the function
// under test. `bench` accepts any real .ttl too — pass the real corpus when you
// have it and label the row accordingly.
// ---------------------------------------------------------------------------

const PREAMBLE: &str = "@prefix wd: <http://www.wikidata.org/entity/> .\n\
                        @prefix wdt: <http://www.wikidata.org/prop/direct/> .\n\
                        @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                        @prefix schema: <http://schema.org/> .\n\
                        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\n";

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        // Numerical Recipes LCG — deterministic and dependency-free; the corpus
        // only needs a fixed pseudo-random shape, not statistical quality.
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 16
    }
}

fn generate(target_bytes: usize, out: &str) -> Result<(), String> {
    let file = fs::File::create(out).map_err(|e| format!("create {}: {}", out, e))?;
    let mut w = std::io::BufWriter::with_capacity(1 << 20, file);
    w.write_all(PREAMBLE.as_bytes()).map_err(|e| e.to_string())?;
    let mut written = PREAMBLE.len();
    let mut rng = Lcg(0x5eed_5eed_5eed_5eed);
    let mut entity: u64 = 1;
    let mut triples: u64 = 0;
    let mut buf = String::with_capacity(4096);
    while written < target_bytes {
        buf.clear();
        // One subject block, `;`-separated — the shape oxttl's serializer emits
        // and the shape the real slice has.
        buf.push_str(&format!("wd:Q{} wdt:P31 wd:Q{} ;\n", entity, 5 + rng.next() % 4096));
        triples += 1;
        let extra = 3 + rng.next() % 6;
        for _ in 0..extra {
            let r = rng.next() % 10;
            if r < 7 {
                // Prefixed-name object: the path under test.
                buf.push_str(&format!(
                    "    wdt:P{} wd:Q{} ;\n",
                    17 + rng.next() % 2048,
                    rng.next() % 8_000_000
                ));
            } else if r < 8 {
                buf.push_str(&format!(
                    "    wdt:P{} \"{}\"^^xsd:decimal ;\n",
                    569 + rng.next() % 64,
                    rng.next() % 100_000
                ));
            } else if r < 9 {
                buf.push_str(&format!("    rdfs:label \"Entity {}\"@en ;\n", entity));
            } else {
                // A minority of absolute IRIREFs, so the corpus is not degenerate.
                buf.push_str(&format!(
                    "    schema:about <http://example.org/doc/{}> ;\n",
                    rng.next() % 1_000_000
                ));
            }
            triples += 1;
        }
        // Replace the trailing " ;\n" with " .\n\n".
        let cut = buf.len() - 3;
        buf.truncate(cut);
        buf.push_str(" .\n\n");
        w.write_all(buf.as_bytes()).map_err(|e| e.to_string())?;
        written += buf.len();
        entity += 1;
    }
    w.flush().map_err(|e| e.to_string())?;
    println!(
        "wrote {} — {} bytes, {} triples, {} subjects",
        out, written, triples, entity - 1
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Measurement.
// ---------------------------------------------------------------------------

struct Run {
    seconds: f64,
    triples: u64,
    digest: u64,
}

fn parse_once(data: &[u8]) -> Result<Run, String> {
    let start = Instant::now();
    let mut digest = FNV_OFFSET;
    let mut triples = 0_u64;
    for t in TurtleParser::new().for_reader(data) {
        let t = t.map_err(|e| e.to_string())?;
        feed_triple(&mut digest, &t);
        triples += 1;
    }
    Ok(Run {
        seconds: start.elapsed().as_secs_f64(),
        triples,
        digest,
    })
}

fn bench(path: &str, iters: usize, json: Option<&str>) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("read {}: {}", path, e))?;
    let bytes = data.len() as f64;

    // One warm-up pass, discarded: it faults in the pages and settles the
    // allocator's arenas, so the counted/timed passes measure steady state.
    let warm = parse_once(&data)?;

    // Harness bookkeeping is allocated BEFORE the window opens: `with_capacity`
    // reserves the whole vector up front, so neither it nor any `push` below can
    // allocate inside the counted region. Otherwise a once-per-invocation harness
    // allocation would be divided by `iters` and reported as parser work, making
    // `allocs/parse` depend on the iteration count.
    let mut runs = Vec::with_capacity(iters);

    #[cfg(feature = "count-alloc")]
    let before = counting::snapshot();

    for _ in 0..iters {
        runs.push(parse_once(&data)?);
    }

    #[cfg(feature = "count-alloc")]
    let after = counting::snapshot();

    for r in &runs {
        if r.digest != warm.digest || r.triples != warm.triples {
            return Err("non-deterministic parse across iterations".to_owned());
        }
    }

    let mut secs: Vec<f64> = runs.iter().map(|r| r.seconds).collect();
    secs.sort_by(|a, b| a.partial_cmp(b).expect("no NaN timings"));
    let min = secs[0];
    let median = secs[secs.len() / 2];

    // Which oxttl this binary linked against. Carried in the wrapper package's
    // `description` field so neither wrapper needs a build.rs to report it.
    let build = env!("CARGO_PKG_NAME");
    let oxttl = env!("CARGO_PKG_DESCRIPTION");

    println!("## {} (oxttl {})", build, oxttl);
    println!();
    println!("- corpus: `{}` — {} bytes, {} triples", path, data.len(), warm.triples);
    println!("- oxttl features: {}", feature_set());
    println!("- allocator: {}", allocator_name());
    println!("- iterations: {} (warm-up discarded)", iters);
    println!("- digest (FNV-1a over parsed components): `{:#018x}`", warm.digest);
    println!();
    println!("| build | oxttl | s (min) | s (median) | MB/s | Mtriples/s |");
    println!("|---|---|---|---|---|---|");
    println!(
        "| {} | {} | {:.3} | {:.3} | {:.0} | {:.2} |",
        build,
        oxttl,
        min,
        median,
        bytes / min / 1e6,
        warm.triples as f64 / min / 1e6
    );

    #[cfg(feature = "count-alloc")]
    {
        let allocs = (after.0 - before.0) / iters as u64;
        let reallocs = (after.1 - before.1) / iters as u64;
        let abytes = (after.2 - before.2) / iters as u64;
        println!();
        println!("| build | allocs/parse | reallocs/parse | allocs/triple | bytes/triple |");
        println!("|---|---|---|---|---|");
        println!(
            "| {} | {} | {} | {:.3} | {:.1} |",
            build,
            allocs,
            reallocs,
            allocs as f64 / warm.triples as f64,
            abytes as f64 / warm.triples as f64
        );
        println!();
        println!("Timings from a `count-alloc` build are NOT comparable — the counting shim");
        println!("adds two relaxed atomics per allocation. Quote the counts, not the seconds.");
    }

    if let Some(json_path) = json {
        // Dependency-free emit, mirroring bench/parse's `--json` convention.
        #[cfg(feature = "count-alloc")]
        let extra = format!(
            ",\n  \"allocs_per_parse\": {},\n  \"reallocs_per_parse\": {}",
            (after.0 - before.0) / iters as u64,
            (after.1 - before.1) / iters as u64
        );
        #[cfg(not(feature = "count-alloc"))]
        let extra = String::new();
        let doc = format!(
            "{{\n  \"note\": \"NON-CANONICAL — whatever box this ran on; see bench/parse/oxttl-prefix-alloc/README.md\",\n  \
             \"build\": \"{}\",\n  \"oxttl\": \"{}\",\n  \"allocator\": \"{}\",\n  \"corpus\": \"{}\",\n  \
             \"bytes\": {},\n  \"triples\": {},\n  \"digest\": \"{:#018x}\",\n  \
             \"seconds_min\": {:.6},\n  \"seconds_median\": {:.6}{}\n}}\n",
            build, oxttl, allocator_name(), path, data.len(), warm.triples, warm.digest, min, median, extra
        );
        fs::write(json_path, doc).map_err(|e| format!("write {}: {}", json_path, e))?;
    }
    Ok(())
}

fn usage() -> String {
    format!(
        "usage:\n  {0} gen <megabytes> <out.ttl>\n  {0} bench <file.ttl> [--iters N] [--json <path>]\n",
        env!("CARGO_PKG_NAME")
    )
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gen") => {
            let mb: usize = args
                .get(1)
                .ok_or_else(usage)?
                .parse()
                .map_err(|e| format!("bad megabytes: {}", e))?;
            let out = args.get(2).ok_or_else(usage)?;
            generate(mb * 1_000_000, out)
        }
        Some("bench") => {
            let path = args.get(1).ok_or_else(usage)?;
            let mut iters = 3;
            let mut json = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--iters" => {
                        i += 1;
                        iters = args
                            .get(i)
                            .ok_or("--iters needs a value")?
                            .parse()
                            .map_err(|e| format!("bad --iters: {}", e))?;
                    }
                    "--json" => {
                        i += 1;
                        json = Some(args.get(i).ok_or("--json needs a path")?.clone());
                    }
                    other => return Err(format!("unknown argument {}\n{}", other, usage())),
                }
                i += 1;
            }
            bench(path, iters, json.as_deref())
        }
        _ => Err(usage()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}", e);
            ExitCode::FAILURE
        }
    }
}
