package org.intralink.android

import android.graphics.BitmapFactory
import android.graphics.Color
import android.graphics.Paint
import android.view.Surface
import kotlin.math.abs

/** Real Android Surface/ImageReader, generated pixels only; no MediaProjection. */
internal class PortableSurfaceFixture(private val maxFrames: Int = 60) : PortableFrameSource.Producer {
    private var surface: Surface? = null
    private var width = 0
    private var height = 0
    private var phase = 0
    private val paint = Paint()
    private val colors = intArrayOf(
        Color.rgb(16,16,16), Color.rgb(240,240,240), Color.rgb(220,24,18),
        Color.rgb(15,200,40), Color.rgb(28,38,228), Color.rgb(45,169,179)
    )

    override fun attach(surface: Surface, width: Int, height: Int) {
        check(this.surface == null && width == 1920 && height == 1080)
        this.surface = surface
        this.width = width
        this.height = height
    }

    override fun demand() {
        check(!complete())
        val output = requireNotNull(surface)
        val canvas = output.lockCanvas(null)
        try {
            for (i in colors.indices) {
                paint.color = colors[i]
                canvas.drawRect((i * width / 6).toFloat(), 0f,
                    ((i + 1) * width / 6).toFloat(), height.toFloat(), paint)
            }
            paint.color = Color.rgb(250,219,35)
            val x = (phase * 19) % (width - 64)
            canvas.drawRect(x.toFloat(), 128f, (x + 64).toFloat(), 192f, paint)
            phase++
        } finally { output.unlockCanvasAndPost(canvas) }
    }

    override fun complete(): Boolean = phase >= maxFrames

    override fun close() { surface = null }

    companion object {
        fun checkJpeg() {
            PortableFrameSource(PortableSurfaceFixture(1), 1_000) { true }.use { source ->
                val rgba = requireNotNull(source.takeFrame(1920, 1080))
                check(rgba.isDirect && rgba.remaining() == 1920 * 1080 * 4)
                val jpeg = source.encodeJpeg(75)
                check(jpeg.size in 4 until 2 * 1024 * 1024)
                val bitmap = requireNotNull(BitmapFactory.decodeByteArray(jpeg, 0, jpeg.size))
                try {
                    check(bitmap.width == 1920 && bitmap.height == 1080)
                    val red = bitmap.getPixel(800, 540)
                    val blue = bitmap.getPixel(1440, 540)
                    check(abs(Color.red(red) - 220) <= 10 && abs(Color.green(red) - 24) <= 10 &&
                        abs(Color.blue(red) - 18) <= 10)
                    check(abs(Color.red(blue) - 28) <= 10 && abs(Color.green(blue) - 38) <= 10 &&
                        abs(Color.blue(blue) - 228) <= 10)
                } finally { bitmap.recycle() }
            }
        }
    }
}
