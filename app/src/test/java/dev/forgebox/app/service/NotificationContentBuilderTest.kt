package dev.forgebox.app.service

import dev.forgebox.app.terminal.TerminalSessionLifecycle
import dev.forgebox.app.terminal.TerminalSessionUi
import org.junit.Assert.*
import org.junit.Test

class NotificationContentBuilderTest {
    private fun s(id: Long, name: String, life: TerminalSessionLifecycle = TerminalSessionLifecycle.RUNNING) =
        TerminalSessionUi(ptyId = id, name = name, cwdLabel = "p", lifecycle = life)

    @Test fun zeroSessionsStopsForeground() {
        val c = NotificationContentBuilder.build(emptyList(), null, 6)
        assertTrue(c.shouldStopForeground)
        assertEquals("No active terminal sessions", c.text)
        assertFalse(c.showStopActive)
    }

    @Test fun oneRunningShowsCountsAndActions() {
        val c = NotificationContentBuilder.build(listOf(s(1, "sh")), 1, 6)
        assertEquals("ForgeBox: sh", c.title)
        assertEquals("1 of 6 sessions running", c.text)
        assertTrue(c.showStopActive)
        assertTrue(c.showEndAll)
        assertFalse(c.shouldStopForeground)
    }

    @Test fun manySessionsAndCapReached() {
        val sessions = (1L..6L).map { s(it, "s$it") }
        val c = NotificationContentBuilder.build(sessions, 3, 6)
        assertEquals("ForgeBox: s3", c.title)
        assertEquals("6 of 6 sessions running", c.text)
        assertTrue(c.showStopAll)
    }

    @Test fun activeExitedFallsBack() {
        val sessions = listOf(s(1, "old", TerminalSessionLifecycle.EXITED), s(2, "run"))
        val c = NotificationContentBuilder.build(sessions, 1, 6)
        assertEquals("ForgeBox: old", c.title)
        assertEquals("1 of 6 sessions running", c.text)
    }
}
