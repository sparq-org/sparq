// [GPT-6] Export an artifact for independent release review and common deployment.
use sparq_proved_evaluator::{embedded_artifact, embedded_pin};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("usage: export_guest NEW_DIRECTORY")?;
    let directory = std::path::PathBuf::from(directory);
    // Refuse to overwrite an existing deployment directory.
    std::fs::create_dir(&directory)?;
    std::fs::write(directory.join("guest.bin"), embedded_artifact())?;
    std::fs::write(
        directory.join("pin.json"),
        serde_json::to_vec_pretty(&embedded_pin())?,
    )?;
    Ok(())
}
