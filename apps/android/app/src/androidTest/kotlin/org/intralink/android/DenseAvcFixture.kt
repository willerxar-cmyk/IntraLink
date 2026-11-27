package org.intralink.android

import android.content.Context
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.SystemClock
import android.view.Surface
import java.io.DataOutputStream
import java.io.File
import java.nio.ByteBuffer
import java.security.DigestOutputStream
import java.security.MessageDigest

/** Trusted generated pixels only; no projection, recording API or remote AVC. */
internal object DenseAvcFixture {
    private const val FILE_NAME = "intralink-generated-dense-avc.ilavc"
    private const val LIMIT = 16 * 1024 * 1024
    private const val FRAMES = 120
    private const val TARGET = 6_000_000

    fun run(context: Context): String {
        check(Build.VERSION.SDK_INT >= 29)
        val codecs = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
        val profile = CaptureProfiles.candidates(1920, 1080).firstOrNull { p ->
            p.size == CaptureSize(1920, 1080) && p.fps == 60 && codecs
                .firstOrNull { it.name == p.codec }?.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
                ?.videoCapabilities?.bitrateRange?.contains(TARGET) == true
        }?.copy(bitrate = TARGET) ?: error("No advertised 1080p60 AVC mode at target bitrate")
        val path = File(context.cacheDir, FILE_NAME)
        check(!path.exists()) { "Previous generated fixture must be collected first" }
        val temp = File.createTempFile("intralink-dense-avc-", ".tmp", context.cacheDir)
        val digest = MessageDigest.getInstance("SHA-256")
        try {
            val summary = DataOutputStream(DigestOutputStream(temp.outputStream().buffered(), digest)).use {
                encode(profile, it)
            }
            check(temp.length() in 32..LIMIT.toLong())
            check(temp.renameTo(path))
            return "$summary file_bytes=${path.length()} sha256=${digest.digest().joinToString("") { "%02x".format(it.toInt() and 255) }}"
        } finally {
            temp.delete()
        }
    }

    private fun encode(profile: CaptureProfile, out: DataOutputStream): String {
        val codec = MediaCodec.createByCodecName(profile.codec)
        var surface: Surface? = null
        var started = false
        var written = 20
        var pictures = 0
        var idrs = 0
        var pictureBytes = 0L
        var previousPts = -1L
        var format = 0
        var eos = false
        val info = MediaCodec.BufferInfo()
        val settings = CaptureProfiles.format(profile).apply {
            setInteger(MediaFormat.KEY_PROFILE, MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
            setInteger(MediaFormat.KEY_LEVEL, MediaCodecInfo.CodecProfileLevel.AVCLevel42)
            setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
            setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
            setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_LIMITED)
            setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
        }
        fun record(kind: Int, flags: Int, pts: Long, source: ByteBuffer) {
            val data = source.duplicate()
            val length = data.remaining()
            check(length in 1..(2 * 1024 * 1024 - 8))
            check(written + 16 + length <= LIMIT)
            val bytes = ByteArray(length)
            data.get(bytes)
            out.writeByte(kind)
            out.writeByte(flags)
            out.writeShort(0)
            out.writeInt(length)
            out.writeLong(pts)
            out.write(bytes)
            written += 16 + length
        }
        out.writeBytes("ILAVC001")
        out.writeShort(1920)
        out.writeShort(1080)
        out.writeInt(60)
        out.writeInt(TARGET)
        try {
            codec.configure(settings, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            surface = codec.createInputSurface()
            codec.start()
            started = true
            val began = SystemClock.elapsedRealtime()
            fun drain(waitUs: Long) {
                repeat(64) { i ->
                    val index = codec.dequeueOutputBuffer(info, if (i == 0) waitUs else 0)
                    if (index == MediaCodec.INFO_TRY_AGAIN_LATER) return
                    if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                        check(++format == 1) { "Unexpected output format transition" }
                        val actual = codec.outputFormat
                        for ((key, kind) in listOf("csd-0" to 1, "csd-1" to 2)) {
                            val csd = requireNotNull(actual.getByteBuffer(key))
                            check(csd.remaining() in 1..1032)
                            record(kind, 0, 0, csd)
                        }
                        return@repeat
                    }
                    if (index < 0) return@repeat
                    try {
                        if (info.size > 0) {
                            check(format == 1 && info.flags and MediaCodec.BUFFER_FLAG_PARTIAL_FRAME == 0)
                            val data = requireNotNull(codec.getOutputBuffer(index))
                            val mask = AvcBuffers.inspect(data, info.size)
                            check(mask > 0) { "Unparsed local AVC buffer" }
                            if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                                check(mask and 0x3e == 0)
                                record(3, 0, 0, data)
                            } else {
                                val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                                check(mask and 0x3e != 0 && key == (mask and (1 shl 5) != 0))
                                check(info.presentationTimeUs > previousPts && pictures < FRAMES)
                                previousPts = info.presentationTimeUs
                                if (key) idrs++
                                pictures++
                                pictureBytes += info.size
                                record(4, if (key) 1 else 0, previousPts, data)
                            }
                        }
                        eos = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                    } finally { codec.releaseOutputBuffer(index, false) }
                    if (eos) return
                }
            }
            MotionPattern().use { pattern ->
                for (frame in 0 until FRAMES) {
                    check(SystemClock.elapsedRealtime() - began < 12_000)
                    drain(0)
                    val canvas = surface.lockHardwareCanvas()
                    try { pattern.draw(canvas, frame + 10, true) }
                    finally { surface.unlockCanvasAndPost(canvas) }
                    drain(0)
                    val due = began + (frame + 1L) * 1000 / 60
                    val delay = due - SystemClock.elapsedRealtime()
                    if (delay > 0) Thread.sleep(delay)
                }
            }
            codec.signalEndOfInputStream()
            val deadline = SystemClock.elapsedRealtime() + 4_000
            while (!eos && SystemClock.elapsedRealtime() < deadline) drain(20_000)
            check(eos && pictures in 100..FRAMES && idrs > 0)
            out.writeByte(255)
            out.write(ByteArray(15))
            return "DENSE_AVC_PASS codec=${profile.codec} submitted=$FRAMES pictures=$pictures idrs=$idrs " +
                "picture_bytes=$pictureBytes elapsed_ms=${SystemClock.elapsedRealtime() - began} target_bps=$TARGET"
        } finally {
            try { if (started) codec.stop() } finally {
                try { codec.release() } finally { surface?.release() }
            }
        }
    }
}
