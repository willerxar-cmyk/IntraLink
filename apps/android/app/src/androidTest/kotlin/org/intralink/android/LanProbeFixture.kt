package org.intralink.android

import android.app.Activity
import android.app.Instrumentation
import android.os.Bundle
import org.json.JSONObject
import java.io.File
import java.nio.file.Files
import java.security.SecureRandom

/** Instrumentation-only explicit private-LAN probe. No screen or input access. */
internal object LanProbeFixture {
    fun run(instrumentation: Instrumentation, args: Bundle) {
        val results = Bundle()
        var directory: File? = null
        var code = Activity.RESULT_CANCELED
        var reservation = 0L
        val password = ByteArray(32).also(SecureRandom()::nextBytes)
        try {
            val direction = requireNotNull(args.getString("lanProbe"))
            check(direction == "inbound" || direction == "outbound")
            val host = requireNotNull(args.getString("host"))
            val port = requireNotNull(args.getString("port")).toInt()
            val peer = requireNotNull(args.getString("peerId"))
            check(port in 1024..65535)
            // ADB instrumentation is the explicit local test operator.
            // The ordinary application never reads these arguments.
            val temp = Files.createTempDirectory(instrumentation.targetContext.cacheDir.toPath(),
                "intralink-lan-test-").toFile()
            directory = temp
            val path = File(temp, "profile.vault").absolutePath
            fun success(value: String): JSONObject = JSONObject(value).also {
                check(it.optBoolean("ok")) { it.optString("error", "Native operation failed") }
            }
            val role = if (direction == "inbound") 1 else 0
            val profile = success(NativeBridge.createProfile(path, role, "LAN test", password))
            val pairing = JSONObject().put("name", "windows-test")
                .put("peer_id", peer).put("direction", direction)
            if (direction == "outbound") pairing.put("host", host).put("port", port)
            success(NativeBridge.pairDevice(path, pairing.toString(), password))
            check(NativeBridge.initializeNetwork(instrumentation.targetContext.applicationContext))
            val request = JSONObject().put("name", "windows-test").put("direction", direction)
            if (direction == "inbound") request.put("host", host).put("port", port)
            val id = NativeBridge.reserveProbe()
            check(id > 0)
            reservation = id
            val identity = profile.getString("public_id")
            instrumentation.sendStatus(1, Bundle().apply {
                putString("stream", "\nLAN_ANDROID_ID $identity\n")
            })
            val surface = args.getString("lanMediaFixture") == "surface"
            val media = surface || args.getString("lanMediaFixture") == "true"
            check(!surface || direction == "inbound")
            fun execute(reservation: Long): String = if (surface) {
                val screenRequest = JSONObject().put("name", "windows-test").put("peer_id", peer)
                    .put("host", host).put("port", port).put("height", 1080).put("fps", 30)
                val secret = password.copyOf()
                try {
                    PortableFrameSource(PortableSurfaceFixture(), 1_000) { true }.use { source ->
                        NativeBridge.runScreen(path, screenRequest.toString(), reservation, secret, source)
                    }
                } finally { secret.fill(0) }
            } else if (media) {
                NativeFixtureBridge.runMediaFixture(path, request.toString(), reservation, password)
            } else NativeBridge.runProbe(path, request.toString(), reservation, password)
            val response = success(execute(id))
            reservation = 0L
            val elapsed = response.getLong(if (surface) "screen_elapsed_ms" else "probe_elapsed_ms")
            val detail = if (media) {
                check(response.getInt("frames") == 60 && response.getLong("media_bytes") > 0)
                " frames=${response.getInt("frames")} bytes=${response.getLong("media_bytes")}"
            } else ""
            check(elapsed >= 0)
            // The completed lease must have released the exclusive vault lock.
            check(success(NativeBridge.revokeDevice(path, "windows-test", password)).getInt("peer_count") == 0)
            val after = NativeBridge.reserveProbe()
            check(after > id)
            reservation = after
            val rejected = JSONObject(execute(after))
            check(!rejected.optBoolean("ok"))
            reservation = 0L
            val next = NativeBridge.reserveProbe()
            check(next > after)
            reservation = next
            NativeBridge.cancelProbe(next)
            reservation = 0L
            results.putString("stream", "\nLAN_ANDROID_PASSED $direction ${elapsed}ms$detail; revocation and lease cleanup passed\n")
            code = Activity.RESULT_OK
        } catch (error: Exception) {
            results.putString("stream", "\nLAN_ANDROID_FAILED: ${error.javaClass.simpleName}: ${error.message}\n")
        } catch (error: LinkageError) {
            results.putString("stream", "\nLAN_ANDROID_FAILED: native library unavailable\n")
        } finally {
            if (reservation > 0) NativeBridge.cancelProbe(reservation)
            password.fill(0)
            directory?.deleteRecursively()
        }
        instrumentation.finish(code, results)
    }
}
