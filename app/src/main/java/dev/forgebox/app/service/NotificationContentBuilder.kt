package dev.forgebox.app.service

import dev.forgebox.app.terminal.TerminalSessionLifecycle
import dev.forgebox.app.terminal.TerminalSessionUi

data class NotificationContent(
    val title: String,
    val text: String,
    val showStopActive: Boolean,
    val showStopAll: Boolean,
    val showEndAll: Boolean,
    val shouldStopForeground: Boolean,
)

object NotificationContentBuilder {
    fun build(sessions: List<TerminalSessionUi>, activeId: Long?, cap: Int): NotificationContent {
        val running = sessions.filter { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING }
        val active = sessions.firstOrNull { it.ptyId == activeId } ?: running.firstOrNull() ?: sessions.firstOrNull()
        val title = active?.name?.let { "ForgeBox: $it" } ?: "ForgeBox terminal"
        val text = when {
            sessions.isEmpty() -> "No active terminal sessions"
            else -> "${running.size} of $cap sessions running"
        }
        return NotificationContent(
            title = title,
            text = text,
            showStopActive = running.isNotEmpty(),
            showStopAll = running.size > 1 || running.isNotEmpty(),
            showEndAll = sessions.isNotEmpty(),
            shouldStopForeground = sessions.isEmpty(),
        )
    }
}
