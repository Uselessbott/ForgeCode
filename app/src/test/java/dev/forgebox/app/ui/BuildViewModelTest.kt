package dev.forgebox.app.ui

import dev.forgebox.data.BuildRunResultModel
import dev.forgebox.proto.ForgeProto
import com.google.protobuf.ByteString
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
class BuildViewModelTest {
    private val dispatcher = StandardTestDispatcher()

    private class FakeBuildBackend : BuildBackend {
        var nextPty = 100L
        val closed = mutableListOf<Long>()
        val runs = mutableListOf<String>()
        val events = ArrayDeque<ForgeProto.Event>()
        var pythonMissing = false
        override fun buildRun(projectId: String, kind: ForgeProto.BuildKind, filePath: String, command: String, cols: Int, rows: Int): Result<BuildRunResultModel> {
            val cmd = if (kind == ForgeProto.BuildKind.BUILD_KIND_PYTHON) "python3 $filePath" else command
            runs += cmd
            return Result.success(BuildRunResultModel(nextPty++, cmd, "ok", pythonMissing))
        }
        override fun ptyClose(ptyId: Long): Result<Unit> { closed += ptyId; return Result.success(Unit) }
        override fun pollEvents(max: Int, timeoutMs: Int): List<ForgeProto.Event> = if (events.isEmpty()) emptyList() else listOf(events.removeFirst())
    }

    @Before fun setUp() { Dispatchers.setMain(dispatcher) }
    @After fun tearDown() { Dispatchers.resetMain() }

    @Test fun b1_runActionStreamsOutput() = runTest(dispatcher) {
        val backend = FakeBuildBackend()
        val vm = BuildViewModel(backend, dispatcher)
        vm.runPython("p1", "main.py")
        advanceUntilIdle()
        val pty = vm.state.value.runningPtyId!!
        vm.onOutputForTest(pty, "hello\n")
        vm.onExitForTest(pty, 0)
        assertTrue(vm.state.value.output.contains("hello"))
        assertEquals(0, vm.state.value.history.first().exitCode)
    }

    @Test fun b2_cancelCallsClose() = runTest(dispatcher) {
        val backend = FakeBuildBackend()
        val vm = BuildViewModel(backend, dispatcher)
        vm.runGeneric("p1", "echo hi")
        advanceUntilIdle()
        val pty = vm.state.value.runningPtyId!!
        vm.cancel(); advanceUntilIdle()
        assertEquals(listOf(pty), backend.closed)
    }

    @Test fun b3_diagnosticsParseAndLink() {
        val diags = dev.forgebox.data.BuildDiagnosticsParser.parse("File \"main.py\", line 3\nmain.py:3:1: NameError: x")
        assertTrue(diags.any { it.filePath.endsWith("main.py") && it.line == 3 })
    }

    @Test fun b4_buildHistoryRenders() = runTest(dispatcher) {
        val backend = FakeBuildBackend()
        val vm = BuildViewModel(backend, dispatcher)
        vm.runGeneric("p1", "echo hi"); advanceUntilIdle()
        assertEquals(1, vm.state.value.history.size)
        assertEquals("echo hi", vm.state.value.history.first().command)
    }

    @Test fun b6_pythonMissingOffersInstall() = runTest(dispatcher) {
        val backend = FakeBuildBackend().apply { pythonMissing = true }
        val vm = BuildViewModel(backend, dispatcher)
        vm.runPython("p1", "main.py"); advanceUntilIdle()
        assertEquals("main.py", vm.state.value.pythonMissingForFile)
    }
}
