package org.intralink.android

import android.content.Context
import android.graphics.ImageFormat
import android.media.Image
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.SystemClock
import java.io.DataInputStream
import java.security.MessageDigest
import kotlin.math.abs

/**
 * Instrumentation only. Decode the single known generated fixture, never a
 * file path, user display, remote byte stream or arbitrary test argument.
 * The application APK does not contain this fixture or decoder harness.
 */
internal object AvcDecoderFixture {
    private const val WIDTH = 1920
    private const val HEIGHT = 1080
    private const val SIZE = 12126
    private const val HASH = "283ab72056ae1b9edf57a6bae51edd255db01ffd72593e1b0113e6914db0cda0"
    private val offsets = intArrayOf(0, 895, 3537, 6100, 6966, 9547)
    private val lengths = intArrayOf(895, 2642, 2563, 866, 2581, 2579)
    private val colors = arrayOf(
        intArrayOf(16,128,128), intArrayOf(235,128,128), intArrayOf(63,102,240),
        intArrayOf(173,42,26), intArrayOf(32,240,118), intArrayOf(126,128,128)
    )

    private fun hex(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }

    fun run(context: Context): String {
        val fixture = ByteArray(SIZE)
        DataInputStream(context.assets.open("headless-bands-1080p.h264")).use {
            it.readFully(fixture)
            check(it.read() == -1) { "Unexpected fixture length" }
        }
        check(hex(MessageDigest.getInstance("SHA-256").digest(fixture)) == HASH) { "Fixture hash mismatch" }
        val format = format()
        val decoder = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { !it.isEncoder && it.supportedTypes.any { type -> type.equals("video/avc", true) } }
            .filter { if (Build.VERSION.SDK_INT >= 29) it.isSoftwareOnly && !it.isAlias
                else it.name == "OMX.google.h264.decoder" }
            .firstOrNull { info ->
                val caps = info.getCapabilitiesForType("video/avc")
                caps.colorFormats.contains(MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible) &&
                    caps.isFormatSupported(format)
            } ?: error("No compatible software AVC decoder")
        val all = decode(decoder.name, fixture, intArrayOf(0,1,2,3,4,5))
        val retained = decode(decoder.name, fixture, intArrayOf(0,3,4,5))
        check(retained.keys == setOf(0,3,4,5))
        for ((phase, hash) in retained) {
            check(all[phase] == hash) { "Recovered decoded pixels differ at frame $phase" }
        }
        return "IntraLink Android software decoder fixture: PASSED; codec=${decoder.name}; " +
            "frames=6+4; recovered YUV bytes=12441600; exact hashes match; sampled error <=12"
    }

    private fun format(): MediaFormat = MediaFormat.createVideoFormat("video/avc", WIDTH, HEIGHT).apply {
        setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible)
    }

    private fun decode(name: String, fixture: ByteArray, indices: IntArray): Map<Int,String> {
        val codec = MediaCodec.createByCodecName(name)
        var started = false
        try {
            codec.configure(format(), null, null, 0)
            codec.start()
            started = true
            var submitted = 0
            var inputEos = false
            var outputEos = false
            val result = linkedMapOf<Int,String>()
            val info = MediaCodec.BufferInfo()
            val deadline = SystemClock.elapsedRealtime() + 15_000L
            while (!outputEos) {
                check(SystemClock.elapsedRealtime() < deadline) { "Decoder deadline" }
                if (!inputEos) {
                    val index = codec.dequeueInputBuffer(10_000)
                    if (index >= 0) {
                        if (submitted == indices.size) {
                            codec.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
                            inputEos = true
                        } else {
                            val phase = indices[submitted++]
                            val input = requireNotNull(codec.getInputBuffer(index))
                            input.clear()
                            check(input.remaining() >= lengths[phase])
                            input.put(fixture, offsets[phase], lengths[phase])
                            codec.queueInputBuffer(index, 0, lengths[phase], phase * 16667L, 0)
                        }
                    }
                }
                val index = codec.dequeueOutputBuffer(info, 10_000)
                if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    val actual = codec.outputFormat
                    check(actual.getInteger(MediaFormat.KEY_WIDTH) >= WIDTH)
                    check(actual.getInteger(MediaFormat.KEY_HEIGHT) >= HEIGHT)
                } else if (index >= 0) {
                    try {
                        if (info.size > 0) {
                            val phase = indices.firstOrNull { it * 16667L == info.presentationTimeUs }
                                ?: error("Unexpected decoder PTS")
                            check(phase !in result) { "Duplicate decoded frame" }
                            val image = requireNotNull(codec.getOutputImage(index))
                            image.use { result[phase] = verifyImage(it, phase) }
                        }
                        outputEos = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                    } finally {
                        codec.releaseOutputBuffer(index, false)
                    }
                }
            }
            check(result.keys.toList() == indices.toList()) { "Wrong decoded frame sequence" }
            return result
        } finally {
            try { if (started) codec.stop() } finally { codec.release() }
        }
    }

    private fun verifyImage(image: Image, phase: Int): String {
        val crop = image.cropRect
        check(image.format == ImageFormat.YUV_420_888 && image.planes.size == 3)
        check(crop.width() == WIDTH && crop.height() == HEIGHT &&
            crop.left >= 0 && crop.top >= 0 && crop.left % 2 == 0 && crop.top % 2 == 0 &&
            crop.right <= image.width && crop.bottom <= image.height)
        val digest = MessageDigest.getInstance("SHA-256")
        for ((planeIndex, plane) in image.planes.withIndex()) {
            val div = if (planeIndex == 0) 1 else 2
            val w = WIDTH / div
            val h = HEIGHT / div
            val stride = plane.pixelStride
            val rowStride = plane.rowStride
            check(stride in 1..2 && rowStride >= (w - 1) * stride + 1 && rowStride <= 16384)
            val buffer = plane.buffer.duplicate()
            val base = buffer.position().toLong() + (crop.top / div).toLong() * rowStride +
                (crop.left / div).toLong() * stride
            val last = base + (h - 1).toLong() * rowStride + (w - 1).toLong() * stride
            check(base >= 0 && last < buffer.limit())
            val row = ByteArray(w)
            for (y in 0 until h) {
                val start = (base + y.toLong() * rowStride).toInt()
                if (stride == 1) {
                    buffer.position(start)
                    buffer.get(row)
                } else {
                    for (x in 0 until w) row[x] = buffer.get(start + x * stride)
                }
                digest.update(row)
            }
            for (band in colors.indices) {
                val x = (((2 * band + 1) * WIDTH / (2 * colors.size)) and -2) / div
                for (y in intArrayOf(HEIGHT/4, HEIGHT/2, HEIGHT*3/4)) {
                    val sample = buffer.get((base + (y/div).toLong()*rowStride + x*stride).toInt()).toInt() and 255
                    check(abs(sample - colors[(band + phase) % colors.size][planeIndex]) <= 12) {
                        "Unexpected decoded color frame=$phase plane=$planeIndex band=$band"
                    }
                }
            }
        }
        return hex(digest.digest())
    }
}
