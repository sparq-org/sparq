//! The linkset PATCH body limit rests on a measured bound: reading a JSON Patch body as a PATCH
//! reads it (numbers held as text, then parsed) allocates at most
//! [`LINKSET_PARSE_AMPLIFICATION`] bytes per byte of text, whatever its shape. This measures the
//! peak live bytes over the smallest syntactic units, numbers, escapes, long keys, deep nesting
//! and mixes of them, repeated up to the body limit, with a counting allocator, so a change in
//! the parser's allocation (a larger map node, a larger initial capacity) fails here first.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use sparq_lws_core::lws::resources::{
    reads_as_json_patch, reads_as_linkset, LINKSET_COST_BUDGET, LINKSET_PARSE_AMPLIFICATION,
    LINKSET_PATCH_BODY, LINKSET_PUT_BODY, LINKSET_PUT_PARSE_AMPLIFICATION,
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Held by each test: the counters are process-wide, so no other test may allocate meanwhile.
static MEASURING: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(l.size(), Ordering::SeqCst) + l.size();
        PEAK.fetch_max(now, Ordering::SeqCst);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // The old buffer and the new one may both be live while it is copied: both count.
        let both = LIVE.fetch_add(n, Ordering::SeqCst) + n;
        PEAK.fetch_max(both, Ordering::SeqCst);
        let q = System.realloc(p, l, n);
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// The peak live bytes allocated while reading `text` as a PATCH body.
fn peak_of(text: &str) -> usize {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    std::hint::black_box(reads_as_json_patch(text.as_bytes()));
    PEAK.load(Ordering::SeqCst) - base
}

/// `unit` repeated in an array, as often as fits `len` bytes.
fn repeated(unit: &str, len: usize) -> String {
    let n = (len.saturating_sub(2) / (unit.len() + 1)).max(1);
    format!("[{}]", vec![unit; n].join(","))
}

#[test]
fn parsing_allocates_within_the_measured_bound() {
    let _alone = MEASURING.lock().unwrap_or_else(|e| e.into_inner());
    const { assert!(LINKSET_PATCH_BODY * LINKSET_PARSE_AMPLIFICATION <= LINKSET_COST_BUDGET) };
    let long_key = format!(r#"{{"{}":""}}"#, "k".repeat(1000));
    let nested = format!("{}{}", "[".repeat(120), "]".repeat(120));
    let nested_objects = format!("{}0{}", r#"{"":"#.repeat(60), "}".repeat(60));
    let op = r#"{"op":"add","path":"/linkset/0/x","value":[{"href":"h","":[0,"\u0000"]}]}"#;
    let mut units: Vec<&str> = vec![
        r#""""#,
        "[]",
        "{}",
        r#"[""]"#,
        "[[]]",
        "[{}]",
        r#"{"":""}"#,
        r#"{"":"0"}"#,
        r#"{"":[]}"#,
        r#"{"":{}}"#,
        r#"[[""]]"#,
        r#"{"a":"","b":""}"#,
        r#"["",""]"#,
        // Numbers, which a PATCH rewrites as marked strings before parsing.
        "0",
        "[0]",
        r#"{"":0}"#,
        "-1e5",
        // Escapes, short in the parsed value and long in the text, or the reverse.
        r#""\u0000""#,
        r#""\n""#,
        r#""""#,
        "\"\u{e000}\"",
        r#"{"\u0000":0}"#,
        &long_key,
        &nested,
        &nested_objects,
        op,
    ];
    // A mix: every unit in turn, in one body.
    let mixed = units.join(",");
    units.push(&mixed);
    for unit in units {
        for len in [LINKSET_PATCH_BODY / 2, LINKSET_PATCH_BODY] {
            let text = repeated(unit, len);
            let peak = peak_of(&text);
            // Within the bound, which is twice the worst measured: half of it is the margin.
            assert!(
                2 * peak <= LINKSET_PARSE_AMPLIFICATION * text.len(),
                "{unit} to {len}: {peak} bytes for {} bytes of text",
                text.len()
            );
        }
    }
}

/// The peak live bytes reading `text` as a linkset PUT reads it, and what its scan charged.
fn put_peak_of(text: &str) -> (usize, Option<usize>) {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let cost = std::hint::black_box(reads_as_linkset(text.as_bytes()));
    (PEAK.load(Ordering::SeqCst) - base, cost)
}

/// A linkset PUT reads a body only once a scan that keeps nothing has charged it, and parsing
/// it then allocates within [`LINKSET_PUT_PARSE_AMPLIFICATION`] times that charge (with half of
/// it as margin), whatever the body's size or shape. A body charged past the scan's budget
/// allocates no more than the scan's scratch (an escaped string's text, at most the body's size).
#[test]
fn a_linkset_put_reads_within_what_its_scan_charged() {
    let _alone = MEASURING.lock().unwrap_or_else(|e| e.into_inner());
    let long_key = format!(r#"{{"{}":""}}"#, "k".repeat(1000));
    let nested = format!("{}{}", "[".repeat(120), "]".repeat(120));
    let nested_objects = format!("{}\"\"{}", r#"{"":"#.repeat(60), "}".repeat(60));
    let target = r#"{"href":"https://e.example/a","title*":[{"value":"v","language":"en"}]}"#;
    let mut units: Vec<&str> = vec![
        r#""""#,
        "[]",
        "{}",
        r#"[""]"#,
        "[[]]",
        "[{}]",
        r#"{"":""}"#,
        r#"{"":[]}"#,
        r#"{"":{}}"#,
        r#"[[""]]"#,
        r#"{"a":"","b":""}"#,
        r#"{"a":"","a":""}"#,
        r#"["",""]"#,
        "true",
        "null",
        r#""\u0000""#,
        r#""\n""#,
        "\"\u{e000}\"",
        r#"{"\u0000":""}"#,
        &long_key,
        &nested,
        &nested_objects,
        target,
    ];
    let mixed = units.join(",");
    units.push(&mixed);
    for unit in units {
        for len in [
            LINKSET_PUT_BODY / 64,
            LINKSET_PUT_BODY / 4,
            LINKSET_PUT_BODY,
        ] {
            let text = repeated(unit, len);
            let (peak, cost) = put_peak_of(&text);
            match cost {
                Some(cost) => assert!(
                    2 * peak <= LINKSET_PUT_PARSE_AMPLIFICATION * cost + 2 * text.len(),
                    "{unit} to {len}: {peak} bytes, charged {cost}, for {} bytes of text",
                    text.len()
                ),
                None => assert!(
                    peak <= text.len() + 4096,
                    "{unit} to {len}: {peak} bytes refused, for {} bytes of text",
                    text.len()
                ),
            }
            assert!(peak <= LINKSET_COST_BUDGET + text.len());
        }
    }
}
