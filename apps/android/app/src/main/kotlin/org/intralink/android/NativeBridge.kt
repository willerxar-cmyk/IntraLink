package org.intralink.android

/** The vault implementation and role policy live in the shared Rust library. */
internal object NativeBridge {
    init {
        System.loadLibrary("intralink_android")
    }

    external fun createProfile(path: String, role: Int, name: String, password: ByteArray): String
    external fun openProfile(path: String, password: ByteArray): String
    external fun changeRole(path: String, role: Int, password: ByteArray): String
    external fun renameDevice(path: String, name: String, password: ByteArray): String
}
