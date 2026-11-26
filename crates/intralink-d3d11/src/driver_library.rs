//! Retain the system NVIDIA driver modules for the lifetime of the process.
//! On hybrid Windows systems, unloading a codec/interop DLL while D3D11 or
//! driver-owned callbacks are still unwinding can crash native teardown.
//! Sessions, contexts, graphics registrations and GPU resources are still
//! destroyed explicitly; only immutable library handles are process-scoped.
use anyhow::{Result, anyhow};
use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};
use std::sync::OnceLock;

type Cached = OnceLock<Result<Library, String>>;
static CUDA: Cached = OnceLock::new();
static CUVID: Cached = OnceLock::new();
static NVENC: Cached = OnceLock::new();

fn load(cache: &'static Cached, name: &'static str) -> Result<&'static Library> {
    let result = cache.get_or_init(|| {
        // SAFETY: only fixed known system DLL names are used; the current
        // directory and PATH are not searched. Static storage retains the
        // handle and all resolved symbols through process termination.
        unsafe { Library::load_with_flags(name, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|error| format!("Load {name} from System32: {error}"))
    });
    result.as_ref().map_err(|error| anyhow!("{error}"))
}

pub(crate) fn cuda() -> Result<&'static Library> {
    load(&CUDA, "nvcuda.dll")
}
pub(crate) fn cuvid() -> Result<&'static Library> {
    load(&CUVID, "nvcuvid.dll")
}
pub(crate) fn nvenc() -> Result<&'static Library> {
    load(&NVENC, "nvEncodeAPI64.dll")
}
