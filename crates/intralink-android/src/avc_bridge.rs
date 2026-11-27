//! Synchronous, read-only observation of locally owned codec output.
use intralink_core::h264::{Framing, MAX_ACCESS_UNIT, inspect_nal_types, inspect_sps};
use jni::{
    JNIEnv,
    objects::{JByteBuffer, JObject},
    sys::{jint, jlong},
};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    slice,
};

const MAX_CSD: usize = 1032;

/// One shared direct-buffer boundary. The callback cannot return borrowed
/// data: only owned metadata may leave the synchronous call.
fn observe<T>(
    env: &JNIEnv,
    buffer: &JByteBuffer,
    size: jint,
    framing: jint,
    max: usize,
    inspect: impl FnOnce(Framing, &[u8]) -> Option<T>,
) -> Option<T> {
    catch_unwind(AssertUnwindSafe(|| {
        let size = usize::try_from(size).ok()?;
        if !(1..=max).contains(&size) {
            return None;
        }
        let framing = match framing {
            0 => Framing::AnnexB,
            1 => Framing::LengthPrefixed4,
            _ => return None,
        };
        if env.get_direct_buffer_capacity(buffer).ok()? != size {
            return None;
        }
        let address = env.get_direct_buffer_address(buffer).ok()?;
        if address.is_null() {
            return None;
        }
        // SAFETY: the direct ByteBuffer is live for this JNI invocation.
        // The single Kotlin codec worker owns the exact sliced valid bytes
        // until the call returns; it cannot release or mutate them meanwhile.
        // No reference, pointer or coded content escapes this callback.
        let bytes = unsafe { slice::from_raw_parts(address.cast_const(), size) };
        inspect(framing, bytes)
    }))
    .ok()
    .flatten()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_inspectAvc(
    env: JNIEnv,
    _receiver: JObject,
    buffer: JByteBuffer,
    size: jint,
    framing: jint,
) -> jint {
    observe(
        &env,
        &buffer,
        size,
        framing,
        MAX_ACCESS_UNIT,
        |framing, bytes| {
            inspect_nal_types(framing, bytes)
                .ok()
                .map(|mask| mask as jint)
        },
    )
    .unwrap_or(-1)
}

/// Pack bounded SPS prefix metadata in 53 bits. -1 is rejected/unsupported.
/// This is not an authorization, complete AVC validation or decoder grant.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_intralink_android_NativeBridge_inspectAvcSps(
    env: JNIEnv,
    _receiver: JObject,
    buffer: JByteBuffer,
    size: jint,
    framing: jint,
) -> jlong {
    observe(&env, &buffer, size, framing, MAX_CSD, |framing, bytes| {
        let sps = inspect_sps(framing, bytes).ok()?;
        Some(
            i64::from(sps.width)
                | (i64::from(sps.height) << 11)
                | (i64::from(sps.profile_idc) << 22)
                | (i64::from(sps.level_idc) << 30)
                | (i64::from(sps.constraints) << 38)
                | (i64::from(sps.references) << 46)
                | (i64::from(sps.poc_type) << 51),
        )
    })
    .unwrap_or(-1)
}
