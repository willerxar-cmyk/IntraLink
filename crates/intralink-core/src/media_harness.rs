//! Standalone tests: rustc --edition=2024 --test -D warnings src/media_harness.rs
//! Compiles the actual protocol and media modules without Cargo or serde.
#![forbid(unsafe_code)]

extern crate self as intralink_core;

pub mod canvas;
pub mod desktop;
pub mod h264;
pub mod media;
pub mod journal;
pub mod network;
pub mod overview;
pub mod pacing;
pub mod presentation;
pub mod quality;
pub mod resources;
pub mod telemetry;
pub mod yuv;
pub mod workspace;

#[cfg(test)]
#[path = "../../../apps/intralink/src/desktop/overview.rs"]
mod viewer_overview;
