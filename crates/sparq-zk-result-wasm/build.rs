use std::{env, fs, path::PathBuf};

fn main() {
    let assets = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../js/zk-result/assets")
        .canonicalize()
        .unwrap();
    let table = assets.join("members.json");
    println!("cargo:rerun-if-changed={}", table.display());
    let members: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(&table).unwrap()).unwrap();
    let mut source = String::from("const MEMBERS: &[(&str, &[u8])] = &[\n");
    for entry in members {
        let member = entry["member"].as_str().unwrap();
        assert!(member.starts_with("result_v5_"));
        let vk = assets
            .join(entry["vk"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert!(vk.starts_with(&assets));
        println!("cargo:rerun-if-changed={}", vk.display());
        source.push_str(&format!(
            "({member:?}, include_bytes!({:?})),\n",
            vk.to_str().unwrap()
        ));
    }
    source.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("members.rs"),
        source,
    )
    .unwrap();
}
