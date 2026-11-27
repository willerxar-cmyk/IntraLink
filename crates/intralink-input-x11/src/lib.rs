//! Native, consented X11 input boundary. No global hooks or privileged access.
#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

mod indicator;
mod keyboard;
mod native;

pub use native::NativeInput;

mod source;
pub use source::{Pad, PadEvent, Poll};
