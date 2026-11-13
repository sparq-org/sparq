// Complete companion carrier using the codec already used by the SDK client.
pub(super) const BINCODE_ENCODING: &str = "bincode-1.3.3-fixint-le-v1";

pub(super) fn bincode_bytes(value: &impl serde::Serialize) -> Vec<u8> {
    bincode::serialize(value).expect("complete bincode carrier encoding")
}
