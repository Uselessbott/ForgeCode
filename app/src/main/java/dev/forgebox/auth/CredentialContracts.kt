package dev.forgebox.auth

data class DeviceFlowStart(
    val deviceCode: String,
    val userCode: String,
    val verificationUri: String,
    val expiresInSeconds: Int,
    val intervalSeconds: Int,
)

data class DeviceFlowPollResult(
    val status: String, // pending | authorized | denied | expired | error
    val accessToken: String? = null,
    val message: String? = null,
)

interface DeviceFlowApi {
    fun startDeviceFlow(provider: String): DeviceFlowStart
    fun pollDeviceFlow(provider: String, deviceCode: String): DeviceFlowPollResult
}

interface TokenStore {
    fun put(provider: String, token: String)
    fun get(provider: String): String?
    fun remove(provider: String)
}

data class CredentialHandle(
    val handle: String,
    val expiresAtEpochMs: Long,
)

interface CredentialBroker {
    fun startDeviceFlow(provider: String): DeviceFlowStart
    fun pollAndStore(provider: String, deviceCode: String): DeviceFlowPollResult
    fun storePat(provider: String, pat: String): String
    fun revoke(provider: String)
    fun credentialForRemote(remoteUrl: String, operation: String, provider: String): CredentialHandle?
    fun resolveHandle(handle: String): String?
}
