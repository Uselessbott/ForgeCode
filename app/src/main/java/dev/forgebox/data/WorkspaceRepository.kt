package dev.forgebox.data

import dev.forgebox.core.ForgeBoxCore
import dev.forgebox.proto.ForgeProto
import java.util.concurrent.atomic.AtomicInteger

class WorkspaceRepository(
    private val core: ForgeBoxCore,
) {
    private val ids = AtomicInteger(1)

    fun listProjects(): Result<List<ProjectSummary>> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder()
                .setListProjects(ForgeProto.ListProjectsRequest.getDefaultInstance()),
        )
        response.listProjects.projectsList.map { it.toSummary() }
    }

    fun createProject(name: String): Result<ProjectSummary> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setCreateProject(
                ForgeProto.CreateProjectRequest.newBuilder().setName(name),
            ),
        )
        response.createProject.project.toSummary()
    }

    fun renameProject(projectId: String, newName: String): Result<ProjectSummary> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setRenameProject(
                ForgeProto.RenameProjectRequest.newBuilder()
                    .setProjectId(projectId)
                    .setNewName(newName),
            ),
        )
        response.renameProject.project.toSummary()
    }

    fun deleteProject(projectId: String): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setDeleteProject(
                ForgeProto.DeleteProjectRequest.newBuilder().setProjectId(projectId),
            ),
        )
    }

    fun listFiles(projectId: String, recursive: Boolean = true): Result<List<FileEntryModel>> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setListFiles(
                ForgeProto.ListFilesRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath("")
                    .setRecursive(recursive),
            ),
        )
        response.listFiles.entriesList.map {
            FileEntryModel(name = it.name, path = it.path, isDir = it.isDir, size = it.size)
        }
    }

    fun readFile(projectId: String, path: String): Result<ByteArray> = runCatching {
        val response = call(
            ForgeProto.Request.newBuilder().setReadFile(
                ForgeProto.ReadFileRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(path),
            ),
        )
        response.readFile.content.toByteArray()
    }

    fun createFile(projectId: String, relPath: String, content: ByteArray = ByteArray(0)): Result<Unit> =
        runCatching {
            call(
                ForgeProto.Request.newBuilder().setWriteFile(
                    ForgeProto.WriteFileRequest.newBuilder()
                        .setProjectId(projectId)
                        .setPath(relPath)
                        .setContent(com.google.protobuf.ByteString.copyFrom(content)),
                ),
            )
        }

    fun createFolder(projectId: String, relPath: String): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setCreateFolder(
                ForgeProto.CreateFolderRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(relPath),
            ),
        )
    }

    fun rename(projectId: String, oldRel: String, newRel: String): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setRenameFile(
                ForgeProto.RenameFileRequest.newBuilder()
                    .setProjectId(projectId)
                    .setOldPath(oldRel)
                    .setNewPath(newRel),
            ),
        )
    }

    fun deletePath(projectId: String, relPath: String, recursive: Boolean): Result<Unit> = runCatching {
        call(
            ForgeProto.Request.newBuilder().setDeletePath(
                ForgeProto.DeletePathRequest.newBuilder()
                    .setProjectId(projectId)
                    .setPath(relPath)
                    .setRecursive(recursive),
            ),
        )
    }

    private fun call(builder: ForgeProto.Request.Builder): ForgeProto.Response {
        val request = builder.setId(ids.getAndIncrement()).build()
        val response = core.call(request)
        if (response.status != ForgeProto.Status.OK) {
            val code = if (response.hasError()) response.error.code else "UNKNOWN"
            val message = if (response.hasError()) response.error.message else "unknown error"
            throw IllegalStateException("$code: $message")
        }
        return response
    }

    private fun ForgeProto.Project.toSummary() = ProjectSummary(
        id = id,
        name = name,
        path = path,
        createdAt = createdAt,
        storageBytes = storageBytes,
    )
}
