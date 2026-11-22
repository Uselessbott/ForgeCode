package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.forgebox.data.WorkspaceRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class EditorTabState(
    val projectId: String,
    val path: String,
    val content: String = "",
    val savedContent: String = "",
    val mtimeMillis: Long = 0L,
    val lineEnding: String = "\n",
    val dirty: Boolean = false,
    val loading: Boolean = true,
    val error: String? = null,
    val conflict: String? = null,
    val tooLarge: Boolean = false,
    val readOnly: Boolean = false,
)

data class EditorUiState(
    val tabs: List<EditorTabState> = emptyList(),
    val activeIndex: Int = -1,
    val maxEditableBytes: Long = 1024 * 1024,
    val actionError: String? = null,
)

class EditorViewModel(
    private val repository: WorkspaceRepository,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    private val _state = MutableStateFlow(EditorUiState())
    val state: StateFlow<EditorUiState> = _state.asStateFlow()

    fun openFile(projectId: String, path: String) {
        val existing = _state.value.tabs.indexOfFirst { it.projectId == projectId && it.path == path }
        if (existing >= 0) {
            _state.value = _state.value.copy(activeIndex = existing)
            return
        }
        val tab = EditorTabState(projectId = projectId, path = path)
        _state.value = _state.value.copy(tabs = _state.value.tabs + tab, activeIndex = _state.value.tabs.size)
        loadTab(_state.value.activeIndex)
    }

    private fun loadTab(index: Int) {
        val tab = _state.value.tabs.getOrNull(index) ?: return
        viewModelScope.launch(io) {
            val info = repository.getInfo().getOrNull()
            val maxBytes = info?.maxEditableFileBytes ?: _state.value.maxEditableBytes
            updateTab(index) { it.copy(loading = true, error = null, conflict = null) }
            val stat = repository.statFile(tab.projectId, tab.path)
            val loaded = stat.fold(
                onSuccess = { s ->
                    when {
                        s.size > maxBytes -> tab.copy(loading = false, tooLarge = true, error = "File too large to open in editor (${s.size} bytes > $maxBytes)")
                        !s.isTextHeuristic -> tab.copy(loading = false, readOnly = true, error = "Not editable in M5: non-text or invalid UTF-8")
                        else -> repository.readFile(tab.projectId, tab.path).fold(
                            onSuccess = { bytes ->
                                val text = bytes.toString(Charsets.UTF_8)
                                val eol = detectLineEnding(text)
                                tab.copy(
                                    content = text,
                                    savedContent = text,
                                    mtimeMillis = s.mtimeMillis,
                                    lineEnding = eol,
                                    dirty = false,
                                    loading = false,
                                    tooLarge = false,
                                    readOnly = false,
                                    error = null,
                                )
                            },
                            onFailure = { tab.copy(loading = false, error = it.message ?: "Failed to read file") },
                        )
                    }
                },
                onFailure = { tab.copy(loading = false, error = it.message ?: "Failed to stat file") },
            )
            updateTab(index) { loaded }
            _state.value = _state.value.copy(maxEditableBytes = maxBytes)
        }
    }

    fun onContentChanged(index: Int, content: String) {
        updateTab(index) { it.copy(content = content, dirty = content != it.savedContent) }
    }

    fun saveActive(onDone: (Result<Unit>) -> Unit = {}) {
        val index = _state.value.activeIndex
        val tab = _state.value.tabs.getOrNull(index) ?: return onDone(Result.failure(IllegalStateException("No active tab")))
        if (tab.readOnly || tab.tooLarge) return onDone(Result.failure(IllegalStateException("Tab is read-only")))
        viewModelScope.launch(io) {
            val content = normalizeToLineEnding(tab.content, tab.lineEnding)
            val bytes = content.toByteArray(Charsets.UTF_8)
            val result = if (tab.mtimeMillis > 0) {
                repository.writeFileIfUnchanged(tab.projectId, tab.path, bytes, tab.mtimeMillis)
            } else {
                repository.createFile(tab.projectId, tab.path, bytes).map { 0L }
            }
            result.fold(
                onSuccess = { mtime ->
                    val newMtime = if (mtime > 0) mtime else repository.statFile(tab.projectId, tab.path).getOrNull()?.mtimeMillis ?: tab.mtimeMillis
                    updateTab(index) { it.copy(savedContent = content, content = content, dirty = false, mtimeMillis = newMtime, conflict = null) }
                    _state.value = _state.value.copy(actionError = null)
                    onDone(Result.success(Unit))
                },
                onFailure = { e ->
                    val msg = e.message ?: "Save failed"
                    if (msg.contains("CONFLICT")) updateTab(index) { it.copy(conflict = msg) }
                    _state.value = _state.value.copy(actionError = msg)
                    onDone(Result.failure(e))
                },
            )
        }
    }

    fun overwriteActive(onDone: (Result<Unit>) -> Unit = {}) {
        val index = _state.value.activeIndex
        val tab = _state.value.tabs.getOrNull(index) ?: return onDone(Result.failure(IllegalStateException("No active tab")))
        if (tab.readOnly || tab.tooLarge) return onDone(Result.failure(IllegalStateException("Tab is read-only")))
        viewModelScope.launch(io) {
            val content = normalizeToLineEnding(tab.content, tab.lineEnding)
            val bytes = content.toByteArray(Charsets.UTF_8)
            val result = repository.createFile(tab.projectId, tab.path, bytes)
            result.fold(
                onSuccess = {
                    val newMtime = repository.statFile(tab.projectId, tab.path).getOrNull()?.mtimeMillis ?: tab.mtimeMillis
                    updateTab(index) { it.copy(savedContent = content, content = content, dirty = false, mtimeMillis = newMtime, conflict = null) }
                    _state.value = _state.value.copy(actionError = null)
                    onDone(Result.success(Unit))
                },
                onFailure = { e ->
                    _state.value = _state.value.copy(actionError = e.message ?: "Overwrite failed")
                    onDone(Result.failure(e))
                },
            )
        }
    }

    fun reloadActive() {
        val index = _state.value.activeIndex
        if (index >= 0) loadTab(index)
    }

    /** M6: terminal may modify files; detect disk changes without silently losing dirty edits. */
    fun checkActiveDiskChange() {
        val index = _state.value.activeIndex
        val tab = _state.value.tabs.getOrNull(index) ?: return
        if (tab.loading || tab.tooLarge || tab.readOnly) return
        viewModelScope.launch(io) {
            val stat = repository.statFile(tab.projectId, tab.path).getOrNull() ?: return@launch
            val latest = _state.value.tabs.getOrNull(index) ?: return@launch
            if (latest.projectId != tab.projectId || latest.path != tab.path) return@launch
            val decision = diskChangeDecision(latest.dirty, latest.mtimeMillis, stat.mtimeMillis)
            when (decision) {
                DiskChangeDecision.NONE -> Unit
                DiskChangeDecision.RELOAD -> loadTab(index)
                DiskChangeDecision.CONFLICT -> updateTab(index) {
                    it.copy(conflict = "File changed on disk by terminal or another process. Reload disk or Overwrite disk.")
                }
            }
        }
    }

    fun selectTab(index: Int) {
        if (index in _state.value.tabs.indices) _state.value = _state.value.copy(activeIndex = index)
    }

    fun closeTab(index: Int, force: Boolean = false): Boolean {
        val tab = _state.value.tabs.getOrNull(index) ?: return true
        if (tab.dirty && !force) return false
        val tabs = _state.value.tabs.toMutableList()
        tabs.removeAt(index)
        val active = when {
            tabs.isEmpty() -> -1
            _state.value.activeIndex == index -> (index - 1).coerceAtLeast(0)
            _state.value.activeIndex > index -> _state.value.activeIndex - 1
            else -> _state.value.activeIndex
        }
        _state.value = _state.value.copy(tabs = tabs, activeIndex = active)
        return true
    }

    fun activeTab(): EditorTabState? = _state.value.tabs.getOrNull(_state.value.activeIndex)

    fun clearActionError() { _state.value = _state.value.copy(actionError = null) }

    private fun updateTab(index: Int, transform: (EditorTabState) -> EditorTabState) {
        val tabs = _state.value.tabs.toMutableList()
        val tab = tabs.getOrNull(index) ?: return
        tabs[index] = transform(tab)
        _state.value = _state.value.copy(tabs = tabs)
    }

    enum class DiskChangeDecision { NONE, RELOAD, CONFLICT }

    companion object {
        fun detectLineEnding(text: String): String = if (text.contains("\r\n")) "\r\n" else "\n"
        fun normalizeToLineEnding(text: String, eol: String): String {
            val normalized = text.replace("\r\n", "\n").replace("\r", "\n")
            return if (eol == "\r\n") normalized.replace("\n", "\r\n") else normalized
        }

        fun diskChangeDecision(dirty: Boolean, knownMtimeMillis: Long, diskMtimeMillis: Long): DiskChangeDecision {
            if (knownMtimeMillis <= 0L || diskMtimeMillis <= 0L) return DiskChangeDecision.NONE
            if (diskMtimeMillis == knownMtimeMillis) return DiskChangeDecision.NONE
            return if (dirty) DiskChangeDecision.CONFLICT else DiskChangeDecision.RELOAD
        }

        fun factory(repository: WorkspaceRepository, io: CoroutineDispatcher = Dispatchers.IO) =
            object : ViewModelProvider.Factory {
                @Suppress("UNCHECKED_CAST")
                override fun <T : ViewModel> create(modelClass: Class<T>): T = EditorViewModel(repository, io) as T
            }
    }
}
