// zkp-14.5: export the separate V5 guest for independent release review.
use sparq_proved_evaluator::{embedded_authrdf_artifact, embedded_authrdf_pin, embedded_pin};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("usage: export_authrdf_guest NEW_DIRECTORY")?;
    let directory = std::path::PathBuf::from(directory);
    let (pin, exact) = (embedded_authrdf_pin(), embedded_pin());
    // Both exports share the `guest.bin`/`pin.json` layout; never let one pass
    // for the other.
    if pin.sha256 == exact.sha256 || pin.image_id == exact.image_id {
        return Err("V5 guest is indistinguishable from the exact guest".into());
    }
    // Refuse to overwrite an existing deployment directory.
    std::fs::create_dir(&directory)?;
    std::fs::write(directory.join("guest.bin"), embedded_authrdf_artifact())?;
    std::fs::write(directory.join("pin.json"), serde_json::to_vec_pretty(&pin)?)?;
    Ok(())
}
