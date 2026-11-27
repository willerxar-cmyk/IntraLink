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
            // Test the actual local capture lease and profile geometry without
            // requesting screen capture or opening a MediaProjection session.
            check(CaptureProfiles.fit(3840, 2160, 1080, 1920, 2, 2) == CaptureSize(1920, 1080))
            check(CaptureProfiles.fit(1080, 2400, 1080, 1920, 16, 16) == CaptureSize(864, 1920))
            check(CaptureProfiles.fit(640, 480, 1080, 1920, 2, 2) == CaptureSize(640, 480))
            check(CaptureProfiles.fit(0, 480, 1080, 1920, 2, 2) == null)
            val capture = CaptureRun.reserve()
            check(capture > 0)
            check(CaptureRun.reserve() == 0L)
            check(CaptureRun.claim(capture))
            check(!CaptureRun.claim(capture))
            CaptureRun.update(CaptureRun.Snapshot(id = capture, width = 1920, height = 1080, fps = 60))
            check(CaptureRun.snapshot().phase == CaptureRun.Phase.RUNNING)
            CaptureRun.cancel(capture)
            CaptureRun.update(CaptureRun.Snapshot(id = capture, buffers = 99))
            check(!CaptureRun.active(capture))
            check(CaptureRun.snapshot().phase == CaptureRun.Phase.STOPPING)
            check(CaptureRun.snapshot().buffers == 0L)
            CaptureRun.finish(capture, CaptureRun.Phase.STOPPED)
            val newCapture = CaptureRun.reserve()
            check(newCapture > capture)
            CaptureRun.cancel(capture)
            check(CaptureRun.active(newCapture))
            CaptureRun.cancel(newCapture)
            check(!CaptureRun.claim(newCapture))
            CaptureRun.finish(newCapture, CaptureRun.Phase.STOPPED)
            // Exercise the exact borrowed direct-buffer JNI boundary with
            // synthetic NAL headers only; no display capture or decode.
            val avc = byteArrayOf(0, 0, 1, 0x65, 0x78)
            val direct = java.nio.ByteBuffer.allocateDirect(avc.size + 3)
            direct.position(3)
            direct.put(avc)
            direct.flip()
            direct.position(3)
            check(AvcBuffers.inspect(direct, avc.size) == (1 shl 5))
            check(direct.position() == 3)
            check(AvcBuffers.inspect(direct, avc.size - 1) == -1)
            check(AvcBuffers.inspect(java.nio.ByteBuffer.wrap(avc), avc.size) == -1)
            check(NativeBridge.inspectAvc(direct.slice(), avc.size, 99) == -1)
            check(NativeBridge.inspectAvc(direct.slice(), avc.size + 1, 0) == -1)
            results.putString("stream", "\nIntraLink JNI onboarding: PASSED\nIntraLink AVC buffer boundary: PASSED\n")
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
