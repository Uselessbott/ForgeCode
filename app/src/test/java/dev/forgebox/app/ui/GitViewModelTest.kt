package dev.forgebox.app.ui

import dev.forgebox.auth.CredentialBroker
import dev.forgebox.auth.CredentialHandle
import dev.forgebox.auth.DeviceFlowPollResult
import dev.forgebox.auth.DeviceFlowStart
import dev.forgebox.core.ForgeBoxCore
import dev.forgebox.data.WorkspaceRepository
import dev.forgebox.proto.ForgeProto
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
class GitViewModelTest {
    private val dispatcher = StandardTestDispatcher()

    private class FakeCore : ForgeBoxCore {
        val requests = mutableListOf<ForgeProto.Request>()
        override fun call(request: ForgeProto.Request): ForgeProto.Response {
            requests += request
            return when {
                request.hasGitStatus() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitStatus(
                    ForgeProto.GitStatusResponse.newBuilder().setBranch("main").addStaged(ForgeProto.GitFileStatus.newBuilder().setPath("a.txt").setStatus("A"))
                ).build()
                request.hasGitBranchList() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitBranchList(
                    ForgeProto.GitBranchListResponse.newBuilder().addBranches(ForgeProto.GitBranchInfo.newBuilder().setName("main").setIsCurrent(true))
                ).build()
                request.hasGitLog() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitLog(ForgeProto.GitLogResponse.getDefaultInstance()).build()
                request.hasGitInit() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitInit(ForgeProto.GitInitResponse.newBuilder().setOk(true)).build()
                request.hasGitStage() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitStage(ForgeProto.GitStageResponse.newBuilder().setOk(true)).build()
                request.hasGitCommit() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitCommit(ForgeProto.GitCommitResponse.newBuilder().setCommitId("abc123")).build()
                request.hasEvaluateCommand() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setEvaluateCommand(ForgeProto.EvaluateCommandResponse.newBuilder().setDecision("CONFIRM").setReason("push").setCommandHash("h").build()).build()
                request.hasIssueApprovalToken() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setIssueApprovalToken(ForgeProto.IssueApprovalTokenResponse.newBuilder().setToken("tok").build()).build()
                request.hasGitRemotePush() -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).setGitRemotePush(ForgeProto.GitRemotePushResponse.newBuilder().setOk(false).setMessage("not runtime-verified").build()).build()
                else -> ForgeProto.Response.newBuilder().setStatus(ForgeProto.Status.OK).build()
            }
        }
    }

    private class FakeBroker : CredentialBroker {
        var revoked = false
        override fun startDeviceFlow(provider: String) = DeviceFlowStart("dc", "UC", "https://example/verify", 900, 5)
        override fun pollAndStore(provider: String, deviceCode: String) = DeviceFlowPollResult("authorized")
        override fun storePat(provider: String, pat: String): String = "cred_test"
        override fun revoke(provider: String) { revoked = true }
        override fun credentialForRemote(remoteUrl: String, operation: String, provider: String) = CredentialHandle("cred_1", System.currentTimeMillis() + 60_000)
        override fun resolveHandle(handle: String): String? = "secret"
    }

    @Before fun setUp() { Dispatchers.setMain(dispatcher) }
    @After fun tearDown() { Dispatchers.resetMain() }

    @Test fun refreshLoadsStatusBranchesLog() = runTest(dispatcher) {
        val core = FakeCore()
        val vm = GitViewModel(WorkspaceRepository(core), null, dispatcher)
        vm.open("p1")
        advanceUntilIdle()
        val st = vm.state.value
        assertEquals("main", st.status?.branch)
        assertEquals(1, st.status?.staged?.size)
        assertEquals("main", st.branches.first().name)
    }

    @Test fun pushUsesBrokerHandleAndApprovalToken() = runTest(dispatcher) {
        val core = FakeCore()
        val broker = FakeBroker()
        val vm = GitViewModel(WorkspaceRepository(core), broker, dispatcher)
        vm.open("p1"); advanceUntilIdle()
        vm.pushRemote(remote = "origin", branch = "main", force = false)
        advanceUntilIdle()
        assertTrue(core.requests.any { it.hasEvaluateCommand() })
        assertTrue(core.requests.any { it.hasIssueApprovalToken() })
        val push = core.requests.first { it.hasGitRemotePush() }.gitRemotePush
        assertEquals("cred_1", push.credentialHandle)
        assertEquals("tok", push.approvalToken)
        assertTrue(vm.state.value.message?.contains("not runtime-verified") == true)
    }
}
