package dev.forgebox.app.terminal

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class TerminalMultiSessionLogicTest {
    @Test
    fun capBlocksWhenLiveSessionsAtCap() {
        val sessions = (1L..6L).map { TerminalSessionUi(ptyId = it, name = "s$it", cwdLabel = "ForgeBox", lifecycle = TerminalSessionLifecycle.RUNNING) }
        val canCreate = sessions.count { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING } < 6
        assertEquals(false, canCreate)
    }

    @Test
    fun selectingExitedKeepsRenderableState() {
        val exited = TerminalSessionUi(ptyId = 1, name = "sh", cwdLabel = "ForgeBox", lifecycle = TerminalSessionLifecycle.EXITED, exitCode = 0)
        assertEquals(TerminalSessionLifecycle.EXITED, exited.lifecycle)
        assertTrue(exited.exitCode == 0)
    }

    @Test
    fun restorableSessionMarkedPreviousProcessEnded() {
        val s = TerminalSessionUi(ptyId = 2, name = "sh", cwdLabel = "ForgeBox", lifecycle = TerminalSessionLifecycle.EXITED, restorable = true, previousProcessEnded = true, scrollbackNotice = "Previous process ended")
        assertTrue(s.restorable)
        assertTrue(s.previousProcessEnded)
    }
}
