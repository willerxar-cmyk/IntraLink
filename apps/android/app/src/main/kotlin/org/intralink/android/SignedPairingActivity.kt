package org.intralink.android

import android.app.Activity
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Intent
import android.os.CancellationSignal
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.os.SystemClock
import android.text.InputFilter
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.Button
import android.widget.CheckBox
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.nio.CharBuffer
import java.nio.charset.StandardCharsets
import java.util.concurrent.Executors

/** Local, offline, role-directed signed enrollment. No network session authority. */
class SignedPairingActivity : Activity() {
    private lateinit var localId: String
    private lateinit var role: String
    private lateinit var body: LinearLayout
    private lateinit var status: TextView
    private var password: EditText? = null
    private var foreground = false
    private var busy = false
    private val worker = Executors.newSingleThreadExecutor()
    private val handler = Handler(Looper.getMainLooper())
    private var expiryTask: Runnable? = null
    private var importField: EditText? = null
    private var importSignal: CancellationSignal? = null
    private var importStarted = 0L
    private var pendingImport: Pair<EditText, String?>? = null
    private val vaultPath: String get() = File(filesDir, "profile.vault").absolutePath

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        localId = intent.getStringExtra(IDENTITY).orEmpty()
        role = intent.getStringExtra(ROLE).orEmpty()
        if (!Regex("[0-9a-f]{64}").matches(localId) || role !in listOf("BASE", "CLIENT", "HYBRID")) {
            finish()
            return
        }
        menu()
    }

    override fun onResume() {
        super.onResume()
        foreground = true
        pendingImport?.let { (field, code) ->
            pendingImport = null
            applyImport(field, code)
        }
    }

    override fun onPause() {
        foreground = false
        password?.text?.clear()
        clearExpiry()
        // The parent dashboard relocks its vault on return. Never restore a
        // review or a signed code from saved state after backgrounding.
        if (importField == null && !isFinishing) {
            importSignal?.cancel()
            finish()
        }
        super.onPause()
    }

    override fun onDestroy() {
        clearExpiry()
        importSignal?.cancel()
        pendingImport = null
        importField = null
        password?.text?.clear()
        worker.shutdown()
        super.onDestroy()
    }

    private fun dp(value: Int) = InterfaceStyle.dp(this, value)

    private fun clearExpiry() {
        expiryTask?.let(handler::removeCallbacks)
        expiryTask = null
    }

    private fun page(title: Int, description: Int) {
        clearExpiry()
        password?.text?.clear()
        password = null
        val scroll = ScrollView(this).apply {
            isFillViewport = true
            setBackgroundColor(InterfaceStyle.background)
        }
        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(18), dp(24), dp(18), dp(32))
            isFocusableInTouchMode = true
        }
        val wrapper = android.widget.FrameLayout(this)
        wrapper.addView(column, android.widget.FrameLayout.LayoutParams(-1, -2, Gravity.TOP or Gravity.CENTER_HORIZONTAL))
        wrapper.addOnLayoutChangeListener { v, left, _, right, _, oldLeft, _, oldRight, _ ->
            if (right-left != oldRight-oldLeft) {
                val width = minOf(v.width, dp(720))
                if (width > 0 && column.layoutParams.width != width)
                    column.layoutParams = column.layoutParams.apply { this.width = width }
            }
        }
        scroll.addView(wrapper)
        body = column
        setContentView(scroll)
        column.requestFocus()
        if (Build.VERSION.SDK_INT >= 30) {
            scroll.setOnApplyWindowInsetsListener { view, insets ->
                val safe = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
                view.setPadding(safe.left,safe.top,safe.right,safe.bottom)
                insets
            }
            scroll.requestApplyInsets()
        }
        label(getString(R.string.brand_label), 13, true)
        label(getString(title), 27, true)
        label(getString(description), 16)
        status = label("", 14)
        status.accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
    }

    private fun label(value: String, size: Int = 15, bold: Boolean = false, selectable: Boolean = false): TextView =
        TextView(this).apply {
            text = value
            textSize = size.toFloat()
            setTextColor(if (bold) InterfaceStyle.ink else InterfaceStyle.muted)
            if (bold) {
                setTypeface(typeface, android.graphics.Typeface.BOLD)
                if (Build.VERSION.SDK_INT >= 28) isAccessibilityHeading = true
            }
            setTextIsSelectable(selectable)
            setPadding(0,dp(6),0,dp(10))
            body.addView(this)
        }

    private fun action(id: Int, primary: Boolean = false, callback: (Button)->Unit): Button =
        Button(this).apply {
            setText(id)
            InterfaceStyle.button(this, if (primary) InterfaceStyle.Action.PRIMARY else InterfaceStyle.Action.SECONDARY)
            setOnClickListener { if (this@SignedPairingActivity.foreground && !busy) callback(this) }
            body.addView(this)
        }

    private fun field(hintId: Int, limit: Int, secret: Boolean = false): EditText =
        EditText(this).apply {
            hint = getString(hintId)
            inputType = InputType.TYPE_CLASS_TEXT or if (secret) InputType.TYPE_TEXT_VARIATION_PASSWORD else InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            setSingleLine(true)
            isSaveEnabled = false
            filters = arrayOf(InputFilter.LengthFilter(limit))
            if (secret) importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            InterfaceStyle.field(this)
            body.addView(this)
        }

    private fun menu() {
        page(R.string.signed_pairing, R.string.signed_pairing_description)
        if (role == "CLIENT" || role == "HYBRID") {
            action(R.string.signed_create, true) { issueForm() }
            action(R.string.signed_confirm) { codeForm(false) }
            action(R.string.signed_cancel_pending) { inspect(request("cancel")) }
        }
        if (role == "BASE" || role == "HYBRID") {
            action(R.string.signed_accept, true) { codeForm(true) }
        }
        label(getString(R.string.public_identity), 15, true)
        label(localId, 14, selectable = true)
        action(R.string.cancel) { finish() }
    }

    private fun request(stage: String) = JSONObject().put("stage", stage).put("local_id",localId)

    private fun issueForm() {
        page(R.string.signed_create, R.string.signed_create_description)
        val identity = field(R.string.peer_identity, 128)
        action(R.string.paste_card) {
            val card = ConnectionCard.paste(this)
            identity.setText(if (card?.address == null) card?.identity.orEmpty() else "")
            status.setText(if (identity.text.isEmpty()) R.string.card_invalid else R.string.card_review)
        }
        val address = field(R.string.local_ip,15).apply { setText(ConnectionCard.localAddress(this@SignedPairingActivity).orEmpty()) }
        val port = field(R.string.port,5).apply { inputType=InputType.TYPE_CLASS_NUMBER; setText(R.string.default_port) }
        action(R.string.review_pair,true) {
            val selected = port.text.toString().toIntOrNull()
            if (selected == null || selected !in 1..65535) status.setText(R.string.pair_invalid)
            else inspect(request("issue").put("controller_id",identity.text.toString().trim()).put("host",address.text.toString().trim()).put("port",selected))
        }
        action(R.string.cancel) { menu() }
    }

    private fun pasteCode(): String? {
        val clip = getSystemService(ClipboardManager::class.java).primaryClip ?: return null
        if (clip.itemCount != 1) return null
        val text = clip.getItemAt(0).text ?: return null
        if (text.length > 2052) return null
        return text.toString().trim().takeIf { it.length in 1..2048 && it.all { ch -> ch.code in 33..126 } }
    }

    private fun codeForm(accept: Boolean) {
        page(if (accept) R.string.signed_accept else R.string.signed_confirm,
            if (accept) R.string.signed_accept_description else R.string.signed_confirm_description)
        val code = field(R.string.signed_code, 2052)
        action(R.string.signed_paste) {
            val value = pasteCode()
            code.setText(value.orEmpty())
            if (value == null) status.setText(R.string.card_invalid) else status.text = ""
        }
        action(R.string.signed_import_image) { chooseImage(code) }
        val name = field(R.string.peer_name,48)
        action(R.string.review_pair,true) {
            inspect(request(if (accept) "accept" else "confirm")
                .put("code",code.text.toString().trim()).put("name",name.text.toString().trim()))
        }
        action(R.string.cancel) { menu() }
    }

    @Suppress("DEPRECATION")
    private fun chooseImage(field: EditText) {
        if (importField != null || importSignal != null) return
        importField = field
        importStarted = SystemClock.elapsedRealtime()
        field.text.clear()
        status.text = ""
        try {
            startActivityForResult(Intent(Intent.ACTION_GET_CONTENT).apply {
                type = "image/*"
                addCategory(Intent.CATEGORY_OPENABLE)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }, QR_IMAGE)
        } catch (_: Exception) {
            importField = null
            status.setText(R.string.signed_import_failed)
        }
    }

    @Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != QR_IMAGE) return
        val field = importField ?: return
        importField = null
        val uri = data?.data
        if (isFinishing || isDestroyed || resultCode != RESULT_OK || uri == null ||
            SystemClock.elapsedRealtime() - importStarted >= 120_000) return
        val signal = CancellationSignal()
        importSignal = signal
        busy = true
        status.setText(R.string.signed_import_running)
        worker.execute {
            val code = try { SignedQrImport.read(applicationContext, uri, signal) }
            catch (_: Exception) { null }
            runOnUiThread {
                importSignal = null
                busy = false
                if (!isFinishing && !isDestroyed && !signal.isCanceled) {
                    if (foreground) applyImport(field, code) else pendingImport = field to code
                }
            }
        }
    }

    private fun applyImport(field: EditText, code: String?) {
        field.setText(code.orEmpty())
        status.setText(if (code == null) R.string.signed_import_failed else R.string.signed_import_review)
    }

    private fun inspect(input: JSONObject) {
        val exact = input.toString()
        val review = try { JSONObject(NativeBridge.inspectEnrollment(exact)) }
        catch (_: Exception) { status.setText(R.string.vault_operation_failed); return }
        catch (_: LinkageError) { status.setText(R.string.native_library_unavailable); return }
        if (!review.optBoolean("ok")) {
            status.text = review.optString("error",getString(R.string.operation_failed))
            return
        }
        reviewPage(exact,review)
    }

    private fun reviewPage(exact: String, review: JSONObject) {
        val stage=review.getString("stage")
        val cancel=stage=="cancel"
        page(R.string.signed_review,if (cancel) R.string.signed_cancel_description else R.string.signed_review_description)
        if (!cancel) {
            label(getString(R.string.signed_base_identity),16,true)
            label(review.getString("controller_id"),14,selectable=true)
            label(getString(R.string.signed_client_identity),16,true)
            label(review.getString("host_id"),14,selectable=true)
            val route=review.getJSONObject("route")
            label(getString(R.string.saved_route,route.getString("host"),route.getInt("port")))
            if (!review.isNull("name")) label(review.getString("name"),16,true)
            if (stage=="issue") {
                val ttl=review.getInt("ttl")
                label(resources.getQuantityString(R.plurals.signed_lifetime,ttl,ttl))
            }
            else label(getString(R.string.signed_expiry,java.text.DateFormat.getDateTimeInstance().format(java.util.Date(review.getLong("expires")*1000))))
        }
        val verified = CheckBox(this).apply {
            setText(if(cancel) R.string.signed_cancel_verify else R.string.signed_verify)
            setTextColor(InterfaceStyle.ink)
            minHeight=dp(56)
            body.addView(this)
        }
        val pass=field(R.string.vault_passphrase,1024,true)
        password=pass
        val submit=action(R.string.apply,true) { button ->
            if (verified.isChecked) commit(button,exact,pass,verified)
        }.apply { isEnabled=false }
        verified.setOnCheckedChangeListener { _, checked -> submit.isEnabled=checked && !busy }
        action(R.string.cancel) { menu() }
    }

    private fun extract(field: EditText): ByteArray {
        val editable=field.text
        val chars=CharArray(editable.length){editable[it]}
        editable.clear()
        return try {
            val buffer=StandardCharsets.UTF_8.encode(CharBuffer.wrap(chars))
            try { ByteArray(buffer.remaining()).also { buffer.get(it) } }
            finally { if(buffer.hasArray()) buffer.array().fill(0) }
        } finally { chars.fill('\u0000') }
    }

    private fun commit(button: Button, exact: String, field: EditText, verified: CheckBox) {
        val secret=extract(field)
        if (secret.size !in 12..1024 || !foreground || busy) {
            secret.fill(0)
            status.setText(R.string.passphrase_length)
            return
        }
        busy=true
        button.isEnabled=false
        verified.isEnabled=false
        status.setText(R.string.processing_vault)
        worker.execute {
            val result=try { JSONObject(NativeBridge.commitEnrollment(vaultPath,exact,secret)) }
            catch (_: Exception) { JSONObject().put("ok",false).put("error",getString(R.string.vault_operation_failed)) }
            catch (_: LinkageError) { JSONObject().put("ok",false).put("error",getString(R.string.native_library_unavailable)) }
            finally { secret.fill(0) }
            runOnUiThread {
                busy=false
                if (foreground && !isFinishing && !isDestroyed) {
                    if(result.optBoolean("ok")) showResult(result.getJSONObject("enrollment"))
                    else {
                        status.text=result.optString("error",getString(R.string.operation_failed))
                        verified.isEnabled=true
                        button.isEnabled=verified.isChecked
                    }
                }
            }
        }
    }

    private fun showResult(data: JSONObject) {
        val code=data.optString("code")
        if (code.isEmpty()) {
            page(R.string.signed_complete,R.string.signed_complete_description)
            action(R.string.signed_done,true) { finish() }
            return
        }
        page(R.string.signed_ready,R.string.signed_ready_description)
        val expires=data.getLong("expires")
        val remaining=(expires-System.currentTimeMillis()/1000).coerceIn(0,300)
        val deadline=SystemClock.elapsedRealtime()+remaining*1000
        val qr=SignedQrView(this).apply { setCode(data.getInt("qr_width"),data.getString("qr_modules")) }
        body.addView(qr,LinearLayout.LayoutParams(-1,dp(340)))
        val copy=action(R.string.signed_copy,true) {
            if (System.currentTimeMillis()/1000 < expires && SystemClock.elapsedRealtime() < deadline) {
                val clip=ClipData.newPlainText(getString(R.string.signed_code),code)
                if (Build.VERSION.SDK_INT >= 33) {
                    clip.description.extras=PersistableBundle().apply { putBoolean(ClipDescription.EXTRA_IS_SENSITIVE,true) }
                }
                getSystemService(ClipboardManager::class.java).setPrimaryClip(clip)
                status.setText(R.string.card_copied)
            } else expire(qr,it)
        }
        action(R.string.signed_done) { finish() }
        val task=object: Runnable {
            override fun run() {
                if (!foreground || isFinishing) return
                if (System.currentTimeMillis()/1000 >= expires || SystemClock.elapsedRealtime() >= deadline) expire(qr,copy)
                else handler.postDelayed(this,500)
            }
        }
        expiryTask=task
        task.run()
    }

    private fun expire(qr: View, copy: Button) {
        qr.visibility=View.GONE
        copy.isEnabled=false
        status.setText(R.string.signed_expired)
        clearExpiry()
    }

    companion object {
        private const val QR_IMAGE = 501
        const val IDENTITY = "identity"
        const val ROLE = "role"
    }
}

/** Integer module size, white quiet zone, no interpolation or image retention. */
internal class SignedQrView(context: android.content.Context) : View(context) {
    private val paint=Paint().apply { color=Color.BLACK; isAntiAlias=false }
    private var modules=0
    private var cells=""
    init { contentDescription=context.getString(R.string.signed_qr_description) }
    fun setCode(size: Int, value: String) {
        require(size in 21..177 && (size-21)%4==0 && value.length==size*size && value.all { it=='0'||it=='1' })
        modules=size
        cells=value
        invalidate()
    }
    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawColor(Color.WHITE)
        if (modules == 0) return
        val size=modules+8
        val scale=minOf(width,height)/size
        if(scale<1)return
        val x0=(width-size*scale)/2+4*scale
        val y0=(height-size*scale)/2+4*scale
        for(y in 0 until modules) for(x in 0 until modules) {
            if(cells[y*modules+x]=='1') canvas.drawRect(
                (x0+x*scale).toFloat(),(y0+y*scale).toFloat(),
                (x0+(x+1)*scale).toFloat(),(y0+(y+1)*scale).toFloat(),paint)
        }
    }
}
