package org.intralink.android

import android.app.Activity
import android.os.Bundle
import android.graphics.Color
import android.text.InputFilter
import android.text.InputType
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.nio.CharBuffer
import java.nio.charset.StandardCharsets

/** A separate, explicitly approved input-only app canvas, never a system service. */
class InputPreviewActivity : Activity() {
    private lateinit var canvas: InputCanvas
    private lateinit var form: LinearLayout
    private lateinit var address: EditText
    private lateinit var port: EditText
    private lateinit var password: EditText
    private lateinit var status: TextView
    private lateinit var start: Button
    private var session = 0L
    private var foreground = false
    private var name = ""
    private var identity = ""

    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        name = intent.getStringExtra(ScreenShareActivity.EXTRA_NAME).orEmpty()
        identity = intent.getStringExtra(ScreenShareActivity.EXTRA_IDENTITY).orEmpty()
        if (!name.matches(Regex("[A-Za-z0-9_-]{1,48}")) || !identity.matches(Regex("[0-9a-f]{64}"))) {
            finish()
            return
        }
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE or WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16),dp(16),dp(16),dp(16))
            setBackgroundColor(Color.WHITE)
        }
        root.addView(TextView(this).apply {
            textSize = 22f
            setText(R.string.input_preview)
        })
        root.addView(TextView(this).apply { text = getString(R.string.input_review, name, identity) })
        form = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        address = field(R.string.local_ip,15).apply { setText(ConnectionCard.localAddress(this@InputPreviewActivity).orEmpty()) }
        port = field(R.string.port,5).apply { inputType=InputType.TYPE_CLASS_NUMBER; setText(R.string.default_port) }
        password = field(R.string.vault_passphrase,1024).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
        }
        for (field in listOf(address,port,password)) form.addView(field)
        root.addView(form)
        status = TextView(this).apply { setText(R.string.input_inactive) }
        root.addView(status)
        canvas = InputCanvas(this)
        root.addView(canvas, LinearLayout.LayoutParams(-1,0,1f))
        start = Button(this).apply {
            setText(R.string.input_approve)
            isAllCaps=false
            setOnClickListener { approve() }
        }
        root.addView(start)
        root.addView(Button(this).apply {
            setText(R.string.capture_stop)
            isAllCaps=false
            setOnClickListener { stop(); finish() }
        })
        setContentView(root)
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            root.setOnApplyWindowInsetsListener { v, inset ->
                val safe = inset.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                v.setPadding(dp(16)+safe.left,dp(16)+safe.top,dp(16)+safe.right,dp(16)+safe.bottom)
                inset
            }
            root.requestApplyInsets()
        }
    }

    private fun dp(n: Int) = (n * resources.displayMetrics.density).toInt()
    private fun field(label: Int, length: Int) = EditText(this).apply {
        setHint(label)
        setSingleLine(true)
        isSaveEnabled=false
        filters=arrayOf(InputFilter.LengthFilter(length))
    }

    private fun approve() {
        if (!foreground || !hasWindowFocus() || session != 0L) return
        val ip=address.text.toString().trim()
        val udp=port.text.toString().toIntOrNull()
        if (!ConnectionCard.privateIp(ip) || udp == null || udp !in 1..65535) {
            status.setText(R.string.pair_invalid); return
        }
        val chars=CharArray(password.text.length) { password.text[it] }
        password.text.clear()
        val secret=try {
            val encoded=StandardCharsets.UTF_8.encode(CharBuffer.wrap(chars))
            try { ByteArray(encoded.remaining()).also { encoded.get(it) } }
            finally { if (encoded.hasArray()) encoded.array().fill(0) }
        } finally { chars.fill('\u0000') }
        if (secret.size !in 12..1024) {
            secret.fill(0); status.setText(R.string.passphrase_length); return
        }
        val id=try { NativeBridge.reserveProbe() }
        catch (_: Exception) { 0L } catch (_: LinkageError) { 0L }
        if (id == 0L) {
            secret.fill(0); status.setText(R.string.probe_busy); return
        }
        session=id
        canvas.begin()
        form.visibility=View.GONE
        getSystemService(android.view.inputmethod.InputMethodManager::class.java)
            .hideSoftInputFromWindow(password.windowToken, 0)
        start.isEnabled=false
        status.setText(R.string.input_waiting)
        val payload=JSONObject().put("name",name).put("peer_id",identity)
            .put("host",ip).put("port",udp).toString()
        val path=File(filesDir,"profile.vault").absolutePath
        try {
            Thread({
                var success=false
                try {
                    check(NativeBridge.initializeNetwork(applicationContext))
                    success=JSONObject(NativeBridge.runInput(path,payload,id,secret,canvas)).optBoolean("ok")
                } catch (_: Exception) {} catch (_: LinkageError) {}
                finally {
                    secret.fill(0)
                    NativeBridge.cancelProbe(id)
                    runOnUiThread {
                        if (session == id) {
                            session=0
                            canvas.stop()
                            if (!isFinishing && !isDestroyed) {
                                status.setText(if (success) R.string.input_complete else R.string.input_stopped)
                                form.visibility=View.VISIBLE
                                start.isEnabled=true
                            }
                        }
                    }
                }
            },"IntraLink-app-input").start()
        } catch (_: Exception) {
            secret.fill(0)
            stop()
            finish()
        }
    }

    private fun stop() {
        if (::canvas.isInitialized) canvas.stop()
        if (::password.isInitialized) password.text.clear()
        if (session != 0L) NativeBridge.cancelProbe(session)
    }

    override fun onResume() { super.onResume(); foreground=true }
    override fun onPause() { foreground=false; stop(); super.onPause() }
    override fun onWindowFocusChanged(focused: Boolean) {
        super.onWindowFocusChanged(focused)
        if (!focused && session != 0L) stop()
    }
    override fun onDestroy() { stop(); super.onDestroy() }
}
