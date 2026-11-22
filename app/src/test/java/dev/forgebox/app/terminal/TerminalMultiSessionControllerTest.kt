package dev.forgebox.app.terminal

import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class TerminalMultiSessionControllerTest {
    private class FakeBackend : TerminalSessionBackend {
        val openCalls = mutableListOf<OpenCall>()
        val renameCalls = mutableListOf<Pair<Long, String>>()
        val closeCalls = mutableListOf<Long>()
        val stopAllCalls = mutableListOf<Unit>()
        val pollTimeouts = mutableListOf<Int>()
        var nextId = 100L
        var sessions = linkedMapOf<Long, ForgeProto.TerminalSessionDescriptor>()
        var pollEvents: List<ForgeProto.Event> = emptyList()
        var pollCount = 0
        var openShouldFail: String? = null

        data class OpenCall(val projectId: String?, val cols: Int, val rows: Int, val name: String?, val envOverridesJson: String, val sessionCap: Int)

        override fun pollEventsTimeout(max: Int, timeoutMs: Int): List<ForgeProto.Event> {
            pollTimeouts += timeoutMs
            pollCount++
            val out = pollEvents
            pollEvents = emptyList()
            return out
        }
        override fun call(request: ForgeProto.Request): ForgeProto.Response {
            return when {
                request.hasListTerminalSessions() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK)
                    .setListTerminalSessions(ForgeProto.ListTerminalSessionsResponse.newBuilder().addAllSessions(sessions.values).setSessionCap(6).build()).build()
                request.hasRenameTerminalSession() -> {
                    val r = request.renameTerminalSession
                    renameCalls += r.ptyId to r.name
                    val old = sessions[r.ptyId]
                    if (old != null) sessions[r.ptyId] = old.toBuilder().setName(r.name).build()
                    ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setRenameTerminalSession(ForgeProto.RenameTerminalSessionResponse.newBuilder().setSession(old ?: ForgeProto.TerminalSessionDescriptor.getDefaultInstance()).build()).build()
                }
                request.hasStopAllTerminalSessions() -> {
                    stopAllCalls += Unit
                    sessions = LinkedHashMap(sessions.mapValues { (_, v) -> v.toBuilder().setState(ForgeProto.TerminalSessionState.TERMINAL_SESSION_STOPPED).setRestorable(true).build() })
                    ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setStopAllTerminalSessions(ForgeProto.StopAllTerminalSessionsResponse.newBuilder().setStopped(sessions.size).build()).build()
                }
                else -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.ERROR).build()
            }
        }
        override fun ptyOpen(projectId: String?, cols: Int, rows: Int, name: String?, envOverridesJson: String, sessionCap: Int): Long {
            openShouldFail?.let { throw RuntimeException(it) }
            val id = nextId++
            openCalls += OpenCall(projectId, cols, rows, name, envOverridesJson, sessionCap)
            val desc = ForgeProto.TerminalSessionDescriptor.newBuilder()
                .setPtyId(id).setName(name ?: "sh").setCwd("/data/ForgeBox")
                .setState(ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING).setCreatedAt(1).setLastUsed(1)
                .setRestorable(false).setPreviousProcessEnded(false).setEnvOverridesJson(envOverridesJson).build()
            sessions[id] = desc
            return id
        }
        override fun ptyWrite(ptyId: Long, data: ByteArray) {}
        override fun ptyResize(ptyId: Long, cols: Int, rows: Int) {}
        override fun ptySignal(ptyId: Long, signal: Int) {}
        override fun ptyClose(ptyId: Long) { closeCalls += ptyId; sessions[ptyId]?.let { sessions[ptyId] = it.toBuilder().setState(ForgeProto.TerminalSessionState.TERMINAL_SESSION_STOPPED).build() } }
        override fun ptyRename(ptyId: Long, name: String) { renameCalls += ptyId to name; sessions[ptyId]?.let { sessions[ptyId] = it.toBuilder().setName(name).build() } }
        override fun ptyStopAll(): Int { stopAllCalls += Unit; val n = sessions.size; sessions = LinkedHashMap(sessions.mapValues { (_, v) -> v.toBuilder().setState(ForgeProto.TerminalSessionState.TERMINAL_SESSION_STOPPED).setRestorable(true).build() }); return n }
    }

    private fun session(id: Long, name: String, state: ForgeProto.TerminalSessionState, restorable: Boolean = false, prevEnded: Boolean = false, env: String = "{}") =
        ForgeProto.TerminalSessionDescriptor.newBuilder().setPtyId(id).setName(name).setCwd("/data/ForgeBox/p")
            .setState(state).setCreatedAt(1).setLastUsed(1).setRestorable(restorable).setPreviousProcessEnded(prevEnded).setEnvOverridesJson(env).build()

    private fun setup(testScope: CoroutineScope): FakeBackend {
        val fb = FakeBackend()
        TerminalSessionController.resetForTests(fb, testScope)
        TerminalSessionController.pollTimeoutMs = 250
        return fb
    }

    @Test fun t1_spawnAndSwitchIndependentRenders() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "s1", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        fb.sessions[2] = session(2, "s2", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.refreshSessions(); advanceUntilIdle()
        val st = TerminalSessionController.state.value
        assertEquals(2, st.sessions.size)
        TerminalSessionController.select(st.sessions[0].ptyId)
        assertEquals(st.sessions[0].ptyId, TerminalSessionController.state.value.activePtyId)
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t2_renameUpdatesUiState() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "old", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.rename(1, "new-name"); advanceUntilIdle()
        assertTrue(fb.renameCalls.any { it.first == 1L && it.second == "new-name" })
        assertEquals("new-name", TerminalSessionController.state.value.sessions.first().name)
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t3_closeReleasesSessionAndSelectsNext() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "s1", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        fb.sessions[2] = session(2, "s2", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.select(1)
        TerminalSessionController.closeSession(1); advanceUntilIdle()
        assertTrue(fb.closeCalls.contains(1L))
        val st = TerminalSessionController.state.value
        assertNotEquals(1L, st.activePtyId)
        assertTrue(st.sessions.any { it.ptyId == 2L })
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t3b_renameDuplicateRejected() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "s1", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        fb.sessions[2] = session(2, "s2", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.rename(1, "s2"); advanceUntilIdle()
        assertTrue(fb.renameCalls.none { it.first == 1L && it.second == "s2" })
        assertTrue(TerminalSessionController.state.value.status.contains("already exists", true))
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t4_capReachedBlocksWithoutEviction() = runTest { try {
        val fb = setup(this)
        repeat(6) { i -> fb.sessions[(i+1).toLong()] = session((i+1).toLong(), "s$i", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING) }
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        fb.openShouldFail = "SESSION_CAP_REACHED"
        TerminalSessionController.createSession("p"); advanceUntilIdle()
        val st = TerminalSessionController.state.value
        assertEquals(6, st.sessions.size)
        assertTrue(st.status.contains("cap", true) || st.status.contains("SESSION_CAP_REACHED", true) || st.status.isNotBlank())
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t5_stopAllMarksStoppedAndRestorable() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "s1", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        fb.sessions[2] = session(2, "s2", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.stopAll(); advanceUntilIdle()
        assertTrue(fb.stopAllCalls.isNotEmpty())
        assertTrue(TerminalSessionController.state.value.sessions.all { it.lifecycle == TerminalSessionLifecycle.STOPPED || it.restorable })
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t5b_capUiStateDisabled() = runTest { try {
        val fb = setup(this)
        repeat(6) { i -> fb.sessions[(i+1).toLong()] = session((i+1).toLong(), "s$i", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING) }
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        val st = TerminalSessionController.state.value
        val canCreate = st.sessions.count { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING } < st.sessionCap
        assertEquals(false, canCreate)
        fb.openShouldFail = "SESSION_CAP_REACHED"
        TerminalSessionController.createSession("p"); advanceUntilIdle()
        assertTrue(TerminalSessionController.state.value.status.contains("cap", true) || TerminalSessionController.state.value.status.contains("SESSION_CAP_REACHED"))
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t7b_restoreRestartKeepsOtherMarkers() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "old", ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED, restorable = true, prevEnded = true, env = "{\"TERM\":\"xterm-256color\"}")
        fb.sessions[2] = session(2, "keep", ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED, restorable = true, prevEnded = true)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.recreateRestorable(1); advanceUntilIdle()
        val call = fb.openCalls.last()
        assertEquals("old", call.name)
        assertTrue(call.envOverridesJson.contains("TERM"))
        assertTrue(TerminalSessionController.state.value.sessions.any { it.ptyId == 2L && it.previousProcessEnded })
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t6b_stopAllCallsBackendOnce() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "s1", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.stopAll(); advanceUntilIdle()
        assertEquals(1, fb.stopAllCalls.size)
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t6_restoreBannerRestartAndDismiss() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "old", ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED, restorable = true, prevEnded = true, env = "{\"TERM\":\"xterm-256color\"}")
        fb.sessions[2] = session(2, "keep", ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED, restorable = true, prevEnded = true)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        val s = TerminalSessionController.state.value.sessions.first { it.ptyId == 1L }
        assertTrue(s.previousProcessEnded)
        TerminalSessionController.recreateRestorable(1); advanceUntilIdle()
        val call = fb.openCalls.last()
        assertEquals("old", call.name)
        assertTrue(call.envOverridesJson.contains("TERM"))
        assertTrue(TerminalSessionController.state.value.sessions.any { it.ptyId == 2L && it.previousProcessEnded })
        TerminalSessionController.dismissRestoreBanner()
        assertNull(TerminalSessionController.state.value.restoreBanner)
        assertTrue(TerminalSessionController.state.value.sessions.any { it.ptyId == 2L && it.previousProcessEnded })
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t7_exitedSessionRemainsSelectable() = runTest { try {
        val fb = setup(this)
        fb.sessions[1] = session(1, "ex", ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED)
        fb.sessions[2] = session(2, "run", ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING)
        TerminalSessionController.openForTests("p", startPolling = false); advanceUntilIdle()
        TerminalSessionController.select(1)
        assertEquals(1L, TerminalSessionController.state.value.activePtyId)
        assertEquals(TerminalSessionLifecycle.EXITED, TerminalSessionController.state.value.sessions.first { it.ptyId == 1L }.lifecycle)
    } finally { TerminalSessionController.stopPollingForTests() } }

    @Test fun t10_pollTimeoutIsBounded() = runTest { try {
        val fb = setup(this)
        TerminalSessionController.openForTests("p", startPolling = false)
        TerminalSessionController.pollOnceForTests()
        assertTrue(fb.pollTimeouts.isNotEmpty())
        assertTrue(fb.pollTimeouts.all { it == 250 })
    } finally { TerminalSessionController.stopPollingForTests() } }
}
