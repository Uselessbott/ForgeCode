package dev.forgebox.data

import dev.forgebox.proto.ForgeProto

enum class NetworkModeUi { LOCAL_ONLY, CONTROLLED, TEMPORARY_BROADER }
enum class NetworkTemporaryScopeUi { PROJECT, SESSION }
enum class NetworkActorKindUi { USER, SESSION, INSTALLED_BINARY }
enum class NetworkConsentDecisionUi { ALLOW_ONCE, ALLOW_REMEMBER, DENY }
enum class NetworkRememberScopeUi { PROJECT, GLOBAL }
enum class NetworkActivityStatusUi { PENDING, IN_FLIGHT, SUCCEEDED, FAILED, CANCELLED }

data class NetworkModeStateModel(
    val mode: NetworkModeUi,
    val temporaryExpiresAtMs: Long,
    val controlledAllowlistCount: Int,
    val temporaryScope: NetworkTemporaryScopeUi?,
    val temporaryScopeId: String,
)

data class NetworkAllowlistEntryModel(
    val hostPattern: String,
    val providerKind: String,
    val addedAt: Long,
    val addedBy: String,
)

data class NetworkActivityEventModel(
    val ts: Long,
    val destinationHost: String,
    val actorKind: NetworkActorKindUi,
    val actorId: String,
    val bytesUp: Long,
    val bytesDown: Long,
    val status: NetworkActivityStatusUi,
    val cancelToken: String,
    val reason: String,
)

fun ForgeProto.NetworkModeStateResponse.toModel() = NetworkModeStateModel(
    mode = when (mode) {
        ForgeProto.NetworkMode.NETWORK_MODE_CONTROLLED -> NetworkModeUi.CONTROLLED
        ForgeProto.NetworkMode.NETWORK_MODE_TEMPORARY_BROADER -> NetworkModeUi.TEMPORARY_BROADER
        else -> NetworkModeUi.LOCAL_ONLY
    },
    temporaryExpiresAtMs = temporaryExpiresAtMs,
    controlledAllowlistCount = controlledAllowlistCount,
    temporaryScope = when (temporaryScope) {
        ForgeProto.NetworkTemporaryScope.NETWORK_TEMPORARY_SCOPE_PROJECT -> NetworkTemporaryScopeUi.PROJECT
        ForgeProto.NetworkTemporaryScope.NETWORK_TEMPORARY_SCOPE_SESSION -> NetworkTemporaryScopeUi.SESSION
        else -> null
    },
    temporaryScopeId = temporaryScopeId,
)

fun ForgeProto.NetworkAllowlistEntry.toModel() = NetworkAllowlistEntryModel(hostPattern, providerKind, addedAt, addedBy)
fun ForgeProto.NetworkActivityEvent.toModel() = NetworkActivityEventModel(
    ts = ts, destinationHost = destinationHost,
    actorKind = when (actorKind) {
        ForgeProto.NetworkActorKind.NETWORK_ACTOR_KIND_SESSION -> NetworkActorKindUi.SESSION
        ForgeProto.NetworkActorKind.NETWORK_ACTOR_KIND_INSTALLED_BINARY -> NetworkActorKindUi.INSTALLED_BINARY
        else -> NetworkActorKindUi.USER
    },
    actorId = actorId, bytesUp = bytesUp, bytesDown = bytesDown,
    status = when (status) {
        ForgeProto.NetworkActivityStatus.NETWORK_ACTIVITY_STATUS_IN_FLIGHT -> NetworkActivityStatusUi.IN_FLIGHT
        ForgeProto.NetworkActivityStatus.NETWORK_ACTIVITY_STATUS_SUCCEEDED -> NetworkActivityStatusUi.SUCCEEDED
        ForgeProto.NetworkActivityStatus.NETWORK_ACTIVITY_STATUS_FAILED -> NetworkActivityStatusUi.FAILED
        ForgeProto.NetworkActivityStatus.NETWORK_ACTIVITY_STATUS_CANCELLED -> NetworkActivityStatusUi.CANCELLED
        else -> NetworkActivityStatusUi.PENDING
    },
    cancelToken = cancelToken, reason = reason,
)

fun NetworkModeUi.toProto() = when (this) {
    NetworkModeUi.LOCAL_ONLY -> ForgeProto.NetworkMode.NETWORK_MODE_LOCAL_ONLY
    NetworkModeUi.CONTROLLED -> ForgeProto.NetworkMode.NETWORK_MODE_CONTROLLED
    NetworkModeUi.TEMPORARY_BROADER -> ForgeProto.NetworkMode.NETWORK_MODE_TEMPORARY_BROADER
}
fun NetworkTemporaryScopeUi.toProto() = when (this) {
    NetworkTemporaryScopeUi.PROJECT -> ForgeProto.NetworkTemporaryScope.NETWORK_TEMPORARY_SCOPE_PROJECT
    NetworkTemporaryScopeUi.SESSION -> ForgeProto.NetworkTemporaryScope.NETWORK_TEMPORARY_SCOPE_SESSION
}
fun NetworkActorKindUi.toProto() = when (this) {
    NetworkActorKindUi.USER -> ForgeProto.NetworkActorKind.NETWORK_ACTOR_KIND_USER
    NetworkActorKindUi.SESSION -> ForgeProto.NetworkActorKind.NETWORK_ACTOR_KIND_SESSION
    NetworkActorKindUi.INSTALLED_BINARY -> ForgeProto.NetworkActorKind.NETWORK_ACTOR_KIND_INSTALLED_BINARY
}


data class NetworkDecisionModel(
    val allow: Boolean,
    val ask: Boolean,
    val reason: String,
    val effectiveMode: String = "",
)
