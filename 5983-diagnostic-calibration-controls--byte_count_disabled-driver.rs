// [GPT-6 Astra] Isolated calibration control; no engine/runtime mutation.
#[path = "byte_count_disabled.rs"] mod counting;
fn main() { counting::calibrate(); }
