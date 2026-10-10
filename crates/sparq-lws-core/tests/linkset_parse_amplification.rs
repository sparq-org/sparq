//! The linkset PATCH body limit rests on a measured bound: parsing JSON allocates at most
//! [`LINKSET_PARSE_AMPLIFICATION`] bytes per byte of text, whatever its shape. This measures
//! the smallest syntactic units, repeated, with a counting allocator, so a change in the
//! parser's allocation (a larger map node, a larger initial capacity) fails here first.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use sparq_lws_core::lws::resources::{
    LINKSET_COST_BUDGET, LINKSET_PARSE_AMPLIFICATION, LINKSET_PATCH_BODY,
};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

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
        if n > l.size() {
            let now = LIVE.fetch_add(n - l.size(), Ordering::SeqCst) + n - l.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        } else {
            LIVE.fetch_sub(l.size() - n, Ordering::SeqCst);
        }
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// The peak bytes allocated while parsing `text`.
fn peak_of(text: &str) -> usize {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let v: serde_json::Value = serde_json::from_str(text).expect("JSON");
    let peak = PEAK.load(Ordering::SeqCst) - base;
    drop(v);
    peak
}

#[test]
fn parsing_allocates_within_the_measured_bound() {
    const { assert!(LINKSET_PATCH_BODY * LINKSET_PARSE_AMPLIFICATION <= LINKSET_COST_BUDGET) };
    let units = [
        r#""""#,
        "[]",
        "{}",
        r#"[""]"#,
        "[[]]",
        "[{}]",
        r#"{"":""}"#,
        r#"{"":"0"}"#,
        r#"{"":[]}"#,
        r#"{"":{}}"#,
        r#"[[""]]"#,
        r#"{"a":"","b":""}"#,
        r#"["",""]"#,
    ];
    for unit in units {
        for n in [1, 10, 100, 1000] {
            let text = format!("[{}]", vec![unit; n].join(","));
            let peak = peak_of(&text);
            // Within the bound, which is twice the worst measured: half of it is the margin.
            assert!(
                2 * peak <= LINKSET_PARSE_AMPLIFICATION * text.len(),
                "{unit} x{n}: {peak} bytes for {} bytes of text",
                text.len()
            );
        }
    }
}
