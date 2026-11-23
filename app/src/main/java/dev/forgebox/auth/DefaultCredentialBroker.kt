package dev.forgebox.auth

import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * M9 credential broker. Stores secrets in TokenStore (Keystore-backed in prod),
 * issues short-lived opaque handles, and never exposes raw tokens to UI/logs.
 */
class DefaultCredentialBroker(
    private val tokenStore: TokenStore,
    private val deviceFlowApi: DeviceFlowApi,
    private val nowMs: () -> Long = { System.currentTimeMillis() },
    private val handleTtlMs: Long = 5 * 60_000L,
) : CredentialBroker {

    private data class HandleEntry(val provider: String, val expiresAt: Long)
    private val handles = ConcurrentHashMap<String, HandleEntry>()

    override fun startDeviceFlow(provider: String): DeviceFlowStart {
        return deviceFlowApi.startDeviceFlow(provider)
    }

    override fun pollAndStore(provider: String, deviceCode: String): DeviceFlowPollResult {
        val result = deviceFlowApi.pollDeviceFlow(provider, deviceCode)
        val token = result.accessToken
        if (result.status == "authorized" && !token.isNullOrBlank()) {
            tokenStore.put(provider, token)
            return result.copy(accessToken = null, message = "authorized")
        }
        return result.copy(accessToken = null)
    }

    override fun storePat(provider: String, pat: String): String {
        require(pat.isNotBlank()) { "PAT cannot be blank" }
        tokenStore.put(provider, pat.trim())
        return issueHandle(provider).handle
    }

    override fun revoke(provider: String) {
        tokenStore.remove(provider)
        handles.entries.removeIf { it.value.provider == provider }
    }

    override fun credentialForRemote(remoteUrl: String, operation: String, provider: String): CredentialHandle? {
        require(remoteUrl.isNotBlank()) { "remoteUrl required" }
        require(operation == "fetch" || operation == "push") { "unsupported operation" }
        val token = tokenStore.get(provider) ?: return null
        if (token.isBlank()) return null
        return issueHandle(provider)
    }

    override fun resolveHandle(handle: String): String? {
        val entry = handles[handle] ?: return null
        if (entry.expiresAt <= nowMs()) {
            handles.remove(handle)
            return null
        }
        return tokenStore.get(entry.provider)
    }

    private fun issueHandle(provider: String): CredentialHandle {
        cleanupExpired()
        val handle = "cred_${UUID.randomUUID()}"
        val expiresAt = nowMs() + handleTtlMs
        handles[handle] = HandleEntry(provider, expiresAt)
        return CredentialHandle(handle = handle, expiresAtEpochMs = expiresAt)
    }

    private fun cleanupExpired() {
        val now = nowMs()
        handles.entries.removeIf { it.value.expiresAt <= now }
    }
}
