package dev.forgebox.app.terminal

object TerminalDiffParser {
    fun parse(bytes: ByteArray, fallback: TerminalRenderState): TerminalRenderState = runCatching {
        val text = bytes.toString(Charsets.UTF_8)
        val hasRows = Regex("\"rows\"\\s*:").containsMatchIn(text)
        val hasCols = Regex("\"cols\"\\s*:").containsMatchIn(text)
        val hasLines = Regex("\"lines\"\\s*:").containsMatchIn(text)
        if (!hasRows && !hasCols && !hasLines) return@runCatching fallback
        val rows = intField(text, "rows")?.coerceAtLeast(1) ?: fallback.rows
        val cols = intField(text, "cols")?.coerceAtLeast(1) ?: fallback.cols
        val cursor = intArrayField(text, "cursor")
        val cursorRow = (cursor.getOrNull(0) ?: 0).coerceIn(0, rows - 1)
        val cursorCol = (cursor.getOrNull(1) ?: 0).coerceIn(0, cols - 1)
        val parsedLines = stringArrayField(text, "lines")
        val lines = List(rows) { i -> parsedLines.getOrNull(i) ?: "" }
        TerminalRenderState(rows = rows, cols = cols, lines = lines, cursorRow = cursorRow, cursorCol = cursorCol)
    }.getOrElse { fallback }

    private fun intField(text: String, name: String): Int? {
        val m = Regex("\"$name\"\\s*:\\s*(-?\\d+)").find(text) ?: return null
        return m.groupValues[1].toIntOrNull()
    }

    private fun intArrayField(text: String, name: String): List<Int> {
        val m = Regex("\"$name\"\\s*:\\s*\\[(.*?)\\]").find(text) ?: return emptyList()
        return m.groupValues[1].split(',').mapNotNull { it.trim().toIntOrNull() }
    }

    private fun stringArrayField(text: String, name: String): List<String> {
        val key = "\"$name\"\\s*:\\s*\\[".toRegex().find(text) ?: return emptyList()
        val start = key.range.last + 1
        var i = start
        val out = mutableListOf<String>()
        while (i < text.length) {
            while (i < text.length && text[i].isWhitespace()) i++
            if (i >= text.length) break
            if (text[i] == ']') break
            if (text[i] == ',') { i++; continue }
            if (text[i] != '"') { i++; continue }
            i++
            val sb = StringBuilder()
            while (i < text.length) {
                val c = text[i++]
                if (c == '"') break
                if (c == '\\' && i < text.length) {
                    when (val esc = text[i++]) {
                        '"', '\\', '/' -> sb.append(esc)
                        'n' -> sb.append('\n')
                        'r' -> sb.append('\r')
                        't' -> sb.append('\t')
                        'b' -> sb.append('\b')
                        'f' -> sb.append('\u000C')
                        'u' -> {
                            if (i + 4 <= text.length) {
                                val hex = text.substring(i, i + 4)
                                hex.toIntOrNull(16)?.let { sb.append(it.toChar()) }
                                i += 4
                            }
                        }
                        else -> sb.append(esc)
                    }
                } else sb.append(c)
            }
            out += sb.toString()
        }
        return out
    }
}