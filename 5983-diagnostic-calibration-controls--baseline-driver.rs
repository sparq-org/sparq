// [GPT-6 Astra] Isolated calibration control; no engine/runtime mutation.
#[path = "baseline.rs"] mod counting;
fn main() { counting::calibrate(); }
