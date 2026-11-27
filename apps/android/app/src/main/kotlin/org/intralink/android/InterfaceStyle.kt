package org.intralink.android

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout

/** Shared native visual tokens. No connection state or authorization lives here. */
internal object InterfaceStyle {
    val background = 0xfff3f6fa.toInt()
    val surface = 0xffffffff.toInt()
    val ink = 0xff142436.toInt()
    val muted = 0xff46576c.toInt()
    val accent = 0xff08685c.toInt()
    val border = 0xffcbd6e2.toInt()
    val danger = 0xffa12535.toInt()
    enum class Action { PRIMARY, SECONDARY, DESTRUCTIVE }

    fun dp(context: Context, value: Int): Int =
        (value * context.resources.displayMetrics.density + 0.5f).toInt()

    private fun shape(context: Context, color: Int, radius: Int, stroke: Int? = null) =
        GradientDrawable().apply {
            setColor(color)
            cornerRadius = dp(context, radius).toFloat()
            if (stroke != null) setStroke(dp(context, 1), stroke)
        }

    fun card(context: Context): LinearLayout = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        background = shape(context, surface, 18, border)
        setPadding(dp(context, 16), dp(context, 12), dp(context, 16), dp(context, 12))
        layoutParams = LinearLayout.LayoutParams(-1, -2).apply {
            topMargin = dp(context, 10)
            bottomMargin = dp(context, 8)
        }
    }

    fun button(view: Button, kind: Action = Action.SECONDARY) {
        val context = view.context
        val fill = if (kind == Action.PRIMARY) accent else surface
        val fg = when (kind) {
            Action.PRIMARY -> surface
            Action.DESTRUCTIVE -> danger
            Action.SECONDARY -> ink
        }
        val stroke = if (kind == Action.PRIMARY) null else border
        val enabled = android.graphics.drawable.StateListDrawable().apply {
            addState(intArrayOf(-android.R.attr.state_enabled), shape(context, background, 12, border))
            addState(intArrayOf(), shape(context, fill, 12, stroke))
        }
        view.backgroundTintList = null
        view.background = RippleDrawable(ColorStateList.valueOf(0x22347788), enabled, shape(context, surface, 12))
        view.setTextColor(ColorStateList(
            arrayOf(intArrayOf(-android.R.attr.state_enabled), intArrayOf()),
            intArrayOf(muted, fg)
        ))
        view.isAllCaps = false
        view.textSize = 15f
        view.minHeight = dp(context, 52)
        view.setPadding(dp(context, 16), dp(context, 12), dp(context, 16), dp(context, 12))
        view.layoutParams = LinearLayout.LayoutParams(-1, -2).apply {
            topMargin = dp(context, 5)
            bottomMargin = dp(context, 5)
        }
    }

    fun field(view: EditText) {
        val context = view.context
        val states = android.graphics.drawable.StateListDrawable().apply {
            addState(intArrayOf(android.R.attr.state_focused), shape(context, surface, 12, accent))
            addState(intArrayOf(), shape(context, surface, 12, border))
        }
        view.backgroundTintList = null
        view.background = states
        view.setTextColor(ink)
        view.setHintTextColor(muted)
        view.textSize = 16f
        view.minHeight = dp(context, 54)
        view.setPadding(dp(context, 14), dp(context, 12), dp(context, 14), dp(context, 12))
        view.layoutParams = LinearLayout.LayoutParams(-1, -2).apply {
            topMargin = dp(context, 5)
            bottomMargin = dp(context, 5)
        }
    }

}
