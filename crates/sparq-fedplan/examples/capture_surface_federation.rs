//! sq-vw3ax.13 — capture harness for the site's federation capability row.
//!
//! Prints the EXACT, deterministic output of the real sparq-fedplan pipeline
//! (`select_sources` + `plan_bgp`) over a small, fully-declared 3-source / 3-pattern
//! fixture, so the site walkthrough (site/src/lib/federation.ts) can paste it
//! BYTE-FOR-BYTE rather than fabricate it (the sq-rnwc lesson; same discipline as
//! crates/sparq-vectors/examples/capture_surface_vector.rs). The planner is pure and
//! deterministic — no network, no clock, no randomness — so the same binary over this
//! fixture yields byte-identical output on any machine. Run with:
//!
//!   cargo run -p sparq-fedplan --features fedplan --example capture_surface_federation
//!
//! Re-CAPTURE (do not hand-edit the site module) if the fixture or the planner changes.
#![cfg(feature = "fedplan")]

use sparq_fedplan::{
    plan_bgp, select_sources, Bgp, CharSet, JoinAlgo, JoinNode, PlanOptions, PredPartition,
    SourceDescriptor, SourceId, Term, TriplePattern, Var,
};

const FOAF_NAME: &str = "http://xmlns.com/foaf/0.1/name";
const FOAF_KNOWS: &str = "http://xmlns.com/foaf/0.1/knows";
const DC_CREATOR: &str = "http://purl.org/dc/terms/creator";
const GEO_LAT: &str = "http://www.w3.org/2003/01/geo/wgs84_pos#lat";
const GEO_LONG: &str = "http://www.w3.org/2003/01/geo/wgs84_pos#long";

fn var(name: &str) -> Term {
    Term::Var(Var::new(name))
}

/// The declared fixture: three endpoints described by served VoID property partitions
/// (+ one mined characteristic set on the social graph). Everything the planner knows
/// is printed on the page — no hidden state.
fn sources() -> Vec<SourceDescriptor> {
    let social = SourceDescriptor::builder(SourceId::new("https://social.example/sparql"))
        .total_triples(50_000)
        .predicate(PredPartition {
            predicate: FOAF_NAME.into(),
            triples: 2_000,
            distinct_subjects: 2_000,
            distinct_objects: 1_970,
        })
        .predicate(PredPartition {
            predicate: FOAF_KNOWS.into(),
            triples: 8_000,
            distinct_subjects: 800,
            distinct_objects: 1_800,
        })
        // Mined characteristic sets: 800 of the 2 000 named subjects also have
        // foaf:knows (10 links each on average) — the predicate-correlation star-join
        // estimate that a per-predicate independence product would miss.
        .char_set(CharSet {
            predicates: vec![FOAF_NAME.into(), FOAF_KNOWS.into()],
            subjects: 800,
            avg_multiplicity: vec![1.0, 10.0],
        })
        .char_set(CharSet {
            predicates: vec![FOAF_NAME.into()],
            subjects: 1_200,
            avg_multiplicity: vec![1.0],
        })
        .build();

    let pubs = SourceDescriptor::builder(SourceId::new("https://pubs.example/sparql"))
        .total_triples(90_000)
        .predicate(PredPartition {
            predicate: DC_CREATOR.into(),
            triples: 12_000,
            distinct_subjects: 9_000,
            distinct_objects: 3_000,
        })
        .predicate(PredPartition {
            predicate: FOAF_NAME.into(),
            triples: 900,
            distinct_subjects: 900,
            distinct_objects: 890,
        })
        .build();

    let geo = SourceDescriptor::builder(SourceId::new("https://geo.example/sparql"))
        .total_triples(30_000)
        .predicate(PredPartition {
            predicate: GEO_LAT.into(),
            triples: 15_000,
            distinct_subjects: 15_000,
            distinct_objects: 14_000,
        })
        .predicate(PredPartition {
            predicate: GEO_LONG.into(),
            triples: 15_000,
            distinct_subjects: 15_000,
            distinct_objects: 14_000,
        })
        .build();

    vec![social, pubs, geo]
}

/// The 3-pattern BGP: papers by authors we can name and whose co-authorship circle we
/// can walk — one pattern only the publications endpoint can answer, one star on
/// `?author` the social endpoint holds, and a name pattern BOTH endpoints hold.
fn bgp() -> Bgp {
    Bgp::new(vec![
        // p0: ?paper dcterms:creator ?author
        TriplePattern::new(var("paper"), Term::Iri(DC_CREATOR.into()), var("author")),
        // p1: ?author foaf:name ?name
        TriplePattern::new(var("author"), Term::Iri(FOAF_NAME.into()), var("name")),
        // p2: ?author foaf:knows ?friend
        TriplePattern::new(var("author"), Term::Iri(FOAF_KNOWS.into()), var("friend")),
    ])
}

fn algo(a: JoinAlgo) -> &'static str {
    match a {
        JoinAlgo::Bind => "Bind",
        JoinAlgo::Hash => "Hash",
        JoinAlgo::Streaming => "Streaming",
    }
}

/// Render the left-deep join tree as one JSON object per join step (innermost first).
fn render_steps(node: &JoinNode, out: &mut Vec<String>) {
    if let JoinNode::Join {
        left,
        right,
        algo: a,
        cardinality,
        cost,
    } = node
    {
        render_steps(left, out);
        out.push(format!(
            "{{\"right\":{right},\"algo\":\"{}\",\"cardinality\":{cardinality},\"cost\":{cost}}}",
            algo(*a),
        ));
    }
}

fn main() {
    let sources = sources();
    let bgp = bgp();

    // ── Phase 1: recall-safe source selection + CostFed cardinality ────────────────
    let selection = select_sources(&bgp, &sources);
    println!("SELECTION");
    for ps in &selection {
        let retained: Vec<String> = ps
            .candidates
            .iter()
            .map(|c| {
                format!(
                    "{{\"source\":{},\"estimatedCardinality\":{}}}",
                    c.source, c.estimated_cardinality
                )
            })
            .collect();
        let pruned: Vec<String> = (0..sources.len())
            .filter(|i| !ps.candidates.iter().any(|c| c.source == *i))
            .map(|i| i.to_string())
            .collect();
        println!(
            "pattern {} retained=[{}] pruned=[{}] total={}",
            ps.pattern,
            retained.join(","),
            pruned.join(","),
            ps.total_cardinality()
        );
    }

    // ── Phase 2: greedy bind-vs-hash-vs-streaming join planning ─────────────────────
    // Three REAL PlanOptions over the same selection, showing the cost model flip the
    // planner docs describe: raising `request_cost` (the bound-request round-trip
    // penalty) flips a bind join to a hash join, and lowering `stream_threshold` marks
    // a large hash-class join for the non-blocking streaming operator.
    let scenarios: [(&str, PlanOptions); 3] = [
        ("default", PlanOptions::default()),
        (
            "request_cost=2",
            PlanOptions {
                request_cost: 2.0,
                ..PlanOptions::default()
            },
        ),
        (
            "request_cost=2 stream_threshold=10000",
            PlanOptions {
                request_cost: 2.0,
                stream_threshold: 10_000.0,
                ..PlanOptions::default()
            },
        ),
    ];
    for (label, opts) in &scenarios {
        let plan = plan_bgp(&bgp, &selection, &sources, opts).expect("non-empty BGP plans");
        println!("PLAN {label}");
        println!("join_order {:?}", plan.join_order());
        let mut steps = Vec::new();
        render_steps(&plan.root, &mut steps);
        for s in &steps {
            println!("step {s}");
        }
        println!("total_cost {}", plan.total_cost);
        println!("root_cardinality {}", plan.root.cardinality());
    }
}
