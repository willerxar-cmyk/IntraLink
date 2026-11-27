package org.intralink.android

import android.app.Activity
import android.app.Instrumentation
import android.os.Bundle
import org.json.JSONObject
import java.io.File
import java.nio.file.Files
import java.security.SecureRandom

/**
 * Executes the actual Rust JNI vault operations in a disposable app-private
 * directory. Never opens the user's profile or starts a remote connection.
 */
class OnboardingInstrumentation : Instrumentation() {
    override fun onCreate(arguments: Bundle?) {
        super.onCreate(arguments)
        start()
    }

    override fun onStart() {
        val results = Bundle()
        var directory: File? = null
        var resultCode = Activity.RESULT_CANCELED
        val password = ByteArray(32).also(SecureRandom()::nextBytes)
        val wrongPassword = password.copyOf().also { it[0] = (it[0].toInt() xor 1).toByte() }
        try {
            val temp = Files.createTempDirectory(targetContext.cacheDir.toPath(), "intralink-test-").toFile()
            directory = temp
            val path = File(temp, "profile.vault").absolutePath
            fun parse(value: String) = JSONObject(value)
            fun success(value: String): JSONObject = parse(value).also {
                check(it.optBoolean("ok")) { "Expected successful native operation" }
            }
            fun reject(value: String) {
                check(!parse(value).optBoolean("ok")) { "Expected rejected native operation" }
            }

            reject(NativeBridge.createProfile(path, 99, "Test device", password))
            check(!File(path).exists())
            val initial = success(NativeBridge.createProfile(path, 0, "Test device", password))
            val identity = initial.getString("public_id")
            check(identity.isNotEmpty())
            check(initial.getString("role") == "BASE")
            check(initial.getInt("peer_count") == 0)
            val original = File(path).readBytes()
            reject(NativeBridge.createProfile(path, 1, "Duplicate", password))
            reject(NativeBridge.openProfile(path, wrongPassword))
            reject(NativeBridge.changeRole(path, 1, wrongPassword))
            check(File(path).readBytes().contentEquals(original))

            val renamed = success(NativeBridge.renameDevice(path, "Renamed device", password))
            check(renamed.getString("name") == "Renamed device")
            check(renamed.getString("public_id") == identity)
            val client = success(NativeBridge.changeRole(path, 1, password))
            check(client.getString("role") == "CLIENT")
            check(client.getString("public_id") == identity)
            check(client.getInt("revoked_grants") == 0)
            val reopened = success(NativeBridge.openProfile(path, password))
            check(reopened.getString("role") == "CLIENT")
            check(reopened.getString("public_id") == identity)
            check(reopened.getString("name") == "Renamed device")
            val finalBase = success(NativeBridge.changeRole(path, 0, password))
            check(finalBase.getString("role") == "BASE")
            check(finalBase.getString("public_id") == identity)
            check(finalBase.getInt("peer_count") == 0)
            // Exercise manual bilateral pairing and cancellation before a
            // diagnostic worker is admitted; no real listener is started.
            val peerPath = File(temp, "peer.vault").absolutePath
            val peer = success(NativeBridge.createProfile(peerPath, 1, "Test peer", password))
            val request = JSONObject().put("name", "test-peer")
                .put("peer_id", peer.getString("public_id")).put("direction", "outbound")
                .put("host", "192.168.1.2").put("port", 45100)
            val invalid = JSONObject(request.toString()).put("host", "8.8.8.8")
            reject(NativeBridge.pairDevice(path, invalid.toString(), password))
            val paired = success(NativeBridge.pairDevice(path, request.toString(), password))
            check(paired.getInt("peer_count") == 1)
            check(paired.getJSONArray("peers").getJSONObject(0).getString("identity") == peer.getString("public_id"))
            reject(NativeBridge.pairDevice(path, request.toString(), password))
            val incoming = JSONObject().put("name", "test-base")
                .put("peer_id", identity).put("direction", "inbound")
            check(success(NativeBridge.pairDevice(peerPath, incoming.toString(), password)).getInt("peer_count") == 1)
            check(NativeBridge.initializeNetwork(targetContext.applicationContext))
            val reservation = NativeBridge.reserveProbe()
            check(reservation > 0)
            check(NativeBridge.reserveProbe() == 0L)
            NativeBridge.cancelProbe(reservation)
            val probe = JSONObject().put("name", "test-peer").put("direction", "outbound")
            reject(NativeBridge.runProbe(path, probe.toString(), reservation, password))
            val nextReservation = NativeBridge.reserveProbe()
            check(nextReservation > reservation)
            NativeBridge.cancelProbe(reservation)
            NativeBridge.cancelProbe(nextReservation)
            reject(NativeBridge.revokeDevice(path, "test-peer", wrongPassword))
            check(success(NativeBridge.revokeDevice(path, "test-peer", password)).getInt("peer_count") == 0)
            check(success(NativeBridge.revokeDevice(peerPath, "test-base", password)).getInt("peer_count") == 0)
            results.putString("stream", "\nIntraLink JNI onboarding: PASSED\n")
            resultCode = Activity.RESULT_OK
        } catch (error: Exception) {
            results.putString("stream", "\nIntraLink JNI onboarding: FAILED: " + error.javaClass.simpleName + "\n")
        } catch (error: LinkageError) {
            results.putString("stream", "\nIntraLink JNI onboarding: native library unavailable\n")
        } finally {
            password.fill(0)
            wrongPassword.fill(0)
            directory?.deleteRecursively()
        }
        finish(resultCode, results)
    }
}
