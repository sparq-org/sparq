// Constants are generated from the actual locally compiled guest image.
include!(concat!(env!("OUT_DIR"), "/methods.rs"));

// zkp-14.5: the separately pinned V5 guest, generated only with this feature.
#[cfg(feature = "authenticated-rdf")]
include!(concat!(env!("OUT_DIR"), "/authrdf_methods.rs"));

// Measurement-only V5 image with per-phase cycle reporting.
#[cfg(feature = "phase-cycles")]
include!(concat!(env!("OUT_DIR"), "/authrdf_phases_methods.rs"));
