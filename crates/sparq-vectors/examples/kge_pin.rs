//! Cross-commit **byte-stability pin** for the `kge` trainer/eval stack.
//!
//! (Kern PR-1, quoted-terms visibility): prints a deterministic 128-bit digest over the
//! canonical little-endian bytes of `(epoch_loss, entity_emb, rel_emb, metrics-as-f64s)` for the
//! three pre-existing **quote-free** synthetic slices (gUFO, relational, provenance) at pinned
//! seeds `{1, 2, 3}`, under every preset default (in particular `TermScope::IriBlank`, the
//! quoted-terms ablation OFF).
//!
//! Review procedure (same box, same pinned toolchain, same thread count — per-build float/id
//! determinism is a crate invariant the `is_deterministic_for_fixed_config` tests pin):
//!
//! ```text
//! git checkout <merge-base> && cargo run -p sparq-vectors --features kge --example kge_pin > a
//! git checkout <pr-head>    && cargo run -p sparq-vectors --features kge --example kge_pin > b
//! diff a b   # must be empty: the flag-off pipeline is byte-identical across the change
//! ```
//!
//! The digest is a splitmix64-fed 2×64-bit fold — **not cryptographic** (no new dependency); its
//! only job is equality diffing between two runs of the same deterministic pipeline.

#[cfg(feature = "kge")]
fn main() {
    use sparq_vectors::structure::{close_for_vectorise, TypeConstraints};
    use sparq_vectors::train::train;
    use sparq_vectors::{
        run_ablation, synthetic_gufo_ttl, synthetic_provenance_ttl, synthetic_relational_ttl,
        EvalConfig, Splits,
    };

    /// SplitMix64 step (the crate's standard deterministic mixer).
    fn splitmix64(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Fold a little-endian byte stream into two independent 64-bit lanes.
    struct Digest {
        a: u64,
        b: u64,
    }
    impl Digest {
        fn new() -> Digest {
            Digest {
                a: 0x0123_4567_89AB_CDEF,
                b: 0xFEDC_BA98_7654_3210,
            }
        }
        fn bytes(&mut self, bytes: &[u8]) {
            for chunk in bytes.chunks(8) {
                let mut w = [0u8; 8];
                w[..chunk.len()].copy_from_slice(chunk);
                let v = u64::from_le_bytes(w);
                self.a ^= v;
                self.a = splitmix64(&mut self.a);
                self.b = self.b.rotate_left(29) ^ v;
                self.b = splitmix64(&mut self.b);
            }
        }
        fn f32s(&mut self, xs: &[f32]) {
            for x in xs {
                self.bytes(&x.to_bits().to_le_bytes());
            }
        }
        fn f64(&mut self, x: f64) {
            self.bytes(&x.to_bits().to_le_bytes());
        }
        fn hex(&self) -> String {
            format!("{:016x}{:016x}", self.a, self.b)
        }
    }

    let slices: [(&str, String); 3] = [
        ("gufo", synthetic_gufo_ttl(60, 1)),
        ("relational", synthetic_relational_ttl(120, 2)),
        ("provenance", synthetic_provenance_ttl(60, 3)),
    ];

    for (name, ttl) in &slices {
        for seed in [1u64, 2, 3] {
            let template = EvalConfig::small(seed);
            let mut d = Digest::new();

            // (a) Raw model bytes: closure + preset trainer over the full graph (exercises
            // collect_positives, the sampler pools, and the SGD float path directly).
            let closed = close_for_vectorise(ttl, "turtle", template.profile)
                .expect("synthetic slice must parse");
            let graph = &closed.graph;
            let splits = Splits::split(
                graph,
                template.train_frac,
                template.valid_frac,
                template.split_seed,
            );
            let tc = TypeConstraints::mine(graph);
            let (model, report) = train(graph, &tc, template.train);
            for l in &report.epoch_loss {
                d.f32s(&[*l]);
            }
            d.f32s(&model.entity_emb);
            d.f32s(&model.rel_emb);
            d.f64(splits.train.len() as f64);
            d.f64(splits.valid.len() as f64);
            d.f64(splits.test.len() as f64);
            d.f64(splits.filter_set_len() as f64);

            // (b) The official pipeline end-to-end: every cell of the 2×2 ablation matrix
            // (split → restrict-to-train → mine → train → filtered metrics) as f64s.
            let cells = run_ablation(ttl, "turtle", template).expect("ablation must run");
            for c in &cells {
                d.f64(u64::from(c.closure) as f64);
                d.f64(u64::from(c.type_constrained) as f64);
                d.f64(c.metrics.queries as f64);
                d.f64(c.metrics.mrr);
                d.f64(c.metrics.hits1);
                d.f64(c.metrics.hits3);
                d.f64(c.metrics.hits10);
                for l in &c.report.epoch_loss {
                    d.f32s(&[*l]);
                }
            }
            println!("{name} seed={seed} pin={}", d.hex());
        }
    }
}

#[cfg(not(feature = "kge"))]
fn main() {
    eprintln!("kge_pin requires --features kge");
}
