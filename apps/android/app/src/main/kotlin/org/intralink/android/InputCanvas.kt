package org.intralink.android

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.View
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** Actual local application input target. No OS injection, IME or other-app access. */
internal class InputCanvas(context: Context) : View(context) {
    private val main = Handler(Looper.getMainLooper())
    private val approved = AtomicBoolean(false)
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val arrow = Path()
    private val keyboard = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD)
    private val keys = BooleanArray(256)
    private val buttons = BooleanArray(3)
    private var sequence = 0
    private var active = false
    private var x = 32768
    private var y = 32768
    private var text = StringBuilder()
    private var clicks = 0
    private var moves = 0
    private var scroll = 0
    private var selected = false

    /** Called on the Activity main thread after separate local approval. */
    fun begin() {
        sequence = 0
        active = false
        keys.fill(false)
        buttons.fill(false)
        x = 32768
        y = 32768
        text = StringBuilder()
        clicks = 0
        moves = 0
        scroll = 0
        selected = false
        approved.set(true)
        invalidate()
    }

    fun stop() {
        approved.set(false)
        release()
        invalidate()
    }

    private fun release() {
        active = false
        keys.fill(false)
        buttons.fill(false)
    }

    /** Called synchronously by JNI. An expired queued UI event never runs. */
    fun acceptRecord(seq: Int, tag: Int, flag: Int, a: Int, b: Int): Boolean {
        if (!approved.get() || Looper.myLooper() == Looper.getMainLooper()) return false
        val deadline = SystemClock.elapsedRealtime() + 220
        val completed = CountDownLatch(1)
        val result = AtomicBoolean(false)
        main.post {
            try {
                if (approved.get() && SystemClock.elapsedRealtime() < deadline &&
                    isAttachedToWindow && isShown && hasWindowFocus()) {
                    result.set(apply(seq, tag, flag, a, b))
                }
            } finally { completed.countDown() }
        }
        val done = try { completed.await(250, TimeUnit.MILLISECONDS) }
        catch (_: InterruptedException) { Thread.currentThread().interrupt(); false }
        if (!done || !result.get()) approved.set(false)
        return done && result.get()
    }

    private fun apply(seq: Int, tag: Int, flag: Int, a: Int, b: Int): Boolean {
        if (seq != sequence + 1 || flag !in 0..1 || a !in 0..65535 || b !in 0..65535) return false
        if (tag in 5..8 && !active) return false
        when (tag) {
            1 -> { if (active) return false; active = true }
            2 -> { if (!active) return false; release() }
            3 -> Unit
            4 -> { release(); approved.set(false) }
            5 -> { x = a; y = b; moves++ }
            6 -> {
                if (a !in keys.indices || keys[a] == (flag == 1)) return false
                keys[a] = flag == 1
                if (flag == 1) type(a)
            }
            7 -> {
                if (a !in 1..3 || buttons[a-1] == (flag == 1)) return false
                buttons[a-1] = flag == 1
                if (flag == 1) {
                    clicks++
                    if (a == 1) selected = !selected
                }
            }
            8 -> {
                val steps = a.toShort().toInt()
                if (steps == 0 || steps !in -8..8) return false
                scroll = (scroll + steps).coerceIn(-1000, 1000)
            }
            else -> return false
        }
        sequence = seq
        invalidate()
        return true
    }

    /** Explicit physical-key subset translated to local app text only. */
    private fun type(usage: Int) {
        if (keys[224] || keys[226] || keys[227] || keys[228] || keys[230] || keys[231]) return
        if (usage == 42) {
            if (text.isNotEmpty()) text.deleteCharAt(text.length-1)
            return
        }
        val code = when (usage) {
            in 4..29 -> KeyEvent.KEYCODE_A + usage - 4
            in 30..38 -> KeyEvent.KEYCODE_1 + usage - 30
            39 -> KeyEvent.KEYCODE_0
            44 -> KeyEvent.KEYCODE_SPACE
            45 -> KeyEvent.KEYCODE_MINUS
            46 -> KeyEvent.KEYCODE_EQUALS
            47 -> KeyEvent.KEYCODE_LEFT_BRACKET
            48 -> KeyEvent.KEYCODE_RIGHT_BRACKET
            49 -> KeyEvent.KEYCODE_BACKSLASH
            51 -> KeyEvent.KEYCODE_SEMICOLON
            52 -> KeyEvent.KEYCODE_APOSTROPHE
            53 -> KeyEvent.KEYCODE_GRAVE
            54 -> KeyEvent.KEYCODE_COMMA
            55 -> KeyEvent.KEYCODE_PERIOD
            56 -> KeyEvent.KEYCODE_SLASH
            else -> return
        }
        val meta = if (keys[225] || keys[229]) KeyEvent.META_SHIFT_ON else 0
        val char = keyboard.get(code, meta)
        if (char in 32..126 && text.length < 128) text.append(char.toChar())
    }

    data class Snapshot(val sequence: Int, val active: Boolean, val x: Int, val y: Int,
        val moves: Int, val clicks: Int, val scroll: Int, val text: String, val held: Boolean)

    /** Instrumentation can inspect only the generated local canvas state. */
    fun snapshot() = Snapshot(sequence, active, x, y, moves, clicks, scroll,
        text.toString(), keys.any { it } || buttons.any { it })

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawColor(Color.rgb(18, 32, 48))
        val unit = resources.displayMetrics.density
        paint.style = Paint.Style.FILL
        paint.color = if (selected) Color.rgb(38, 143, 121) else Color.rgb(49, 76, 101)
        val margin = 24 * unit
        canvas.drawRoundRect(margin, margin, (width-margin).coerceAtLeast(margin),
            (height-margin).coerceAtLeast(margin), 16*unit,16*unit,paint)
        paint.color = Color.WHITE
        paint.textSize = 18*unit
        canvas.drawText(context.getString(if (active) R.string.input_active else R.string.input_inactive), margin*2, margin*2, paint)
        paint.textSize = 16*unit
        canvas.drawText(context.getString(R.string.input_counts, moves, clicks, scroll), margin*2, margin*3.5f, paint)
        canvas.drawText(context.getString(R.string.input_text), margin*2, margin*5, paint)
        canvas.drawText(text.takeLast(40).toString(), margin*2, margin*6.5f, paint)
        if (!active) return
        // Cursor lives on the CLIENT's actual input canvas. Hotspot is exact;
        // scale only visual size, not normalized coordinates.
        val px = x.toFloat() / 65535f * (width-1).coerceAtLeast(0)
        val py = y.toFloat() / 65535f * (height-1).coerceAtLeast(0)
        val s = 1.2f * unit
        // Keep the hotspot at the true edge coordinate, but turn the glyph
        // inward at edges so the local owner never loses sight of the cursor.
        val sx = if (px+24*s >= width) -s else s
        val sy = if (py+30*s >= height) -s else s
        arrow.reset()
        arrow.moveTo(px, py)
        arrow.lineTo(px, py+24*sy)
        arrow.lineTo(px+6*sx, py+18*sy)
        arrow.lineTo(px+11*sx, py+28*sy)
        arrow.lineTo(px+16*sx, py+26*sy)
        arrow.lineTo(px+11*sx, py+16*sy)
        arrow.lineTo(px+22*sx, py+16*sy)
        arrow.close()
        paint.color = Color.BLACK
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = 3*unit
        canvas.drawPath(arrow,paint)
        paint.style = Paint.Style.FILL
        paint.color = if (buttons.any { it }) Color.rgb(104,216,191) else Color.WHITE
        canvas.drawPath(arrow,paint)
    }
}
