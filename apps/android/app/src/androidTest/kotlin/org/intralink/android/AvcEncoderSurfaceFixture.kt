package org.intralink.android

import android.graphics.Color
import android.graphics.Paint
import android.media.MediaCodec
import android.os.Bundle
import android.os.SystemClock
import android.view.Surface

/** Actual Surface -> MediaCodec encoder, no screen capture or personal pixels. */
internal object AvcEncoderSurfaceFixture {
    fun run(): String {
        val candidates = CaptureProfiles.candidates(1920, 1080)
            .filter { it.size == CaptureSize(1920, 1080) }.take(8)
        check(candidates.isNotEmpty()) { "No advertised 1080p surface encoder" }
        var last = "No usable encoder"
        for (profile in candidates) {
            try {
                return encode(profile)
            } catch (error: Exception) {
                last = "${profile.codec}: ${error.javaClass.simpleName}: ${error.message}"
            }
        }
        error(last)
    }

    private fun encode(profile: CaptureProfile): String {
        val codec = MediaCodec.createByCodecName(profile.codec)
        var surface: Surface? = null
        var started = false
        val info = MediaCodec.BufferInfo()
        val observation = AvcObservation()
        val sync = KeyframeProbe()
        var bytes = 0L
        var formatChanges = 0
        var eos = false
        try {
            codec.configure(CaptureProfiles.format(profile), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = codec.createInputSurface()
            codec.start()
            started = true
            val start = SystemClock.elapsedRealtime()
            val paint = Paint()
            fun drain(waitUs: Long) {
                for (i in 0 until 64) {
                    val index = codec.dequeueOutputBuffer(info, if (i == 0) waitUs else 0)
                    if (index == MediaCodec.INFO_TRY_AGAIN_LATER) break
                    if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                        observation.recordFormat(AvcOutputFormat.read(codec.outputFormat, ++formatChanges))
                        continue
                    }
                    if (index < 0) continue
                    try {
                        if (info.size > 0) {
                            val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                            val partial = info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME != 0
                            val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                            val mask = if (observation.canInspect(partial))
                                AvcBuffers.inspect(codec.getOutputBuffer(index), info.size) else -1
                            observation.record(mask, config, partial, key, info.presentationTimeUs)
                            if (!config) {
                                bytes += info.size
                                if (!partial && mask > 0) sync.observe(mask, key, info.presentationTimeUs,
                                    SystemClock.elapsedRealtime())
                            }
                        }
                        eos = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                    } finally { codec.releaseOutputBuffer(index, false) }
                    if (eos) break
                }
            }
            val frames = profile.fps * 2
            for (frame in 0 until frames) {
                check(SystemClock.elapsedRealtime() - start < 12_000) { "Encoder deadline" }
                drain(0)
                val canvas = surface.lockHardwareCanvas()
                try {
                    canvas.drawColor(Color.rgb(20, 36, 54))
                    paint.color = Color.rgb(104, 216, 191)
                    val x = (frame * 17 % (profile.size.width - 128)).toFloat()
                    canvas.drawRect(x, 200f, x + 128f, 500f, paint)
                } finally { surface.unlockCanvasAndPost(canvas) }
                drain(0)
                if (sync.ready()) {
                    val accepted = try {
                        codec.setParameters(Bundle().apply {
                            putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
                        })
                        true
                    } catch (_: Exception) { false }
                    sync.requested(SystemClock.elapsedRealtime(), accepted)
                }
                val due = start + (frame + 1L) * 1000 / profile.fps
                val left = due - SystemClock.elapsedRealtime()
                if (left > 0) Thread.sleep(left)
            }
            codec.signalEndOfInputStream()
            val deadline = SystemClock.elapsedRealtime() + 3_000
            while (!eos && SystemClock.elapsedRealtime() < deadline) drain(20_000)
            check(eos) { "No encoder EOS" }
            val elapsed = SystemClock.elapsedRealtime() - start
            val stats = observation.snapshot()
            check(stats.pictureBuffers > 0 && stats.idrBuffers > 0 && bytes > 0)
            sync.expire(SystemClock.elapsedRealtime())
            val sps = stats.format.sps
            val key = sync.snapshot()
            return "IntraLink physical Surface AVC encoder: PASSED; codec=${profile.codec}; " +
                "size=${profile.size.width}x${profile.size.height}; configured=${profile.fps}; " +
                "submitted=$frames; pictures=${stats.pictureBuffers}; bytes=$bytes; elapsed_ms=$elapsed; " +
                "idr=${stats.idrBuffers}; unparsed=${stats.unparsedBuffers}; " +
                "pts_non_increasing=${stats.nonIncreasingPts}; sps=${sps?.profileIdc}/${sps?.levelIdc}; " +
                "sps_size=${sps?.width}x${sps?.height}; sync=${key.phase}; sync_ms=${key.latencyMs}"
        } finally {
            try { if (started) codec.stop() } finally {
                try { codec.release() } finally { surface?.release() }
            }
        }
    }
}
