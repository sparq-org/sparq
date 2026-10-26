//! [OPUS-4.8] Dictionary (term-index) BASELINE harness — bead sq-9w0t.
//!
//! `research/term-index-compression.md` §2 records that "sparq has NO measured
//! dictionary bytes/term or load-throughput baseline yet" and that "establishing
//! that baseline is itself the first action (§3)". This harness IS that action: it
//! reports the SHIPPED dictionary's bytes-per-term and dict-construction/load
//! throughput on representative Wikidata-shaped + Uniprot-shaped vocabularies, so
//! every later dictionary-compression %-claim — A1 IRI prefix factoring (sq-xhwf),
//! A2 extended inline ValueIds, A3 lang/datatype interning, the prototype FSST/PFC
//! tier — is gated on OUR repo's measured baseline rather than borrowed prior-art
//! numbers (the empirical-honesty prerequisite).
//!
//! It measures the production interner via `Graph::load_reader_parallel` (the exact
//! path `sparq-cli ingest` uses), reads its footprint from the public
//! `Dict::heap_bytes()` / `Dict::len()` (the same `B/term` the CLI load summary
//! prints), and adds a per-term-class composition + IRI prefix/suffix split so each
//! lever can be sized against the measured arena (not a quoted ratio).
//!
//! Subcommands (the angle-bracket placeholders are literal CLI args):
//!
//! ```text
//! gen <entities> <out-dir>   write two deterministic N-Triples vocabularies into
//!                            <out-dir>: wikidata.nt (wd/wdt-prefixed entity +
//!                            property IRIs, mixed literals) and uniprot.nt (the
//!                            most prefix-rich class — long purl.uniprot.org IRIs).
//!                            Fixed-seed SplitMix64, so byte-for-byte reproducible.
//! bench <file.nt>            load via the production parallel ingest path, then
//!                            report a markdown table: bytes/term (arena +
//!                            blob/browser mode), per-class composition, the IRI
//!                            prefix/suffix split that gates A1, and dict-build
//!                            throughput (terms/s, MB/s over the input bytes).
//! selftest                   tiny in-process invariant checks (CI-runnable; no
//!                            dataset, no wall-clock claim).
//! ```
//!
//! Numbers + analysis land in research/term-index-compression.md §3. Wall-clock
//! throughput is QUIET-BOX-sensitive (this box is frequently busy); the bytes/term
//! and composition figures are DETERMINISTIC and load-robust — trust those as the
//! gate, and report throughput only from an idle run.

use sparq_core::dict::TermParts;
use sparq_core::Graph;
use std::io::Write;
use std::time::Instant;

const ITERS: usize = 3;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => gen(args[2].parse().expect("entities must be a u32"), &args[3]),
        Some("bench") => bench(&args[2]),
        Some("selftest") => selftest(),
        _ => {
            eprintln!("usage: dict-baseline gen <entities> <out-dir> | bench <file.nt> | selftest");
            std::process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// deterministic dataset generation
// ---------------------------------------------------------------------------

/// SplitMix64 — the same fixed-seed generator family `bench/parse` and
/// `crates/sparq-bench` use, so every dataset is byte-for-byte reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 1
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
}

/// Writes two deterministic N-Triples vocabularies into `dir`:
///
/// * `wikidata.nt` — Wikidata-shaped: `wd:Q…` entity subjects, `wdt:P…` property
///   predicates, a mix of IRI objects (other entities), language-tagged `rdfs:label`
///   literals, plain `xsd:string` descriptions, and `xsd:integer` / `xsd:decimal` /
///   `xsd:dateTime` statement values. This exercises A1 (the `wd`/`wdt` namespaces
///   are the redundancy prefix-factoring targets), A2 (the numeric/date literals are
///   the inline-ValueId targets) and A3 (the language tags + repeated datatype IRIs).
///
/// * `uniprot.nt` — the most prefix-rich class (the design doc's lower-bound dataset):
///   long `http://purl.uniprot.org/uniprot/…` subject IRIs and a handful of
///   `purl.uniprot.org/core/` predicates, so the per-IRI namespace dwarfs the local
///   suffix — the case where A1 prefix factoring has the most to gain.
fn gen(entities: u32, dir: &str) {
    let entities = entities.max(1);
    std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {dir}: {e}"));

    gen_wikidata(entities, &format!("{dir}/wikidata.nt"));
    gen_uniprot(entities, &format!("{dir}/uniprot.nt"));
}

fn gen_wikidata(entities: u32, path: &str) {
    const WD: &str = "http://www.wikidata.org/entity/";
    const WDT: &str = "http://www.wikidata.org/prop/direct/";
    const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
    const SCHEMA: &str = "http://schema.org/";
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
    const LANGS: [&str; 5] = ["en", "de", "fr", "es", "zh"];

    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut w = std::io::BufWriter::new(
        std::fs::File::create(path).unwrap_or_else(|e| panic!("create {path}: {e}")),
    );
    let mut triples = 0usize;
    for i in 0..entities {
        let s = format!("<{WD}Q{i}>");
        // instance-of another entity (IRI object — exercises SO-shared vocab)
        writeln!(w, "{s} <{WDT}P31> <{WD}Q{}> .", rng.below(entities)).unwrap();
        // labels in several languages (language-tagged literals — A3)
        for k in 0..3 {
            let lang = LANGS[(rng.below(LANGS.len() as u32)) as usize];
            writeln!(w, "{s} <{RDFS}label> \"label {i} {k}\"@{lang} .").unwrap();
        }
        // a plain xsd:string description (the residual after A2 inlines numerics)
        writeln!(
            w,
            "{s} <{SCHEMA}description> \"description of entity {i}\" ."
        )
        .unwrap();
        // statement values: integer / decimal / dateTime (A2 inline-ValueId targets)
        writeln!(
            w,
            "{s} <{WDT}P1082> \"{}\"^^<{XSD}integer> .",
            rng.below(2_000_000)
        )
        .unwrap();
        writeln!(
            w,
            "{s} <{WDT}P2044> \"{}.{}\"^^<{XSD}decimal> .",
            rng.below(8000),
            rng.below(100)
        )
        .unwrap();
        writeln!(
            w,
            "{s} <{WDT}P585> \"{:04}-{:02}-{:02}T00:00:00Z\"^^<{XSD}dateTime> .",
            1900 + rng.below(125),
            1 + rng.below(12),
            1 + rng.below(28)
        )
        .unwrap();
        // a few entity-to-entity links (more SO-shared IRIs)
        for _ in 0..3 {
            writeln!(w, "{s} <{WDT}P279> <{WD}Q{}> .", rng.below(entities)).unwrap();
        }
        triples += 9;
    }
    w.flush().unwrap();
    eprintln!("gen: wikidata-shaped {entities} entities, {triples} triples -> {path}");
}

fn gen_uniprot(entities: u32, path: &str) {
    const UP: &str = "http://purl.uniprot.org/uniprot/";
    const CORE: &str = "http://purl.uniprot.org/core/";
    const TAXON: &str = "http://purl.uniprot.org/taxonomy/";
    const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
    const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

    // Deterministic 6-char UniProt-style accession from an index (A0Q000 style).
    fn accession(i: u32) -> String {
        let a = (b'A' + (i % 26) as u8) as char;
        let b = (b'0' + ((i / 26) % 10) as u8) as char;
        let c = (b'A' + ((i / 260) % 26) as u8) as char;
        let d = (i / 6760) % 1000;
        format!("{a}{b}{c}{d:03}")
    }

    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut w = std::io::BufWriter::new(
        std::fs::File::create(path).unwrap_or_else(|e| panic!("create {path}: {e}")),
    );
    let mut triples = 0usize;
    for i in 0..entities {
        let s = format!("<{UP}{}>", accession(i));
        writeln!(w, "{s} <{RDF}type> <{CORE}Protein> .").unwrap();
        writeln!(w, "{s} <{CORE}organism> <{TAXON}{}> .", rng.below(50_000)).unwrap();
        writeln!(w, "{s} <{CORE}mnemonic> \"{}_HUMAN\" .", accession(i)).unwrap();
        writeln!(
            w,
            "{s} <{CORE}reviewed> \"{}\"^^<{XSD}boolean> .",
            rng.below(2) == 1
        )
        .unwrap();
        writeln!(
            w,
            "{s} <{CORE}created> \"{:04}-{:02}-{:02}\"^^<{XSD}date> .",
            2000 + rng.below(25),
            1 + rng.below(12),
            1 + rng.below(28)
        )
        .unwrap();
        // cross-references to other proteins (more long-prefix SO-shared IRIs)
        for _ in 0..3 {
            writeln!(
                w,
                "{s} <{CORE}interaction> <{UP}{}> .",
                accession(rng.below(entities))
            )
            .unwrap();
        }
        triples += 8;
    }
    w.flush().unwrap();
    eprintln!("gen: uniprot-shaped {entities} entities, {triples} triples -> {path}");
}

// ---------------------------------------------------------------------------
// the measurement
// ---------------------------------------------------------------------------

fn dataset_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Per-term-class composition of the dictionary, summed by walking the public
/// `Dict::iter()` once. `raw_utf8` is the un-deduplicated UTF-8 byte size of the
/// term's string components (IRI = prefix + suffix bytes; literal = value + datatype
/// IRI + lang bytes), i.e. what a naive whole-string store would hold — the
/// denominator each compression lever's %-claim is measured against.
#[derive(Default)]
struct Composition {
    iris: usize,
    literals: usize,
    blanks: usize,
    triples: usize,
    lang_literals: usize,
    iri_prefix_bytes: usize,
    iri_suffix_bytes: usize,
    lit_value_bytes: usize,
    lit_datatype_bytes: usize,
    lit_lang_bytes: usize,
    blank_bytes: usize,
    /// distinct datatype IRIs (the A3 lever's interning target).
    distinct_datatypes: std::collections::HashSet<String>,
    /// distinct IRI namespace prefixes (the A1 lever's interning target).
    distinct_prefixes: std::collections::HashSet<String>,
}

impl Composition {
    fn of(g: &Graph) -> Composition {
        let mut c = Composition::default();
        for (_id, parts) in g.dict().iter() {
            match parts {
                TermParts::Iri { prefix, suffix } => {
                    c.iris += 1;
                    c.iri_prefix_bytes += prefix.len();
                    c.iri_suffix_bytes += suffix.len();
                    c.distinct_prefixes.insert(prefix.to_string());
                }
                TermParts::Lit {
                    value,
                    datatype,
                    lang,
                } => {
                    c.literals += 1;
                    c.lit_value_bytes += value.len();
                    c.lit_datatype_bytes += datatype.len();
                    if let Some(l) = lang {
                        c.lang_literals += 1;
                        c.lit_lang_bytes += l.len();
                    }
                    c.distinct_datatypes.insert(datatype.to_string());
                }
                TermParts::Blank(b) => {
                    c.blanks += 1;
                    c.blank_bytes += b.len();
                }
                TermParts::Triple(_) => c.triples += 1,
            }
        }
        c
    }

    fn raw_utf8(&self) -> usize {
        self.iri_prefix_bytes
            + self.iri_suffix_bytes
            + self.lit_value_bytes
            + self.lit_datatype_bytes
            + self.lit_lang_bytes
            + self.blank_bytes
    }
}

/// Loads `path` via the production parallel ingest path and prints the dictionary
/// baseline: bytes/term (arena + blob/browser mode), per-class composition, the IRI
/// prefix/suffix split (which gates A1 prefix factoring), and dict-build throughput.
fn bench(path: &str) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let name = dataset_name(path);
    let input_mb = bytes.len() as f64 / 1e6;

    // Throughput: median over ITERS of the full production load (parse + intern +
    // index). This is the dict-CONSTRUCTION throughput the bead asks for; it is
    // measured over the SHIPPED `load_reader_parallel`, so it tracks the actual
    // ingest cost, not a synthetic intern-only micro-benchmark.
    let mut secs: Vec<f64> = (0..ITERS)
        .map(|_| {
            let t = Instant::now();
            let g = Graph::load_reader_parallel(std::io::Cursor::new(&bytes), "ntriples")
                .expect("dataset must load");
            let e = t.elapsed().as_secs_f64();
            std::hint::black_box(g.dict().len());
            e
        })
        .collect();
    secs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let load_s = secs[secs.len() / 2];

    // One more load whose dict we keep for the (deterministic) footprint figures.
    let g = Graph::load_reader_parallel(std::io::Cursor::new(&bytes), "ntriples")
        .expect("dataset must load");
    let terms = g.dict().len();
    let triples = g.len();
    let arena_bytes = g.dict().heap_bytes();

    // Blob / browser storage mode: `into_blob` drops the per-`Stored` slot + per-term
    // `Box<str>` for a single concatenated record blob + u32 offsets — the memory-bound
    // (WASM) dictionary. Measuring both modes here gives the two bytes/term numbers the
    // ARCHITECTURE.md storage modes care about.
    let comp = Composition::of(&g);
    let blob = g.into_dict().into_blob();
    let blob_bytes = blob.heap_bytes();
    assert_eq!(blob.len(), terms, "into_blob must preserve term count");

    let raw_utf8 = comp.raw_utf8();
    let avg_prefix = comp.iri_prefix_bytes as f64 / comp.iris.max(1) as f64;
    let avg_suffix = comp.iri_suffix_bytes as f64 / comp.iris.max(1) as f64;

    println!("## dictionary baseline: {name}");
    println!();
    println!(
        "- input: {input_mb:.1} MB N-Triples, {triples} triples, {terms} distinct dictionary terms"
    );
    println!("- load (median of {ITERS}): {load_s:.3}s  =>  {:.2} M triples/s, {:.2} M terms/s, {:.0} MB/s",
        triples as f64 / 1e6 / load_s, terms as f64 / 1e6 / load_s, input_mb / load_s);
    println!();
    println!("### bytes/term (the gate for every later compression %-claim)");
    println!();
    println!("`bytes/term = Dict::heap_bytes() / Dict::len()` — the same figure the CLI load");
    println!("summary prints. `naive whole-string/term` is the un-deduplicated UTF-8 of every");
    println!("term's components (IRI prefix+suffix, literal value+datatype-IRI+lang): the");
    println!("denominator prior-art %-claims are quoted against. sparq's arena is ALREADY below");
    println!("it (IRI namespaces + datatype IRIs are stored ONCE in side tables, not per term),");
    println!(
        "so `vs naive` < 100% reflects the single-storage + prefix-factoring already shipped."
    );
    println!();
    println!("| storage mode | dict heap | bytes/term | naive whole-string/term | vs naive |");
    println!("|---|---|---|---|---|");
    let raw_per = raw_utf8 as f64 / terms.max(1) as f64;
    row_bytes("arena (native, current)", arena_bytes, terms, raw_per);
    row_bytes("blob (browser / compacted)", blob_bytes, terms, raw_per);
    println!();
    println!(
        "### composition (per-class counts + naive UTF-8 bytes — size each lever against THIS)"
    );
    println!();
    println!("| class | count | % of terms | naive UTF-8 B | avg B/term |");
    println!("|---|---|---|---|---|");
    row_class(
        "IRI",
        comp.iris,
        terms,
        comp.iri_prefix_bytes + comp.iri_suffix_bytes,
    );
    row_class(
        "literal",
        comp.literals,
        terms,
        comp.lit_value_bytes + comp.lit_datatype_bytes + comp.lit_lang_bytes,
    );
    row_class(
        "  └ language-tagged",
        comp.lang_literals,
        terms,
        comp.lit_lang_bytes,
    );
    row_class("blank", comp.blanks, terms, comp.blank_bytes);
    row_class("triple term", comp.triples, terms, 0);
    println!();
    println!("### lever-sizing levers (measured on this vocab)");
    println!();
    println!(
        "- A1 (IRI prefix factoring): {} distinct namespace prefixes over {} IRIs; \
        avg prefix {avg_prefix:.1} B vs avg suffix {avg_suffix:.1} B \
        ({:.0}% of each IRI's bytes is the shared namespace prefix this lever dedups)",
        comp.distinct_prefixes.len(),
        comp.iris,
        100.0 * comp.iri_prefix_bytes as f64
            / (comp.iri_prefix_bytes + comp.iri_suffix_bytes).max(1) as f64
    );
    println!(
        "- A2 (extended inline ValueIds): {} literal terms reached the dictionary — the \
        candidate pool for the date/decimal/double/boolean inlining that would EVICT them entirely \
        (canonical xsd:integers in [0, 2^30) are already inlined and never appear here)",
        comp.literals
    );
    println!(
        "- A3 (lang-tag + datatype-IRI interning): {} distinct datatype IRIs already \
        deduplicated into the side table; {} naive datatype-IRI bytes if stored per-literal; \
        {} language-tagged literals ({} lang bytes) — the per-literal residual A3 targets",
        comp.distinct_datatypes.len(),
        comp.lit_datatype_bytes,
        comp.lang_literals,
        comp.lit_lang_bytes
    );
    println!();
    println!(
        "> Throughput is QUIET-BOX-sensitive; trust it only from an idle run. \
        The bytes/term + composition figures are deterministic and load-robust."
    );
}

fn row_bytes(mode: &str, heap: usize, terms: usize, raw_per: f64) {
    let per = heap as f64 / terms.max(1) as f64;
    let vs = 100.0 * per / raw_per.max(f64::MIN_POSITIVE);
    println!(
        "| {mode} | {:.2} MB | {per:.1} B | {raw_per:.1} B | {vs:.0}% |",
        heap as f64 / 1e6
    );
}

fn row_class(class: &str, count: usize, terms: usize, raw_bytes: usize) {
    let pct = 100.0 * count as f64 / terms.max(1) as f64;
    let avg = raw_bytes as f64 / count.max(1) as f64;
    println!("| {class} | {count} | {pct:.1}% | {raw_bytes} | {avg:.1} |");
}

// ---------------------------------------------------------------------------
// selftest — CI-runnable invariant checks (no dataset, no wall-clock claim)
// ---------------------------------------------------------------------------

/// In-process sanity checks so CI can run `cargo run -- selftest` without a dataset
/// and catch a harness regression (e.g. a composition miscount, or `into_blob`
/// dropping terms). NOT a performance claim — purely structural.
fn selftest() {
    let nt = concat!(
        "<http://www.wikidata.org/entity/Q1> <http://www.wikidata.org/prop/direct/P31> <http://www.wikidata.org/entity/Q5> .\n",
        "<http://www.wikidata.org/entity/Q1> <http://www.w3.org/2000/01/rdf-schema#label> \"Universe\"@en .\n",
        "<http://www.wikidata.org/entity/Q1> <http://www.wikidata.org/prop/direct/P1082> \"7900000000\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "_:b0 <http://schema.org/name> \"blank\" .\n",
    );
    let g = Graph::load_reader_parallel(std::io::Cursor::new(nt.as_bytes()), "ntriples")
        .expect("selftest data must load");
    let comp = Composition::of(&g);

    // Distinct terms here: Q1, P31, Q5, rdfs:label, "Universe"@en, P1082,
    // "7900000000"^^xsd:integer (7.9e9 > the inline 2^30 range, so it IS a dict
    // term), schema:name, "blank", _:b0. (xsd:integer/rdfs:label/etc. datatype IRIs
    // live in the side datatype table, not as standalone terms.)
    assert_eq!(g.len(), 4, "4 triples");
    assert!(
        comp.iris >= 5,
        "at least Q1/P31/Q5/rdfs:label/P1082/schema:name IRIs, got {}",
        comp.iris
    );
    assert_eq!(comp.blanks, 1, "one blank node, got {}", comp.blanks);
    assert!(
        comp.lang_literals >= 1,
        "the @en label is language-tagged, got {}",
        comp.lang_literals
    );
    assert!(
        comp.distinct_prefixes.len() >= 2,
        "wd + wdt + rdfs + schema namespaces, got {}",
        comp.distinct_prefixes.len()
    );
    assert!(comp.raw_utf8() > 0, "raw UTF-8 must be non-zero");

    // Composition's class counts must total the dictionary's term count exactly
    // (every term lands in exactly one class) — the invariant the bytes/term gate
    // relies on.
    let classified = comp.iris + comp.literals + comp.blanks + comp.triples;
    assert_eq!(
        classified,
        g.dict().len(),
        "every term classified exactly once: {classified} != {}",
        g.dict().len()
    );

    // into_blob preserves the term count + footprint accounting stays positive.
    let terms = g.dict().len();
    let arena = g.dict().heap_bytes();
    let blob = g.into_dict().into_blob();
    assert_eq!(blob.len(), terms, "into_blob preserves term count");
    assert!(
        blob.heap_bytes() > 0 && arena > 0,
        "both footprints positive"
    );

    println!("selftest OK: {} terms, {} IRIs, {} literals ({} lang), {} blanks, raw UTF-8 {} B; arena {} B, blob {} B",
        terms, comp.iris, comp.literals, comp.lang_literals, comp.blanks, comp.raw_utf8(), arena, blob.heap_bytes());
}
