package dev.forgebox.app.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.viewModelScope
import dev.forgebox.data.NetworkActivityEventModel
import dev.forgebox.data.NetworkAllowlistEntryModel
import dev.forgebox.data.NetworkModeStateModel
import dev.forgebox.data.NetworkModeUi
import dev.forgebox.data.NetworkTemporaryScopeUi
import dev.forgebox.net.ConsentChoice
import dev.forgebox.net.NetworkBroker
import dev.forgebox.net.NetworkConsentHandler
import dev.forgebox.net.NetworkConsentPrompt
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

data class NetworkUiState(
    val mode: NetworkModeStateModel? = null,
    val allowlist: List<NetworkAllowlistEntryModel> = emptyList(),
    val activity: List<NetworkActivityEventModel> = emptyList(),
    val loading: Boolean = false,
    val error: String? = null,
    val message: String? = null,
    val consentPrompt: NetworkConsentPrompt? = null,
    val countdownSeconds: Long = 0,
    val sessionNetworkEnabled: Boolean = true,
    val sessionBanner: String? = null,
) {
    companion object {
        const val MANDATORY_ISOLATION_TEXT =
            "ForgeBox does not enforce per-process network isolation. The per-session toggle kills the session's PTY on disable; the guest's own network stack may have already contacted hosts."
        const val MANDATORY_SANDBOX_TEXT =
            "Linux compatibility environment. Not a security sandbox."
    }
}

class NetworkViewModel(
    private val broker: NetworkBroker,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
) : ViewModel(), NetworkConsentHandler {
    private val _state = MutableStateFlow(NetworkUiState())
    val state: StateFlow<NetworkUiState> = _state.asStateFlow()
    @Volatile private var pendingConsentChoice: ConsentChoice? = null

    fun refresh() {
        viewModelScope.launch(io) {
            _state.value = _state.value.copy(loading = true, error = null)
            val mode = broker.modeState().getOrNull()
            val allow = broker.listAllowlist().getOrDefault(emptyList())
            val act = broker.listActivity(100).getOrDefault(emptyList())
            val now = nowMs()
            val countdown = ((mode?.temporaryExpiresAtMs ?: 0L) - now).coerceAtLeast(0L) / 1000L
            _state.value = _state.value.copy(loading = false, mode = mode, allowlist = allow, activity = act, countdownSeconds = countdown)
        }
    }

    fun setMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi = NetworkTemporaryScopeUi.PROJECT, scopeId: String = "", durationSeconds: Int = 900) {
        viewModelScope.launch(io) {
            broker.setMode(mode, scope, scopeId, durationSeconds).onFailure { _state.value = _state.value.copy(error = it.message) }
            refresh()
        }
    }

    fun tickCountdown() {
        val mode = _state.value.mode ?: return
        if (mode.mode != NetworkModeUi.TEMPORARY_BROADER) { _state.value = _state.value.copy(countdownSeconds = 0); return }
        val now = nowMs()
        val remaining = ((mode.temporaryExpiresAtMs - now).coerceAtLeast(0L)) / 1000L
        _state.value = _state.value.copy(countdownSeconds = remaining)
        if (remaining <= 0L) refresh()
    }

    fun upsertAllowlist(host: String, providerKind: String) { viewModelScope.launch(io) { broker.upsertAllowlist(host, providerKind); refresh() } }
    fun removeAllowlist(host: String) { viewModelScope.launch(io) { broker.removeAllowlist(host); refresh() } }
    fun cancel(token: String) { viewModelScope.launch(io) { broker.cancel(token); refresh() } }

    fun setSessionNetworkEnabled(sessionId: String, enabled: Boolean) {
        viewModelScope.launch(io) {
            val res = broker.setSessionPolicy(sessionId, enabled)
            _state.value = _state.value.copy(
                sessionNetworkEnabled = enabled,
                sessionBanner = if (!enabled) "network disabled for this session" else null,
                message = res.getOrNull()?.message,
                error = res.exceptionOrNull()?.message,
            )
            refresh()
        }
    }

    override fun requestConsent(prompt: NetworkConsentPrompt): ConsentChoice {
        _state.value = _state.value.copy(consentPrompt = prompt)
        while (pendingConsentChoice == null) Thread.sleep(10)
        val c = pendingConsentChoice ?: ConsentChoice.DENY
        pendingConsentChoice = null
        _state.value = _state.value.copy(consentPrompt = null)
        return c
    }

    fun answerConsent(choice: ConsentChoice) { pendingConsentChoice = choice }

    companion object {
        fun factory(broker: NetworkBroker, io: CoroutineDispatcher = Dispatchers.IO) = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = NetworkViewModel(broker, io) as T
        }
    }
}
