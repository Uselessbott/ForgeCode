package dev.forgebox.app.ui

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
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class EditorViewModelTest {
    private val dispatcher = StandardTestDispatcher()

    @Before fun setUp() { Dispatchers.setMain(dispatcher) }
    @After fun tearDown() { Dispatchers.resetMain() }

    @Test
    fun loadSaveConflictAndLargeFileGuard() = runTest(dispatcher) {
        var stored = "hello".toByteArray()
        var mtime = 100L
        val fake = FakeCore { req ->
            when {
                req.hasGetInfo() -> ok(req.id) { setGetInfo(ForgeProto.GetInfoResponse.newBuilder().setMaxEditableFileBytes(1024)) }
                req.hasStatFile() -> ok(req.id) {
                    setStatFile(ForgeProto.StatFileResponse.newBuilder()
                        .setPath(req.statFile.path).setSize(stored.size.toLong()).setMtimeMillis(mtime).setIsTextHeuristic(true).setExists(true))
                }
                req.hasReadFile() -> ok(req.id) { setReadFile(ForgeProto.ReadFileResponse.newBuilder().setContent(com.google.protobuf.ByteString.copyFrom(stored))) }
                req.hasWriteFileIfUnchanged() -> {
                    val w = req.writeFileIfUnchanged
                    if (w.expectedMtimeMillis != mtime) err(req.id, "INVALID_ARGUMENT", "CONFLICT: file changed on disk")
                    else {
                        stored = w.content.toByteArray(); mtime += 1
                        ok(req.id) { setWriteFileIfUnchanged(ForgeProto.WriteFileIfUnchangedResponse.newBuilder().setPath(w.path).setSize(stored.size.toLong()).setMtimeMillis(mtime)) }
                    }
                }
                else -> err(req.id, "UNKNOWN", "unexpected ${req.kindCase}")
            }
        }
        val vm = EditorViewModel(WorkspaceRepository(fake), dispatcher)
        vm.openFile("P", "a.txt")
        advanceUntilIdle()
        val tab = vm.state.value.tabs.single()
        assertEquals("hello", tab.content)
        assertFalse(tab.dirty)

        vm.onContentChanged(0, "hello!")
        assertTrue(vm.state.value.tabs[0].dirty)

        mtime = 999L // simulate external change
        var result: Result<Unit>? = null
        vm.saveActive { result = it }
        advanceUntilIdle()
        assertTrue(result?.isFailure == true)
        assertTrue(vm.state.value.tabs[0].conflict?.contains("CONFLICT") == true)
    }

    @Test
    fun tabLifecycleAndUnsavedCloseGuard() = runTest(dispatcher) {
        val fake = FakeCore { req ->
            when {
                req.hasGetInfo() -> ok(req.id) { setGetInfo(ForgeProto.GetInfoResponse.newBuilder().setMaxEditableFileBytes(1024 * 1024)) }
                req.hasStatFile() -> ok(req.id) { setStatFile(ForgeProto.StatFileResponse.newBuilder().setPath(req.statFile.path).setSize(1).setMtimeMillis(1).setIsTextHeuristic(true).setExists(true)) }
                req.hasReadFile() -> ok(req.id) { setReadFile(ForgeProto.ReadFileResponse.newBuilder().setContent(com.google.protobuf.ByteString.copyFromUtf8("x"))) }
                else -> err(req.id, "UNKNOWN", "unexpected")
            }
        }
        val vm = EditorViewModel(WorkspaceRepository(fake), dispatcher)
        vm.openFile("P", "a.txt")
        vm.openFile("P", "b.txt")
        advanceUntilIdle()
        assertEquals(2, vm.state.value.tabs.size)
        assertEquals(1, vm.state.value.activeIndex)

        vm.selectTab(0)
        assertEquals(0, vm.state.value.activeIndex)
        vm.onContentChanged(0, "changed")
        assertFalse(vm.closeTab(0, force = false))
        assertEquals(2, vm.state.value.tabs.size)
        assertTrue(vm.closeTab(0, force = true))
        assertEquals(1, vm.state.value.tabs.size)
    }

    @Test
    fun largeFileGuard() = runTest(dispatcher) {
        val fake = FakeCore { req ->
            when {
                req.hasGetInfo() -> ok(req.id) { setGetInfo(ForgeProto.GetInfoResponse.newBuilder().setMaxEditableFileBytes(10)) }
                req.hasStatFile() -> ok(req.id) { setStatFile(ForgeProto.StatFileResponse.newBuilder().setPath(req.statFile.path).setSize(100).setMtimeMillis(1).setIsTextHeuristic(true).setExists(true)) }
                else -> err(req.id, "UNKNOWN", "unexpected")
            }
        }
        val vm = EditorViewModel(WorkspaceRepository(fake), dispatcher)
        vm.openFile("P", "big.txt")
        advanceUntilIdle()
        val tab = vm.state.value.tabs.single()
        assertTrue(tab.tooLarge)
        assertTrue(tab.error?.contains("too large") == true)
    }

    @Test
    fun toolbarMapping() {
        assertEquals(EditorToolbarAction.Tab, EditorToolbarKeyMapper.map("TAB"))
        assertEquals(EditorToolbarAction.Left, EditorToolbarKeyMapper.map("←"))
        assertEquals(EditorToolbarAction.Insert("{"), EditorToolbarKeyMapper.map("{"))
        assertTrue(EditorToolbarKeyMapper.keys.contains("CTRL"))
    }

    private class FakeCore(val handler: (ForgeProto.Request) -> ForgeProto.Response) : ForgeBoxCore {
        override fun call(request: ForgeProto.Request): ForgeProto.Response = handler(request)
    }

    private fun ok(id: Int, block: ForgeProto.Response.Builder.() -> Unit): ForgeProto.Response =
        ForgeProto.Response.newBuilder().setId(id).setStatus(ForgeProto.Status.OK).apply(block).build()

    private fun err(id: Int, code: String, message: String): ForgeProto.Response =
        ForgeProto.Response.newBuilder().setId(id).setStatus(ForgeProto.Status.ERROR)
            .setError(ForgeProto.Error.newBuilder().setCode(code).setMessage(message)).build()
}
