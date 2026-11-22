package dev.forgebox.app.terminal

data class TerminalRenderState(
    val rows: Int = 24,
    val cols: Int = 80,
    val lines: List<String> = List(24) { "" },
    val cursorRow: Int = 0,
    val cursorCol: Int = 0,
)

data class TerminalUiState(
    val open: Boolean = false,
    val ptyId: Long? = null,
    val projectId: String? = null,
    val cwdLabel: String = "ForgeBox",
    val status: String = "stopped",
    val render: TerminalRenderState = TerminalRenderState(),
    val exitCode: Int? = null,
    val hintShown: Boolean = false,
)