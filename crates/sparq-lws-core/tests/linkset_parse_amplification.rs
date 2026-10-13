//! The linkset PATCH body limit rests on a measured bound: reading a JSON Patch body as a PATCH
//! reads it (numbers held as text, then parsed) allocates at most
//! [`LINKSET_PARSE_AMPLIFICATION`] bytes per byte of text, whatever its shape. This measures the
//! peak live bytes over the smallest syntactic units, numbers, escapes, long keys, deep nesting
//! and mixes of them, repeated up to the body limit, with a counting allocator, so a change in
//! the parser's allocation (a larger map node, a larger initial capacity) fails here first.

#![warn(clippy::undocumented_unsafe_blocks)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use sparq_lws_core::lws::resources::{
    reads_as_json_patch, LINKSET_COST_BUDGET, LINKSET_PARSE_AMPLIFICATION, LINKSET_PATCH_BODY,
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards verbatim to `System` with the same arguments, so `System`
// discharges all of `GlobalAlloc`'s obligations; the wrapper only reads `Layout::size()` / the
// new size and updates atomic counters, and never dereferences or keeps a returned pointer.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's `layout` contract is forwarded to `System.alloc` unchanged.
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(l.size(), Ordering::SeqCst) + l.size();
        PEAK.fetch_max(now, Ordering::SeqCst);
        System.alloc(l)
    }
    // SAFETY: `p` came from this allocator with `l`, so from `System` (every allocation here
    // is forwarded to it).
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        System.dealloc(p, l)
    }
    // SAFETY: the caller's `p` / `l` / `n` contract is forwarded to `System.realloc` unchanged.
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // The old buffer and the new one may both be live while it is copied: both count.
        let both = LIVE.fetch_add(n, Ordering::SeqCst) + n;
        PEAK.fetch_max(both, Ordering::SeqCst);
        let q = System.realloc(p, l, n);
        // On failure the old buffer stays and the new one never existed.
        LIVE.fetch_sub(if q.is_null() { n } else { l.size() }, Ordering::SeqCst);
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
