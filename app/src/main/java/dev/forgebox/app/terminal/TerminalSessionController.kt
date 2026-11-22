package dev.forgebox.app.terminal

import android.content.Context
import android.content.res.Configuration
import dev.forgebox.core.ForgeBoxRuntime
import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

interface TerminalSessionBackend {
    fun pollEventsTimeout(max: Int, timeoutMs: Int): List<ForgeProto.Event>
    fun call(request: ForgeProto.Request): ForgeProto.Response
    fun ptyOpen(projectId: String?, cols: Int, rows: Int, name: String?, envOverridesJson: String, sessionCap: Int): Long
    fun ptyWrite(ptyId: Long, data: ByteArray)
    fun ptyResize(ptyId: Long, cols: Int, rows: Int)
    fun ptySignal(ptyId: Long, signal: Int)
    fun ptyClose(ptyId: Long)
    fun ptyRename(ptyId: Long, name: String)
    fun ptyStopAll(): Int
}

object RuntimeTerminalSessionBackend : TerminalSessionBackend {
    override fun pollEventsTimeout(max: Int, timeoutMs: Int) = ForgeBoxRuntime.pollEventsTimeout(max, timeoutMs)
    override fun call(request: ForgeProto.Request) = ForgeBoxRuntime.call(request)
    override fun ptyOpen(projectId: String?, cols: Int, rows: Int, name: String?, envOverridesJson: String, sessionCap: Int) =
        ForgeBoxRuntime.ptyOpen(projectId, cols, rows, name, envOverridesJson, sessionCap)
    override fun ptyWrite(ptyId: Long, data: ByteArray) = ForgeBoxRuntime.ptyWrite(ptyId, data)
    override fun ptyResize(ptyId: Long, cols: Int, rows: Int) = ForgeBoxRuntime.ptyResize(ptyId, cols, rows)
    override fun ptySignal(ptyId: Long, signal: Int) = ForgeBoxRuntime.ptySignal(ptyId, signal)
    override fun ptyClose(ptyId: Long) = ForgeBoxRuntime.ptyClose(ptyId)
    override fun ptyRename(ptyId: Long, name: String) = ForgeBoxRuntime.ptyRename(ptyId, name)
    override fun ptyStopAll(): Int = ForgeBoxRuntime.ptyStopAll()
}

/**
 * M7: process-local multi-session terminal controller. Rust runtime remains
 * process-wide singleton; SessionService owns foreground liveness.
 */
object TerminalSessionController {
    private var scope: CoroutineScope = CoroutineScope(Dispatchers.IO + Job())
    private val _state = MutableStateFlow(TerminalUiState())
    val state: StateFlow<TerminalUiState> = _state.asStateFlow()
    private var pollJob: Job? = null
    private val opMutex = Mutex()
    @Volatile internal var backend: TerminalSessionBackend = RuntimeTerminalSessionBackend
    internal var pollTimeoutMs: Int = 250
    internal var pollCountForTests: Int = 0

    internal fun resetForTests(testBackend: TerminalSessionBackend, testScope: CoroutineScope) {
        pollJob?.cancel(); pollJob = null
        scope = testScope
        backend = testBackend
        pollCountForTests = 0
        _state.value = TerminalUiState()
    }

    internal fun openForTests(projectId: String?, cap: Int = 6, startPolling: Boolean = false) {
        _state.value = _state.value.copy(open = true, projectId = projectId, sessionCap = cap, hintShown = true)
        if (startPolling) startPolling()
        refreshSessions()
    }

    internal fun startPollingForTests() = startPolling()
    internal fun stopPollingForTests() { pollJob?.cancel(); pollJob = null }
    internal fun pollOnceForTests() {
        pollCountForTests++
        val events = runCatching { backend.pollEventsTimeout(64, pollTimeoutMs) }.getOrDefault(emptyList())
        if (events.isNotEmpty()) handleEvents(events)
    }

    fun sessionCapFor(context: Context): Int {
        val cfg = context.resources.configuration
        val tablet = cfg.smallestScreenWidthDp >= 600 || (cfg.screenLayout and Configuration.SCREENLAYOUT_SIZE_MASK) >= Configuration.SCREENLAYOUT_SIZE_LARGE
        return if (tablet) 12 else 6
    }

    fun open(context: Context, projectId: String?) {
        val cap = sessionCapFor(context)
        _state.value = _state.value.copy(open = true, projectId = projectId, sessionCap = cap, hintShown = true)
        startPolling()
        refreshSessions()
        if (_state.value.sessions.none { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING }) {
            createSession(projectId)
        }
    }

    fun toggle(context: Context, projectId: String?) {
        if (_state.value.open) _state.value = _state.value.copy(open = false) else open(context, projectId)
    }

    fun closePanel() { _state.value = _state.value.copy(open = false) }

    fun activeSession(): TerminalSessionUi? = _state.value.sessions.firstOrNull { it.ptyId == _state.value.activePtyId }

    fun createSession(projectId: String? = _state.value.projectId) {
        scope.launch {
            opMutex.withLock {
                val st = _state.value
                val liveCount = st.sessions.count { it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING }
                if (liveCount >= st.sessionCap) {
                    _state.value = st.copy(status = "Session cap reached (${st.sessionCap}). Close a session first.")
                    return@withLock
                }
                runCatching { backend.ptyOpen(projectId, 80, 24, null, "{}", st.sessionCap) }
                    .onSuccess { id ->
                        val ui = TerminalSessionUi(ptyId = id, name = "sh", cwd = "", cwdLabel = projectId ?: "ForgeBox", lifecycle = TerminalSessionLifecycle.RUNNING, envOverridesJson = "{}")
                        _state.value = _state.value.copy(
                            sessions = st.sessions.filterNot { it.ptyId == id } + ui,
                            activePtyId = id,
                            status = "running",
                        )
                        refreshSessions()
                    }
                    .onFailure { e -> _state.value = _state.value.copy(status = e.message ?: "Failed to open session") }
            }
        }
    }

    fun select(ptyId: Long) { _state.value = _state.value.copy(activePtyId = ptyId) }

    fun rename(ptyId: Long, name: String) {
        val st = _state.value
        if (st.sessions.any { it.ptyId != ptyId && it.name == name }) {
            _state.value = st.copy(status = "Session name already exists")
            return
        }
        scope.launch {
            runCatching { backend.ptyRename(ptyId, name) }
                .onSuccess { refreshSessions() }
                .onFailure { e -> _state.value = _state.value.copy(status = e.message ?: "Rename failed") }
        }
    }

    fun write(text: String) { val id = _state.value.activePtyId ?: return; scope.launch { runCatching { backend.ptyWrite(id, text.toByteArray(Charsets.UTF_8)) } } }
    fun writeBytes(bytes: ByteArray) { val id = _state.value.activePtyId ?: return; scope.launch { runCatching { backend.ptyWrite(id, bytes) } } }
    fun resize(cols: Int, rows: Int) { val id = _state.value.activePtyId ?: return; scope.launch { runCatching { backend.ptyResize(id, cols, rows) } } }
    fun signal(sig: Int) { val id = _state.value.activePtyId ?: return; scope.launch { runCatching { backend.ptySignal(id, sig) } } }
    fun stop() { signal(15) }

    fun closeSession(ptyId: Long? = null) {
        val id = ptyId ?: _state.value.activePtyId ?: return
        scope.launch {
            runCatching { backend.ptyClose(id) }
            val sessions = _state.value.sessions.map { if (it.ptyId == id) it.copy(lifecycle = TerminalSessionLifecycle.STOPPED) else it }
            val next = sessions.firstOrNull { it.ptyId != id && (it.lifecycle == TerminalSessionLifecycle.RUNNING || it.lifecycle == TerminalSessionLifecycle.STARTING || it.lifecycle == TerminalSessionLifecycle.RESTORABLE) }?.ptyId
            _state.value = _state.value.copy(sessions = sessions, activePtyId = if (_state.value.activePtyId == id) next else _state.value.activePtyId)
            refreshSessions()
        }
    }

    fun stopAll() {
        scope.launch {
            runCatching { backend.ptyStopAll() }
            refreshSessions()
        }
    }

    fun recreateRestorable(ptyId: Long) {
        val sess = _state.value.sessions.firstOrNull { it.ptyId == ptyId } ?: return
        if (!sess.restorable) return
        scope.launch {
            opMutex.withLock {
                val envJson = sess.envOverridesJson.ifBlank { "{}" }
                runCatching { backend.ptyOpen(_state.value.projectId, 80, 24, sess.name, envJson, _state.value.sessionCap) }
                    .onSuccess { newId ->
                        _state.value = _state.value.copy(
                            sessions = _state.value.sessions.filterNot { it.ptyId == ptyId } + TerminalSessionUi(
                                ptyId = newId, name = sess.name, cwd = sess.cwd, cwdLabel = sess.cwdLabel, lifecycle = TerminalSessionLifecycle.RUNNING,
                                scrollbackNotice = "Previous process ended. This is a new shell in the same working directory.",
                                envOverridesJson = envJson,
                            ),
                            activePtyId = newId,
                        )
                        refreshSessions()
                    }
                    .onFailure { e -> _state.value = _state.value.copy(status = e.message ?: "Restore failed") }
            }
        }
    }

    fun dismissRestoreBanner() { _state.value = _state.value.copy(restoreBanner = null) }

    private fun startPolling() {
        if (pollJob?.isActive == true) return
        pollJob = scope.launch {
            while (isActive) {
                pollCountForTests++
                val events = runCatching { backend.pollEventsTimeout(64, pollTimeoutMs) }.getOrDefault(emptyList())
                if (events.isNotEmpty()) handleEvents(events)
            }
        }
    }

    fun refreshSessions() {
        scope.launch {
            val resp = runCatching {
                backend.call(
                    ForgeProto.Request.newBuilder()
                        .setListTerminalSessions(ForgeProto.ListTerminalSessionsRequest.getDefaultInstance())
                        .build()
                )
            }.getOrNull() ?: return@launch
            if (resp.status != ForgeProto.Status.OK || !resp.hasListTerminalSessions()) return@launch
            val list = resp.listTerminalSessions
            val existing = _state.value.sessions.associateBy { it.ptyId }
            val mapped = list.sessionsList.map { d ->
                val old = existing[d.ptyId]
                TerminalSessionUi(
                    ptyId = d.ptyId,
                    name = d.name,
                    cwd = d.cwd,
                    cwdLabel = d.cwd.substringAfterLast('/').ifEmpty { d.cwd },
                    lifecycle = fromProtoState(d.state),
                    exitCode = d.exitCode.takeIf { it != 0 || d.state == ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED },
                    render = old?.render ?: TerminalRenderState(),
                    previousProcessEnded = d.previousProcessEnded,
                    restorable = d.restorable || d.previousProcessEnded,
                    scrollbackNotice = old?.scrollbackNotice ?: if (d.previousProcessEnded) "Previous process ended; live output was not restored." else null,
                    envOverridesJson = d.envOverridesJson.ifBlank { "{}" },
                )
            }
            val active = _state.value.activePtyId?.takeIf { id -> mapped.any { it.ptyId == id } }
                ?: mapped.firstOrNull { it.lifecycle == TerminalSessionLifecycle.RUNNING }?.ptyId
                ?: mapped.firstOrNull()?.ptyId
            val restoreBanner = mapped.firstOrNull { it.restorable && it.previousProcessEnded }?.let { "${it.name}: previous process ended. Recreate shell?" }
            _state.value = _state.value.copy(sessions = mapped, activePtyId = active, sessionCap = if (list.sessionCap > 0) list.sessionCap.toInt() else _state.value.sessionCap, restoreBanner = restoreBanner)
        }
    }

    private fun handleEvents(events: List<ForgeProto.Event>) {
        var refresh = false
        for (ev in events) {
            when {
                ev.hasTerminalOutput() -> {
                    val id = ev.terminalOutput.ptyId
                    updateSession(id) { s ->
                        s.copy(render = TerminalDiffParser.parse(ev.terminalOutput.diffBytes.toByteArray(), s.render))
                    }
                }
                ev.hasTerminalExit() -> updateSession(ev.terminalExit.ptyId) {
                    it.copy(lifecycle = TerminalSessionLifecycle.EXITED, exitCode = ev.terminalExit.code)
                }
                ev.hasTerminalSessionSpawned() || ev.hasTerminalSessionRenamed() || ev.hasTerminalSessionState() || ev.hasTerminalSessionRestoreAvailable() -> refresh = true
            }
        }
        if (refresh) refreshSessions()
    }

    private fun updateSession(ptyId: Long, f: (TerminalSessionUi) -> TerminalSessionUi) {
        val sessions = _state.value.sessions.map { if (it.ptyId == ptyId) f(it) else it }
        _state.value = _state.value.copy(sessions = sessions)
    }

    private fun fromProtoState(s: ForgeProto.TerminalSessionState): TerminalSessionLifecycle = when (s) {
        ForgeProto.TerminalSessionState.TERMINAL_SESSION_STARTING -> TerminalSessionLifecycle.STARTING
        ForgeProto.TerminalSessionState.TERMINAL_SESSION_RUNNING -> TerminalSessionLifecycle.RUNNING
        ForgeProto.TerminalSessionState.TERMINAL_SESSION_EXITED -> TerminalSessionLifecycle.EXITED
        ForgeProto.TerminalSessionState.TERMINAL_SESSION_STOPPED -> TerminalSessionLifecycle.STOPPED
        ForgeProto.TerminalSessionState.TERMINAL_SESSION_CRASHED -> TerminalSessionLifecycle.CRASHED
        else -> TerminalSessionLifecycle.EXITED
    }
}
