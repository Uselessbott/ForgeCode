package dev.forgebox.data

import dev.forgebox.proto.ForgeProto

data class BuildRunResultModel(
    val ptyId: Long,
    val command: String,
    val message: String,
    val pythonMissing: Boolean,
)

data class BuildDiagnostic(
    val filePath: String,
    val line: Int,
    val column: Int,
    val message: String,
    val severity: String = "error",
)

data class BuildHistoryEntry(
    val id: String,
    val command: String,
    val kind: String,
    val startedAtMs: Long,
    var endedAtMs: Long? = null,
    var exitCode: Int? = null,
    var output: String = "",
    val diagnostics: MutableList<BuildDiagnostic> = mutableListOf(),
    var resourceSummary: String = "unavailable",
)

object BuildDiagnosticsParser {
    private val patterns = listOf(
        Regex("""(?<file>[A-Za-z0-9_./\\-]+\.py):(?<line>\d+):(?<col>\d+):\s*(?<msg>.+)"""),
        Regex("""File "(?<file>[^"]+)", line (?<line>\d+)(?:, in .*)?"""),
        Regex("""(?<file>[A-Za-z0-9_./\\-]+\.[A-Za-z0-9]+):(?<line>\d+):(?<col>\d+):\s*(?<sev>error|warning|note):\s*(?<msg>.+)"""),
        Regex("""(?<file>[A-Za-z0-9_./\\-]+\.[A-Za-z0-9]+):(?<line>\d+):\s*(?<msg>.+)"""),
    )

    fun parse(text: String): List<BuildDiagnostic> {
        val out = mutableListOf<BuildDiagnostic>()
        text.lineSequence().forEach { raw ->
            val line = raw.trimEnd()
            for (rx in patterns) {
                val m = rx.find(line) ?: continue
                val file = m.groups["file"]?.value?.replace('\\', '/') ?: continue
                val ln = m.groups["line"]?.value?.toIntOrNull() ?: continue
                val col = m.groups["col"]?.value?.toIntOrNull() ?: 1
                val sev = m.groups["sev"]?.value ?: "error"
                val msg = m.groups["msg"]?.value ?: line
                out += BuildDiagnostic(filePath = file, line = ln, column = col, message = msg.trim(), severity = sev)
                break
            }
        }
        return out.distinctBy { Triple(it.filePath, it.line, it.message) }
    }
}
