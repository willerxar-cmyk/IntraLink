//! Synchronous, read-only observation of locally owned codec output.
use intralink_core::h264::{Framing, MAX_ACCESS_UNIT, inspect_nal_types};
use jni::{
    JNIEnv,
    objects::{JByteBuffer, JObject},
    sys::jint,
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    slice,
};

const REJECTED: jint = -1;

/// Java must pass an exclusively owned, sliced direct buffer. The codec
/// worker retains ownership until this synchronous call returns.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_inspectAvc(
    env: JNIEnv,
    _receiver: JObject,
    buffer: JByteBuffer,
    size: jint,
    framing: jint,
) -> jint {
    catch_unwind(AssertUnwindSafe(|| {
        let Ok(size) = usize::try_from(size) else {
            return REJECTED;
        };
        if !(1..=MAX_ACCESS_UNIT).contains(&size) {
            return REJECTED;
        }
        let framing = match framing {
            0 => Framing::AnnexB,
            1 => Framing::LengthPrefixed4,
            _ => return REJECTED,
        };
        let Ok(capacity) = env.get_direct_buffer_capacity(&buffer) else {
            return REJECTED;
        };
        if capacity != size {
            return REJECTED;
        }
        let Ok(address) = env.get_direct_buffer_address(&buffer) else {
            return REJECTED;
        };
        if address.is_null() {
            return REJECTED;
        }
        // SAFETY: the direct ByteBuffer is live for this JNI invocation;
        // Kotlin passes a slice limited to exactly the valid MediaCodec
        // bytes, on the sole codec owner thread. Neither Java nor the codec
        // may release or modify it until this call returns. No reference,
        // address, image content or decoder state escapes this function.
        let bytes = unsafe { slice::from_raw_parts(address.cast_const(), size) };
        inspect_nal_types(framing, bytes).map_or(REJECTED, |mask| mask as jint)
    }))
    .unwrap_or(REJECTED)
}
