#![doc = include_str!("../README.md")]
#![warn(clippy::undocumented_unsafe_blocks)]

#[cfg(test)]
mod patch;

include!(concat!(env!("OUT_DIR"), "/engine/lib.rs"));
