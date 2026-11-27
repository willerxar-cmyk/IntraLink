package org.intralink.android

import android.app.Activity
import android.app.Instrumentation
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.EditText
import android.widget.RadioButton
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.nio.file.Files
import java.security.SecureRandom

/** Real widget and JNI acceptance on a dedicated disposable emulator only. */
internal object DashboardFixture {
    fun run(instrumentation: Instrumentation) {
        val results = Bundle()
        var resultCode = Activity.RESULT_CANCELED
        var activity: Activity? = null
        var directory: File? = null
        val target = instrumentation.targetContext
        val path = File(target.filesDir, "profile.vault")
        val lock = File(target.filesDir, "profile.vault.lock")
        val random = ByteArray(20).also(SecureRandom()::nextBytes)
        val password = random.joinToString("") { "%02x".format(it) }
        random.fill(0)
        val secret = password.toByteArray(Charsets.UTF_8)
        var ownedProfile = false
        var clipboardUsed = false
        try {
            check(Build.HARDWARE == "ranchu" || Build.HARDWARE == "goldfish") { "Disposable emulator required" }
            check(!path.exists() && !lock.exists()) { "Preserve existing profile" }
            val temp = Files.createTempDirectory(target.cacheDir.toPath(), "dashboard-test-").toFile()
            directory = temp
            val peer = JSONObject(NativeBridge.createProfile(File(temp, "base.vault").absolutePath, 0, "Test BASE", secret))
            check(peer.optBoolean("ok")) { "Test BASE creation" }
            val peerId = peer.getString("public_id")
            val app = instrumentation.startActivitySync(Intent(target, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            activity = app

            fun views(root: View): Sequence<View> = sequence {
                yield(root)
                if (root is ViewGroup) for (index in 0 until root.childCount) yieldAll(views(root.getChildAt(index)))
            }
            fun <T> main(block: () -> T): T {
                var result: Result<T>? = null
                instrumentation.runOnMainSync { result = runCatching(block) }
                return checkNotNull(result).getOrThrow()
            }
            fun find(label: String): View? = views(app.findViewById<View>(android.R.id.content))
                .filter { it.isShown }.firstOrNull {
                    it is TextView && (it.text.toString() == label || (it is EditText && it.hint?.toString() == label))
                }
            fun await(label: String) {
                val deadline = SystemClock.elapsedRealtime() + 15000
                while (SystemClock.elapsedRealtime() < deadline) {
                    if (main { find(label) != null }) return
                    SystemClock.sleep(100)
                }
                error("Expected visible widget: $label")
            }
            fun click(label: String) {
                await(label)
                main {
                    val view = checkNotNull(find(label))
                    check(view.isEnabled && view.performClick()) { "Widget action failed: $label" }
                }
                instrumentation.waitForIdleSync()
            }
            fun type(label: String, value: String) {
                await(label)
                main { (find(label) as EditText).setText(value) }
            }
            await("How will you use this device?")
            main {
                val role = find("CLIENT - Receive control") as RadioButton
                role.performClick()
                check(role.isChecked)
            }
            type("Device name", "UX test client")
            type("Vault passphrase", password)
            type("Repeat passphrase", password)
            ownedProfile = true
            click("Create local identity")
            await("No paired devices yet")
            check(main { find("Public identity") == null })
            click("Show my public identity")
            await("Public identity")
            click("Hide my public identity")
            check(main { find("Public identity") == null })
            click("Pair a BASE")
            await("Paste connection card")
            check(main { find("Complete peer public identity") == null })
            click("Enter details manually")
            await("Complete peer public identity")
            click("Hide manual details")
            check(main { find("Complete peer public identity") == null })
            click("Cancel")
            await("No paired devices yet")
            click("Pair a BASE")
            await("Paste connection card")
            // Only the disposable emulator's clipboard is replaced; no existing
            // clipboard contents are read or retained by the test.
            main { ConnectionCard.copy(app, "invalid") }
            clipboardUsed = true
            click("Paste connection card")
            await(target.getString(R.string.card_invalid))
            main { ConnectionCard.copy(app, ConnectionCard(peerId, null, null).encode()) }
            click("Paste connection card")
            await(target.getString(R.string.card_review))
            type("Peer label (letters, digits, underscore or hyphen)", "ux-base")
            type("Vault passphrase", password)
            click("Review pairing")
            val deadline = SystemClock.elapsedRealtime() + 10000
            var approved = false
            while (SystemClock.elapsedRealtime() < deadline && !approved) {
                val root = instrumentation.uiAutomation.rootInActiveWindow
                if (root != null) {
                    val matches = root.findAccessibilityNodeInfosByText(peerId)
                    val buttons = root.findAccessibilityNodeInfosByText("Apply")
                    val confirm = buttons.firstOrNull {
                        it.packageName?.toString() == target.packageName &&
                            it.text?.toString()?.equals("Apply", ignoreCase = true) == true && it.isClickable
                    }
                    if (matches.any { it.text?.toString()?.contains(peerId) == true } && confirm != null) {
                        val before = JSONObject(NativeBridge.openProfile(path.absolutePath, secret))
                        check(before.optInt("peer_count", -1) == 0) { "Pairing occurred before confirmation" }
                        approved = confirm.performAction(AccessibilityNodeInfo.ACTION_CLICK)
                    }
                }
                if (!approved) SystemClock.sleep(150)
            }
            check(approved) { "Full identity review and approval not observed" }
            await("ux-base")
            await("Paired BASE · Not connected")
            await("Share screen preview")
            await("Mouse and keyboard preview")
            val stored = JSONObject(NativeBridge.openProfile(path.absolutePath, secret))
            check(stored.optBoolean("ok") && stored.getInt("peer_count") == 1)
            check(stored.getJSONArray("peers").getJSONObject(0).getString("identity") == peerId)
            check(main { find(peerId) == null }) { "Peer details must be collapsed" }
            results.putString("stream", "\nDASHBOARD_UX_PASS first_run=true details_collapsed=true cancel_returns=true invalid_card_rejected=true full_identity_review=true native_pair=true independent_scopes=true\n")
            resultCode = Activity.RESULT_OK
        } catch (error: Exception) {
            results.putString("stream", "\nDashboard UX FAILED: ${error.javaClass.simpleName}: ${error.message}\n")
        } finally {
            secret.fill(0)
            instrumentation.runOnMainSync {
                if (clipboardUsed) target.getSystemService(android.content.ClipboardManager::class.java).clearPrimaryClip()
                activity?.finish()
            }
            instrumentation.waitForIdleSync()
            if (ownedProfile) { path.delete(); lock.delete() }
            directory?.deleteRecursively()
        }
        instrumentation.finish(resultCode, results)
    }
}
