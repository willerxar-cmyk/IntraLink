//! Synchronous Android platform frame adapter for the shared safe Rust encoder.
use anyhow::{Context, Result, ensure};
use intralink_core::{
    desktop::{MAX_FRAME_BYTES, VideoConfig},
    portable_capture::{Decision, Encoder},
    quality::ReceiptPressure,
};
use intralink_transport::desktop::Capture;
use jni::{
    JNIEnv,
    objects::{JByteArray, JByteBuffer, JObject, JValue},
};
use std::slice;

pub(crate) struct Source<'local, 'env> {
    pub env: &'env mut JNIEnv<'local>,
    pub object: JObject<'local>,
    encoder: Option<Encoder>,
}

impl<'local, 'env> Source<'local, 'env> {
    pub fn new(env: &'env mut JNIEnv<'local>, object: JObject<'local>) -> Self {
        Self {
            env,
            object,
            encoder: None,
        }
    }

    pub fn capture(
        &mut self,
        config: VideoConfig,
        pressure: ReceiptPressure,
        mut output: Vec<u8>,
    ) -> Result<Capture> {
        let encoder = self.encoder.get_or_insert_with(|| Encoder::new(config));
        ensure!(
            encoder.config() == config,
            "Portable capture selection changed"
        );
        let object = &self.object;
        let result = self.env.with_local_frame(16, |env| -> Result<Capture> {
            ensure!(
                env.call_method(object, "stillActive", "()Z", &[])?.z()?,
                "Capture canceled"
            );
            if env.call_method(object, "isComplete", "()Z", &[])?.z()? {
                return Ok(Capture::End);
            }
            let frame = env
                .call_method(
                    object,
                    "takeFrame",
                    "(II)Ljava/nio/ByteBuffer;",
                    &[
                        JValue::Int(config.width() as i32),
                        JValue::Int(config.height() as i32),
                    ],
                )?
                .l()?;
            if frame.is_null() {
                ensure!(encoder.has_frame(), "No initial capture");
                output.clear();
                return Ok(Capture::NoUpdate(output));
            }
            let frame = JByteBuffer::from(frame);
            let expected = (config.width() * config.height() * 4) as usize;
            ensure!(
                env.get_direct_buffer_capacity(&frame)? == expected,
                "Unexpected RGBA buffer capacity"
            );
            let address = env.get_direct_buffer_address(&frame)?;
            ensure!(!address.is_null(), "Missing RGBA buffer");
            // SAFETY: the single synchronous Kotlin source owns packed direct
            // storage; no mutation or release until this invocation completes.
            // Only owned RGB canvas bytes survive the prepare call.
            let decision = encoder
                .prepare(
                    unsafe { slice::from_raw_parts(address.cast_const(), expected) },
                    pressure,
                    &mut output,
                )
                .map_err(|e| anyhow::anyhow!("Portable encode: {e:?}"))?;
            if decision == Decision::Repeat {
                return Ok(Capture::Repeat(output));
            }
            let jpeg = match decision {
                Decision::Jpeg { quality } => {
                    let bytes = JByteArray::from(
                        env.call_method(
                            object,
                            "encodeJpeg",
                            "(I)[B",
                            &[JValue::Int(i32::from(quality))],
                        )?
                        .l()?,
                    );
                    let size = env.get_array_length(&bytes)? as usize;
                    ensure!(
                        (4..MAX_FRAME_BYTES).contains(&size),
                        "Local JPEG exceeds media budget"
                    );
                    Some(env.convert_byte_array(&bytes)?)
                }
                _ => None,
            };
            encoder
                .commit(decision, &mut output, jpeg.as_deref())
                .map_err(|e| anyhow::anyhow!("Portable encode commit: {e:?}"))?;
            ensure!(
                env.call_method(object, "stillActive", "()Z", &[])?.z()?,
                "Capture canceled"
            );
            Ok(Capture::AdaptiveFrame {
                height: config.height() as u16,
                data: output,
            })
        });
        // Convert a local callback exception into a closed session; never
        // leave a pending Java exception across the JNI JSON response.
        if self.env.exception_check()? {
            self.env.exception_clear()?;
            anyhow::bail!("Local image source failed or canceled");
        }
        result.context("Local portable capture failed")
    }
}
