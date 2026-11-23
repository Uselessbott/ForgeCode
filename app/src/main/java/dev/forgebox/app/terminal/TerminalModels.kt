package dev.forgebox.app.terminal

data class TerminalRenderState(
    val rows: Int = 24,
    val cols: Int = 80,
    val lines: List<String> = List(24) { "" },
    val cursorRow: Int = 0,
    val cursorCol: Int = 0,
    val scrollTop: Int = 0,
    val totalLines: Int = 24,
    val fg: Int = 0xFFDDDDDD.toInt(),
    val bg: Int = 0xFF111111.toInt(),
    val bold: Boolean = false,
    val underline: Boolean = false,
    val inverse: Boolean = false,
)

enum class TerminalSessionLifecycle { STARTING, RUNNING, EXITED, STOPPED, CRASHED, RESTORABLE }
enum class TerminalShellKind { BIONIC, LINUX }

data class TerminalSessionUi(
    val ptyId: Long,
    val name: String,
    val cwdLabel: String,
    val cwd: String = "",
    val lifecycle: TerminalSessionLifecycle,
    val exitCode: Int? = null,
    val render: TerminalRenderState = TerminalRenderState(),
    val previousProcessEnded: Boolean = false,
    val restorable: Boolean = false,
    val scrollbackNotice: String? = null,
    val envOverridesJson: String = "{}",
    val shellKind: TerminalShellKind = TerminalShellKind.BIONIC,
)

data class TerminalPendingApproval(
    val command: String,
    val reason: String,
    val commandHash: String,
    val sessionId: String,
    val cwd: String,
    val fileCountEstimate: Int = 0,
    val affectsOutsideProject: Boolean = false,
    val usesNetwork: Boolean = false,
    val createsCheckpoint: Boolean = false,
)

data class TerminalUiState(
    val open: Boolean = false,
    val projectId: String? = null,
    val sessions: List<TerminalSessionUi> = emptyList(),
    val activePtyId: Long? = null,
    val sessionCap: Int = 6,
    val status: String = "stopped",
    val hintShown: Boolean = false,
    val restoreBanner: String? = null,
    val pendingApproval: TerminalPendingApproval? = null,
    val linuxEnvReady: Boolean = false,
    val preferredShellKind: TerminalShellKind = TerminalShellKind.BIONIC,
)