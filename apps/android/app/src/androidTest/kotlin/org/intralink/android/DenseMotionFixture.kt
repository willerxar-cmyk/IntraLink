package org.intralink.android

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.view.Surface

/** Reproducible offline dense-motion fixture. Generated pixels only. */
internal class MotionPattern : AutoCloseable {
    private val paint = Paint()
    private val texture = Bitmap.createBitmap(1920, 1080, Bitmap.Config.ARGB_8888).apply {
        val pixels = IntArray(1920 * 1080) { index ->
            val x = index % 1920
            val y = index / 1920
            Color.rgb(
                (x * 3 + y * 2 + (x / 24 % 2) * 70) and 255,
                (x + y * 5 + (y / 32 % 2) * 64) and 255,
                (x * 2 + y * 3 + (x / 64 % 2) * 92) and 255
            )
        }
        setPixels(pixels, 0, 1920, 0, 0, 1920, 1080)
        pixels.fill(0)
    }

    fun draw(canvas: Canvas, frame: Int, dense: Boolean) {
        if (dense) {
            val x = (frame * 19) % 1920
            canvas.drawBitmap(texture, -x.toFloat(), 0f, paint)
            canvas.drawBitmap(texture, (1920 - x).toFloat(), 0f, paint)
        } else canvas.drawColor(Color.rgb(20, 36, 54))
        paint.color = Color.rgb(104, 216, 191)
        val marker = (frame * 17 % (1920 - 128)).toFloat()
        canvas.drawRect(marker, 200f, marker + 128f, 500f, paint)
    }

    override fun close() { texture.recycle() }
}

/** The normal portable JNI/media path receives these actual Surface images. */
internal class DenseMotionFixture : PortableFrameSource.Producer {
    private val pattern = MotionPattern()
    private var surface: Surface? = null
    private var frame = 0

    override fun attach(surface: Surface, width: Int, height: Int) {
        check(this.surface == null && width == 1920 && height == 1080)
        this.surface = surface
    }

    override fun demand() {
        check(!complete())
        val out = requireNotNull(surface)
        val canvas = out.lockCanvas(null)
        try {
            pattern.draw(canvas, frame, frame in 10 until 150)
            frame++
        } finally { out.unlockCanvasAndPost(canvas) }
    }

    override fun complete(): Boolean = frame >= FRAMES

    override fun close() {
        surface = null
        pattern.close()
    }

    companion object { const val FRAMES = 210 }
}
