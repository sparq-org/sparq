// [GPT-6 Astra] Isolated calibration control; no engine/runtime mutation.
#[path = "peak_disabled.rs"] mod counting;
fn main() { counting::calibrate(); }
