package org.intralink.android

import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.media.Image
import android.media.ImageReader
import android.os.SystemClock
import android.view.Surface
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer

/**
 * One worker owns this reader and its buffers. Native code consumes the packed
 * direct RGBA view synchronously before requesting another frame. The actual
 * surface is not created until the authenticated transport issues a demand.
 */
internal class PortableFrameSource(
    private val producer: Producer,
    private val waitMs: Long = 10,
    private val active: () -> Boolean
) : AutoCloseable {
    interface Producer : AutoCloseable {
        fun attach(surface: Surface, width: Int, height: Int)
        fun demand()
        fun complete(): Boolean
    }

    private var reader: ImageReader? = null
    private var packed: ByteBuffer? = null
    private var bitmap: Bitmap? = null
    private var width = 0
    private var height = 0
    private var delivered = false
    private var closed = false

    fun isComplete(): Boolean = producer.complete()

    // Called only from the synchronous native capture owner.
    fun takeFrame(w: Int, h: Int): ByteBuffer? {
        check(!closed && active())
        check(h in listOf(540, 720, 1080) && w == h * 16 / 9)
        if (reader == null) {
            width = w
            height = h
            val next = ImageReader.newInstance(w, h, PixelFormat.RGBA_8888, 2)
            reader = next
            packed = ByteBuffer.allocateDirect(w * h * 4)
            producer.attach(next.surface, w, h)
        }
        check(width == w && height == h) { "Capture geometry changed" }
        val current = requireNotNull(reader)
        producer.demand()
        // First frame must exist. Later empty polls are NoUpdate, never Repeat.
        // A native producer must return promptly; no unbounded image queues.
        val deadline = SystemClock.elapsedRealtime() + if (delivered) waitMs.coerceIn(0, 1000) else 1_000L
        while (active()) {
            current.acquireLatestImage()?.use { image ->
                val buffer = requireNotNull(packed)
                copyRgba(image, width, height, buffer)
                check(active())
                delivered = true
                return buffer.asReadOnlyBuffer()
            }
            if (SystemClock.elapsedRealtime() >= deadline) break
            Thread.sleep(3)
        }
        check(active() && delivered) { "No initial image or capture canceled" }
        return null
    }

    /** Local bounded JPEG fallback only when Compact cannot encode the image. */
    fun encodeJpeg(quality: Int): ByteArray {
        check(!closed && delivered && active() && quality in 35..90)
        val image = bitmap ?: Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).also { bitmap = it }
        val rgba = requireNotNull(packed).duplicate()
        rgba.position(0)
        image.copyPixelsFromBuffer(rgba)
        val output = BoundedJpeg()
        check(image.compress(Bitmap.CompressFormat.JPEG, quality, output))
        check(active())
        return output.toByteArray()
    }

    fun stillActive(): Boolean = !closed && active()

    override fun close() {
        if (closed) return
        closed = true
        try { producer.close() } finally {
            try { reader?.close() } finally {
                reader = null
                // Wipe reusable pixel storage before releasing references.
                bitmap?.eraseColor(0)
                bitmap?.recycle()
                bitmap = null
                packed?.let { data ->
                    data.clear()
                    while (data.remaining() >= 8) data.putLong(0)
                    while (data.hasRemaining()) data.put(0)
                }
                packed = null
            }
        }
    }

    private class BoundedJpeg : ByteArrayOutputStream(64 * 1024) {
        override fun write(value: Int) {
            check(count < LIMIT)
            super.write(value)
        }
        override fun write(data: ByteArray, offset: Int, length: Int) {
            check(length >= 0 && length <= LIMIT - count)
            super.write(data, offset, length)
        }
        companion object { private const val LIMIT = 2 * 1024 * 1024 - 1 }
    }

    companion object {
        fun copyRgba(image: Image, width: Int, height: Int, target: ByteBuffer) {
            check(image.format == PixelFormat.RGBA_8888 && image.planes.size == 1)
            check(image.width == width && image.height == height)
            val crop = image.cropRect
            check(crop.left == 0 && crop.top == 0 && crop.right == width && crop.bottom == height)
            val plane = image.planes[0]
            check(plane.pixelStride == 4 && target.isDirect && target.capacity() == width * height * 4)
            val rowBytes = width * 4
            val rowStride = plane.rowStride
            val src = plane.buffer.duplicate()
            val base = src.position()
            val limit = src.limit()
            check(rowStride >= rowBytes && rowStride <= rowBytes + 16384)
            check(base.toLong() + (height - 1L) * rowStride + rowBytes <= limit)
            target.clear()
            if (rowStride == rowBytes) {
                src.limit(base + rowBytes * height)
                target.put(src)
            } else {
                for (row in 0 until height) {
                    val start = base + row * rowStride
                    src.limit(limit)
                    src.position(start)
                    src.limit(start + rowBytes)
                    target.put(src)
                }
            }
            target.flip()
        }
    }
}
