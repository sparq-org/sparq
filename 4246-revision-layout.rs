// [GPT-6 Astra] Exact added field layout on the benchmark toolchain/host.
fn main() {
 let bytes = std::mem::size_of::<[std::sync::OnceLock<Vec<[u32; 3]>>; 6]>();
 println!("{{\"deleted_projection_metadata_bytes\":{bytes},\"triple_bytes\":{}}}", std::mem::size_of::<[u32; 3]>());
}
