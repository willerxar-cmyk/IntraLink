package org.intralink.android

/** Explicit opt-in native fixture build only; no method exists in standard native library. */
internal object NativeFixtureBridge {
    init { System.loadLibrary("intralink_android") }
    external fun runMediaFixture(path: String, request: String, id: Long, password: ByteArray): String
}
