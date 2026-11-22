package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.forgebox.data.ProjectSummary
import dev.forgebox.data.WorkspaceRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class ProjectListUiState(
    val loading: Boolean = true,
    val projects: List<ProjectSummary> = emptyList(),
    val error: String? = null,
    val actionError: String? = null,
)

class ProjectListViewModel(
    private val repository: WorkspaceRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    private val _state = MutableStateFlow(ProjectListUiState())
    val state: StateFlow<ProjectListUiState> = _state.asStateFlow()

    fun refresh() {
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(loading = true, error = null)
            val result = repository.listProjects()
            _state.value = result.fold(
                onSuccess = { ProjectListUiState(loading = false, projects = it) },
                onFailure = { ProjectListUiState(loading = false, error = it.message ?: "Failed to load projects") },
            )
        }
    }

    fun createProject(name: String, onDone: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch(io) {
            val result = repository.createProject(name).map { Unit }
            if (result.isFailure) {
                _state.value = _state.value.copy(actionError = result.exceptionOrNull()?.message)
            } else {
                _state.value = _state.value.copy(actionError = null)
            }
            onDone(result)
            if (result.isSuccess) refresh()
        }
    }

    fun renameProject(projectId: String, newName: String, onDone: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch(io) {
            val result = repository.renameProject(projectId, newName).map { Unit }
            _state.value = _state.value.copy(actionError = result.exceptionOrNull()?.message)
            onDone(result)
            if (result.isSuccess) refresh()
        }
    }

    fun deleteProject(projectId: String, onDone: (Result<Unit>) -> Unit = {}) {
        viewModelScope.launch(io) {
            val result = repository.deleteProject(projectId)
            _state.value = _state.value.copy(actionError = result.exceptionOrNull()?.message)
            onDone(result)
            if (result.isSuccess) refresh()
        }
    }

    fun clearActionError() {
        _state.value = _state.value.copy(actionError = null)
    }

    companion object {
        fun factory(repository: WorkspaceRepository, io: CoroutineDispatcher = Dispatchers.IO) =
            object : ViewModelProvider.Factory {
                @Suppress("UNCHECKED_CAST")
                override fun <T : ViewModel> create(modelClass: Class<T>): T =
                    ProjectListViewModel(repository, io) as T
            }
    }
}
