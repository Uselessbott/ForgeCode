package dev.forgebox.app.terminal

import dev.forgebox.core.ForgeBoxRuntime
import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Process-local terminal controller owned by SessionService for M6.
 * Single session only; Rust runtime remains process-wide singleton.
 */
object TerminalSessionController {
    private val scope = CoroutineScope(Dispatchers.IO + Job())
    private val _state = MutableStateFlow(TerminalUiState())
    val state: StateFlow<TerminalUiState> = _state.asStateFlow()
    private var pollJob: Job? = null

    fun open(projectId: String?) {
        _state.value = _state.value.copy(open = true, projectId = projectId, cwdLabel = projectId ?: "ForgeBox", status = "starting", exitCode = null)
        if (_state.value.ptyId != null) return
        scope.launch {
            runCatching { ForgeBoxRuntime.ptyOpen(projectId, 80, 24) }
                .onSuccess { id ->
                    _state.value = _state.value.copy(ptyId = id, status = "running", hintShown = true)
                    startPolling()
                }
                .onFailure { e -> _state.value = _state.value.copy(status = "error: ${e.message}") }
        }
    }

    fun toggle(projectId: String?) {
        if (_state.value.open) _state.value = _state.value.copy(open = false) else open(projectId)
    }

    fun closePanel() { _state.value = _state.value.copy(open = false) }

    fun write(text: String) { val id = _state.value.ptyId ?: return; scope.launch { runCatching { ForgeBoxRuntime.ptyWrite(id, text.toByteArray(Charsets.UTF_8)) } } }
    fun writeBytes(bytes: ByteArray) { val id = _state.value.ptyId ?: return; scope.launch { runCatching { ForgeBoxRuntime.ptyWrite(id, bytes) } } }
    fun resize(cols: Int, rows: Int) { val id = _state.value.ptyId ?: return; scope.launch { runCatching { ForgeBoxRuntime.ptyResize(id, cols, rows) } } }
    fun signal(sig: Int) { val id = _state.value.ptyId ?: return; scope.launch { runCatching { ForgeBoxRuntime.ptySignal(id, sig) } } }
    fun stop() { signal(15) }

    fun closeSession() {
        val id = _state.value.ptyId ?: return
        scope.launch {
            runCatching { ForgeBoxRuntime.ptyClose(id) }
            _state.value = _state.value.copy(ptyId = null, status = "stopped")
        }
    }

    private fun startPolling() {
        if (pollJob?.isActive == true) return
        pollJob = scope.launch {
            while (isActive) {
                val events = runCatching { ForgeBoxRuntime.pollEvents(64) }.getOrDefault(emptyList())
                if (events.isNotEmpty()) handleEvents(events)
                delay(100)
            }
        }
    }

    private fun handleEvents(events: List<ForgeProto.Event>) {
        val id = _state.value.ptyId ?: return
        for (ev in events) {
            when {
                ev.hasTerminalOutput() && ev.terminalOutput.ptyId == id -> {
                    val render = TerminalDiffParser.parse(ev.terminalOutput.diffBytes.toByteArray(), _state.value.render)
                    _state.value = _state.value.copy(render = render)
                }
                ev.hasTerminalExit() && ev.terminalExit.ptyId == id -> {
                    _state.value = _state.value.copy(status = "process ended (${ev.terminalExit.code})", exitCode = ev.terminalExit.code, ptyId = null)
                }
            }
        }
    }
}