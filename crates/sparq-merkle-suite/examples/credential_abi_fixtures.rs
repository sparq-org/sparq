//! Independently varied private salts and configuration halves for the ABI check.
#[path = "../tests/support/credential.rs"]
mod credential;
fn main() {
    let rows: Vec<_> = [
        (7, [9; 32]),
        (11, [9; 32]),
        (7, std::array::from_fn(|i| i as u8)),
    ]
    .into_iter()
    .map(|(salt, config)| {
        let (input, message) = credential::input(salt, config);
        serde_json::json!({"input":input,"message":sparq_zk::field::field_to_hex(&message)})
    })
    .collect();
    println!("{}", serde_json::to_string(&rows).unwrap());
}
