package org.intralink.android

import android.content.Context

/** The vault implementation and role policy live in the shared Rust library. */
internal object NativeBridge {
    init {
        System.loadLibrary("intralink_android")
    }

    external fun createProfile(path: String, role: Int, name: String, password: ByteArray): String
    external fun openProfile(path: String, password: ByteArray): String
    external fun changeRole(path: String, role: Int, password: ByteArray): String
    external fun renameDevice(path: String, name: String, password: ByteArray): String
    external fun pairDevice(path: String, request: String, password: ByteArray): String
    external fun revokeDevice(path: String, name: String, password: ByteArray): String
    external fun initializeNetwork(context: Context): Boolean
    external fun inspectAvc(buffer: java.nio.ByteBuffer, size: Int, framing: Int): Int
    external fun inspectAvcSps(buffer: java.nio.ByteBuffer, size: Int, framing: Int): Long
    external fun reserveProbe(): Long
    external fun cancelProbe(id: Long)
    external fun runProbe(path: String, request: String, id: Long, password: ByteArray): String
}
