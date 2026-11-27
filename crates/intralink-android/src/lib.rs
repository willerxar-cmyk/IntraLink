//! Thin Android JNI boundary. All authorization and vault state use the shared Rust crates.
//! JNI export attributes are the only unsafe boundary in this crate.
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_os = "android")]
mod bridge;
