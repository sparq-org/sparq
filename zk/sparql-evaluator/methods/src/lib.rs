// [GPT-6] Constants are generated from the actual locally compiled guest image.
include!(concat!(env!("OUT_DIR"), "/methods.rs"));

// [OPUS-5.5] zkp-14.5: the separately pinned V5 guest, generated only with this feature.
#[cfg(feature = "authenticated-rdf")]
include!(concat!(env!("OUT_DIR"), "/authrdf_methods.rs"));
