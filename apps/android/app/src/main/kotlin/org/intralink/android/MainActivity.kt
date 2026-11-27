package org.intralink.android

import android.app.Activity
import android.app.AlertDialog
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.text.InputFilter
import android.text.InputType
import android.view.InputDevice
import android.view.View
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.nio.CharBuffer
import java.nio.charset.StandardCharsets
import java.util.concurrent.Executors

/** Foreground onboarding and explicitly approved one-shot QUIC diagnostics. */
class MainActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private lateinit var body: LinearLayout
    private lateinit var status: TextView
    private var busy = false
    private var foreground = false
    private var lockOnResume = false
    private var activeProbe = 0L
    private val secretFields = mutableListOf<EditText>()
    private val vaultPath: String get() = File(filesDir, "profile.vault").absolutePath

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        if (File(vaultPath).exists()) showUnlock() else showSetup()
    }

    override fun onResume() {
        super.onResume()
        foreground = true
        if (lockOnResume && !busy) returnToLockedPage()
    }

    override fun onPause() {
        foreground = false
        lockOnResume = true
        stopDiagnostic()
        secretFields.forEach { it.text.clear() }
        super.onPause()
    }

    private fun returnToLockedPage() {
        lockOnResume = false
        if (File(vaultPath).exists()) showUnlock() else showSetup()
    }

    private fun stopDiagnostic() {
        if (activeProbe != 0L) {
            NativeBridge.cancelProbe(activeProbe)
        }
    }

    override fun onDestroy() {
        stopDiagnostic()
        clearSecretFields()
        worker.shutdown()
        super.onDestroy()
    }

    private fun clearSecretFields() {
        secretFields.forEach { it.text.clear() }
        secretFields.clear()
    }

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    private fun page(title: String, description: String) {
        clearSecretFields()
        val scroll = ScrollView(this).apply {
            isFillViewport = true
            setBackgroundColor(Color.WHITE)
        }
        body = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(24), dp(24), dp(24), dp(32))
        }
        scroll.addView(body)
        setContentView(scroll)
        if (Build.VERSION.SDK_INT >= 30) {
            scroll.setOnApplyWindowInsetsListener { view, insets ->
                val safe = insets.getInsets(
                    WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime()
                )
                view.setPadding(safe.left, safe.top, safe.right, safe.bottom)
                insets
            }
            scroll.requestApplyInsets()
        }
        label(getString(R.string.brand_label), 14, 0xff087f72.toInt(), true)
        label(title, 27, Color.rgb(20, 36, 54), true)
        label(description, 16)
        status = TextView(this).apply {
            setTextColor(Color.rgb(158, 41, 45))
            textSize = 14f
            accessibilityLiveRegion = View.ACCESSIBILITY_LIVE_REGION_POLITE
        }
        body.addView(status)
    }

    private fun label(text: String, size: Int = 15, color: Int = Color.DKGRAY, bold: Boolean = false, selectable: Boolean = false) {
        body.addView(TextView(this).apply {
            this.text = text
            textSize = size.toFloat()
            setTextColor(color)
            if (bold) setTypeface(typeface, Typeface.BOLD)
            setTextIsSelectable(selectable)
            setPadding(0, dp(6), 0, dp(10))
        })
    }

    private fun field(hint: String, secret: Boolean = false, limit: Int = if (secret) 1024 else 64): EditText {
        val view = EditText(this).apply {
            this.hint = hint
            inputType = if (secret)
                InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD
                else InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            setSingleLine(true)
            isSaveEnabled = false
            filters = arrayOf(InputFilter.LengthFilter(limit))
            if (secret) importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
        }
        if (secret) secretFields.add(view)
        body.addView(view)
        return view
    }

    private fun action(text: String, callback: (Button) -> Unit): Button {
        val button = Button(this).apply {
            this.text = text
            isAllCaps = false
            setOnClickListener { if (!busy) callback(this) }
        }
        body.addView(button)
        return button
    }

    private fun roleChoice(): RadioGroup {
        val group = RadioGroup(this).apply { orientation = RadioGroup.VERTICAL }
        group.addView(RadioButton(this).apply {
            id = BASE
            setText(R.string.role_base)
        })
        group.addView(RadioButton(this).apply {
            id = CLIENT
            setText(R.string.role_client)
        })
        body.addView(group)
        return group
    }

    private fun peripherals(): String {
        val devices = InputDevice.getDeviceIds().asSequence()
            .mapNotNull(InputDevice::getDevice)
            .filterNot { it.isVirtual }
            .toList()
        val keyboard = devices.any { it.keyboardType == InputDevice.KEYBOARD_TYPE_ALPHABETIC }
        val mouse = devices.any { it.supportsSource(InputDevice.SOURCE_MOUSE) || it.supportsSource(InputDevice.SOURCE_TOUCHPAD) }
        val touch = devices.any { it.supportsSource(InputDevice.SOURCE_TOUCHSCREEN) }
        fun state(found: Boolean) = getString(if (found) R.string.detected else R.string.not_detected)
        return listOf(
            getString(R.string.keyboard_status, state(keyboard)),
            getString(R.string.mouse_status, state(mouse)),
            getString(R.string.touch_status, state(touch)),
            getString(if (keyboard || mouse) R.string.base_recommendation else R.string.client_recommendation)
        ).joinToString("\n")
    }

    private fun secret(field: EditText): ByteArray {
        val editable = field.text
        val chars = CharArray(editable.length) { editable[it] }
        editable.clear()
        return try {
            val encoded = StandardCharsets.UTF_8.encode(CharBuffer.wrap(chars))
            try {
                ByteArray(encoded.remaining()).also { encoded.get(it) }
            } finally {
                if (encoded.hasArray()) encoded.array().fill(0)
            }
        } finally {
            chars.fill('\u0000')
        }
    }

    private fun run(button: Button, password: ByteArray, probeId: Long = 0L, operation: (ByteArray) -> String) {
        if (!foreground || busy) {
            password.fill(0)
            if (probeId != 0L) NativeBridge.cancelProbe(probeId)
            return
        }
        if (password.size !in 12..1024) {
            password.fill(0)
            if (probeId != 0L) NativeBridge.cancelProbe(probeId)
            status.setText(R.string.passphrase_length)
            return
        }
        busy = true
        activeProbe = probeId
        button.isEnabled = false
        status.setText(if (probeId == 0L) R.string.processing_vault else R.string.probe_running)
        worker.execute {
            val result = try {
                JSONObject(operation(password))
            } catch (error: Exception) {
                JSONObject().put("ok", false).put("error", getString(R.string.vault_operation_failed))
            } catch (error: LinkageError) {
                JSONObject().put("ok", false).put("error", getString(R.string.native_library_unavailable))
            } finally {
                password.fill(0)
                if (probeId != 0L) NativeBridge.cancelProbe(probeId)
            }
            runOnUiThread {
                busy = false
                activeProbe = 0L
                if (!isFinishing && !isDestroyed && foreground) {
                    button.isEnabled = true
                    if (lockOnResume) returnToLockedPage()
                    else if (result.optBoolean("ok")) showProfile(result)
                    else status.text = result.optString("error", getString(R.string.operation_failed))
                }
            }
        }
    }

    private fun showSetup() {
        page(getString(R.string.setup_title), getString(R.string.setup_description))
        val roles = roleChoice()
        label(peripherals(), 14)
        val name = field(getString(R.string.device_name))
        val password = field(getString(R.string.vault_passphrase), true)
        val confirmation = field(getString(R.string.repeat_passphrase), true)
        action(getString(R.string.create_identity)) { button ->
            val selected = roles.checkedRadioButtonId
            if (selected != BASE && selected != CLIENT) {
                status.setText(R.string.choose_role)
                return@action
            }
            val localName = name.text.toString().trim()
            if (localName.isEmpty()) {
                status.setText(R.string.enter_name)
                return@action
            }
            val first = secret(password)
            val second = secret(confirmation)
            val matched = first.contentEquals(second)
            second.fill(0)
            if (!matched) {
                first.fill(0)
                status.setText(R.string.passphrase_mismatch)
                return@action
            }
            run(button, first) { NativeBridge.createProfile(vaultPath, if (selected == BASE) 0 else 1, localName, it) }
        }
    }

    private fun showUnlock() {
        page(getString(R.string.unlock_title), getString(R.string.unlock_description))
        val password = field(getString(R.string.vault_passphrase), true)
        action(getString(R.string.unlock)) { button -> run(button, secret(password)) { NativeBridge.openProfile(vaultPath, it) } }
    }

    private fun showProfile(profile: JSONObject) {
        val role = profile.optString("role")
        page(profile.optString("name", getString(R.string.local_device)), getString(R.string.role_status, role))
        label(getString(R.string.public_identity), 16, bold = true)
        label(profile.getString("public_id"), 14, selectable = true)
        label(getString(R.string.saved_grants, profile.optInt("peer_count")))
        if (profile.has("revoked_grants")) {
            label(getString(R.string.revoked_grants, profile.optInt("revoked_grants")))
        }
        if (profile.has("probe_elapsed_ms")) {
            label(getString(R.string.probe_result, profile.getLong("probe_elapsed_ms")), 14)
        }
        label(getString(R.string.preview_scope), 14)
        val peers = profile.optJSONArray("peers")
        if (peers != null) {
            for (index in 0 until peers.length()) {
                val peer = peers.getJSONObject(index)
                val name = peer.getString("name")
                val outbound = peer.getString("direction") == "outbound"
                label(getString(R.string.peer_label, name, getString(if (outbound) R.string.outbound else R.string.inbound)), 17, bold = true)
                label(peer.getString("identity"), 13, selectable = true)
                peer.optJSONObject("route")?.let { route ->
                    label(getString(R.string.saved_route, route.getString("host"), route.getInt("port")), 14)
                }
                action(getString(if (outbound) R.string.run_probe else R.string.listen_probe)) { showProbe(peer) }
                if (!outbound && Build.VERSION.SDK_INT >= 32) {
                    action(getString(R.string.screen_share_title)) {
                        startActivity(android.content.Intent(this, ScreenShareActivity::class.java)
                            .putExtra(ScreenShareActivity.EXTRA_NAME, name)
                            .putExtra(ScreenShareActivity.EXTRA_IDENTITY, peer.getString("identity")))
                    }
                }
                action(getString(R.string.revoke_peer)) { showRevoke(name) }
            }
        }
        if (role == "BASE" || role == "HYBRID") action(getString(R.string.pair_client)) { showPair(true) }
        if (role == "CLIENT" || role == "HYBRID") action(getString(R.string.pair_base)) { showPair(false) }
        if (role == "CLIENT" || role == "HYBRID") {
            action(getString(R.string.capture_title)) {
                startActivity(android.content.Intent(this, CaptureActivity::class.java))
            }
        }
        action(getString(R.string.change_role)) { showRoleChange(role) }
        action(getString(R.string.rename_device)) { showRename() }
        action(getString(R.string.lock)) { showUnlock() }
    }

    private fun portField(): EditText = field(getString(R.string.port), limit = 5).apply {
        inputType = InputType.TYPE_CLASS_NUMBER
        setText(R.string.default_port)
    }

    private fun confirm(title: String, message: String, password: EditText, approved: () -> Unit) {
        AlertDialog.Builder(this)
            .setTitle(title)
            .setMessage(message)
            .setNegativeButton(getString(R.string.cancel)) { _, _ -> password.text.clear() }
            .setPositiveButton(getString(R.string.apply)) { _, _ ->
                if (foreground && !busy) approved() else password.text.clear()
            }
            .setOnCancelListener { password.text.clear() }
            .show()
    }

    private fun showPair(outbound: Boolean) {
        page(getString(if (outbound) R.string.pair_client else R.string.pair_base), getString(R.string.pair_description))
        val name = field(getString(R.string.peer_name), limit = 48)
        val identity = field(getString(R.string.peer_identity), limit = 128)
        val address = if (outbound) field(getString(R.string.peer_ip), limit = 15) else null
        val port = if (outbound) portField() else null
        val password = field(getString(R.string.vault_passphrase), true)
        action(getString(R.string.review_pair)) { button ->
            val peerName = name.text.toString().trim()
            val peerId = identity.text.toString().trim()
            val ip = address?.text?.toString()?.trim()
            val selectedPort = port?.text?.toString()?.toIntOrNull()
            if (peerName.isEmpty() || peerId.isEmpty() || (outbound && (ip.isNullOrEmpty() || (selectedPort == null || selectedPort !in 1..65535)))) {
                status.setText(R.string.pair_invalid)
                return@action
            }
            val request = JSONObject().put("name", peerName).put("peer_id", peerId)
                .put("direction", if (outbound) "outbound" else "inbound")
            if (outbound) request.put("host", ip).put("port", selectedPort)
            val route = if (outbound) getString(R.string.saved_route, ip, selectedPort) else getString(R.string.inbound)
            confirm(getString(R.string.review_pair), getString(R.string.pair_review, peerName, peerId, route), password) {
                run(button, secret(password)) { NativeBridge.pairDevice(vaultPath, request.toString(), it) }
            }
        }
        action(getString(R.string.cancel)) { showUnlock() }
    }

    private fun showRevoke(name: String) {
        page(getString(R.string.revoke_peer), getString(R.string.revoke_description, name))
        val password = field(getString(R.string.vault_passphrase), true)
        action(getString(R.string.revoke_peer)) { button ->
            confirm(getString(R.string.revoke_peer), getString(R.string.revoke_description, name), password) {
                run(button, secret(password)) { NativeBridge.revokeDevice(vaultPath, name, it) }
            }
        }
        action(getString(R.string.cancel)) { showUnlock() }
    }

    private fun showProbe(peer: JSONObject) {
        val name = peer.getString("name")
        val direction = peer.getString("direction")
        val outbound = direction == "outbound"
        page(getString(if (outbound) R.string.run_probe else R.string.listen_probe), getString(R.string.probe_description))
        label(name, 17, bold = true)
        label(peer.getString("identity"), 14, selectable = true)
        val route = peer.optJSONObject("route")
        if (outbound && route != null) {
            label(getString(R.string.saved_route, route.getString("host"), route.getInt("port")))
        }
        val address = if (!outbound) field(getString(R.string.local_ip), limit = 15) else null
        val port = if (!outbound) portField() else null
        val password = field(getString(R.string.vault_passphrase), true)
        action(getString(R.string.start_probe)) { button ->
            val ip = address?.text?.toString()?.trim()
            val selectedPort = port?.text?.toString()?.toIntOrNull()
            if (!outbound && (ip.isNullOrEmpty() || (selectedPort == null || selectedPort !in 1..65535))) {
                status.setText(R.string.pair_invalid)
                return@action
            }
            val request = JSONObject().put("name", name).put("direction", direction)
            if (!outbound) request.put("host", ip).put("port", selectedPort)
            confirm(getString(R.string.start_probe), getString(R.string.probe_confirmation, name), password) {
                val pass = secret(password)
                if (pass.size !in 12..1024) {
                    pass.fill(0)
                    status.setText(R.string.passphrase_length)
                    return@confirm
                }
                val id = try {
                    NativeBridge.reserveProbe()
                } catch (error: LinkageError) {
                    pass.fill(0)
                    status.setText(R.string.native_library_unavailable)
                    return@confirm
                }
                if (id == 0L) {
                    pass.fill(0)
                    status.setText(R.string.probe_busy)
                    return@confirm
                }
                run(button, pass, id) {
                    check(NativeBridge.initializeNetwork(applicationContext)) { "Network context unavailable" }
                    NativeBridge.runProbe(vaultPath, request.toString(), id, it)
                }
            }
        }
        // Stop must remain available while the asynchronous diagnostic is busy.
        body.addView(Button(this).apply {
            setText(R.string.stop_probe)
            isAllCaps = false
            setOnClickListener {
                stopDiagnostic()
                if (activeProbe != 0L) status.setText(R.string.probe_stopping)
            }
        })
        action(getString(R.string.cancel)) { showUnlock() }
    }

    private fun showRoleChange(current: String) {
        page(getString(R.string.change_role_title), getString(R.string.change_role_description, current))
        val roles = roleChoice()
        val password = field(getString(R.string.vault_passphrase), true)
        action(getString(R.string.apply_role_change)) { button ->
            val selected = roles.checkedRadioButtonId
            if (selected != BASE && selected != CLIENT) {
                status.setText(R.string.choose_role)
                return@action
            }
            // The passphrase is not copied out of the field until confirmation.
            AlertDialog.Builder(this)
                .setTitle(getString(R.string.confirm_role_change))
                .setMessage(getString(R.string.confirm_role_description))
                .setNegativeButton(getString(R.string.cancel)) { _, _ -> password.text.clear() }
                .setPositiveButton(getString(R.string.apply)) { _, _ ->
                    run(button, secret(password)) {
                        NativeBridge.changeRole(vaultPath, if (selected == BASE) 0 else 1, it)
                    }
                }
                .setOnCancelListener { password.text.clear() }
                .show()
        }
        action(getString(R.string.cancel)) { password.text.clear(); showUnlock() }
    }

    private fun showRename() {
        page(getString(R.string.rename_device), getString(R.string.rename_description))
        val name = field(getString(R.string.new_device_name))
        val password = field(getString(R.string.vault_passphrase), true)
        action(getString(R.string.save_name)) { button ->
            val localName = name.text.toString().trim()
            if (localName.isEmpty()) {
                status.setText(R.string.enter_name)
                return@action
            }
            run(button, secret(password)) { NativeBridge.renameDevice(vaultPath, localName, it) }
        }
        action(getString(R.string.cancel)) { password.text.clear(); showUnlock() }
    }

    companion object {
        private const val BASE = 1001
        private const val CLIENT = 1002
    }
}
