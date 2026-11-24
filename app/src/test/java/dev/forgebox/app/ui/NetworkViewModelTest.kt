package dev.forgebox.app.ui

import dev.forgebox.data.*
import dev.forgebox.net.ConsentChoice
import dev.forgebox.net.NetworkBroker
import dev.forgebox.net.NetworkConsentHandler
import dev.forgebox.net.NetworkConsentPrompt
import dev.forgebox.net.NetworkRepository
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class NetworkViewModelTest {
    private val dispatcher = StandardTestDispatcher()

    private class FakeRepo : NetworkRepository {
        var mode = NetworkModeStateModel(NetworkModeUi.LOCAL_ONLY, 0L, 0, null, "")
        val allowlist = mutableListOf<NetworkAllowlistEntryModel>()
        val activity = mutableListOf<NetworkActivityEventModel>()
        val sessionEnabled = mutableMapOf<String, Boolean>()
        var cancelCalls = 0
        override fun networkModeState() = Result.success(mode)
        override fun networkSetMode(mode: NetworkModeUi, scope: NetworkTemporaryScopeUi, scopeId: String, durationSeconds: Int) =
            Result.success(dev.forgebox.proto.ForgeProto.NetworkSetModeResponse.newBuilder().setMode(mode.toProto()).build()).also {
                val exp = if (mode == NetworkModeUi.TEMPORARY_BROADER) 1_000L + durationSeconds * 1000L else 0L
                this.mode = this.mode.copy(mode = mode, temporaryExpiresAtMs = exp, temporaryScope = if (mode == NetworkModeUi.TEMPORARY_BROADER) scope else null, temporaryScopeId = if (mode == NetworkModeUi.TEMPORARY_BROADER) scopeId else "")
            }
        override fun networkAllowlistList() = Result.success(allowlist.toList())
        override fun networkAllowlistUpsert(hostPattern: String, providerKind: String) = Result.success(NetworkAllowlistEntryModel(hostPattern, providerKind, 1L, "user")).also { allowlist.removeAll { it.hostPattern == hostPattern }; allowlist.add(it.getOrThrow()) }
        override fun networkAllowlistRemove(hostPattern: String) = Result.success(allowlist.removeAll { it.hostPattern == hostPattern })
        override fun networkActivityList(limit: Int) = Result.success(activity.take(limit))
        override fun networkCancel(token: String): Result<Boolean> { cancelCalls++; val i = activity.indexOfFirst { it.cancelToken == token }; if (i >= 0) activity[i] = activity[i].copy(status = NetworkActivityStatusUi.CANCELLED); return Result.success(i >= 0) }
        override fun networkSessionPolicy(sessionId: String, enabled: Boolean) = Result.success(dev.forgebox.proto.ForgeProto.NetworkSessionPolicyResponse.newBuilder().setSessionId(sessionId).setNetworkEnabled(enabled).setOk(true).build()).also { sessionEnabled[sessionId] = enabled }
        override fun networkDecision(host: String, actorKind: NetworkActorKindUi, actorId: String, sessionId: String): Result<NetworkDecisionModel> {
            if (sessionEnabled[sessionId] == false) return Result.success(NetworkDecisionModel(false, false, "per-session network toggle disabled", mode.mode.name))
            return when (mode.mode) {
                NetworkModeUi.LOCAL_ONLY -> Result.success(if (host == "127.0.0.1" || host == "localhost" || host == "::1") NetworkDecisionModel(true, false, "loopback", mode.mode.name) else NetworkDecisionModel(false, false, "M12 configures SSH endpoints", mode.mode.name))
                NetworkModeUi.CONTROLLED -> Result.success(if (allowlist.any { it.hostPattern == host }) NetworkDecisionModel(true, false, "allowlist hit", mode.mode.name) else NetworkDecisionModel(false, false, "not in allowlist; switch mode or add entry", mode.mode.name))
                NetworkModeUi.TEMPORARY_BROADER -> Result.success(NetworkDecisionModel(true, false, "temporary broader", mode.mode.name))
            }
        }
    }

    @Before fun setUp() { Dispatchers.setMain(dispatcher) }
    @After fun tearDown() { Dispatchers.resetMain() }

    @Test fun d1_modeCountdownAutoRevert() = runTest(dispatcher) {
        var now = 1_000L
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
        val vm = NetworkViewModel(NetworkBroker(repo), io = dispatcher, nowMs = { now })
        vm.refresh(); advanceUntilIdle()
        assertEquals(NetworkModeUi.CONTROLLED, vm.state.value.mode?.mode)

        vm.setMode(NetworkModeUi.TEMPORARY_BROADER, NetworkTemporaryScopeUi.PROJECT, "p1", 2); advanceUntilIdle()
        assertEquals(NetworkModeUi.TEMPORARY_BROADER, vm.state.value.mode?.mode)
        assertTrue(vm.state.value.countdownSeconds > 0)

        now = 5_000L // past expiry
        repo.mode = repo.mode.copy(mode = NetworkModeUi.CONTROLLED, temporaryExpiresAtMs = 0L, temporaryScope = null, temporaryScopeId = "")
        vm.tickCountdown(); advanceUntilIdle()
        assertEquals(0L, vm.state.value.countdownSeconds)
        assertEquals(NetworkModeUi.CONTROLLED, vm.state.value.mode?.mode)
    }

    @Test fun d3_activityListAndCancel() = runTest(dispatcher) {
        val repo = FakeRepo().apply {
            activity.add(NetworkActivityEventModel(1L, "example.com", NetworkActorKindUi.USER, "u", 0, 0, NetworkActivityStatusUi.IN_FLIGHT, "tok-1", ""))
        }
        val vm = NetworkViewModel(NetworkBroker(repo), io = dispatcher, nowMs = { 1_000L })
        vm.refresh(); advanceUntilIdle()
        assertEquals(1, vm.state.value.activity.size)
        assertEquals("tok-1", vm.state.value.activity[0].cancelToken)
        vm.cancel("tok-1"); advanceUntilIdle()
        assertEquals(1, repo.cancelCalls)
        assertEquals(NetworkActivityStatusUi.CANCELLED, vm.state.value.activity[0].status)
    }

    @Test fun d5_consentAllowOnceRememberDeny() = runTest(dispatcher) {
        val repo = FakeRepo().apply { mode = mode.copy(mode = NetworkModeUi.CONTROLLED) }
        val vm = NetworkViewModel(NetworkBroker(repo), io = dispatcher, nowMs = { 1_000L })
        vm.refresh(); advanceUntilIdle()

        // allow once - no allowlist write
        Thread { Thread.sleep(20); vm.answerConsent(ConsentChoice.ALLOW_ONCE) }.start()
        val once = vm.requestConsent(NetworkConsentPrompt("a.com", "test", 1, 10, NetworkActorKindUi.USER, "u"))
        assertEquals(ConsentChoice.ALLOW_ONCE, once)
        assertTrue(repo.allowlist.none { it.hostPattern == "a.com" })

        // remember - caller/broker writes allowlist; VM exposes choice; simulate broker behavior
        Thread { Thread.sleep(20); vm.answerConsent(ConsentChoice.ALLOW_REMEMBER) }.start()
        val rem = vm.requestConsent(NetworkConsentPrompt("b.com", "test", 1, 10, NetworkActorKindUi.USER, "u"))
        assertEquals(ConsentChoice.ALLOW_REMEMBER, rem)

        // deny
        Thread { Thread.sleep(20); vm.answerConsent(ConsentChoice.DENY) }.start()
        val deny = vm.requestConsent(NetworkConsentPrompt("c.com", "test", 1, 10, NetworkActorKindUi.USER, "u"))
        assertEquals(ConsentChoice.DENY, deny)
        assertNull(vm.state.value.consentPrompt)
    }

    @Test fun d7_mandatoryTextPresent() {
        assertEquals(
            "ForgeBox does not enforce per-process network isolation. The per-session toggle kills the session's PTY on disable; the guest's own network stack may have already contacted hosts.",
            NetworkUiState.MANDATORY_ISOLATION_TEXT
        )
        assertEquals(
            "Linux compatibility environment. Not a security sandbox.",
            NetworkUiState.MANDATORY_SANDBOX_TEXT
        )
    }
}
