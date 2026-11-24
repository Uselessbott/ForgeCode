package dev.forgebox.net

import dev.forgebox.data.NetworkActivityEventModel
import dev.forgebox.data.NetworkActivityStatusUi
import dev.forgebox.data.NetworkActorKindUi
import dev.forgebox.data.NetworkAllowlistEntryModel
import dev.forgebox.data.NetworkModeStateModel
import dev.forgebox.data.NetworkModeUi
import dev.forgebox.data.NetworkTemporaryScopeUi
import okhttp3.Call
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

data class NetworkDecision(
    val allow: Boolean,
    val ask: Boolean,
    val reason: String,
    val effectiveMode: String = "",
)

data class NetworkConsentPrompt(
    val destinationHost: String,
    val providerKind: String,
    val fileCount: Int,
    val byteEstimate: Long,
    val actorKind: NetworkActorKindUi,
    val actorId: String,
)

enum class ConsentChoice { ALLOW_ONCE, ALLOW_REMEMBER, DENY }

interface NetworkConsentHandler {
    fun requestConsent(prompt: NetworkConsentPrompt): ConsentChoice
}

class NetworkBroker(
    private val repository: NetworkRepository,
    private val okHttpClient: OkHttpClient = OkHttpClient(),
    private val consentHandler: NetworkConsentHandler? = null,
) {
    private val activeCalls = ConcurrentHashMap<String, Call>()
    private val localId = AtomicLong(1)

    fun modeState(): Result<NetworkModeStateModel> = repository.networkModeState()
    fun setMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi = NetworkTemporaryScopeUi.PROJECT, scopeId: String = "", durationSeconds: Int = 900) = repository.networkSetMode(mode, scope, scopeId, durationSeconds)
    fun listAllowlist(): Result<List<NetworkAllowlistEntryModel>> = repository.networkAllowlistList()
    fun upsertAllowlist(host: String, providerKind: String) = repository.networkAllowlistUpsert(host, providerKind)
    fun removeAllowlist(host: String) = repository.networkAllowlistRemove(host)
    fun listActivity(limit: Int = 100): Result<List<NetworkActivityEventModel>> = repository.networkActivityList(limit)
    fun cancel(token: String): Result<Boolean> {
        activeCalls.remove(token)?.cancel()
        return repository.networkCancel(token)
    }
    fun setSessionPolicy(sessionId: String, enabled: Boolean) = repository.networkSessionPolicy(sessionId, enabled)

    fun decide(host: String, actorKind: NetworkActorKindUi, actorId: String, sessionId: String = actorId): NetworkDecision {
        val res = repository.networkDecision(host, actorKind, actorId, sessionId)
        return res.fold(
            onSuccess = { NetworkDecision(it.allow, it.ask, it.reason, it.effectiveMode) },
            onFailure = { NetworkDecision(allow = false, ask = false, reason = it.message ?: "decision failed") },
        )
    }

    @Throws(IOException::class)
    fun execute(request: Request, actorKind: NetworkActorKindUi = NetworkActorKindUi.USER, actorId: String = "user", sessionId: String = "", requireConsent: Boolean = false, providerKind: String = "", fileCount: Int = 0, byteEstimate: Long = 0L): Response {
        val host = request.url.host
        val decision = decide(host, actorKind, actorId, sessionId)
        if (!decision.allow) {
            if (requireConsent && decision.reason.contains("not in allowlist")) {
                val choice = consentHandler?.requestConsent(NetworkConsentPrompt(host, providerKind, fileCount, byteEstimate, actorKind, actorId)) ?: ConsentChoice.DENY
                when (choice) {
                    ConsentChoice.ALLOW_ONCE -> { /* proceed this once */ }
                    ConsentChoice.ALLOW_REMEMBER -> { repository.networkAllowlistUpsert(host, providerKind.ifBlank { "unknown" }); }
                    ConsentChoice.DENY -> throw IOException("Network denied: ${decision.reason}")
                }
                val second = decide(host, actorKind, actorId, sessionId)
                if (!second.allow && choice != ConsentChoice.ALLOW_ONCE) throw IOException("Network denied: ${second.reason}")
            } else {
                throw IOException("Network denied: ${decision.reason}")
            }
        }
        val call = okHttpClient.newCall(request)
        val token = "okhttp-${localId.getAndIncrement()}"
        activeCalls[token] = call
        try {
            return call.execute()
        } finally {
            activeCalls.remove(token)
        }
    }
}
