package dev.forgebox.app.terminal

data class TerminalTextStyle(
    val colorArgb: Int,
    val backgroundArgb: Int,
    val bold: Boolean,
    val underline: Boolean,
)

object TerminalStyleMapper {
    fun map(fg: Int, bg: Int, bold: Boolean, underline: Boolean, inverse: Boolean): TerminalTextStyle {
        val fgColor = if (inverse) bg else fg
        val bgColor = if (inverse) fg else bg
        return TerminalTextStyle(colorArgb = fgColor, backgroundArgb = bgColor, bold = bold, underline = underline)
    }
}
