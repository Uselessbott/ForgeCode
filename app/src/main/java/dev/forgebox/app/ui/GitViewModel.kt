package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.forgebox.auth.CredentialBroker
import dev.forgebox.auth.DeviceFlowStart
import dev.forgebox.data.GitBranchModel
import dev.forgebox.data.GitCommitModel
import dev.forgebox.data.GitStatusModel
import dev.forgebox.data.WorkspaceRepository
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class GitUiState(
    val loading: Boolean = false,
    val projectId: String = "",
    val status: GitStatusModel? = null,
    val branches: List<GitBranchModel> = emptyList(),
    val log: List<GitCommitModel> = emptyList(),
    val diff: String = "",
    val selectedPath: String? = null,
    val error: String? = null,
    val message: String? = null,
    val deviceFlow: DeviceFlowStart? = null,
    val authStatus: String? = null,
    val credentialHandle: String? = null,
)

class GitViewModel(
    private val repository: WorkspaceRepository,
    private val credentialBroker: CredentialBroker? = null,
    private val io: CoroutineDispatcher = Dispatchers.IO,
) : ViewModel() {
    private val _state = MutableStateFlow(GitUiState())
    val state: StateFlow<GitUiState> = _state.asStateFlow()

    fun open(projectId: String) {
        _state.value = GitUiState(projectId = projectId, loading = true)
        refresh()
    }

    fun refresh() {
        val projectId = _state.value.projectId
        if (projectId.isBlank()) return
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(loading = true, error = null)
            val status = repository.gitStatus(projectId)
            val branches = repository.gitBranchList(projectId)
            val log = repository.gitLog(projectId, 30)
            _state.value = _state.value.copy(
                loading = false,
                status = status.getOrNull(),
                branches = branches.getOrDefault(emptyList()),
                log = log.getOrDefault(emptyList()),
                error = status.exceptionOrNull()?.message ?: branches.exceptionOrNull()?.message ?: log.exceptionOrNull()?.message,
            )
        }
    }

    fun initRepo() = mutate { repository.gitInit(_state.value.projectId) }
    fun stage(files: List<String>) = mutate { repository.gitStage(_state.value.projectId, files) }
    fun unstage(files: List<String>) = mutate { repository.gitUnstage(_state.value.projectId, files) }
    fun commit(message: String, authorName: String, authorEmail: String) = mutate {
        repository.gitCommit(_state.value.projectId, message, authorName, authorEmail).map { _state.value = _state.value.copy(message = "Committed $it"); Unit }
    }
    fun createBranch(name: String) = mutate { repository.gitBranchCreate(_state.value.projectId, name) }
    fun switchBranch(name: String) = mutate { repository.gitBranchSwitch(_state.value.projectId, name) }

    fun diff(scope: String, path: String = "") {
        val projectId = _state.value.projectId
        viewModelScope.launch(io) {
            val res = repository.gitDiff(projectId, scope, path)
            _state.value = _state.value.copy(diff = res.getOrDefault(""), selectedPath = path.ifBlank { null }, error = res.exceptionOrNull()?.message)
        }
    }

    fun showCommit(commitId: String) {
        val projectId = _state.value.projectId
        viewModelScope.launch(io) {
            val res = repository.gitShow(projectId, commitId)
            _state.value = _state.value.copy(diff = res.getOrDefault(""), error = res.exceptionOrNull()?.message)
        }
    }

    fun startDeviceFlow(provider: String = "github") {
        val broker = credentialBroker ?: run { _state.value = _state.value.copy(error = "Credential broker unavailable"); return }
        viewModelScope.launch(io) {
            runCatching { broker.startDeviceFlow(provider) }
                .onSuccess { _state.value = _state.value.copy(deviceFlow = it, authStatus = "pending", error = null) }
                .onFailure { _state.value = _state.value.copy(error = it.message) }
        }
    }

    fun pollDeviceFlow(provider: String = "github") {
        val broker = credentialBroker ?: return
        val code = _state.value.deviceFlow?.deviceCode ?: return
        viewModelScope.launch(io) {
            runCatching { broker.pollAndStore(provider, code) }
                .onSuccess { res ->
                    _state.value = _state.value.copy(authStatus = res.status, message = res.message, error = if (res.status == "error") res.message else null)
                    if (res.status == "authorized") _state.value = _state.value.copy(deviceFlow = null)
                }
                .onFailure { _state.value = _state.value.copy(error = it.message) }
        }
    }

    fun storePat(provider: String = "github", pat: String) {
        val broker = credentialBroker ?: run { _state.value = _state.value.copy(error = "Credential broker unavailable"); return }
        viewModelScope.launch(io) {
            runCatching { broker.storePat(provider, pat) }
                .onSuccess { _state.value = _state.value.copy(authStatus = "pat_stored", message = "PAT stored in Keystore-backed store", credentialHandle = it, error = null) }
                .onFailure { _state.value = _state.value.copy(error = it.message) }
        }
    }

    fun revokeAuth(provider: String = "github") {
        val broker = credentialBroker ?: return
        viewModelScope.launch(io) {
            runCatching { broker.revoke(provider) }
                .onSuccess { _state.value = _state.value.copy(authStatus = "revoked", credentialHandle = null, deviceFlow = null, message = "Credential revoked") }
                .onFailure { _state.value = _state.value.copy(error = it.message) }
        }
    }

    fun fetchRemote(remote: String = "origin", provider: String = "github") {
        val broker = credentialBroker ?: run { _state.value = _state.value.copy(error = "Credential broker unavailable"); return }
        val projectId = _state.value.projectId
        viewModelScope.launch(io) {
            val handle = broker.credentialForRemote(remote, "fetch", provider)
            if (handle == null) { _state.value = _state.value.copy(error = "No stored credential; authorize first"); return@launch }
            val res = repository.gitRemoteFetch(projectId, remote, handle.handle)
            _state.value = _state.value.copy(message = res.getOrNull(), error = res.exceptionOrNull()?.message)
        }
    }

    fun pushRemote(remote: String = "origin", branch: String, force: Boolean, provider: String = "github") {
        val broker = credentialBroker ?: run { _state.value = _state.value.copy(error = "Credential broker unavailable"); return }
        val projectId = _state.value.projectId
        viewModelScope.launch(io) {
            val handle = broker.credentialForRemote(remote, "push", provider)
            if (handle == null) { _state.value = _state.value.copy(error = "No stored credential; authorize first"); return@launch }
            val cmd = if (force) "git push --force $remote $branch" else "git push $remote $branch"
            val eval = repository.evaluateCommand(cmd, actorIsAgent = false, sessionId = projectId, cwd = projectId)
            val token = eval.fold(
                onSuccess = { ev ->
                    when (ev.decision) {
                        "DENY" -> { _state.value = _state.value.copy(error = ev.reason); return@launch }
                        "CONFIRM" -> repository.issueApprovalToken(ev.commandHash, projectId).getOrElse { e -> _state.value = _state.value.copy(error = e.message); return@launch }
                        else -> ""
                    }
                },
                onFailure = { e -> _state.value = _state.value.copy(error = e.message); return@launch },
            )
            val res = repository.gitRemotePush(projectId, remote, branch, force, handle.handle, token)
            _state.value = _state.value.copy(message = res.getOrNull(), error = res.exceptionOrNull()?.message)
        }
    }

    private fun mutate(op: suspend () -> Result<Unit>) {
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(loading = true, error = null)
            val res = op()
            _state.value = _state.value.copy(loading = false, error = res.exceptionOrNull()?.message)
            if (res.isSuccess) refresh()
        }
    }

    companion object {
        fun factory(repository: WorkspaceRepository, broker: CredentialBroker?, io: CoroutineDispatcher = Dispatchers.IO) = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = GitViewModel(repository, broker, io) as T
        }
    }
}

