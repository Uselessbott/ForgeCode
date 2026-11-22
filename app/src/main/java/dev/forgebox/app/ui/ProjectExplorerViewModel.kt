package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.forgebox.data.FileEntryModel
import dev.forgebox.data.WorkspaceRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class ProjectExplorerUiState(
    val loading: Boolean = true,
    val projectId: String = "",
    val entries: List<FileEntryModel> = emptyList(),
    val error: String? = null,
    val actionError: String? = null,
    val selectedPath: String? = null,
    val previewText: String? = null,
    val previewError: String? = null,
)

class ProjectExplorerViewModel(
    private val repository: WorkspaceRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    private val _state = MutableStateFlow(ProjectExplorerUiState())
    val state: StateFlow<ProjectExplorerUiState> = _state.asStateFlow()

    fun openProject(projectId: String) {
        if (_state.value.projectId == projectId && !_state.value.loading) return
        _state.value = ProjectExplorerUiState(loading = true, projectId = projectId)
        refresh()
    }

    fun refresh() {
        val projectId = _state.value.projectId
        if (projectId.isBlank()) return
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(loading = true, error = null)
            val result = repository.listFiles(projectId, recursive = true)
            _state.value = _state.value.copy(
                loading = false,
                entries = result.getOrDefault(emptyList()),
                error = result.exceptionOrNull()?.message,
            )
        }
    }

    fun createFile(path: String, onDone: (Result<Unit>) -> Unit = {}) = mutate(onDone) {
        repository.createFile(_state.value.projectId, path)
    }

    fun createFolder(path: String, onDone: (Result<Unit>) -> Unit = {}) = mutate(onDone) {
        repository.createFolder(_state.value.projectId, path)
    }

    fun rename(oldPath: String, newPath: String, onDone: (Result<Unit>) -> Unit = {}) = mutate(onDone) {
        repository.rename(_state.value.projectId, oldPath, newPath)
    }

    fun deletePath(path: String, recursive: Boolean, onDone: (Result<Unit>) -> Unit = {}) = mutate(onDone) {
        repository.deletePath(_state.value.projectId, path, recursive)
    }

    fun selectFile(path: String) {
        val projectId = _state.value.projectId
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(selectedPath = path, previewText = null, previewError = null)
            val result = repository.readFile(projectId, path)
            _state.value = result.fold(
                onSuccess = { bytes ->
                    val text = bytes.toString(Charsets.UTF_8)
                    val printable = text.count { it == '\n' || it == '\t' || !it.isISOControl() }
                    if (bytes.size <= 64_000 && printable == text.length) {
                        _state.value.copy(previewText = text, previewError = null)
                    } else {
                        _state.value.copy(previewText = null, previewError = "Not editable/previewable in M5")
                    }
                },
                onFailure = { _state.value.copy(previewText = null, previewError = it.message ?: "Failed to read file") },
            )
        }
    }

    fun clearActionError() { _state.value = _state.value.copy(actionError = null) }

    private fun mutate(onDone: (Result<Unit>) -> Unit, op: suspend () -> Result<Unit>) {
        viewModelScope.launch(io) {
            val result = op()
            _state.value = _state.value.copy(actionError = result.exceptionOrNull()?.message)
            onDone(result)
            if (result.isSuccess) refresh()
        }
    }

    companion object {
        fun factory(repository: WorkspaceRepository, io: CoroutineDispatcher = Dispatchers.IO) =
            object : ViewModelProvider.Factory {
                @Suppress("UNCHECKED_CAST")
                override fun <T : ViewModel> create(modelClass: Class<T>): T =
                    ProjectExplorerViewModel(repository, io) as T
            }
    }
}
