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
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class WorkspaceViewModelTest {
    private val dispatcher = StandardTestDispatcher()

    @Before fun setUp() { Dispatchers.setMain(dispatcher) }
    @After fun tearDown() { Dispatchers.resetMain() }

    @Test
    fun projectListLoadsAndSurfacesRustError() = runTest(dispatcher) {
        val fake = FakeCore { req ->
            when {
                req.hasListProjects() -> ok(req.id) { setListProjects(
                    ForgeProto.ListProjectsResponse.newBuilder().addProjects(
                        ForgeProto.Project.newBuilder().setId("P").setName("P").setPath("/p").setCreatedAt(1).setStorageBytes(42)
                    )
                ) }
                req.hasCreateProject() -> err(req.id, "INVALID_ARGUMENT", "bad name")
                else -> err(req.id, "UNKNOWN", "unexpected")
            }
        }
        val vm = ProjectListViewModel(WorkspaceRepository(fake), dispatcher)
        vm.refresh()
        advanceUntilIdle()
        assertEquals(1, vm.state.value.projects.size)
        assertEquals("P", vm.state.value.projects[0].name)

        var result: Result<Unit>? = null
        vm.createProject("bad") { result = it }
        advanceUntilIdle()
        assertTrue(result?.isFailure == true)
        assertTrue(vm.state.value.actionError?.contains("INVALID_ARGUMENT") == true)
    }

    @Test
    fun explorerRefreshRenameDeleteAndPreview() = runTest(dispatcher) {
        val fake = FakeCore { req ->
            when {
                req.hasListFiles() -> ok(req.id) { setListFiles(
                    ForgeProto.ListFilesResponse.newBuilder().addEntries(
                        ForgeProto.FileEntry.newBuilder().setName("main.py").setPath("main.py").setIsDir(false).setSize(5)
                    ).addEntries(
                        ForgeProto.FileEntry.newBuilder().setName("src").setPath("src").setIsDir(true)
                    )
                ) }
                req.hasReadFile() -> ok(req.id) { setReadFile(ForgeProto.ReadFileResponse.newBuilder().setContent(com.google.protobuf.ByteString.copyFromUtf8("print(1)"))) }
                req.hasRenameFile() -> ok(req.id) { setRenameFile(ForgeProto.RenameFileResponse.newBuilder().setOldPath("a").setNewPath("b")) }
                req.hasDeletePath() -> err(req.id, "INVALID_ARGUMENT", "directory is not empty; recursive delete required")
                else -> err(req.id, "UNKNOWN", "unexpected")
            }
        }
        val vm = ProjectExplorerViewModel(WorkspaceRepository(fake), dispatcher)
        vm.openProject("P")
        advanceUntilIdle()
        assertEquals(2, vm.state.value.entries.size)

        vm.selectFile("main.py")
        advanceUntilIdle()
        assertEquals("print(1)", vm.state.value.previewText)

        var renameResult: Result<Unit>? = null
        vm.rename("main.py", "app.py") { renameResult = it }
        advanceUntilIdle()
        assertTrue(renameResult?.isSuccess == true)

        var deleteResult: Result<Unit>? = null
        vm.deletePath("src", false) { deleteResult = it }
        advanceUntilIdle()
        assertTrue(deleteResult?.isFailure == true)
        assertTrue(vm.state.value.actionError?.contains("recursive delete required") == true)
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
