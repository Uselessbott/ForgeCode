package dev.forgebox.core

/**
 * Single JNI boundary. All request/response payloads are protobuf bytes defined
 * in `proto/forgebox.proto`.
 */
object ForgeNative {
    init {
        System.loadLibrary("forgebox_core")
    }

    external fun nativeVersion(): String
    external fun nativeInit(appDataDir: String, nativeLibDir: String, dbPath: String): Long
    external fun nativeCall(handle: Long, request: ByteArray): ByteArray
    external fun nativePollEvents(handle: Long, max: Int): Array<ByteArray>
    external fun nativePtyOpen(handle: Long, projectId: String?, cols: Int, rows: Int): Long
    external fun nativePtyWrite(handle: Long, ptyId: Long, data: ByteArray)
    external fun nativePtyResize(handle: Long, ptyId: Long, cols: Int, rows: Int)
    external fun nativePtySignal(handle: Long, ptyId: Long, signal: Int)
    external fun nativePtyClose(handle: Long, ptyId: Long)
    external fun nativeShutdown(handle: Long)

    val version: String
        get() = nativeVersion()
}
