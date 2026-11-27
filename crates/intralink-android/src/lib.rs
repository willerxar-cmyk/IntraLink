//! Thin Android JNI boundary. All authorization and vault state use the shared Rust crates.
//! The JNI boundary installs the process-lifetime Android context required by Iroh.
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_os = "android")]
mod bridge;

#[cfg(any(target_os = "android", test))]
mod session;
