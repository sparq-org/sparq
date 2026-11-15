pub(super) fn limits() -> sparq_canon::CanonicalizationLimits {
    sparq_canon::CanonicalizationLimits {
        max_quads: 64,
        max_input_bytes: 16 << 20,
        max_output_bytes: 16 << 20,
        max_hndq_calls: 4_000,
        max_permutation_steps: 1 << 28,
    }
}
