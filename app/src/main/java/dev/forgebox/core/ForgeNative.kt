package dev.forgebox.core

/**
 * The single JNI façade. M3 replaces these two handshake methods with the
 * protobuf `call`/`pollEvents` API while keeping this class as the boundary.
 */
object ForgeNative {
    init {
        System.loadLibrary("forgebox_core")
    }

    external fun nativeVersion(): String
    external fun nativeEcho(input: String): String

    val version: String
        get() = nativeVersion()
}
