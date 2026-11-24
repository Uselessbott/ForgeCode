package dev.forgebox.net

import dev.forgebox.data.NetworkActivityEventModel
import dev.forgebox.data.NetworkAllowlistEntryModel
import dev.forgebox.data.NetworkModeStateModel
import dev.forgebox.data.NetworkModeUi
import dev.forgebox.data.NetworkTemporaryScopeUi
import dev.forgebox.data.NetworkActorKindUi
import dev.forgebox.data.NetworkDecisionModel

interface NetworkRepository {
    fun networkModeState(): Result<NetworkModeStateModel>
    fun networkSetMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi = NetworkTemporaryScopeUi.PROJECT, scopeId: String = "", durationSeconds: Int = 900): Result<dev.forgebox.proto.ForgeProto.NetworkSetModeResponse>
    fun networkAllowlistList(): Result<List<NetworkAllowlistEntryModel>>
    fun networkAllowlistUpsert(hostPattern: String, providerKind: String): Result<NetworkAllowlistEntryModel>
    fun networkAllowlistRemove(hostPattern: String): Result<Boolean>
    fun networkActivityList(limit: Int = 100): Result<List<NetworkActivityEventModel>>
    fun networkCancel(token: String): Result<Boolean>
    fun networkSessionPolicy(sessionId: String, enabled: Boolean): Result<dev.forgebox.proto.ForgeProto.NetworkSessionPolicyResponse>
    fun networkDecision(host: String, actorKind: NetworkActorKindUi, actorId: String, sessionId: String): Result<NetworkDecisionModel>
}
