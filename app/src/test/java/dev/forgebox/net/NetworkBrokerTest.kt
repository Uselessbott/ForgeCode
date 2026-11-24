package dev.forgebox.net

import dev.forgebox.data.*
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.*
import org.junit.Test

private class FakeRepo : NetworkRepository {
    var mode = NetworkModeStateModel(NetworkModeUi.LOCAL_ONLY, 0, 0, null, "")
    val allowlist = mutableListOf<NetworkAllowlistEntryModel>()
    val activity = mutableListOf<NetworkActivityEventModel>()
    val sessionEnabled = mutableMapOf<String, Boolean>()
    override fun networkModeState() = Result.success(mode)
    override fun networkSetMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi, scopeId: String, durationSeconds: Int) = Result.success(dev.forgebox.proto.ForgeProto.NetworkSetModeResponse.newBuilder().setMode(mode.toProto()).build()).also { this.mode = this.mode.copy(mode = mode) }
    override fun networkAllowlistList() = Result.success(allowlist.toList())
    override fun networkAllowlistUpsert(hostPattern: String, providerKind: String): Result<NetworkAllowlistEntryModel> { val e = NetworkAllowlistEntryModel(hostPattern, providerKind, 1, "user"); allowlist.removeAll { it.hostPattern == hostPattern }; allowlist.add(e); return Result.success(e) }
    override fun networkAllowlistRemove(hostPattern: String): Result<Boolean> { return Result.success(allowlist.removeAll { it.hostPattern == hostPattern }) }
    override fun networkActivityList(limit: Int) = Result.success(activity.take(limit))
    override fun networkCancel(token: String): Result<Boolean> { val i = activity.indexOfFirst { it.cancelToken == token }; if (i>=0) activity[i]=activity[i].copy(status=NetworkActivityStatusUi.CANCELLED); return Result.success(i>=0) }
    override fun networkSessionPolicy(sessionId: String, enabled: Boolean) = Result.success(dev.forgebox.proto.ForgeProto.NetworkSessionPolicyResponse.newBuilder().setSessionId(sessionId).setNetworkEnabled(enabled).setOk(true).build()).also { sessionEnabled[sessionId]=enabled }
    override fun networkDecision(host: String, actorKind: NetworkActorKindUi, actorId: String, sessionId: String): Result<NetworkDecisionModel> {
        if (sessionEnabled[sessionId] == false) return Result.success(NetworkDecisionModel(false, false, "per-session network toggle disabled", mode.mode.name))
        return when (mode.mode) {
            NetworkModeUi.LOCAL_ONLY -> Result.success(if (host=="127.0.0.1"||host=="localhost"||host=="::1") NetworkDecisionModel(true,false,"loopback",mode.mode.name) else NetworkDecisionModel(false,false,"M12 configures SSH endpoints",mode.mode.name))
            NetworkModeUi.CONTROLLED -> Result.success(if (allowlist.any { it.hostPattern==host }) NetworkDecisionModel(true,false,"allowlist hit",mode.mode.name) else NetworkDecisionModel(false,false,"not in allowlist; switch mode or add entry",mode.mode.name))
            NetworkModeUi.TEMPORARY_BROADER -> Result.success(NetworkDecisionModel(true,false,"temporary broader",mode.mode.name))
        }
    }
}

private class FakeConsent(private val choice: ConsentChoice) : NetworkConsentHandler {
    var prompts = 0
    override fun requestConsent(prompt: NetworkConsentPrompt): ConsentChoice { prompts++; return choice }
}

class NetworkBrokerTest {
    @Test fun localOnlyBlocks() {
        val repo = FakeRepo()
        val broker = NetworkBroker(repo)
        val req = Request.Builder().url("http://example.com/").build()
        try { broker.execute(req); fail("should deny") } catch (e: Exception) { assertTrue(e.message!!.contains("denied", true) || e.message!!.contains("M12")) }
        assertEquals(0, repo.allowlist.size)
    }

    @Test fun controlledAllowlistHitAllows() {
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED); networkAllowlistUpsert("127.0.0.1", "local") }
        val broker = NetworkBroker(repo)
        val server = MockWebServer(); server.enqueue(MockResponse().setBody("ok")); server.start()
        try {
            val resp = broker.execute(Request.Builder().url(server.url("/")).build())
            assertEquals(200, resp.code)
        } finally { server.shutdown() }
    }

    @Test fun consentRememberWritesAllowlist() {
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
        val consent = FakeConsent(ConsentChoice.ALLOW_REMEMBER)
        val broker = NetworkBroker(repo, consentHandler = consent)
        val server = MockWebServer(); server.enqueue(MockResponse().setBody("ok")); server.start()
        try {
            val resp = broker.execute(Request.Builder().url(server.url("/")).build(), requireConsent = true, providerKind = "test")
            assertEquals(200, resp.code)
            assertEquals(1, consent.prompts)
            assertTrue(repo.allowlist.any { it.hostPattern == "127.0.0.1" })
        } finally { server.shutdown() }
    }

    @Test fun perSessionToggleDenies() {
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.TEMPORARY_BROADER); networkSessionPolicy("s1", false) }
        val broker = NetworkBroker(repo)
        val d = broker.decide("example.com", NetworkActorKindUi.SESSION, "s1", "s1")
        assertFalse(d.allow)
        assertTrue(d.reason.contains("per-session"))
    }

    // D2: allowlist add/remove affects controlled-mode decisions
    @Test fun d2_allowlistAddRemoveAffectsDecisions() {
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
        val broker = NetworkBroker(repo)
        val host = "api.example.com"
        val before = broker.decide(host, NetworkActorKindUi.USER, "u")
        assertFalse(before.allow)
        assertTrue(before.reason.contains("not in allowlist"))
        broker.upsertAllowlist(host, "test")
        val afterAdd = broker.decide(host, NetworkActorKindUi.USER, "u")
        assertTrue(afterAdd.allow)
        broker.removeAllowlist(host)
        val afterRemove = broker.decide(host, NetworkActorKindUi.USER, "u")
        assertFalse(afterRemove.allow)
    }

    // D4: per-session toggle - broker issues NetworkSessionPolicy RPC and repo records disable.
    // PTY kill is enforced by Rust runtime (runtime.network_session_policy closes PTY); host-safe
    // Kotlin test verifies policy RPC/effect boundary. Rust test covers PTY-kill path.
    @Test fun d4_perSessionToggleIssuesPolicyAndDisablesSession() {
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.TEMPORARY_BROADER) }
        val broker = NetworkBroker(repo)
        val before = broker.decide("example.com", NetworkActorKindUi.SESSION, "s1", "s1")
        assertTrue(before.allow)
        val res = broker.setSessionPolicy("s1", false)
        assertTrue(res.isSuccess)
        assertEquals(false, repo.sessionEnabled["s1"])
        val after = broker.decide("example.com", NetworkActorKindUi.SESSION, "s1", "s1")
        assertFalse(after.allow)
        assertTrue(after.reason.contains("per-session"))
    }

    // D5: consent allow-once/remember/deny
    @Test fun d5_consentAllowOnceRememberDeny() {
        val server = MockWebServer(); server.enqueue(MockResponse().setBody("ok")); server.enqueue(MockResponse().setBody("ok")); server.start()
        try {
            val baseUrl = server.url("/").toString()
            val host = server.hostName

            // allow once - no allowlist write
            val repoOnce = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
            val consentOnce = FakeConsent(ConsentChoice.ALLOW_ONCE)
            val brokerOnce = NetworkBroker(repoOnce, consentHandler = consentOnce)
            val respOnce = brokerOnce.execute(Request.Builder().url(baseUrl).build(), requireConsent = true, providerKind = "test")
            assertEquals(200, respOnce.code)
            assertEquals(1, consentOnce.prompts)
            assertTrue(repoOnce.allowlist.none { it.hostPattern == host })

            // remember - writes allowlist
            val repoRem = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
            val consentRem = FakeConsent(ConsentChoice.ALLOW_REMEMBER)
            val brokerRem = NetworkBroker(repoRem, consentHandler = consentRem)
            val respRem = brokerRem.execute(Request.Builder().url(baseUrl).build(), requireConsent = true, providerKind = "test")
            assertEquals(200, respRem.code)
            assertTrue(repoRem.allowlist.any { it.hostPattern == host })

            // deny - blocks
            val repoDeny = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
            val consentDeny = FakeConsent(ConsentChoice.DENY)
            val brokerDeny = NetworkBroker(repoDeny, consentHandler = consentDeny)
            try {
                brokerDeny.execute(Request.Builder().url(baseUrl).build(), requireConsent = true, providerKind = "test")
                fail("should deny")
            } catch (e: Exception) {
                assertTrue(e.message!!.contains("denied", true))
            }
        } finally { server.shutdown() }
    }

    // D6: redaction - auth headers never in activity/audit payloads exposed by repo
    @Test fun d6_authHeadersNeverInActivityOrAudit() {
        val secret = "Bearer super-secret-token-12345"
        val base = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.TEMPORARY_BROADER) }
        val recorded = mutableListOf<String>()
        val repo = object : NetworkRepository {
            override fun networkModeState() = base.networkModeState()
            override fun networkSetMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi, scopeId: String, durationSeconds: Int) = base.networkSetMode(mode, scope, scopeId, durationSeconds)
            override fun networkAllowlistList() = base.networkAllowlistList()
            override fun networkAllowlistUpsert(hostPattern: String, providerKind: String) = base.networkAllowlistUpsert(hostPattern, providerKind)
            override fun networkAllowlistRemove(hostPattern: String) = base.networkAllowlistRemove(hostPattern)
            override fun networkActivityList(limit: Int) = base.networkActivityList(limit)
            override fun networkCancel(token: String) = base.networkCancel(token)
            override fun networkSessionPolicy(sessionId: String, enabled: Boolean) = base.networkSessionPolicy(sessionId, enabled)
            override fun networkDecision(host: String, actorKind: NetworkActorKindUi, actorId: String, sessionId: String): Result<NetworkDecisionModel> {
                recorded += "$host|$actorKind|$actorId|$sessionId"
                return Result.success(NetworkDecisionModel(true, false, "ok", "TEMPORARY_BROADER"))
            }
        }
        val broker = NetworkBroker(repo)
        val server = MockWebServer(); server.enqueue(MockResponse().setBody("ok")); server.start()
        try {
            val req = Request.Builder().url(server.url("/")).header("Authorization", secret).build()
            val resp = broker.execute(req)
            assertEquals(200, resp.code)
            val auditLike = recorded.joinToString("\n")
            assertFalse(auditLike.contains(secret))
            assertFalse(auditLike.contains("super-secret-token"))
        } finally { server.shutdown() }
    }
}
