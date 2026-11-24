package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.forgebox.data.BuildDiagnostic
import dev.forgebox.data.BuildDiagnosticsParser
import dev.forgebox.data.BuildHistoryEntry
import dev.forgebox.data.WorkspaceRepository
import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.UUID

interface BuildBackend {
    fun buildRun(projectId: String, kind: ForgeProto.BuildKind, filePath: String = "", command: String = "", cols: Int = 80, rows: Int = 24): Result<dev.forgebox.data.BuildRunResultModel>
    fun ptyClose(ptyId: Long): Result<Unit>
    fun pollEvents(max: Int = 64, timeoutMs: Int = 50): List<ForgeProto.Event>
}

class RepoBuildBackend(private val repo: WorkspaceRepository, private val runtime: dev.forgebox.core.ForgeBoxRuntime) : BuildBackend {
    override fun buildRun(projectId: String, kind: ForgeProto.BuildKind, filePath: String, command: String, cols: Int, rows: Int) = repo.buildRun(projectId, kind, filePath, command, cols, rows)
    override fun ptyClose(ptyId: Long): Result<Unit> = runCatching { runtime.ptyClose(ptyId) }
    override fun pollEvents(max: Int, timeoutMs: Int): List<ForgeProto.Event> = runCatching { runtime.pollEventsTimeout(max, timeoutMs) }.getOrDefault(emptyList())
}

data class BuildUiState(
    val runningPtyId: Long? = null,
    val runningCommand: String? = null,
    val output: String = "",
    val diagnostics: List<BuildDiagnostic> = emptyList(),
    val history: List<BuildHistoryEntry> = emptyList(),
    val error: String? = null,
    val pythonMissingForFile: String? = null,
    val resourceSummary: String = "unavailable",
)

class BuildViewModel(
    private val backend: BuildBackend,
    private val io: kotlinx.coroutines.CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    private val _state = MutableStateFlow(BuildUiState())
    val state: StateFlow<BuildUiState> = _state.asStateFlow()
    private var pollJob: kotlinx.coroutines.Job? = null

    fun runPython(projectId: String, filePath: String) = run(projectId, ForgeProto.BuildKind.BUILD_KIND_PYTHON, filePath = filePath)
    fun runGeneric(projectId: String, command: String) = run(projectId, ForgeProto.BuildKind.BUILD_KIND_GENERIC_COMMAND, command = command)

    private fun run(projectId: String, kind: ForgeProto.BuildKind, filePath: String = "", command: String = "") {
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(error = null, pythonMissingForFile = null)
            val res = backend.buildRun(projectId, kind, filePath, command)
            res.onSuccess { r ->
                if (r.pythonMissing && kind == ForgeProto.BuildKind.BUILD_KIND_PYTHON) {
                    _state.value = _state.value.copy(pythonMissingForFile = filePath, output = "", diagnostics = emptyList(), error = null)
                    return@onSuccess
                }
                if (r.ptyId <= 0L) {
                    _state.value = _state.value.copy(error = r.message.ifBlank { "Build did not start" })
                    return@onSuccess
                }
                val entry = BuildHistoryEntry(
                    id = UUID.randomUUID().toString(),
                    command = r.command,
                    kind = kind.name,
                    startedAtMs = System.currentTimeMillis(),
                )
                _state.value = _state.value.copy(
                    runningPtyId = r.ptyId,
                    runningCommand = r.command,
                    output = "",
                    diagnostics = emptyList(),
                    history = listOf(entry) + _state.value.history,
                    resourceSummary = "unavailable",
                )
                startPolling(r.ptyId)
            }.onFailure { e -> _state.value = _state.value.copy(error = e.message ?: "build failed") }
        }
    }

    fun cancel() {
        val ptyId = _state.value.runningPtyId ?: return
        viewModelScope.launch(io) { backend.ptyClose(ptyId) }
        pollJob?.cancel()
        val hist = _state.value.history.toMutableList()
        if (hist.isNotEmpty() && hist[0].exitCode == null) {
            hist[0] = hist[0].copy(endedAtMs = System.currentTimeMillis(), exitCode = -130, output = _state.value.output, diagnostics = _state.value.diagnostics.toMutableList(), resourceSummary = _state.value.resourceSummary)
        }
        _state.value = _state.value.copy(runningPtyId = null, runningCommand = null, history = hist)
    }

    fun clearPythonMissing() { _state.value = _state.value.copy(pythonMissingForFile = null) }

    private fun startPolling(ptyId: Long) {
        pollJob?.cancel()
        pollJob = viewModelScope.launch(io) {
            val buf = StringBuilder()
            while (isActive) {
                if (_state.value.runningPtyId != ptyId) break
                val events = backend.pollEvents(64, 50)
                var exited = false
                for (ev in events) {
                    when {
                        ev.hasTerminalOutput() && ev.terminalOutput.ptyId == ptyId -> {
                            buf.append(ev.terminalOutput.diffBytes.toStringUtf8())
                        }
                        ev.hasTerminalExit() && ev.terminalExit.ptyId == ptyId -> {
                            exited = true
                            val exit = ev.terminalExit.code
                            val out = buf.toString()
                            val diagnostics = BuildDiagnosticsParser.parse(out)
                            val hist = _state.value.history.toMutableList()
                            if (hist.isNotEmpty()) {
                                hist[0] = hist[0].copy(endedAtMs = System.currentTimeMillis(), exitCode = exit, output = out, diagnostics = diagnostics.toMutableList(), resourceSummary = _state.value.resourceSummary)
                            }
                            _state.value = _state.value.copy(
                                runningPtyId = null,
                                runningCommand = null,
                                output = out,
                                diagnostics = diagnostics,
                                history = hist,
                            )
                        }
                    }
                }
                if (exited) break
                val out = buf.toString()
                if (out != _state.value.output) {
                    _state.value = _state.value.copy(output = out, diagnostics = BuildDiagnosticsParser.parse(out))
                }
                kotlinx.coroutines.delay(50)
            }
        }
    }

    fun onOutputForTest(ptyId: Long, chunk: String) {
        val out = _state.value.output + chunk
        _state.value = _state.value.copy(output = out, diagnostics = BuildDiagnosticsParser.parse(out))
    }
    fun onExitForTest(ptyId: Long, exitCode: Int) {
        val hist = _state.value.history.toMutableList()
        if (hist.isNotEmpty()) hist[0] = hist[0].copy(endedAtMs = System.currentTimeMillis(), exitCode = exitCode, output = _state.value.output, diagnostics = _state.value.diagnostics.toMutableList())
        _state.value = _state.value.copy(runningPtyId = null, runningCommand = null, history = hist)
    }
}
