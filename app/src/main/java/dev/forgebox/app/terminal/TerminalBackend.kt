package dev.forgebox.app.terminal

interface TerminalBackend {
    fun open(projectId: String?, cols: Int, rows: Int): Long
    fun write(ptyId: Long, data: ByteArray)
    fun resize(ptyId: Long, cols: Int, rows: Int)
    fun signal(ptyId: Long, signal: Int)
    fun close(ptyId: Long)
    fun pollEvents(max: Int): List<dev.forgebox.proto.ForgeProto.Event>
}