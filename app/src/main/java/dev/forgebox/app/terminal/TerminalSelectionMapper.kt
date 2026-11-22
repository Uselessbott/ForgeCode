package dev.forgebox.app.terminal

object TerminalSelectionMapper {
    fun selectedText(lines: List<String>, startRow: Int, endRow: Int): String {
        if (lines.isEmpty()) return ""
        val s = minOf(startRow, endRow).coerceIn(0, lines.lastIndex)
        val e = maxOf(startRow, endRow).coerceIn(0, lines.lastIndex)
        return lines.subList(s, e + 1).joinToString("\n")
    }
}
