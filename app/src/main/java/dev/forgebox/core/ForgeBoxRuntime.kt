package dev.forgebox.core

import android.content.Context
import dev.forgebox.proto.ForgeProto
import java.io.File
import java.util.concurrent.atomic.AtomicLong

/**
 * Process-local runtime owner. M7 will move ownership behind a lifecycle-aware
 * service boundary; M3 keeps it simple while preserving the single-handle rule.
 */
object ForgeBoxRuntime : ForgeBoxCore {
    private val handle = AtomicLong(0L)
    private val lock = Any()

    fun ensureInitialized(context: Context): Long {
        val existing = handle.get()
        if (existing != 0L) return existing
        synchronized(lock) {
            val again = handle.get()
            if (again != 0L) return again
            val appDataDir = context.filesDir.absolutePath
            val nativeLibDir = context.applicationInfo.nativeLibraryDir ?: File(context.filesDir, "lib").absolutePath
            val dbPath = File(context.filesDir, "internal/db/forgebox.sqlite").absolutePath
            val created = ForgeNative.nativeInit(appDataDir, nativeLibDir, dbPath)
            check(created != 0L) { "Rust runtime initialization returned null handle" }
            handle.set(created)
            return created
        }
    }

    override fun call(request: ForgeProto.Request): ForgeProto.Response {
        val h = handle.get()
        check(h != 0L) { "ForgeBoxRuntime is not initialized" }
        val bytes = ForgeNative.nativeCall(h, request.toByteArray())
        return ForgeProto.Response.parseFrom(bytes)
    }

    fun pollEvents(max: Int = 64): List<ForgeProto.Event> {
        val h = handle.get()
        if (h == 0L) return emptyList()
        return ForgeNative.nativePollEvents(h, max).map { ForgeProto.Event.parseFrom(it) }
    }

    fun pollEventsTimeout(max: Int = 64, timeoutMs: Int = 250): List<ForgeProto.Event> {
        val h = handle.get()
        if (h == 0L) return emptyList()
        return ForgeNative.nativePollEventsTimeout(h, max, timeoutMs).map { ForgeProto.Event.parseFrom(it) }
    }

    fun ptyOpen(
        projectId: String?,
        cols: Int,
        rows: Int,
        name: String? = null,
        envOverridesJson: String = "{}",
        sessionCap: Int = 6,
    ): Long {
        val h = handle.get()
        check(h != 0L) { "ForgeBoxRuntime is not initialized" }
        return ForgeNative.nativePtyOpen(h, projectId, name, envOverridesJson, cols, rows, sessionCap)
    }

    fun ptyWrite(ptyId: Long, data: ByteArray) {
        val h = handle.get()
        check(h != 0L) { "ForgeBoxRuntime is not initialized" }
        ForgeNative.nativePtyWrite(h, ptyId, data)
    }

    fun ptyResize(ptyId: Long, cols: Int, rows: Int) {
        val h = handle.get()
        check(h != 0L) { "ForgeBoxRuntime is not initialized" }
        ForgeNative.nativePtyResize(h, ptyId, cols, rows)
    }

    fun ptySignal(ptyId: Long, signal: Int) {
        val h = handle.get()
        check(h != 0L) { "ForgeBoxRuntime is not initialized" }
        ForgeNative.nativePtySignal(h, ptyId, signal)
    }

    fun ptyClose(ptyId: Long) {
        val h = handle.get()
        if (h == 0L) return
        ForgeNative.nativePtyClose(h, ptyId)
    }

    fun ptyRename(ptyId: Long, name: String) {
        val h = handle.get()
        check(h != 0L) { "ForgeBoxRuntime is not initialized" }
        ForgeNative.nativePtyRename(h, ptyId, name)
    }

    fun ptyStopAll(): Int {
        val h = handle.get()
        if (h == 0L) return 0
        return ForgeNative.nativePtyStopAll(h)
    }

    fun shutdown() {
        val h = handle.getAndSet(0L)
        if (h != 0L) {
            ForgeNative.nativeShutdown(h)
        }
    }

    fun isInitialized(): Boolean = handle.get() != 0L
}
