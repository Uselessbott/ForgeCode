package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.forgebox.core.ForgeBoxCore
import dev.forgebox.data.LinuxBootstrapModeUi
import dev.forgebox.data.LinuxDistroUi
import dev.forgebox.data.LinuxEnvStatusModel
import dev.forgebox.data.LinuxEnvUiState
import dev.forgebox.data.LinuxPackageManagerUi
import dev.forgebox.data.WorkspaceRepository
import dev.forgebox.proto.ForgeProto
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

data class LinuxEnvUiStateModel(
    val status: LinuxEnvStatusModel? = null,
    val loading: Boolean = false,
    val progressPercent: Int = 0,
    val progressMessage: String = "",
    val error: String? = null,
    val sandboxStatement: String = "This installs a Linux compatibility environment. It is not a security sandbox.",
)

class LinuxEnvViewModel(
    private val repository: WorkspaceRepository,
    private val runtime: dev.forgebox.core.ForgeBoxRuntime? = null,
    private val pollIntervalMs: Long = 500L,
    private val sourcePathProvider: ((LinuxDistroUi, LinuxBootstrapModeUi) -> Result<String>)? = null,
) : ViewModel() {
    private val _state = MutableStateFlow(LinuxEnvUiStateModel())
    val state: StateFlow<LinuxEnvUiStateModel> = _state.asStateFlow()
    private var pollJob: Job? = null

    fun refresh() {
        viewModelScope.launch(Dispatchers.IO) {
            _state.value = _state.value.copy(loading = true, error = null)
            repository.linuxEnvStatus()
                .onSuccess { st -> _state.value = _state.value.copy(status = st, loading = false, progressMessage = st.message) }
                .onFailure { e -> _state.value = _state.value.copy(loading = false, error = e.message) }
        }
    }

    fun install(distro: LinuxDistroUi, bootstrapMode: LinuxBootstrapModeUi = LinuxBootstrapModeUi.BUNDLED_OFFLINE) {
        viewModelScope.launch(Dispatchers.IO) {
            _state.value = _state.value.copy(loading = true, error = null, progressPercent = 0, progressMessage = "Preparing bootstrap source...")
            val source = sourcePathProvider?.invoke(distro, bootstrapMode) ?: Result.success("")
            val sourcePath = source.getOrElse { e ->
                _state.value = _state.value.copy(loading = false, error = e.message ?: "Failed to prepare bootstrap source")
                return@launch
            }
            startProgressPoll()
            repository.linuxEnvInstall(distro, bootstrapMode, sourcePath)
                .onSuccess { resp ->
                    _state.value = _state.value.copy(
                        progressMessage = resp.message,
                        loading = resp.state == ForgeProto.LinuxEnvState.LINUX_ENV_INSTALLING,
                    )
                }
                .onFailure { e -> _state.value = _state.value.copy(loading = false, error = e.message) }
            refresh()
        }
    }

    fun uninstall() {
        viewModelScope.launch(Dispatchers.IO) {
            _state.value = _state.value.copy(loading = true, error = null)
            repository.linuxEnvUninstall()
                .onSuccess { resp -> _state.value = _state.value.copy(loading = false, progressMessage = resp.message) }
                .onFailure { e -> _state.value = _state.value.copy(loading = false, error = e.message) }
            refresh()
        }
    }

    fun packageInstallCommand(manager: LinuxPackageManagerUi, spec: String): String = when (manager) {
        LinuxPackageManagerUi.NPM -> "npm install -g ${spec.trim()}"
        LinuxPackageManagerUi.APT -> "apt-get install -y ${spec.trim()}"
        LinuxPackageManagerUi.APK -> "apk add ${spec.trim()}"
    }

    fun requestPackageInstall(manager: LinuxPackageManagerUi, spec: String, projectId: String, onResult: (Result<Long>) -> Unit = {}) {
        viewModelScope.launch(Dispatchers.IO) {
            val res = repository.linuxPackageInstall(manager, spec, projectId).map { it.ptyId }
            onResult(res)
        }
    }

    private fun startProgressPoll() {
        if (runtime == null) return
        pollJob?.cancel()
        pollJob = viewModelScope.launch(Dispatchers.IO) {
            while (isActive) {
                val events = runCatching { runtime.pollEventsTimeout(32, 100) }.getOrDefault(emptyList())
                events.forEach { ev ->
                    if (ev.hasLinuxEnvInstallProgress()) {
                        val p = ev.linuxEnvInstallProgress
                        _state.value = _state.value.copy(
                            progressPercent = p.percent,
                            progressMessage = p.message,
                            loading = p.state == ForgeProto.LinuxEnvState.LINUX_ENV_INSTALLING,
                        )
                    }
                }
                val st = repository.linuxEnvStatus().getOrNull()
                if (st != null) {
                    _state.value = _state.value.copy(status = st, loading = st.state == LinuxEnvUiState.INSTALLING)
                    if (st.state != LinuxEnvUiState.INSTALLING) break
                }
                delay(pollIntervalMs)
            }
        }
    }

    override fun onCleared() {
        pollJob?.cancel()
        super.onCleared()
    }
}
